//! A native `IXAsync`: the object games get back for `CLSID_XASYNC`.
//!
//! `XAsync` is the GDK's asynchronous-call protocol. Every async API is a *provider* - a
//! single callback that the runtime invokes with an [`XAsyncOp`] to drive one call
//! through its lifetime:
//!
//! ```text
//! Begin -> (DoWork)* -> XAsyncComplete -> completion callback -> GetResult -> Cleanup
//! ```
//!
//! The runtime's job is the bookkeeping around that: parking the per-call state, running
//! `DoWork` on the queue's work port, delivering the caller's completion callback on its
//! completion port, and handing back the result exactly once. `DoWork` returning
//! `E_PENDING` means "not finished, I will schedule myself again", which is how a provider
//! that cannot answer in one pass keeps its place in the queue.
//!
//! ## Where the state lives
//!
//! Games allocate `XAsyncBlock` themselves, so the runtime gets exactly the 32 opaque
//! bytes of `XAsyncBlock::internal` to work with. Those bytes are documented as private
//! to the implementation, so we define the layout: a signature word, then a pointer to
//! the heap-allocated [`AsyncState`]. The signature is what lets a stale or
//! never-started block be rejected instead of dereferenced.

use super::core as async_core;
use super::task_queue::{
    self, DispatchMode, MonitorCallback, PortHandle, PortKind, Queue, QueueHandle, TaskCallback,
    TerminatedCallback,
};
use super::{IXAsync, IXAsync_Impl, XAsyncBlock, XAsyncOp, XAsyncProviderData};
use crate::results::*;
use async_core::{
    AsyncState, Inner, block_name, cleanup, complete_state, current_identity, is_lhc_internal,
    name_block, queue_for, state_of, work_callback, write_internal,
};

use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::diag::diag;
use windows_core::{HRESULT, implement};
use windows_sys::core::BOOL;

type XAsyncProvider =
    unsafe extern "system" fn(op: XAsyncOp, data: *const XAsyncProviderData) -> HRESULT;
/// `HRESULT CALLBACK (*)(XAsyncBlock*)`, the simplified provider `XAsyncRun` takes.
type XAsyncWork = unsafe extern "system" fn(async_block: *mut XAsyncBlock) -> HRESULT;

#[implement(IXAsync)]
pub struct XAsyncObject;

impl IXAsync_Impl for XAsyncObject_Impl {
    unsafe fn XAsyncBegin(
        &self,
        async_block: *mut c_void,
        context: *mut c_void,
        identity: *mut c_void,
        identity_name: *mut c_char,
        provider: *mut c_void,
    ) -> HRESULT {
        let block = async_block as *mut XAsyncBlock;
        if !identity_name.is_null() {
            // SAFETY: GDK-caller-supplied `identityName` is contractually a NUL-terminated
            // C string, checked non-null just above.
            let name = unsafe { std::ffi::CStr::from_ptr(identity_name) }.to_string_lossy();
            let name = async_core::intern(&name);
            name_block(block, name);
            diag!("XAsyncBegin name={name:?} block={block:p}");
            if is_lhc_internal(name) {
                diag!(
                    "XAsyncBegin PARENT parent={:?} -> internal {name} block={block:p}",
                    current_identity()
                );
            }
        }
        // SAFETY: `block` is the caller-supplied `asyncBlock` cast to `*mut XAsyncBlock`,
        // and is checked non-null via the `Some` match here before any dereference.
        let Some(block_ref) = (unsafe { block.as_mut() }) else {
            return E_POINTER;
        };
        if provider.is_null() {
            return E_POINTER;
        }
        // SAFETY: `state_of` requires `block` be null or a valid `XAsyncBlock`; it was just
        // dereferenced above via `block.as_mut()`, so it is valid.
        if unsafe { state_of(block) }.is_some() {
            diag!(
                "XAsyncBegin REJECT block={block:p} queue={:#x} reason=block-in-use",
                block_ref.queue as u64
            );
            // The block is still driving another call; reusing it would strand that one.
            return E_INVALIDARG;
        }
        let mut queue = match queue_for(block_ref) {
            Some(queue) => queue,
            None => {
                diag!(
                    "XAsyncBegin REJECT block={block:p} queue={:#x} reason=unknown-queue",
                    block_ref.queue as u64
                );
                return E_INVALIDARG;
            }
        };
        diag!(
            "XAsyncBegin queue-mode block={block:p} queue={:#x} work={:?} completion={:?} queue_ptr={:p}",
            block_ref.queue as u64,
            queue.port(PortKind::Work).mode(),
            queue.port(PortKind::Completion).mode(),
            Arc::as_ptr(&queue)
        );
        let original_queue = block_ref.queue;
        if block_ref.queue.is_null() {
            // libHttpClient-style providers read asyncBlock->queue directly to decide on
            // a dispatch queue, and a NULL queue makes Begin fail E_INVALIDARG. Give the
            // block a real process-queue handle up front so those providers work - the
            // same effect the wrapper's temporary FIXQ injection had, but natively.
            let handle = task_queue::process_queue_handle();
            block_ref.queue = handle as *mut c_void;
            diag!("XAsyncBegin BACKFILL block={block:p} queue=0 -> handle={handle:#x}");
        } else if queue.port(PortKind::Work).mode() == DispatchMode::Manual
            && !task_queue::is_process_queue(&queue)
        {
            // The game only ever dispatches the process task queue (its render/UI thread
            // pumps exactly one queue). Any other queue with a Manual work port is never
            // pumped under Wine, so a block parked there would never run its DoWork
            // (e.g. libHttpClient's SocialManager decoration HTTP call hangs, leaving the
            // friends list empty). Route it through the pumped process queue instead,
            // exactly like the NULL-queue BACKFILL above.
            let handle = task_queue::process_queue_handle();
            block_ref.queue = handle as *mut c_void;
            diag!(
                "XAsyncBegin MANUAL->PROCESS block={block:p} old_queue={:#x} -> handle={handle:#x}",
                original_queue as u64
            );
            let Some(process_queue) = QueueHandle::get(handle) else {
                return E_ILLEGAL_METHOD_CALL;
            };
            queue = process_queue;
        }

        // SAFETY: GDK guarantees `provider` matches `XAsyncProvider` when calling
        // `XAsyncBegin`.
        let provider: XAsyncProvider = unsafe { crate::ffi_util::fn_ptr_cast(provider) };
        let state = Arc::new(AsyncState {
            provider,
            context,
            identity,
            queue,
            began_ms: crate::diag::now_ms(),
            block,
            inner: Mutex::new(Inner {
                status: E_PENDING,
                result_size: 0,
                canceled: false,
                cleaned_up: false,
            }),
            completed: Condvar::new(),
        });
        write_internal(block_ref, Arc::into_raw(state.clone()));

        let hr = state.invoke(XAsyncOp::Begin, std::ptr::null_mut(), 0);
        if hr != S_OK {
            diag!(
                "XAsyncBegin BEGIN-FAILED block={block:p} queue={:#x} hr={hr:?}",
                block_ref.queue as u64
            );
            // Begin failing means the call never started, so there is no completion to
            // deliver - tear down synchronously and report the failure to the caller.
            let mut inner = state.inner.lock().expect("async state poisoned");
            inner.status = hr;
            drop(inner);
            cleanup(&state);
            return hr;
        }
        S_OK
    }

