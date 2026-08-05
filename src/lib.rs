//! Native runtime behind the Game Development Kit's `xgameruntime.dll`, exposing XTaskQueue,
//! XAsync, XUser, XStore, XGameSave and friends to statically-linked XSAPI titles.
//!
//! The COM surface deliberately mirrors the Microsoft GDK API: method and field names match the
//! published headers (PascalCase), so the whole crate opts out of `non_snake_case` rather than
//! annotate each binding. This is the same convention the `winapi`/`windows` crates use.

#![allow(non_snake_case)]

use std::ffi::{CStr, c_char, c_void};
use std::ptr::null_mut;
use std::sync::{Arc, Mutex};

use windows::minwindef::LPARAM;
use windows::windef::HWND;
use windows::winuser::{EnumWindows, MB_OK, MessageBoxW};

use crate::xuser::{IXUserImpl5, XUserPlatformRemoteConnectEventHandlers};
use windows_core::{GUID, HRESULT, Interface};

mod com;
mod gdk_extra;
mod ipc;
mod results;
mod task_queue;
mod xasync;
mod xasync_impl;
mod xgamesave;
mod xuser;

type Ulong = u32;
type Char = i8;
type Lpcstr = *const c_char;

const S_OK: HRESULT = HRESULT(0);
const E_FAIL: HRESULT = HRESULT(0x80004005u32 as i32);
const E_NOTIMPL: HRESULT = HRESULT(0x80004001u32 as i32);

#[repr(C)]
pub struct InitializeOptions;

/// How many times `InitializeApiImpl` has been called without a matching
/// `UninitializeApiImpl`. Games in the same process can initialize independently, and
/// the last one out is the one that tears down.
static INIT_REF_COUNT: Mutex<usize> = Mutex::new(0);

unsafe extern "system" fn find_window(hwnd: HWND, lp: LPARAM) -> windows_core::BOOL {
    unsafe {
        let result: &mut HWND = &mut *(lp.0 as *mut HWND);
        *result = hwnd;
    }
    false.into()
}

unsafe extern "system" fn show(
    _context: *const c_void,
    _user_identifierr: u32,
    _operation: u32,
    url: *const c_char,
    code: *const c_char,
    _qr_code_size: usize,
    _qr_code: *const c_char,
) {
    unsafe {
        let url = CStr::from_ptr(url);
        let code = CStr::from_ptr(code);
        let mut search: HWND = HWND(null_mut());
        _ = EnumWindows(
            Some(find_window),
            LPARAM((&mut search as *mut HWND) as isize),
        );
        MessageBoxW(
            if search.0.is_null() {
                None
            } else {
                Some(search)
            },
            windows_strings::PCWSTR::from_raw(
                windows::core::HSTRING::from(format!(
                    "{} {}",
                    url.to_string_lossy(),
                    code.to_string_lossy()
                ))
                .as_ptr(),
            ),
            windows::core::h!("Xbox Live Remote Login"),
            MB_OK,
        );
    }
}

unsafe extern "system" fn hide() {}

fn initialize(
    _gdk_ver: Ulong,
    _gs_ver: Ulong,
    _mode: Char,
    _options: *mut InitializeOptions,
) -> HRESULT {
    let mut count = INIT_REF_COUNT.lock().expect("init refcount poisoned");
    *count += 1;
    if *count > 1 {
        return S_OK;
    }

    // Real GDK hosts install a process task queue (XTaskQueueSetCurrentProcessTaskQueue)
    // before any async API runs. WineGDK's host never does, so libHttpClient - which falls
    // back to XTaskQueueGetCurrentProcessTaskQueue whenever an async block's queue is NULL,
    // as XSAPI's service calls are - would resolve a NULL queue and abort its own operations
    // with E_INVALIDARG/E_ABORT. Install one here so those NULL-queue asyncs always have a
    // valid ThreadPool queue to run on. See PLAN.md milestone 29 / open risk entry 8.
    if !task_queue::has_process_queue() {
        let queue = task_queue::Queue::new(
            task_queue::DispatchMode::ThreadPool,
            task_queue::DispatchMode::ThreadPool,
        );
        eprintln!(
            "[diag] initialize: install process task queue work_port={:#x} completion_port={:#x}",
            Arc::as_ptr(queue.port(task_queue::PortKind::Work)) as usize,
            Arc::as_ptr(queue.port(task_queue::PortKind::Completion)) as usize,
        );
        task_queue::set_process_queue(Some(queue));
    }

    // Wine has no CloudExperienceHost, so the remote-connect prompt that would normally
    // be shown by the shell has to come from us.
    let mut out: *mut c_void = null_mut();
    if com::query_api_impl(&xuser::CLSID_XUSER, &IXUserImpl5::IID, &mut out) == S_OK
        && let Some(platform) = unsafe { IXUserImpl5::from_raw_borrowed(&out) }
    {
        let handlers = XUserPlatformRemoteConnectEventHandlers {
            show: Some(show),
            close: Some(hide),
            context: null_mut(),
        };
        let _ = unsafe { platform.XUserPlatformRemoteConnectSetEventHandlers(0, &handlers) };
    }

    S_OK
}

