//! The async engine's per-call state: what the runtime parks inside `XAsyncBlock::internal`
//! while a call is alive, and the helpers that drive provider invocation, completion, and
//! result delivery. The COM face lives in [`super::r#impl`]; the queue in [`super::task_queue`].

use super::task_queue::{self, PortKind, Queue, QueueHandle};
use super::*;
use crate::results::{E_ABORT, E_PENDING, S_OK};

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub(crate) const SIGNATURE: u64 = 0x584153594e435f31; // "XASYNC_1"

pub(crate) struct Inner {
    /// `E_PENDING` until `XAsyncComplete`.
    pub(crate) status: HRESULT,
    pub(crate) result_size: usize,
    pub(crate) canceled: bool,
    /// Set once the provider has been sent `Cleanup`, so it happens exactly once.
    pub(crate) cleaned_up: bool,
}

pub(crate) struct AsyncState {
    pub(crate) provider: XAsyncProvider,
    pub(crate) context: *mut c_void,
    /// An opaque tag the caller passes to `XAsyncGetResult` to prove it is asking the
    /// API that actually started the call.
    pub(crate) identity: *mut c_void,
    pub(crate) queue: Arc<Queue>,
    /// Instrumentation: [`task_queue::now_ms`] at `XAsyncBegin`, so completion can report
    /// end-to-end latency per API name - the number the load-time lag is actually made of.
    pub(crate) began_ms: u128,
    pub(crate) block: *mut XAsyncBlock,
    pub(crate) inner: Mutex<Inner>,
    /// Signalled on completion, for `XAsyncGetStatus(wait: true)`.
    pub(crate) completed: Condvar,
}

// The block and the provider context belong to the caller, who is responsible for
// keeping them alive for the duration of the call; the GDK contract explicitly allows
// the runtime to touch them from its own threads.
unsafe impl Send for AsyncState {}
unsafe impl Sync for AsyncState {}

impl AsyncState {
    pub(crate) fn provider_data(
        &self,
        buffer: *mut c_void,
        buffer_size: usize,
    ) -> XAsyncProviderData {
        XAsyncProviderData {
            async_: self.block,
            bufferSize: buffer_size,
            buffer,
            context: self.context,
        }
    }

    pub(crate) fn invoke(&self, op: XAsyncOp, buffer: *mut c_void, buffer_size: usize) -> HRESULT {
        let data = self.provider_data(buffer, buffer_size);
        unsafe { (self.provider)(op, &data) }
    }
}

/// Read the state pointer out of a block without taking ownership of it.
///
/// # Safety
/// `block` must be null or point at a valid `XAsyncBlock`.
pub(crate) unsafe fn state_of(block: *mut XAsyncBlock) -> Option<Arc<AsyncState>> {
    let block = unsafe { block.as_mut() }?;
    let (signature, pointer) = read_internal(block);
    if signature != SIGNATURE || pointer.is_null() {
        return None;
    }
    let state = unsafe { Arc::from_raw(pointer) };
    // The block keeps its reference; hand the caller a new one.
    let clone = state.clone();
    std::mem::forget(state);
    Some(clone)
}

/// Diagnostic-only: block pointer -> the `identityName` the caller supplied to
/// `XAsyncBegin`, so completion/DoWork diag can say *which* API an async belonged to.
pub(crate) static BLOCK_NAMES: Mutex<Option<HashMap<usize, &'static str>>> = Mutex::new(None);
/// Interns an `identityName` so the registry above can hold a `&'static str` without
/// leaking a fresh allocation per `XAsyncBegin`. Titles reuse a small fixed set of names
/// (one per XSAPI entry point), and a busy session calls `XAsyncBegin` tens of thousands of
/// times, so interning turns an unbounded leak into a bounded one.
pub(crate) fn intern(name: &str) -> &'static str {
    static NAMES: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut names = NAMES.lock().expect("name interner poisoned");
    let names = names.get_or_insert_with(HashSet::new);
    if let Some(interned) = names.get(name) {
        return interned;
    }
    let interned: &'static str = Box::leak(name.to_owned().into_boxed_str());
    names.insert(interned);
    interned
}