    unsafe fn XAsyncSchedule(&self, async_block: *mut c_void, delay_in_ms: u32) -> HRESULT {
        // SAFETY: `state_of` requires `async_block` be null or a valid `XAsyncBlock`; GDK
        // guarantees the caller passes back the block it obtained from `XAsyncBegin`.
        let Some(state) = (unsafe { state_of(async_block as *mut XAsyncBlock) }) else {
            return E_ILLEGAL_METHOD_CALL;
        };
        let name = block_name(async_block as *mut XAsyncBlock);
        let context = Arc::into_raw(state.clone()) as *mut c_void;
        if state.queue.submit(
            PortKind::Work,
            context,
            work_callback,
            Duration::from_millis(delay_in_ms as u64),
        ) {
            diag!(
                "XAsyncSchedule submitted name={name:?} block={async_block:p} work_port={:p} delay={delay_in_ms}",
                Arc::as_ptr(state.queue.port(PortKind::Work))
            );
            S_OK
        } else {
            // Reclaim the reference the failed submit did not take, then report the
            // termination the way a cancelled call reports it.
            diag!(
                "XAsyncSchedule submit FAILED (work port terminated) block={:p} queue={:p} work_port={:p} -> E_ABORT",
                async_block,
                Arc::as_ptr(&state.queue),
                Arc::as_ptr(state.queue.port(PortKind::Work))
            );
            // SAFETY: `context` is the `Arc::into_raw` pointer just above; the failed
            // `submit` means the queue never took ownership, so this is its only reclaim.
            drop(unsafe { Arc::from_raw(context as *const AsyncState) });
            complete_state(&state, E_ABORT, 0);
            E_ABORT
        }
    }

    unsafe fn XAsyncComplete(
        &self,
        async_block: *mut c_void,
        result: i32,
        required_buffer_size: u64,
    ) {
        // SAFETY: `state_of` requires `async_block` be null or a valid `XAsyncBlock`, which
        // GDK guarantees for a block obtained from `XAsyncBegin`.
        let Some(state) = (unsafe { state_of(async_block as *mut XAsyncBlock) }) else {
            return;
        };
        complete_state(&state, HRESULT(result), required_buffer_size as usize);
    }

    unsafe fn XAsyncGetStatus(&self, async_block: *mut c_void, wait: BOOL) -> HRESULT {
        // SAFETY: `state_of` requires `async_block` be null or a valid `XAsyncBlock`, which
        // GDK guarantees for a block obtained from `XAsyncBegin`.
        let Some(state) = (unsafe { state_of(async_block as *mut XAsyncBlock) }) else {
            return E_ILLEGAL_METHOD_CALL;
        };

        let (status, result_size) = {
            let mut inner = state.inner.lock().expect("async state poisoned");
            if wait != 0 && inner.status == E_PENDING {
                if task_queue::in_dispatch() {
                    // We are inside a dispatch callback running on the pump thread. Blocking
                    // here would deadlock: the async's completing DoWork is queued on a queue
                    // only this same thread dispatches, and we cannot pump it while blocked.
                    // Return E_PENDING so the caller polls and control returns to the pump.
                    diag!(
                        "XAsyncGetStatus wait=true -> E_PENDING (in dispatch callback; avoiding pump deadlock) block={:p}",
                        async_block
                    );
                } else {
                    diag!(
                        "XAsyncGetStatus wait=true blocking, block={:p}",
                        async_block
                    );
                    while inner.status == E_PENDING {
                        inner = state
                            .completed
                            .wait(inner)
                            .expect("async state poisoned while waiting");
                    }
                }
            }
            (inner.status, inner.result_size)
        };

        // A call with no result payload is finished the moment its status is known -
        // there is no XAsyncGetResult coming to release the state, so this is it. Calls
        // that do produce a result are cleaned up by XAsyncGetResult instead.
        if status != E_PENDING && result_size == 0 {
            cleanup(&state);
        }
        status
    }

