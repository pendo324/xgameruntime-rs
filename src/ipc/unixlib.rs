//! PE-side half of the unixlib transport: reaches `xodus-service`'s Unix socket by calling
//! out to native Linux code, which no Winsock route can do.
//!
//! Wine's ws2_32 has no AF_UNIX (`dlls/ntdll/unix/socket.c` converts only
//! INET/INET6/IPX/IRDA/UNSPEC), which is the whole reason the loopback TCP transport in the
//! parent module exists. This is the other way across: ntdll exports
//! `__wine_unix_call(handle, code, args)`, whose dispatcher switches to the unix stack and
//! calls `handle[code](args)`. The unix half is `unixlib/src/lib.rs`, shipped as
//! `xgameruntime.so` next to the DLL.
//!
//! # Getting a handle
//!
//! The handle is just the address of the unix half's `__wine_unix_call_funcs` table, and
//! the dispatcher does not care where it came from - it is a raw indexed call. Two ways to
//! learn it, tried in order:
//!
//! 1. `XODUS_UNIXLIB_HANDLE`, published by the unix half's ELF constructor when it is
//!    `LD_PRELOAD`ed. This is how it ships: constructors run before Wine's init, so the
//!    value is in `environ` before ntdll snapshots the PE environment block.
//! 2. `NtQueryVirtualMemory(MemoryWineUnixFuncs)` against this module, the way a real Wine
//!    builtin gets it. Only answers for modules on Wine's `builtin_modules` list, which a
//!    DLL dropped into the prefix is not - so this is the path that would light up if this
//!    ever ships as a builtin, not one that works today.
//!
//! Absence of both is normal and not an error: it just means this runtime is not running
//! under a launcher that set the transport up, and the caller falls back to TCP.

use std::ffi::{CString, c_void};
use std::sync::OnceLock;
use std::time::Duration;

use windows_core::HRESULT;

use crate::E_FAIL;
use crate::diag::diag;

/// Absolute *Unix* path to `xodus-service`'s `xodus.sock`, published by `xodus-cli`
/// (`xodus::ipc::ENV_SOCKET_PATH`). It is deliberately not a `Z:`-rooted DOS path: the
/// consumer is native Linux code on the other side of `__wine_unix_call`, which wants the
/// real path, and Wine's own translation never enters the picture.
const ENV_SOCKET_PATH: &str = "XODUS_SOCKET_PATH";

/// Address of the unix half's dispatch table, as lowercase hex. See the module docs.
const ENV_UNIXLIB_HANDLE: &str = "XODUS_UNIXLIB_HANDLE";

/// Indices into the unix half's `__wine_unix_call_funcs`. Append-only - these are ABI.
const CALL_EXCHANGE: u32 = 0;
const CALL_FETCH_REPLY: u32 = 1;

const STATUS_SUCCESS: u32 = 0;

/// `MEMORY_INFORMATION_CLASS::MemoryWineUnixFuncs` (`include/winternl.h:2460`). Wine-specific,
/// so it is spelled out here rather than pulled from a Windows crate that has no reason to
/// know it.
const MEMORY_WINE_UNIX_FUNCS: u32 = 1000;

/// Mirrors `unixlib::ExchangeParams`. Every field is fixed-width, pointers included, because
/// the two halves are built by different toolchains for different targets.
#[repr(C)]
struct ExchangeParams {
    socket_path: u64,
    request: u64,
    request_len: u64,
    timeout_ms: u64,
    msg_type: u32,
    reply_type: u32,
    reply_len: u64,
    reply_handle: u64,
}

/// Mirrors `unixlib::FetchReplyParams`.
#[repr(C)]
struct FetchReplyParams {
    reply_handle: u64,
    buffer: u64,
    buffer_len: u64,
}

type WineUnixCall = unsafe extern "system" fn(handle: u64, code: u32, args: *mut c_void) -> u32;