pub(crate) fn name_block(block: *mut XAsyncBlock, name: &'static str) {
    BLOCK_NAMES
        .lock()
        .expect("block-name registry poisoned")
        .get_or_insert_with(HashMap::new)
        .insert(block as usize, name);
}
pub(crate) fn block_name(block: *mut XAsyncBlock) -> Option<String> {
    BLOCK_NAMES
        .lock()
        .expect("block-name registry poisoned")
        .as_ref()?
        .get(&(block as usize))
        .map(|s| s.to_string())
}

// Per-thread "which XSAPI/game async is currently running on this thread" — set while a
// completion callback or DoWork is executing, so a libHttpClient sub-call created inside
// it can be attributed to the exact parent XSAPI API that triggered it.
thread_local! {
    static CURRENT_IDENTITY: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}
pub(crate) fn current_identity() -> Option<String> {
    CURRENT_IDENTITY.with(|c| c.borrow().clone())
}
pub(crate) fn set_current_identity(name: Option<String>) {
    CURRENT_IDENTITY.with(|c| *c.borrow_mut() = name);
}
/// libHttpClient-internal asyncs (the sub-calls we want to attribute to a parent XSAPI API).
pub(crate) fn is_lhc_internal(name: &str) -> bool {
    name.starts_with("HC_") || name.contains("httpclient") || name == "run_async"
}

/// Detach the state from the block, taking the block's reference with it.
pub(crate) unsafe fn take_state(block: *mut XAsyncBlock) -> Option<Arc<AsyncState>> {
    let block = unsafe { block.as_mut() }?;
    let (signature, pointer) = read_internal(block);
    if signature != SIGNATURE || pointer.is_null() {
        return None;
    }
    block.internal.fill(0);
    Some(unsafe { Arc::from_raw(pointer) })
}

pub(crate) fn read_internal(block: &XAsyncBlock) -> (u64, *const AsyncState) {
    let mut signature = [0u8; 8];
    let mut pointer = [0u8; 8];
    signature.copy_from_slice(&block.internal[0..8]);
    pointer.copy_from_slice(&block.internal[8..16]);
    (
        u64::from_ne_bytes(signature),
        usize::from_ne_bytes(pointer) as *const AsyncState,
    )
}

pub(crate) fn write_internal(block: &mut XAsyncBlock, state: *const AsyncState) {
    block.internal.fill(0);
    block.internal[0..8].copy_from_slice(&SIGNATURE.to_ne_bytes());
    block.internal[8..16].copy_from_slice(&(state as usize).to_ne_bytes());
}

/// Send the provider `Cleanup` and release the state. Idempotent, because both
/// `XAsyncGetResult` and the void-result path in `XAsyncGetStatus` can reach it.
pub(crate) fn cleanup(state: &Arc<AsyncState>) {
    {
        let mut inner = state.inner.lock().expect("async state poisoned");
        if inner.cleaned_up {
            return;
        }
        inner.cleaned_up = true;
    }
    let _ = state.invoke(XAsyncOp::Cleanup, std::ptr::null_mut(), 0);
    // Drops the block's reference; any callback still holding one keeps the allocation
    // alive until it returns.
    unsafe { take_state(state.block) };
}

/// Runs on the work port. One `DoWork` pass.
pub(crate) unsafe extern "system" fn work_callback(context: *mut c_void, canceled: bool) {
    let state = unsafe { Arc::from_raw(context as *const AsyncState) };
    let name = block_name(state.block);
    eprintln!(
        "[diag] work_callback (DoWork dispatch) name={name:?} block={:p} canceled={canceled}",
        state.block
    );

    if canceled {
        complete_state(&state, E_ABORT, 0);
        return;
    }

    let prev = current_identity();
    set_current_identity(name.clone());
    let hr = state.invoke(XAsyncOp::DoWork, std::ptr::null_mut(), 0);
    set_current_identity(prev);
    if hr == E_PENDING {
        // The provider will schedule itself again; this is the future-polling path.
        return;
    }
    if hr != S_OK {
        eprintln!(
            "[diag] provider DoWork returned non-pending {hr:?} for block={:p} (completing with it)",
            state.block
        );
    }
    // Anything else means the provider is done. Well-behaved ones have already called
    // XAsyncComplete, in which case this is a no-op; the rest get completed for them so
    // a failing provider cannot leave the call hanging forever.
    complete_state(&state, hr, 0);
}