    unsafe fn XAsyncGetResultSize(
        &self,
        async_block: *mut c_void,
        buffer_size: *mut usize,
    ) -> HRESULT {
        // SAFETY: `buffer_size` is the caller-supplied out-pointer for `XAsyncGetResultSize`,
        // checked non-null via the `Some` match here before any dereference.
        let Some(size_out) = (unsafe { buffer_size.as_mut() }) else {
            return E_POINTER;
        };
        // SAFETY: `state_of` requires `async_block` be null or a valid `XAsyncBlock`, which
        // GDK guarantees for a block obtained from `XAsyncBegin`.
        let Some(state) = (unsafe { state_of(async_block as *mut XAsyncBlock) }) else {
            return E_ILLEGAL_METHOD_CALL;
        };
        let inner = state.inner.lock().expect("async state poisoned");
        if inner.status == E_PENDING {
            return E_PENDING;
        }
        diag!(
            "XAsyncGetResultSize block={async_block:p} status={:?} result_size={}",
            inner.status,
            inner.result_size
        );
        *size_out = inner.result_size;
        inner.status
    }

    unsafe fn XAsyncGetResult(
        &self,
        async_block: *mut c_void,
        identity: *mut c_void,
        buffer_size: u64,
        buffer: *mut c_void,
        buffer_used: *mut usize,
    ) -> HRESULT {
        // SAFETY: `state_of` requires `async_block` be null or a valid `XAsyncBlock`, which
        // GDK guarantees for a block obtained from `XAsyncBegin`.
        let Some(state) = (unsafe { state_of(async_block as *mut XAsyncBlock) }) else {
            return E_ILLEGAL_METHOD_CALL;
        };
        diag!(
            "XAsyncGetResult enter block={async_block:p} identity={:p} state_identity={:p} buffer={:p} buffer_size={buffer_size}",
            identity,
            state.identity,
            buffer
        );

        let (status, result_size) = {
            let inner = state.inner.lock().expect("async state poisoned");
            (inner.status, inner.result_size)
        };
        if status == E_PENDING {
            diag!("XAsyncGetResult block={async_block:p} -> E_PENDING (async not yet complete)");
            return E_PENDING;
        }
        if status != S_OK {
            diag!(
                "XAsyncGetResult block={async_block:p} status={status:?} result_size={result_size}"
            );
            cleanup(&state);
            return status;
        }
        // A non-null identity is the caller asserting which API it thinks this block
        // belongs to. Mismatches are a caller bug worth surfacing, not something to
        // paper over by returning another API's result.
        if !identity.is_null() && identity != state.identity {
            diag!(
                "XAsyncGetResult block={async_block:p} IDENTITY MISMATCH identity={identity:p} != state_identity={:p} -> E_INVALIDARG",
                state.identity
            );
            return E_INVALIDARG;
        }
        if (buffer_size as usize) < result_size {
            diag!(
                "XAsyncGetResult block={async_block:p} buffer_size={buffer_size} < result_size={result_size} -> E_NOT_SUFFICIENT_BUFFER"
            );
            return E_NOT_SUFFICIENT_BUFFER;
        }

        let hr = state.invoke(XAsyncOp::GetResult, buffer, buffer_size as usize);
        // SAFETY: `buffer_used` is the caller-supplied out-pointer for `XAsyncGetResult`,
        // and `as_mut` handles a null one by yielding `None`.
        if let Some(used) = unsafe { buffer_used.as_mut() } {
            *used = result_size;
        }
        diag!(
            "XAsyncGetResult block={async_block:p} invoked GetResult hr={hr:?} buffer_used={result_size}"
        );
        cleanup(&state);
        hr
    }

    unsafe fn XAsyncCancel(&self, async_block: *mut c_void) {
        diag!(
            "XAsyncCancel called for block={:p} on {:?}",
            async_block,
            std::thread::current().id()
        );
        // SAFETY: `state_of` requires `async_block` be null or a valid `XAsyncBlock`, which
        // GDK guarantees for a block obtained from `XAsyncBegin`.
        let Some(state) = (unsafe { state_of(async_block as *mut XAsyncBlock) }) else {
            return;
        };
        {
            let mut inner = state.inner.lock().expect("async state poisoned");
            if inner.status != E_PENDING || inner.canceled {
                return;
            }
            inner.canceled = true;
        }
        let _ = state.invoke(XAsyncOp::Cancel, std::ptr::null_mut(), 0);

        // Deliberately not completing the call here. The provider may have work in
        // flight that still refers to its context, and completing would let
        // XAsyncGetResult free that context underneath it. Instead give the provider a
        // pass in which to observe the cancellation and complete itself, which is what
        // every provider in this crate does.
        let still_pending = state.inner.lock().expect("async state poisoned").status == E_PENDING;
        if still_pending {
            let context = Arc::into_raw(state.clone()) as *mut c_void;
            if !state
                .queue
                .submit(PortKind::Work, context, work_callback, Duration::ZERO)
            {
                // SAFETY: `context` is the `Arc::into_raw` pointer just above; the failed
                // `submit` means the queue never took ownership, so this is its only reclaim.
                drop(unsafe { Arc::from_raw(context as *const AsyncState) });
                complete_state(&state, E_ABORT, 0);
            }
        }
    }

    unsafe fn XAsyncRun(&self, async_block: *mut c_void, work: *mut c_void) -> HRESULT {
        if work.is_null() {
            return E_POINTER;
        }
        // SAFETY: `XAsyncRun`'s own contract forwards to `XAsyncBegin` with `run_provider`,
        // whose signature matches the required `XAsyncProvider` type.
        unsafe {
            self.XAsyncBegin(
                async_block,
                work,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                run_provider as *mut c_void,
            )
        }
    }

    unsafe fn XTaskQueueCreate(
        &self,
        work_dispatch_mode: u64,
        completion_dispatch_mode: u64,
        queue: *mut u64,
    ) -> HRESULT {
        // SAFETY: `queue` is the caller-supplied out-pointer for `XTaskQueueCreate`, checked
        // non-null via the `Some` match here before any dereference.
        let Some(out) = (unsafe { queue.as_mut() }) else {
            return E_POINTER;
        };
        let (Some(work), Some(completion)) = (
            DispatchMode::from_raw(work_dispatch_mode),
            DispatchMode::from_raw(completion_dispatch_mode),
        ) else {
            return E_INVALIDARG;
        };
        let q = Queue::new(work, completion);
        *out = QueueHandle::create(q.clone());
        diag!(
            "XTaskQueueCreate mode=({work:?},{completion:?}) work_port={:#x} completion_port={:#x} -> handle={:#x}",
            Arc::as_ptr(q.port(PortKind::Work)) as usize,
            Arc::as_ptr(q.port(PortKind::Completion)) as usize,
            *out
        );
        S_OK
    }