/// The resolved transport, or `None` if this process has no unixlib available.
struct Transport {
    call: WineUnixCall,
    handle: u64,
    socket_path: CString,
}

// The three fields are a code pointer, an integer, and an immutable string: shared use from
// any thread is what the Wine dispatcher already assumes.
unsafe impl Send for Transport {}
unsafe impl Sync for Transport {}

fn transport() -> Option<&'static Transport> {
    static TRANSPORT: OnceLock<Option<Transport>> = OnceLock::new();
    TRANSPORT.get_or_init(resolve).as_ref()
}

/// Whether the unixlib transport is usable in this process. Cheap after the first call.
pub(crate) fn available() -> bool {
    transport().is_some()
}

fn resolve() -> Option<Transport> {
    let socket_path = match std::env::var(ENV_SOCKET_PATH) {
        Ok(path) if !path.is_empty() => path,
        _ => {
            diag!("unixlib: {ENV_SOCKET_PATH} unset - falling back to TCP");
            return None;
        }
    };
    let socket_path = CString::new(socket_path).ok()?;

    let call = wine_unix_call()?;
    let handle = handle_from_env().or_else(handle_from_builtin_query)?;

    diag!("unixlib: ready, handle={handle:#x} socket={socket_path:?}");
    Some(Transport {
        call,
        handle,
        socket_path,
    })
}

/// Resolves ntdll's unix-call entry point, which Wine exports two different ways.
///
/// `__wine_unix_call` is a plain `stdcall` function, exported from Wine 5.14 until
/// commit 127650c293b (2024-05-23, "ntdll: Make __wine_unix_call() an inline function",
/// released in 9.10) dropped it from `ntdll.spec`. Since 90adeb125f3 (2022-11-30, released
/// in 8.0) there is `__wine_unix_call_dispatcher` instead, which is not a function but a
/// *pointer variable* holding one - `include/wine/unixlib.h` declares it as
/// `extern NTSTATUS (WINAPI *__wine_unix_call_dispatcher)(...)`, and the PE-side
/// `__wine_unix_call` is now an inline that dereferences it. So the second form needs one
/// extra load, and getting that wrong means calling the address of a pointer as if it were
/// code.
///
/// Trying both in this order covers every Wine that has the funcs-array unixlib interface at
/// all (7.0 onward, after 4f58d8144c5 removed `__wine_init_unix_lib`), which is every Proton
/// anyone still launches a GDK title with. The signature itself has not changed in that
/// window.
///
/// Absent entirely on real Windows, and on a Wine predating unixlibs - both mean "no unixlib
/// here", which is a fallback to TCP, not a failure.
fn wine_unix_call() -> Option<WineUnixCall> {
    use windows_sys::libloaderapi::{GetModuleHandleA, GetProcAddress};

    let ntdll = unsafe { GetModuleHandleA(c"ntdll.dll".as_ptr() as *const u8) };
    if ntdll.is_null() {
        return None;
    }

    if let Some(proc) = unsafe { GetProcAddress(ntdll, c"__wine_unix_call".as_ptr() as *const u8) }
    {
        diag!("unixlib: using ntdll!__wine_unix_call");
        return Some(unsafe { std::mem::transmute::<_, WineUnixCall>(proc) });
    }

    let slot =
        unsafe { GetProcAddress(ntdll, c"__wine_unix_call_dispatcher".as_ptr() as *const u8) };
    let slot = slot.or_else(|| {
        diag!("unixlib: ntdll exports no unix-call entry point - falling back to TCP");
        None
    })?;
    // A data export: the symbol's address is where the function pointer lives, not the
    // function. Wine fills this in during ntdll init, so a null here means we ran too early.
    let dispatcher = unsafe { *(slot as *const *const c_void) };
    if dispatcher.is_null() {
        diag!("unixlib: __wine_unix_call_dispatcher is null - falling back to TCP");
        return None;
    }
    diag!("unixlib: using ntdll!__wine_unix_call_dispatcher");
    Some(unsafe { std::mem::transmute::<_, WineUnixCall>(dispatcher) })
}

