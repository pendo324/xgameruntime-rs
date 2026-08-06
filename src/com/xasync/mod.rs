//! `XAsync`/`XTaskQueue` COM-face surface: the `IXAsync` interface, the ABI types
//! (`XAsyncBlock`, `XAsyncOp`, `XAsyncProviderData`), and the public wrapper functions
//! titles call (`begin`, `run`, `run_sync`, `get_result`, …) that drive the `XAsyncObject`
//! in [`r#impl`]. The engine lives in [`core`] (per-call async state) and [`task_queue`]
//! (the queue implementation).
//!
//! [`r#impl`]: crate::com::xasync::impl
//! [`core`]: crate::com::xasync::core
//! [`task_queue`]: crate::com::xasync::task_queue

pub mod core;
pub mod r#impl;
pub mod task_queue;

use crate::S_OK;
use crate::com::query_api_impl;

use crate::results::*;
use std::ffi::{c_char, c_void};
use std::mem::size_of;
use std::ptr::null_mut;
use std::sync::Arc;
use windows_core::{GUID, HRESULT, IUnknown, Interface, interface};
use windows_sys::core::BOOL;

type XTaskQueueHandle = *mut c_void;
pub type XAsyncCompletionRoutine = unsafe extern "system" fn(async_block: *mut XAsyncBlock);
type XAsyncProvider =
    unsafe extern "system" fn(op: XAsyncOp, data: *const XAsyncProviderData) -> HRESULT;

pub const CLSID_XASYNC: GUID = GUID::from_u128(0x073b7dcb_1fcf_4030_94be_e3c9eb623428);