    unsafe fn XTaskQueueCreateComposite(
        &self,
        work_port: u64,
        completion_port: u64,
        queue: *mut u64,
    ) -> HRESULT {
        // SAFETY: `queue` is the caller-supplied out-pointer for `XTaskQueueCreateComposite`,
        // checked non-null via the `Some` match here before any dereference.
        let Some(out) = (unsafe { queue.as_mut() }) else {
            return E_POINTER;
        };
        let (Some(work), Some(completion)) =
            (PortHandle::get(work_port), PortHandle::get(completion_port))
        else {
            return E_INVALIDARG;
        };
        *out = QueueHandle::create(Queue::composite(work, completion));
        diag!(
            "XTaskQueueCreateComposite work_port={:#x} completion_port={:#x} -> handle={:#x}",
            work_port,
            completion_port,
            *out
        );
        S_OK
    }

    unsafe fn XTaskQueueGetPort(&self, queue: u64, port: u64, port_handle: *mut u64) -> HRESULT {
        let queue_orig_for_log = queue;
        // SAFETY: `port_handle` is the caller-supplied out-pointer for `XTaskQueueGetPort`,
        // checked non-null via the `Some` match here before any dereference.
        let Some(out) = (unsafe { port_handle.as_mut() }) else {
            return E_POINTER;
        };
        let (Some(queue), Some(kind)) = (QueueHandle::get(queue), PortKind::from_raw(port)) else {
            return E_INVALIDARG;
        };
        *out = PortHandle::create(queue.port(kind).clone());
        diag!(
            "XTaskQueueGetPort queue={:#x} kind={} -> port_handle={:#x}",
            queue_orig_for_log,
            port,
            *out
        );
        S_OK
    }

    unsafe fn XTaskQueueDuplicateHandle(
        &self,
        queue_handle: u64,
        duplicated_handle: *mut u64,
    ) -> HRESULT {
        // SAFETY: `duplicated_handle` is the caller-supplied out-pointer for
        // `XTaskQueueDuplicateHandle`, checked non-null via the `Some` match before use.
        let Some(out) = (unsafe { duplicated_handle.as_mut() }) else {
            return E_POINTER;
        };
        let Some(queue) = QueueHandle::get(queue_handle) else {
            return E_INVALIDARG;
        };
        *out = QueueHandle::create(queue);
        diag!(
            "XTaskQueueDuplicateHandle in={:#x} -> out={:#x}",
            queue_handle,
            *out
        );
        S_OK
    }

    unsafe fn XTaskQueueCloseHandle(&self, queue: u64) {
        diag!("XTaskQueueCloseHandle handle={:#x}", queue);
        QueueHandle::close(queue);
    }

    unsafe fn XTaskQueueDispatch(&self, queue: u64, port: u64, timeout_in_ms: u32) -> BOOL {
        diag!("XTaskQueueDispatch enter handle={:#x} port={}", queue, port);
        let (Some(queue), Some(kind)) = (QueueHandle::get(queue), PortKind::from_raw(port)) else {
            diag!("XTaskQueueDispatch bad handle/port, bailing");
            return 0;
        };
        let port_arc = queue.port(kind);
        diag!(
            "XTaskQueueDispatch queue_ptr={:p} port_ptr={:p}",
            std::sync::Arc::as_ptr(&queue),
            std::sync::Arc::as_ptr(port_arc)
        );
        port_arc.dispatch(timeout_in_ms).into()
    }

    unsafe fn XTaskQueueSubmitCallback(
        &self,
        queue: u64,
        port: u64,
        callback_context: *mut c_void,
        callback: *mut c_void,
    ) -> HRESULT {
        // SAFETY: forwards this call's own arguments unchanged to
        // `XTaskQueueSubmitDelayedCallback` with `delay_ms: 0`; that fn re-validates them.
        unsafe { self.XTaskQueueSubmitDelayedCallback(queue, port, 0, callback_context, callback) }
    }

    unsafe fn XTaskQueueSubmitDelayedCallback(
        &self,
        queue: u64,
        port: u64,
        delay_ms: u32,
        callback_context: *mut c_void,
        callback: *mut c_void,
    ) -> HRESULT {
        if callback.is_null() {
            return E_POINTER;
        }
        let (Some(queue), Some(kind)) = (QueueHandle::get(queue), PortKind::from_raw(port)) else {
            return E_INVALIDARG;
        };
        // SAFETY: GDK guarantees `callback` matches `TaskCallback` when calling
        // `XTaskQueueSubmitDelayedCallback`.
        let callback: TaskCallback = unsafe { crate::ffi_util::fn_ptr_cast(callback) };
        diag!(
            "XTaskQueueSubmitDelayedCallback queue_ptr={:#x} kind={} work_port={:#x} delay={delay_ms} ctx={:#x}",
            Arc::as_ptr(&queue) as usize,
            port,
            Arc::as_ptr(queue.port(kind)) as usize,
            callback_context as usize
        );
        if queue.submit(
            kind,
            callback_context,
            callback,
            Duration::from_millis(delay_ms as u64),
        ) {
            S_OK
        } else {
            E_ABORT
        }
    }