/// The `LD_PRELOAD` path: the unix half published its own table address before Wine started.
fn handle_from_env() -> Option<u64> {
    let raw = std::env::var(ENV_UNIXLIB_HANDLE).ok()?;
    match u64::from_str_radix(raw.trim(), 16) {
        Ok(handle) if handle != 0 => Some(handle),
        _ => {
            diag!("unixlib: {ENV_UNIXLIB_HANDLE}={raw:?} is not a non-zero hex address");
            None
        }
    }
}

/// The builtin path: ask Wine for the unixlib it loaded alongside this module. Only answers
/// for modules Wine loaded as builtins, so today this is expected to fail - it is here so a
/// future builtin deployment needs no code change, and so the diagnostic says which of the
/// two mechanisms was missing.
fn handle_from_builtin_query() -> Option<u64> {
    use windows_sys::libloaderapi::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        GetModuleHandleExA,
    };

    type NtQueryVirtualMemory = unsafe extern "system" fn(
        process: *mut c_void,
        address: *mut c_void,
        class: u32,
        buffer: *mut c_void,
        len: usize,
        result_len: *mut usize,
    ) -> i32;

    let mut module = std::ptr::null_mut();
    let ok = unsafe {
        GetModuleHandleExA(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            handle_from_builtin_query as *const u8,
            &mut module,
        )
    };
    if ok == 0 || module.is_null() {
        return None;
    }

    let query: NtQueryVirtualMemory = {
        use windows_sys::libloaderapi::{GetModuleHandleA, GetProcAddress};
        let ntdll = unsafe { GetModuleHandleA(c"ntdll.dll".as_ptr() as *const u8) };
        if ntdll.is_null() {
            return None;
        }
        let proc = unsafe { GetProcAddress(ntdll, c"NtQueryVirtualMemory".as_ptr() as *const u8) }?;
        unsafe { std::mem::transmute::<_, NtQueryVirtualMemory>(proc) }
    };

    let mut handle: u64 = 0;
    let mut result_len: usize = 0;
    let status = unsafe {
        query(
            (-1isize) as *mut c_void, // GetCurrentProcess()
            module as *mut c_void,
            MEMORY_WINE_UNIX_FUNCS,
            (&raw mut handle) as *mut c_void,
            size_of::<u64>(),
            &mut result_len,
        )
    };
    if status != 0 || handle == 0 {
        diag!("unixlib: MemoryWineUnixFuncs query returned {status:#x} (not a Wine builtin)");
        return None;
    }
    Some(handle)
}

/// One request/response round trip over the Unix socket, with the same signature and framing
/// contract as the parent module's `request_with_timeout`.
pub(crate) fn request_with_timeout(
    msg_type: u16,
    payload: &[u8],
    io_timeout: Duration,
) -> Result<(u16, Vec<u8>), HRESULT> {
    let transport = transport().ok_or_else(|| {
        diag!("unixlib: request msg_type={msg_type} with no transport");
        E_FAIL
    })?;

    let mut params = ExchangeParams {
        socket_path: transport.socket_path.as_ptr() as u64,
        request: payload.as_ptr() as u64,
        request_len: payload.len() as u64,
        timeout_ms: io_timeout.as_millis().max(1) as u64,
        msg_type: msg_type as u32,
        reply_type: 0,
        reply_len: 0,
        reply_handle: 0,
    };

    let status = unsafe {
        (transport.call)(
            transport.handle,
            CALL_EXCHANGE,
            (&raw mut params) as *mut c_void,
        )
    };
    if status != STATUS_SUCCESS {
        diag!("unixlib: request msg_type={msg_type} exchange failed, status={status:#x}");
        return Err(E_FAIL);
    }

    // The reply is parked on the unix side; this second call copies it out and releases it,
    // whether or not the copy succeeds.
    let mut body = vec![0u8; params.reply_len as usize];
    let mut fetch = FetchReplyParams {
        reply_handle: params.reply_handle,
        buffer: body.as_mut_ptr() as u64,
        buffer_len: body.len() as u64,
    };
    let status = unsafe {
        (transport.call)(
            transport.handle,
            CALL_FETCH_REPLY,
            (&raw mut fetch) as *mut c_void,
        )
    };
    if status != STATUS_SUCCESS {
        diag!("unixlib: request msg_type={msg_type} fetch failed, status={status:#x}");
        return Err(E_FAIL);
    }

    let reply_type = params.reply_type as u16;
    diag!(
        "unixlib: request msg_type={msg_type} succeeded, reply_type={reply_type} size={}",
        body.len()
    );
    Ok((reply_type, body))
}

