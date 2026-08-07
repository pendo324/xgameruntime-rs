//! Running work on a title's own thread, once it is back in its message loop.
//!
//! Both halves of a pick have to happen on the thread that asked for it. The dialog does because
//! it is modal to that thread's window, and the completion does because a handler invoked
//! anywhere else tries to marshal itself back to the apartment it came from - a context switch
//! this runtime does not implement, and whose failure is fatal to the title. But neither can
//! happen *during* the call that starts the pick: a title expects `PickSaveFileAsync` to return
//! promptly, and expects its completion handler to run some time after it registers one, not
//! from inside the registration.
//!
//! The way to be on a thread later rather than now is that thread's message queue. A
//! message-only window created here belongs to the thread that created it, and a message posted
//! to it comes back through whatever loop the title already runs - same thread, same apartment,
//! and only once the title has finished what it was doing.

use std::ffi::c_void;
use std::sync::OnceLock;

use windows::minwindef::{ATOM, LPARAM, LRESULT, WPARAM};
use windows::windef::HWND;
use windows::winuser::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetWindowLongPtrW, HWND_MESSAGE,
    PostMessageW, RegisterClassW, SetWindowLongPtrW, WM_APP, WNDCLASSW,
};
use windows_core::{HSTRING, PCWSTR};

use crate::diag::stub;

/// The message that carries one piece of deferred work.
const WM_RUN_DEFERRED: u32 = WM_APP;

/// The window class every deferred-work window is created from. Registered once per process and
/// never unregistered, so its name has to outlive the registration - which is what keeps it here
/// rather than in the function that uses it.
static CLASS_NAME: OnceLock<HSTRING> = OnceLock::new();

/// Arranges for `work` to run on this thread, after the caller has returned to its message loop.
///
/// Returns whether the work was queued. A caller that gets `false` has to deal with the work
/// itself, because nothing else will.
pub(super) fn run_later(work: Box<dyn FnOnce()>) -> bool {
    let class_name = CLASS_NAME.get_or_init(|| {
        let name = HSTRING::from("XodusDeferredWork");
        let class = WNDCLASSW {
            lpfnWndProc: Some(deferred_wnd_proc),
            lpszClassName: PCWSTR::from_raw(name.as_ptr()),
            ..Default::default()
        };
        // SAFETY: `class` is fully initialized, and the name it points at is the string being
        // stored here, which lives for the rest of the process.
        if unsafe { RegisterClassW(&class) } == ATOM(0) {
            stub!("run_later: could not register the deferred-work window class");
        }
        name
    });

    // The work is handed to the window as a raw pointer and reclaimed when its message arrives.
    let work = Box::into_raw(Box::new(work));

    // SAFETY: the class was registered above, and the pointer stored below is reclaimed exactly
    // once, in the window procedure that handles the message posted after it.
    let posted = unsafe {
        let window = CreateWindowExW(
            0,
            PCWSTR::from_raw(class_name.as_ptr()),
            PCWSTR::null(),
            0,
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            None,
            None,
        );
        if window.0.is_null() {
            false
        } else {
            SetWindowLongPtrW(window, GWLP_USERDATA, work as isize);
            if PostMessageW(Some(window), WM_RUN_DEFERRED, WPARAM(0), LPARAM(0)).as_bool() {
                true
            } else {
                let _ = DestroyWindow(window);
                false
            }
        }
    };

    if !posted {
        stub!("run_later: could not queue work on the calling thread");
        // SAFETY: nothing took the pointer, so this reclaims the only reference to it.
        drop(unsafe { Box::from_raw(work) });
    }
    posted
}

/// Runs one piece of deferred work and tears down the window that carried it.
///
/// # Safety
/// Called by the window manager for a window this module created.
unsafe extern "system" fn deferred_wnd_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message != WM_RUN_DEFERRED {
        // SAFETY: forwarding a message this procedure does not handle is always allowed.
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    }

    // SAFETY: the pointer was stored by `run_later` before the message was posted, and this is
    // the only place that reads it - the window is destroyed below, so no second message can
    // arrive for the same work.
    unsafe {
        let stored = GetWindowLongPtrW(window, GWLP_USERDATA);
        SetWindowLongPtrW(window, GWLP_USERDATA, 0);
        let _ = DestroyWindow(window);
        if stored != 0 {
            let work = Box::from_raw(stored as *mut Box<dyn FnOnce()>);
            work();
        }
    }
    LRESULT(0)
}

/// Keeps the opaque-pointer type this module stores in window data honest about its size: an
/// `isize` slot can only hold a thin pointer.
const _: () = assert!(size_of::<*mut c_void>() == size_of::<isize>());