    unsafe fn XTaskQueueRegisterWaiter(
        &self,
        queue: u64,
        port: u64,
        wait_handle: *mut c_void,
        callback_context: *mut c_void,
        callback: *mut c_void,
        token: *mut u64,
    ) -> HRESULT {
        // SAFETY: `token` is the caller-supplied out-pointer for `XTaskQueueRegisterWaiter`,
        // checked non-null via the `Some` match here before any dereference.
        let Some(out) = (unsafe { token.as_mut() }) else {
            return E_POINTER;
        };
        if callback.is_null() {
            return E_POINTER;
        }
        let (Some(queue), Some(kind)) = (QueueHandle::get(queue), PortKind::from_raw(port)) else {
            return E_INVALIDARG;
        };
        // SAFETY: GDK guarantees `callback` matches `TaskCallback` when calling
        // `XTaskQueueRegisterWaiter`.
        let callback: TaskCallback = unsafe { crate::ffi_util::fn_ptr_cast(callback) };
        *out = waiter::register(queue, kind, wait_handle, callback_context, callback);
        S_OK
    }

    unsafe fn XTaskQueueUnregisterWaiter(&self, _queue: u64, token: u64) {
        // SAFETY: `waiter::unregister`'s own contract requires `token` come from `register`
        // and not have been unregistered already; GDK's `XTaskQueueUnregisterWaiter`
        // contract requires the same of its caller.
        unsafe { waiter::unregister(token) };
    }

    unsafe fn XTaskQueueTerminate(
        &self,
        queue: u64,
        wait: BOOL,
        callback_context: *mut c_void,
        callback: *mut c_void,
    ) -> HRESULT {
        let Some(queue) = QueueHandle::get(queue) else {
            return E_INVALIDARG;
        };
        diag!(
            "XTaskQueueTerminate queue={:#x} wait={wait} has_cb={}",
            Arc::as_ptr(&queue) as usize,
            !callback.is_null()
        );
        // SAFETY: GDK guarantees `callback` matches `TerminatedCallback` when calling
        // `XTaskQueueTerminate`.
        let terminated: Option<TerminatedCallback> =
            (!callback.is_null()).then(|| unsafe { crate::ffi_util::fn_ptr_cast(callback) });

        if wait != 0 {
            queue.terminate(true);
            if let Some(terminated) = terminated {
                // SAFETY: `terminated` was caller-registered via `XTaskQueueTerminate`'s
                // `callback` argument, cast above to `TerminatedCallback`; `callback_context`
                // is the matching caller-supplied context.
                unsafe { terminated(callback_context) };
            }
            return S_OK;
        }

        // Without `wait` the caller must not block, but the termination callback still
        // has to fire after everything has actually drained - so the waiting moves to a
        // thread of our own.
        struct Context(*mut c_void);
        // SAFETY: `callback_context` is an opaque pointer the caller owns and is responsible
        // for keeping valid until the termination callback fires; the GDK contract allows
        // touching it from another thread.
        unsafe impl Send for Context {}
        let context = Context(callback_context);
        std::thread::spawn(move || {
            let context = context;
            queue.terminate(true);
            if let Some(terminated) = terminated {
                // SAFETY: `terminated` was caller-registered via `XTaskQueueTerminate`'s
                // `callback` argument; `context.0` is the matching caller-supplied context.
                unsafe { terminated(context.0) };
            }
        });
        S_OK
    }

    unsafe fn XTaskQueueRegisterMonitor(
        &self,
        queue: u64,
        callback_context: *mut c_void,
        callback: *mut c_void,
        token: *mut u64,
    ) -> HRESULT {
        // SAFETY: `token` is the caller-supplied out-pointer for `XTaskQueueRegisterMonitor`,
        // checked non-null via the `Some` match here before any dereference.
        let Some(out) = (unsafe { token.as_mut() }) else {
            return E_POINTER;
        };
        if callback.is_null() {
            return E_POINTER;
        }
        let Some(queue) = QueueHandle::get(queue) else {
            return E_INVALIDARG;
        };
        // SAFETY: GDK guarantees `callback` matches `MonitorCallback` when calling
        // `XTaskQueueRegisterMonitor`.
        let callback: MonitorCallback = unsafe { crate::ffi_util::fn_ptr_cast(callback) };
        *out = queue.register_monitor(callback_context, callback);
        S_OK
    }

    unsafe fn XTaskQueueUnregisterMonitor(&self, queue: u64, token: u64) {
        if let Some(queue) = QueueHandle::get(queue) {
            queue.unregister_monitor(token);
        }
    }

    unsafe fn XTaskQueueGetCurrentProcessTaskQueue(&self, queue: *mut u64) -> BOOL {
        // SAFETY: `queue` is the caller-supplied out-pointer for
        // `XTaskQueueGetCurrentProcessTaskQueue`, checked non-null via the `Some` match here.
        let Some(out) = (unsafe { queue.as_mut() }) else {
            return 0;
        };
        // Reports whether one exists rather than creating one: the documented use is to
        // find out if the host has installed a queue, and answering "yes" by
        // manufacturing a thread-pool queue would hide that it had not.
        if !task_queue::has_process_queue() {
            *out = 0;
            return 0;
        }
        *out = QueueHandle::create(task_queue::default_process_queue());
        diag!("XTaskQueueGetCurrentProcessTaskQueue -> handle={:#x}", *out);
        1
    }

    unsafe fn XTaskQueueSetCurrentProcessTaskQueue(&self, queue: u64) {
        let resolved = QueueHandle::get(queue);
        // Which *ports* the process queue is made of is the load-bearing detail: the game
        // pumps a handle of its own choosing, and if that handle's ports are not these
        // ports, everything we submit here waits for a dispatch that never comes.
        match &resolved {
            Some(q) => diag!(
                "set_process_queue handle={queue:#x} queue_ptr={:p} work_port={:p} ({:?}) completion_port={:p} ({:?})",
                Arc::as_ptr(q),
                Arc::as_ptr(q.port(PortKind::Work)),
                q.port(PortKind::Work).mode(),
                Arc::as_ptr(q.port(PortKind::Completion)),
                q.port(PortKind::Completion).mode(),
            ),
            None => diag!("set_process_queue handle={queue:#x} -> UNRESOLVED"),
        }
        task_queue::set_process_queue(resolved);
    }