#[cfg(test)]
mod tests {
    use super::{ExchangeParams, FetchReplyParams};

    /// The unix half declares these structs independently (`unixlib/src/lib.rs`), compiled by
    /// a different toolchain for a different target. Nothing at build time links the two, so
    /// a field reordered on one side would be silently misread on the other - at runtime,
    /// inside a syscall frame, with a game's credentials in the buffer. These numbers are the
    /// contract; changing one means changing both halves deliberately.
    #[test]
    fn exchange_params_layout_is_the_agreed_abi() {
        assert_eq!(size_of::<ExchangeParams>(), 56);
        assert_eq!(align_of::<ExchangeParams>(), 8);
        assert_eq!(std::mem::offset_of!(ExchangeParams, socket_path), 0);
        assert_eq!(std::mem::offset_of!(ExchangeParams, request), 8);
        assert_eq!(std::mem::offset_of!(ExchangeParams, request_len), 16);
        assert_eq!(std::mem::offset_of!(ExchangeParams, timeout_ms), 24);
        assert_eq!(std::mem::offset_of!(ExchangeParams, msg_type), 32);
        assert_eq!(std::mem::offset_of!(ExchangeParams, reply_type), 36);
        assert_eq!(std::mem::offset_of!(ExchangeParams, reply_len), 40);
        assert_eq!(std::mem::offset_of!(ExchangeParams, reply_handle), 48);
    }

    #[test]
    fn fetch_reply_params_layout_is_the_agreed_abi() {
        assert_eq!(size_of::<FetchReplyParams>(), 24);
        assert_eq!(align_of::<FetchReplyParams>(), 8);
        assert_eq!(std::mem::offset_of!(FetchReplyParams, reply_handle), 0);
        assert_eq!(std::mem::offset_of!(FetchReplyParams, buffer), 8);
        assert_eq!(std::mem::offset_of!(FetchReplyParams, buffer_len), 16);
    }

    /// The premise of the whole transport, end to end and under Wine: a handle obtained from
    /// `LD_PRELOAD` (not from `MemoryWineUnixFuncs`, which only answers for builtins) is
    /// accepted by `__wine_unix_call`, and a round trip reaches a real AF_UNIX socket.
    ///
    /// Ignored by default because it needs a listener on `XODUS_SOCKET_PATH` and the library
    /// preloaded - see `unixlib/README.md` for the one-liner that sets both up.
    #[test]
    #[ignore = "needs XODUS_SOCKET_PATH + LD_PRELOAD; see unixlib/README.md"]
    fn round_trip_over_the_unix_socket() {
        assert!(
            super::available(),
            "no unixlib transport in this environment"
        );
        let (reply_type, body) =
            super::request_with_timeout(3, b"<Req/>", std::time::Duration::from_secs(5))
                .expect("exchange failed");
        assert_eq!(reply_type, 4);
        assert_eq!(body, b"<Resp>hello</Resp>");
    }

    /// Absence of a launcher-provided transport must read as "use TCP", not as an error: the
    /// test process has neither env var set, which is exactly the shipped-on-Windows case.
    #[test]
    fn unavailable_without_launcher_environment() {
        assert!(!super::available());
    }
}