fn uninitialize() -> HRESULT {
    let mut count = INIT_REF_COUNT.lock().expect("init refcount poisoned");
    if *count == 0 {
        return E_NOTIMPL;
    }
    *count -= 1;
    if *count == 0 {
        // Last init out: drop the process queue we installed so a later re-init recreates
        // it fresh (its worker threads have been stopped by then anyway).
        task_queue::set_process_queue(None);
    }
    S_OK
}

#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_OK
}

/// DLL entry point. Nothing needs doing at attach or detach: every piece of runtime state is
/// built lazily on first use, so returning success is the whole job.
///
/// # Safety
/// `hinst` and `reserved` are only valid under the Windows loader's standard guarantees and are
/// never dereferenced here, so the unsafe bounds are those implied by Windows calling this export.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllMain(
    _hinst: *mut c_void,
    _reason: u32,
    _reserved: *mut c_void,
) -> i32 {
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn InitializeApiImplEx2(
    gdk_ver: Ulong,
    gs_ver: Ulong,
    mode: Char,
    options: *mut InitializeOptions,
) -> HRESULT {
    initialize(gdk_ver, gs_ver, mode, options)
}

#[unsafe(no_mangle)]
pub extern "system" fn InitializeApiImplEx(gdk_ver: Ulong, gs_ver: Ulong, mode: Char) -> HRESULT {
    InitializeApiImplEx2(gdk_ver, gs_ver, mode, std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub extern "system" fn InitializeApiImpl(gdk_ver: Ulong, gs_ver: Ulong) -> HRESULT {
    InitializeApiImplEx(gdk_ver, gs_ver, 0)
}

#[unsafe(no_mangle)]
pub extern "system" fn QueryApiImpl(
    runtime_class_id: *const GUID,
    interface_id: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    com::query_api_impl(runtime_class_id, interface_id, out)
}

#[unsafe(no_mangle)]
pub extern "system" fn UninitializeApiImpl() -> HRESULT {
    uninitialize()
}

#[unsafe(no_mangle)]
pub extern "system" fn XErrorReport(_status: HRESULT, _message: Lpcstr) -> HRESULT {
    E_NOTIMPL
}

const CLASS_E_CLASSNOTAVAILABLE: HRESULT = HRESULT(0x80040111u32 as i32);

/// Diagnostic-only for now: this crate has never exported `DllGetClassObject`, so classic COM
/// `CoCreateInstance` against any CLSID this DLL should serve (e.g. XSAPI's Xbox Live context,
/// `CLSID_XsapiContext` in WineGDK's `main.c`) fails silently rather than reaching us at all.
/// Logging which CLSIDs are actually requested here - before committing to porting WineGDK's
/// large reverse-engineered service-broker vtable - tells us whether that path is even used by
/// this title, rather than guessing.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "system" fn DllGetClassObject(
    clsid: *const GUID,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    if out.is_null() {
        return windows_core::HRESULT(0x80004003u32 as i32); // E_POINTER
    }
    unsafe {
        *out = null_mut();
    }
    let (clsid, iid) = unsafe { (clsid.as_ref(), iid.as_ref()) };
    println!(
        "DllGetClassObject: clsid {:?}, iid {:?} - not implemented",
        clsid.map(|g| g.to_u128()),
        iid.map(|g| g.to_u128()),
    );
    CLASS_E_CLASSNOTAVAILABLE
}