    unsafe fn XThreadSetTimeSensitive(&self, is_time_sensitive_thread: BOOL) -> HRESULT {
        TIME_SENSITIVE.with(|flag| flag.store(is_time_sensitive_thread != 0, Ordering::Relaxed));
        S_OK
    }

    unsafe fn XThreadIsTimeSensitive(&self) -> BOOL {
        TIME_SENSITIVE
            .with(|flag| flag.load(Ordering::Relaxed))
            .into()
    }

    unsafe fn XThreadAssertNotTimeSensitive(&self) {
        // A debug aid on real hardware, where it trips a deadline check. There is no
        // deadline to miss here, so recording the violation is all it can usefully do.
        if TIME_SENSITIVE.with(|flag| flag.load(Ordering::Relaxed)) {
            eprintln!("xgameruntime: blocking call on a thread marked time-sensitive");
        }
    }

    unsafe fn __ReservedSlot8(&self) -> HRESULT {
        E_NOTIMPL
    }

    unsafe fn __ReservedSlot28(&self) -> HRESULT {
        E_NOTIMPL
    }
}

const E_NOTIMPL: HRESULT = HRESULT(0x80004001u32 as i32);

thread_local! {
    static TIME_SENSITIVE: AtomicBool = const { AtomicBool::new(false) };
}

/// The provider behind `XAsyncRun`: run the caller's function once, on the work port.
unsafe extern "system" fn run_provider(op: XAsyncOp, data: *const XAsyncProviderData) -> HRESULT {
    // SAFETY: XAsync always passes a non-null `data` to provider callbacks; checked here
    // defensively before dereferencing.
    let Some(data) = (unsafe { data.as_ref() }) else {
        return E_POINTER;
    };
    // SAFETY: `data.context` was set to an `XAsyncWork` by `XAsyncRun`'s caller and GDK
    // hands it back unchanged.
    let work: XAsyncWork = unsafe { crate::ffi_util::fn_ptr_cast(data.context) };
    match op {
        XAsyncOp::Begin => {
            // SAFETY: `state_of` requires `data.async_` be null or a valid `XAsyncBlock`;
            // XAsync always supplies the block this provider is driving.
            let Some(state) = (unsafe { state_of(data.async_) }) else {
                return E_ILLEGAL_METHOD_CALL;
            };
            let context = Arc::into_raw(state.clone()) as *mut c_void;
            if state
                .queue
                .submit(PortKind::Work, context, work_callback, Duration::ZERO)
            {
                S_OK
            } else {
                // SAFETY: `context` is the `Arc::into_raw` pointer just above; the failed
                // `submit` means the queue never took ownership, so this is its only reclaim.
                drop(unsafe { Arc::from_raw(context as *const AsyncState) });
                E_ABORT
            }
        }
        // SAFETY: `work` was set to an `XAsyncWork` by `XAsyncRun`'s caller (cast above from
        // `data.context`); `data.async_` is the block XAsync supplied to this callback.
        XAsyncOp::DoWork => unsafe { work(data.async_) },
        XAsyncOp::GetResult | XAsyncOp::Cancel | XAsyncOp::Cleanup => S_OK,
    }
}

mod waiter {
    use super::*;
    use windows::handleapi::CloseHandle;
    use windows::synchapi::{CreateEventW, SetEvent, WaitForMultipleObjects};
    use windows::winnt::HANDLE;
    use windows_core::PCWSTR;

    struct Waiter {
        /// Signalled by `unregister` so the waiting thread can stop.
        cancel: HANDLE,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    // SAFETY: `cancel` is a HANDLE owned solely by this `Waiter` and only ever signalled or
    // closed from `unregister`, and `thread`'s `JoinHandle` is already `Send`.
    unsafe impl Send for Waiter {}

    /// Returns the token the caller passes back to [`unregister`].
    pub fn register(
        queue: Arc<Queue>,
        kind: PortKind,
        wait_handle: *mut c_void,
        callback_context: *mut c_void,
        callback: TaskCallback,
    ) -> u64 {
        // Manual reset: once cancelled, stay cancelled.
        // SAFETY: all arguments are valid: `None` security attributes, plain bools for
        // manual-reset/initial-state, and a null name.
        let cancel = unsafe { CreateEventW(None, true, false, PCWSTR::null()) };

        struct Handles {
            wait: HANDLE,
            cancel: HANDLE,
            context: *mut c_void,
        }
        // SAFETY: `wait` is the caller's handle and `context` an opaque caller-owned
        // pointer - the GDK `XTaskQueueRegisterWaiter` contract requires both stay valid for
        // this waiter's lifetime, which is safe to touch from the spawned wait thread.
        unsafe impl Send for Handles {}
        let handles = Handles {
            wait: HANDLE(wait_handle),
            cancel,
            context: callback_context,
        };

        // A thread per waiter rather than a shared wait pool: waiters are rare (they
        // exist for interop with code that already owns an event), and one thread each
        // keeps unregistration from having to disturb unrelated waits.
        let thread = std::thread::spawn(move || {
            let handles = handles;
            let objects = [handles.wait, handles.cancel];
            loop {
                // WAIT_OBJECT_0 is 0, so index 0 means the caller's handle signalled
                // and index 1 means we were unregistered.
                // SAFETY: `objects` holds two live handles (`handles.wait` owned by the
                // caller, `handles.cancel` owned by this waiter), `false` (wait-any) and
                // `u32::MAX` (infinite) are plain values.
                let signalled = unsafe { WaitForMultipleObjects(&objects, false, u32::MAX) };
                if signalled != 0 {
                    break;
                }
                if !queue.submit(kind, handles.context, callback, Duration::ZERO) {
                    break;
                }
            }
        });

        crate::com::xasync::ctx_box_into_raw(Waiter {
            cancel,
            thread: Some(thread),
        }) as u64
    }