/// Runs on the completion port: hand the call back to the game.
pub(crate) struct Completion {
    state: Arc<AsyncState>,
    callback: XAsyncCompletionRoutine,
}

pub(crate) unsafe extern "system" fn completion_callback(context: *mut c_void, _canceled: bool) {
    let completion = unsafe { Box::from_raw(context as *mut Completion) };
    let name = block_name(completion.state.block);
    // The number that matters: XAsyncBegin -> the game hearing about it. Split into the
    // work half and the wait-for-a-pump half by comparing against complete_state's
    // `work_ms` for the same block.
    eprintln!(
        "[qdiag t={}] completion_callback name={name:?} block={:p} total_ms={}",
        task_queue::now_ms(),
        completion.state.block,
        task_queue::now_ms().saturating_sub(completion.state.began_ms),
    );
    let prev = current_identity();
    set_current_identity(name);
    unsafe { (completion.callback)(completion.state.block) };
    set_current_identity(prev);
}

pub(crate) fn complete_state(
    state: &Arc<AsyncState>,
    result: HRESULT,
    required_buffer_size: usize,
) {
    eprintln!(
        "[diag] complete_state block={:p} result={result:?} size={required_buffer_size}",
        state.block
    );
    {
        let mut inner = state.inner.lock().expect("async state poisoned");
        if result == E_ABORT {
            eprintln!(
                "[diag] E_ABORT completion block={:p} canceled_flag={}",
                state.block, inner.canceled
            );
        }
        if inner.status != E_PENDING {
            // Already completed. Providers routinely both call XAsyncComplete and return
            // a status from DoWork, so this is the normal path, not an error.
            eprintln!(
                "[diag] complete_state block={:p} already completed with status={:?}, ignoring",
                state.block, inner.status
            );
            return;
        }
        inner.status = result;
        inner.result_size = required_buffer_size;
    }
    // Read the callback out now, while the block is certainly still alive. Deferring
    // the read to the completion port would be a use-after-free for the common case of
    // a block with no callback: its owner is entitled to wait on XAsyncGetStatus and
    // then drop the block, and with nothing to invoke there is no reason to touch it
    // again at all.
    let callback = unsafe { state.block.as_ref() }.and_then(|block| block.callback);
    state.completed.notify_all();

    let Some(callback) = callback else {
        eprintln!(
            "[diag] complete_state block={:p} has no callback set, relying on polling",
            state.block
        );
        return;
    };
    let context = Box::into_raw(Box::new(Completion {
        state: state.clone(),
        callback,
    })) as *mut c_void;
    let submitted = state.queue.submit(
        PortKind::Completion,
        context,
        completion_callback,
        Duration::ZERO,
    );
    eprintln!(
        "[qdiag t={}] complete_state name={:?} block={:p} submitted={submitted} completion_port={:p} mode={:?} queue_ptr={:p} work_ms={}",
        task_queue::now_ms(),
        block_name(state.block),
        state.block,
        Arc::as_ptr(state.queue.port(PortKind::Completion)),
        state.queue.port(PortKind::Completion).mode(),
        Arc::as_ptr(&state.queue),
        task_queue::now_ms().saturating_sub(state.began_ms),
    );
    if !submitted {
        // The completion port is gone. The game still has to hear about the call, so
        // deliver it here rather than dropping it on the floor.
        unsafe { completion_callback(context, true) };
    }
}

pub(crate) fn queue_for(block: &XAsyncBlock) -> Option<Arc<Queue>> {
    if block.queue.is_null() {
        // A block that names no queue uses the process queue, which is what makes the
        // common `XAsyncBlock { queue: null, .. }` in sample code work.
        return Some(task_queue::default_process_queue());
    }
    QueueHandle::get(block.queue as u64)
}