#[repr(C)]
pub struct XAsyncBlock {
    pub queue: XTaskQueueHandle,
    pub context: *mut c_void,
    pub callback: Option<XAsyncCompletionRoutine>,
    pub internal: [u8; size_of::<*mut c_void>() * 4],
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XAsyncOp {
    Begin = 0,
    DoWork = 1,
    GetResult = 2,
    Cancel = 3,
    Cleanup = 4,
}

#[repr(C)]
pub struct XAsyncProviderData {
    pub async_: *mut XAsyncBlock,
    pub bufferSize: usize,
    pub buffer: *mut c_void,
    pub context: *mut c_void,
}

#[interface("073b7dcb-1fcf-4030-94be-e3c9eb623428")]
pub unsafe trait IXAsync: IUnknown {
    pub unsafe fn XAsyncGetStatus(&self, asyncBlock: *mut c_void, wait: BOOL) -> HRESULT;
    pub unsafe fn XAsyncGetResultSize(
        &self,
        asyncBlock: *mut c_void,
        bufferSize: *mut usize,
    ) -> HRESULT;
    pub unsafe fn XAsyncCancel(&self, asyncBlock: *mut c_void) -> ();
    pub unsafe fn XAsyncRun(&self, asyncBlock: *mut c_void, work: *mut c_void) -> HRESULT;
    pub unsafe fn XAsyncBegin(
        &self,
        asyncBlock: *mut c_void,
        context: *mut c_void,
        identity: *mut c_void,
        identityName: *mut c_char,
        provider: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn __ReservedSlot8(&self) -> HRESULT;
    pub unsafe fn XAsyncSchedule(&self, asyncBlock: *mut c_void, delayInMs: u32) -> HRESULT;
    pub unsafe fn XAsyncComplete(
        &self,
        asyncBlock: *mut c_void,
        result: i32,
        requiredBufferSize: u64,
    ) -> ();
    pub unsafe fn XAsyncGetResult(
        &self,
        asyncBlock: *mut c_void,
        identity: *mut c_void,
        bufferSize: u64,
        buffer: *mut c_void,
        bufferUsed: *mut usize,
    ) -> HRESULT;
    pub unsafe fn XTaskQueueCreate(
        &self,
        workDispatchMode: u64,
        completionDispatchMode: u64,
        queue: *mut u64,
    ) -> HRESULT;
    pub unsafe fn XTaskQueueCreateComposite(
        &self,
        workPort: u64,
        completionPort: u64,
        queue: *mut u64,
    ) -> HRESULT;
    pub unsafe fn XTaskQueueGetPort(&self, queue: u64, port: u64, portHandle: *mut u64) -> HRESULT;
    pub unsafe fn XTaskQueueDuplicateHandle(
        &self,
        queueHandle: u64,
        duplicatedHandle: *mut u64,
    ) -> HRESULT;
    pub unsafe fn XTaskQueueDispatch(&self, queue: u64, port: u64, timeoutInMs: u32) -> BOOL;
    pub unsafe fn XTaskQueueCloseHandle(&self, queue: u64) -> ();
    pub unsafe fn XTaskQueueSubmitCallback(
        &self,
        queue: u64,
        port: u64,
        callbackContext: *mut c_void,
        callback: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XTaskQueueSubmitDelayedCallback(
        &self,
        queue: u64,
        port: u64,
        delayMs: u32,
        callbackContext: *mut c_void,
        callback: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XTaskQueueRegisterWaiter(
        &self,
        queue: u64,
        port: u64,
        waitHandle: *mut c_void,
        callbackContext: *mut c_void,
        callback: *mut c_void,
        token: *mut u64,
    ) -> HRESULT;
    pub unsafe fn XTaskQueueUnregisterWaiter(&self, queue: u64, token: u64) -> ();
    pub unsafe fn XTaskQueueTerminate(
        &self,
        queue: u64,
        wait: BOOL,
        callbackContext: *mut c_void,
        callback: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XTaskQueueRegisterMonitor(
        &self,
        queue: u64,
        callbackContext: *mut c_void,
        callback: *mut c_void,
        token: *mut u64,
    ) -> HRESULT;
    pub unsafe fn XTaskQueueUnregisterMonitor(&self, queue: u64, token: u64) -> ();
    pub unsafe fn XTaskQueueGetCurrentProcessTaskQueue(&self, queue: *mut u64) -> BOOL;
    pub unsafe fn XTaskQueueSetCurrentProcessTaskQueue(&self, queue: u64) -> ();
    pub unsafe fn XThreadSetTimeSensitive(&self, isTimeSensitiveThread: BOOL) -> HRESULT;
    pub unsafe fn __ReservedSlot28(&self) -> HRESULT;
    pub unsafe fn XThreadAssertNotTimeSensitive(&self) -> ();
    pub unsafe fn XThreadIsTimeSensitive(&self) -> BOOL;
}

fn interface() -> Result<IXAsync, HRESULT> {
    let mut out = std::ptr::null_mut();
    let hr = query_api_impl(&CLSID_XASYNC, &IXAsync::IID, &mut out);
    if hr != S_OK {
        return Err(hr);
    }
    Ok(unsafe { IXAsync::from_raw(out) })
}

fn result<T>(r: T, h: HRESULT) -> Result<T, HRESULT> {
    if h == S_OK { Ok(r) } else { Err(h) }
}

pub unsafe fn begin(
    async_block: *mut XAsyncBlock,
    context: *mut c_void,
    identity: *const c_void,
    identity_name: *const c_char,
    provider: XAsyncProvider,
) -> Result<(), HRESULT> {
    let xasync = interface()?;
    let hr = unsafe {
        xasync.XAsyncBegin(
            async_block.cast(),
            context,
            identity.cast_mut(),
            identity_name.cast_mut(),
            provider as *mut c_void,
        )
    };
    result((), hr)
}

pub unsafe fn complete(
    async_block: *mut XAsyncBlock,
    result: HRESULT,
    required_buffer_size: usize,
) -> Result<(), HRESULT> {
    let xasync = interface()?;
    unsafe { xasync.XAsyncComplete(async_block.cast(), result.0, required_buffer_size as u64) };
    Ok(())
}

pub unsafe fn get_result<T>(
    async_block: *mut XAsyncBlock,
    identity: *const c_void,
    out: *mut T,
) -> Result<(), HRESULT> {
    let xasync = interface()?;
    let mut buffer_used = 0usize;
    let hr = unsafe {
        xasync.XAsyncGetResult(
            async_block.cast(),
            identity.cast_mut(),
            size_of::<T>() as u64,
            out.cast(),
            &mut buffer_used,
        )
    };
    result((), hr)
}

/// Only the tests wait on a block this way; the shipped paths all complete through the
/// completion callback that `run_sync` installs.
#[cfg(test)]
pub unsafe fn get_status(async_block: *mut XAsyncBlock, wait: bool) -> Result<(), HRESULT> {
    let xasync = interface()?;
    let hr = unsafe { xasync.XAsyncGetStatus(async_block.cast(), wait.into()) };
    result((), hr)
}

pub unsafe fn get_result_size(async_block: *mut XAsyncBlock) -> Result<usize, HRESULT> {
    let xasync = interface()?;
    let mut buffer_size: usize = 0;
    let hr = unsafe { xasync.XAsyncGetResultSize(async_block.cast(), &mut buffer_size) };
    result(buffer_size, hr)
}

/// Worker pool for [`run_sync`]'s blocking bodies.
///
/// Deliberately a queue of our own rather than the process queue: the whole point is to
/// get this work *off* whatever thread called the API. XSAPI issues these from inside
/// DoWork callbacks that the game dispatches on its own manual work port, so running them
/// inline (as this helper used to) meant the game's pump thread performed a ~1s blocking
/// IPC round-trip per callback - ten per dispatch measured at 12.1s of frozen pump, which
/// was the load-time lag. Only the *work* moves; completion is still delivered on the
/// block's own completion port, so callers that must observe completions on the game
/// thread (XSAPI's SocialManager, which mutates its social graph there) are unaffected.
static RUN_SYNC_QUEUE: std::sync::OnceLock<Arc<task_queue::Queue>> = std::sync::OnceLock::new();
fn run_sync_queue() -> &'static Arc<task_queue::Queue> {
    RUN_SYNC_QUEUE.get_or_init(|| {
        task_queue::Queue::new(
            task_queue::DispatchMode::ThreadPool,
            task_queue::DispatchMode::ThreadPool,
        )
    })
}

/// A raw pointer moved to a worker thread. The pointee is a heap allocation whose lifetime
/// the XAsync protocol pins for us: the context is only freed on `Cleanup`, which the
/// caller cannot reach until it has seen the completion this worker posts.
struct SendPtr<T>(*mut T);
unsafe impl<T> Send for SendPtr<T> {}

type BoxedWork = Box<dyn FnOnce(bool) + Send>;

unsafe extern "system" fn run_sync_worker(context: *mut c_void, canceled: bool) {
    let work = unsafe { Box::from_raw(context as *mut BoxedWork) };
    work(canceled);
}

struct XsyncContextHelper<T: Sized, F: Fn() -> Result<T, HRESULT>> {
    result: HRESULT,
    canceled: bool,
    payload: Option<T>,
    future: F,
}

unsafe extern "system" fn run_sync_helper<
    T: Sized + Send + 'static,
    F: Fn() -> Result<T, HRESULT> + Send + 'static,
>(
    op: XAsyncOp,
    data: *const XAsyncProviderData,
) -> HRESULT {
    let Some(data) = (unsafe { data.as_ref() }) else {
        return E_POINTER;
    };
    let Some(async_context) = (unsafe { (data.context as *mut XsyncContextHelper<T, F>).as_mut() })
    else {
        return E_POINTER;
    };

    match op {
        XAsyncOp::Begin => {
            // Hand the blocking body to a worker and return immediately. The context and
            // the block both outlive this: `Cleanup` (which frees the context) cannot run
            // until the caller has observed the completion posted at the end of the work.
            let context = SendPtr(async_context as *mut XsyncContextHelper<T, F>);
            let block = SendPtr(data.async_);
            let work: BoxedWork = Box::new(move |canceled: bool| {
                let context = context;
                let block = block;
                let async_context = unsafe { &mut *context.0 };
                if canceled {
                    // The pool is going away. Still complete, or the caller waits forever.
                    async_context.result = E_ABORT;
                } else {
                    match (async_context.future)() {
                        Ok(value) => {
                            async_context.result = S_OK;
                            async_context.payload = Some(value);
                        }
                        Err(hr) => async_context.result = hr,
                    }
                }
                let _ = unsafe { complete(block.0, async_context.result, size_of::<T>()) };
            });
            let submitted = run_sync_queue().submit(
                task_queue::PortKind::Work,
                Box::into_raw(Box::new(work)) as *mut c_void,
                run_sync_worker,
                std::time::Duration::ZERO,
            );
            if !submitted {
                // No worker to run it on; fall back to the old inline behaviour rather
                // than strand the call.
                match (async_context.future)() {
                    Ok(value) => {
                        async_context.result = S_OK;
                        async_context.payload = Some(value);
                    }
                    Err(hr) => async_context.result = hr,
                }
                return unsafe { complete(data.async_, async_context.result, size_of::<T>()) }
                    .map(|_| S_OK)
                    .unwrap_or_else(|hr| hr);
            }
            S_OK
        }
        XAsyncOp::DoWork => S_OK,
        XAsyncOp::GetResult => {
            if async_context.result == S_OK
                && let Some(payload) = &async_context.payload
            {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        (payload as *const T).cast::<u8>(),
                        data.buffer.cast::<u8>(),
                        size_of::<T>(),
                    );
                }
            }
            S_OK
        }
        XAsyncOp::Cancel => {
            async_context.canceled = true;
            S_OK
        }
        XAsyncOp::Cleanup => {
            unsafe {
                drop(Box::from_raw(async_context));
            }
            S_OK
        }
    }
}

pub unsafe fn run_sync<T: Sized + Send + 'static, F>(async_: *mut XAsyncBlock, future: F) -> HRESULT
where
    F: Fn() -> Result<T, HRESULT> + Send + 'static,
{
    if async_.is_null() {
        return S_OK;
    }

    let async_context = Box::into_raw(Box::new(XsyncContextHelper {
        canceled: false,
        payload: None as Option<T>,
        result: E_ABORT,
        future,
    }));
    match unsafe {
        begin(
            async_,
            async_context.cast(),
            null_mut(),
            c"run_async".as_ptr(),
            run_sync_helper::<T, F>,
        )
    } {
        Ok(_) => S_OK,
        Err(hr) => {
            unsafe {
                drop(Box::from_raw(async_context));
            }
            hr
        }
    }
}