    /// # Safety
    /// `token` must come from [`register`] and not have been unregistered.
    pub unsafe fn unregister(token: u64) {
        if token == 0 {
            return;
        }
        // SAFETY: `token` is a still-live pointer from `register`'s `ctx_box_into_raw`
        // call above, and the caller's contract (this function's `# Safety` doc) is that
        // it is unregistered at most once.
        let mut waiter =
            unsafe { crate::com::xasync::ctx_box_from_raw::<Waiter>(token as *mut c_void) };
        // SAFETY: `waiter.cancel` is a live handle owned by this waiter, just reclaimed above.
        let _ = unsafe { SetEvent(waiter.cancel) };
        if let Some(thread) = waiter.thread.take() {
            let _ = thread.join();
        }
        // SAFETY: `waiter.cancel` is still live here; the wait thread has already exited
        // (joined above) so nothing else can be using it.
        let _ = unsafe { CloseHandle(waiter.cancel) };
    }
}

#[cfg(test)]
// Test code exercises this crate's own already-documented internal APIs against
// synthetic, controlled inputs, not untrusted FFI callers - a per-site SAFETY comment
// here would just restate the production contract already documented at each fn.
#[allow(clippy::undocumented_unsafe_blocks)]
mod tests {
    use super::*;
    use crate::com::xasync::{XAsyncBlock, get_result, get_result_size, get_status, run_sync};
    use std::sync::atomic::AtomicUsize;
    use windows_core::Interface;

    fn empty_block() -> XAsyncBlock {
        XAsyncBlock {
            queue: std::ptr::null_mut(),
            context: std::ptr::null_mut(),
            callback: None,
            internal: [0; std::mem::size_of::<*mut c_void>() * 4],
        }
    }

    /// The object is reached the way a game reaches it - through `QueryApiImpl` - so
    /// these tests also cover the registration under `CLSID_XASYNC`.
    fn xasync() -> IXAsync {
        let mut out = std::ptr::null_mut();
        let hr =
            crate::com::query_api_impl(&crate::com::xasync::CLSID_XASYNC, &IXAsync::IID, &mut out);
        assert_eq!(
            hr, S_OK,
            "CLSID_XASYNC should resolve without a delegate DLL"
        );
        unsafe { IXAsync::from_raw(out) }
    }

    #[test]
    fn a_synchronous_provider_completes_and_yields_its_result() {
        let mut block = empty_block();
        let hr = unsafe { run_sync(&mut block, || Ok::<u32, HRESULT>(0xC0FFEE)) };
        assert_eq!(hr, S_OK);

        assert_eq!(unsafe { get_status(&mut block, true) }, Ok(()));
        assert_eq!(unsafe { get_result_size(&mut block) }, Ok(4));

        let mut value = 0u32;
        assert_eq!(
            unsafe { get_result(&mut block, std::ptr::null(), &mut value) },
            Ok(())
        );
        assert_eq!(value, 0xC0FFEE);
    }

    #[test]
    fn a_provider_that_fails_reports_its_error_not_success() {
        let mut block = empty_block();
        let hr = unsafe { run_sync(&mut block, || Err::<u32, HRESULT>(E_ABORT)) };
        assert_eq!(
            hr, S_OK,
            "starting the call succeeds even though the work fails"
        );
        assert_eq!(unsafe { get_status(&mut block, true) }, Err(E_ABORT));
    }

    #[test]
    fn the_completion_callback_runs_and_sees_its_context() {
        static SEEN: AtomicUsize = AtomicUsize::new(0);

        unsafe extern "system" fn on_complete(block: *mut XAsyncBlock) {
            let context = unsafe { (*block).context } as usize;
            // The documented pattern for a call with a result: retrieve it here.
            let mut value = 0u32;
            assert_eq!(
                unsafe { get_result(block, std::ptr::null(), &mut value) },
                Ok(())
            );
            assert_eq!(value, 7);
            // Published last: the test returns as soon as it sees this, taking the
            // block on its stack with it.
            SEEN.store(context, Ordering::SeqCst);
        }

        let mut block = empty_block();
        block.context = 0x1234 as *mut c_void;
        block.callback = Some(on_complete);

        assert_eq!(
            unsafe { run_sync(&mut block, || Ok::<u32, HRESULT>(7)) },
            S_OK
        );

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while SEEN.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(SEEN.load(Ordering::SeqCst), 0x1234);
    }

    #[test]
    fn a_block_is_reusable_once_its_result_has_been_taken() {
        let mut block = empty_block();
        for expected in [1u32, 2, 3] {
            assert_eq!(
                unsafe { run_sync(&mut block, move || Ok::<u32, HRESULT>(expected)) },
                S_OK,
                "the block must be free again after the previous result was taken"
            );
            assert_eq!(unsafe { get_status(&mut block, true) }, Ok(()));
            let mut value = 0u32;
            assert_eq!(
                unsafe { get_result(&mut block, std::ptr::null(), &mut value) },
                Ok(())
            );
            assert_eq!(value, expected);
        }
    }

    #[test]
    fn a_block_still_running_cannot_be_reused() {
        let mut block = empty_block();
        assert_eq!(
            unsafe { run_sync(&mut block, || Ok::<u32, HRESULT>(1)) },
            S_OK
        );
        // Deliberately not retrieving the result, so the block is still in use.
        let hr = unsafe { run_sync(&mut block, || Ok::<u32, HRESULT>(2)) };
        assert_eq!(hr, E_INVALIDARG);

        // `run_sync` runs its body on a worker, so the result is not there the instant it
        // returns; wait for the call it started before reading it.
        assert_eq!(
            unsafe { crate::com::xasync::get_status(&mut block, true) },
            Ok(())
        );
        let mut value = 0u32;
        assert_eq!(
            unsafe { get_result(&mut block, std::ptr::null(), &mut value) },
            Ok(())
        );
        assert_eq!(
            value, 1,
            "the original call is unaffected by the rejected reuse"
        );
    }

