//! Opt-in diagnostic logging.
//!
//! The runtime traces a lot: every dispatched task-queue callback, every async begin and
//! completion, every IPC round trip. That is what makes a stalled title diagnosable, but it
//! is also a locked stderr write on paths that run hundreds of times a second, so it is off
//! unless `XODUS_DIAG` is set to something other than `0`.
//!
//! Unconditional logging is reserved for things a user should see without opting in: an
//! unimplemented API a title actually called, and a failure that changes what the title does.

use std::sync::OnceLock;
use std::time::Instant;

// Declared by hand rather than pulled from `windows-sys` because these two are the whole
// requirement, and the alternative is taking a feature dependency on the console and debug
// API surfaces to reach them.
unsafe extern "system" {
    fn GetStdHandle(nStdHandle: u32) -> isize;
    fn OutputDebugStringA(lpOutputString: *const u8);
}

/// Writes one already-formatted line somewhere a developer will actually see it.
///
/// The obvious implementation - `eprintln!` - silently discards everything here. Titles that
/// load this runtime are `IMAGE_SUBSYSTEM_WINDOWS_GUI` and are not launched from a console, so
/// `GetStdHandle(STD_ERROR_HANDLE)` is NULL and Rust's stderr writes go nowhere. Wine's own
/// TRACEs are visible in the same session only because they bypass Win32 and write to unix
/// fd 2 directly, which made this failure look like "the DLL never loaded" rather than "the
/// DLL is loaded and mute".
///
/// So: use stderr when there is one, and fall back to `OutputDebugStringA`, which Wine prints
/// to that same unix stderr (as `trace:debugstr`) under `WINEDEBUG=+debugstr`, and which a
/// debugger picks up on real Windows.
pub(crate) fn emit(line: &str) {
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    const INVALID_HANDLE_VALUE: isize = -1;

    static HAS_STDERR: OnceLock<bool> = OnceLock::new();
    let has_stderr = *HAS_STDERR.get_or_init(|| {
        // SAFETY: `STD_ERROR_HANDLE` is the fixed, well-known Win32 constant `-12`.
        let handle = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
        handle != 0 && handle != INVALID_HANDLE_VALUE
    });

    if has_stderr {
        eprintln!("{line}");
    } else {
        // `OutputDebugStringA` takes a C string, and a NUL anywhere in a format argument would
        // truncate the line rather than corrupt anything, so replacing is enough.
        let mut bytes = line.replace('\0', "?").into_bytes();
        bytes.push(b'\n');
        bytes.push(0);
        // SAFETY: `bytes` is a live local buffer, just NUL-terminated by the push above.
        unsafe { OutputDebugStringA(bytes.as_ptr()) };
    }
}

pub(crate) fn enabled() -> bool {
    static DIAG: OnceLock<bool> = OnceLock::new();
    *DIAG.get_or_init(|| {
        std::env::var("XODUS_DIAG")
            .map(|v| v != "0")
            .unwrap_or(false)
    })
}

/// Origin for diagnostic timestamps, so every line carries a comparable millisecond
/// stamp. Latency is the whole question this logging exists to answer - "which port is
/// starving, and by how long" - and untimestamped lines cannot answer it.
pub(crate) fn now_ms() -> u128 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_millis()
}

/// Writes one diagnostic line, stamped with the elapsed milliseconds and the calling thread,
/// when `XODUS_DIAG` is set. Arguments are only evaluated if it is.
macro_rules! diag {
    ($($arg:tt)*) => {
        if $crate::diag::enabled() {
            $crate::diag::emit(&format!(
                "[diag t={} {:?}] {}",
                $crate::diag::now_ms(),
                std::thread::current().id(),
                format_args!($($arg)*)
            ));
        }
    };
}

/// Reports an API this runtime does not implement, always. A title calling a stub is a
/// concrete gap between it and a working session, so unlike [`diag!`] this is not gated:
/// the line is the thing a user needs to see to know what is missing.
macro_rules! stub {
    ($($arg:tt)*) => {
        $crate::diag::emit(&format!(
            "[stub {:?}] {}",
            std::thread::current().id(),
            format_args!($($arg)*)
        ));
    };
}

pub(crate) use {diag, stub};