    #[test]
    fn asking_for_a_result_before_completion_is_refused() {
        let mut block = empty_block();
        let mut value = 0u32;
        // Nothing has begun on this block at all.
        assert_eq!(
            unsafe { get_result(&mut block, std::ptr::null(), &mut value) },
            Err(E_ILLEGAL_METHOD_CALL)
        );
    }

    #[test]
    fn a_too_small_buffer_is_refused_rather_than_overflowed() {
        let mut block = empty_block();
        assert_eq!(
            unsafe { run_sync(&mut block, || Ok::<u64, HRESULT>(9)) },
            S_OK
        );
        assert_eq!(unsafe { get_status(&mut block, true) }, Ok(()));

        let xasync = xasync();
        let mut small = 0u16;
        let mut used = 0usize;
        let hr = unsafe {
            xasync.XAsyncGetResult(
                (&mut block as *mut XAsyncBlock).cast(),
                std::ptr::null_mut(),
                std::mem::size_of::<u16>() as u64,
                (&mut small as *mut u16).cast(),
                &mut used,
            )
        };
        assert_eq!(hr, E_NOT_SUFFICIENT_BUFFER);

        // Refusing must not have consumed the call; the right-sized read still works.
        let mut value = 0u64;
        assert_eq!(
            unsafe { get_result(&mut block, std::ptr::null(), &mut value) },
            Ok(())
        );
        assert_eq!(value, 9);
    }

    #[test]
    fn queue_handles_round_trip_through_the_interface() {
        let xasync = xasync();
        unsafe {
            let mut queue = 0u64;
            // Manual/Manual, so nothing runs behind the test's back.
            assert_eq!(xasync.XTaskQueueCreate(0, 0, &mut queue), S_OK);
            assert_ne!(queue, 0);

            let mut duplicate = 0u64;
            assert_eq!(
                xasync.XTaskQueueDuplicateHandle(queue, &mut duplicate),
                S_OK
            );
            assert_ne!(duplicate, queue);

            let mut work_port = 0u64;
            let mut completion_port = 0u64;
            assert_eq!(xasync.XTaskQueueGetPort(queue, 0, &mut work_port), S_OK);
            assert_eq!(
                xasync.XTaskQueueGetPort(queue, 1, &mut completion_port),
                S_OK
            );

            let mut composite = 0u64;
            assert_eq!(
                xasync.XTaskQueueCreateComposite(work_port, completion_port, &mut composite),
                S_OK
            );
            assert_ne!(composite, 0);

            // Closing the original leaves the duplicate and the composite working.
            xasync.XTaskQueueCloseHandle(queue);

            static RAN: AtomicUsize = AtomicUsize::new(0);
            unsafe extern "system" fn tick(_context: *mut c_void, canceled: bool) {
                if !canceled {
                    RAN.fetch_add(1, Ordering::SeqCst);
                }
            }
            assert_eq!(
                xasync.XTaskQueueSubmitCallback(
                    composite,
                    0,
                    std::ptr::null_mut(),
                    tick as *mut c_void,
                ),
                S_OK
            );
            assert_ne!(xasync.XTaskQueueDispatch(duplicate, 0, 0), 0);
            assert_eq!(RAN.load(Ordering::SeqCst), 1);

            xasync.XTaskQueueCloseHandle(duplicate);
            xasync.XTaskQueueCloseHandle(composite);
        }
    }

    #[test]
    fn an_invalid_dispatch_mode_is_rejected() {
        let xasync = xasync();
        let mut queue = 0u64;
        assert_eq!(
            unsafe { xasync.XTaskQueueCreate(99, 0, &mut queue) },
            E_INVALIDARG
        );
        assert_eq!(queue, 0);
    }

    #[test]
    fn terminating_a_queue_stops_it_accepting_callbacks() {
        let xasync = xasync();
        unsafe extern "system" fn nop(_context: *mut c_void, _canceled: bool) {}
        unsafe {
            let mut queue = 0u64;
            assert_eq!(xasync.XTaskQueueCreate(0, 0, &mut queue), S_OK);
            assert_eq!(
                xasync.XTaskQueueTerminate(queue, 1, std::ptr::null_mut(), std::ptr::null_mut()),
                S_OK
            );
            assert_eq!(
                xasync.XTaskQueueSubmitCallback(queue, 0, std::ptr::null_mut(), nop as *mut c_void),
                E_ABORT
            );
            xasync.XTaskQueueCloseHandle(queue);
        }
    }

    #[test]
    fn the_process_queue_is_only_reported_once_one_is_set() {
        let xasync = xasync();
        unsafe {
            let saved = {
                let mut existing = 0u64;
                (xasync.XTaskQueueGetCurrentProcessTaskQueue(&mut existing) != 0)
                    .then_some(existing)
            };

            let mut queue = 0u64;
            assert_eq!(xasync.XTaskQueueCreate(0, 0, &mut queue), S_OK);
            xasync.XTaskQueueSetCurrentProcessTaskQueue(queue);

            let mut reported = 0u64;
            assert_ne!(
                xasync.XTaskQueueGetCurrentProcessTaskQueue(&mut reported),
                0
            );
            assert_ne!(reported, 0);
            xasync.XTaskQueueCloseHandle(reported);

            // Other tests share this process, so put back whatever was there.
            match saved {
                Some(saved) => xasync.XTaskQueueSetCurrentProcessTaskQueue(saved),
                None => task_queue::set_process_queue(None),
            }
            xasync.XTaskQueueCloseHandle(queue);
        }
    }
}
