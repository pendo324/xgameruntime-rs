//! Wine unixlib half of `xgameruntime.dll`'s IPC: the native-Linux code that can reach
//! `xodus-service`'s Unix socket, which Wine's Winsock cannot.
//!
//! Wine's ws2_32 has no AF_UNIX at all - `dlls/ntdll/unix/socket.c`'s sockaddr conversion
//! handles only INET/INET6/IPX/IRDA/UNSPEC - so a PE cannot open `xodus.sock` by any
//! Winsock route. A unixlib is the sanctioned way across: the PE side calls ntdll's
//! `__wine_unix_call(handle, code, args)`, whose dispatcher switches to the unix stack and
//! does `callq *(%r10,%rdx,8)` - the handle is simply a pointer to [`__wine_unix_call_funcs`]
//! and `code` indexes it. Nothing validates where that pointer came from, which is why the
//! `LD_PRELOAD` discovery path in [`publish_handle`] works without this being a Wine builtin.
//!
//! # ABI
//!
//! The parameter structs are `#[repr(C)]` and shared verbatim with the PE side
//! (`src/ipc/unixlib.rs`). Both halves are x86_64, but they are compiled by different
//! toolchains for different targets, so every field is a fixed-width integer - pointers
//! included, passed as `u64` - and nothing here relies on Rust type layout agreeing across
//! the boundary.
//!
//! # Safety
//!
//! Entry points must never unwind into Wine (`panic = "abort"`) and must never block
//! forever: a wedged service would otherwise hang a game thread inside a syscall frame,
//! where Wine cannot interrupt it. Every socket carries `SO_SNDTIMEO`/`SO_RCVTIMEO`.

#![warn(clippy::undocumented_unsafe_blocks)]
// zig cc's linker doesn't recognize the GNU-ld `-O1` hint rustc passes by default when
// linking with an ld/lld-flavored cc, and warns (harmlessly) instead of accepting it.
#![allow(linker_messages)]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};

/// `NTSTATUS`, as the dispatcher expects it back.
pub type NtStatus = u32;

const STATUS_SUCCESS: NtStatus = 0;
const STATUS_INVALID_PARAMETER: NtStatus = 0xC000_000D;
const STATUS_BUFFER_TOO_SMALL: NtStatus = 0xC000_0023;
const STATUS_CONNECTION_REFUSED: NtStatus = 0xC000_0236;
const STATUS_OBJECT_NAME_NOT_FOUND: NtStatus = 0xC000_0034;
const STATUS_ACCESS_DENIED: NtStatus = 0xC000_0022;
const STATUS_UNEXPECTED_IO_ERROR: NtStatus = 0xC000_00E9;
const STATUS_IO_TIMEOUT: NtStatus = 0xC000_00B5;
/// The reply's leading magic was not the one the framing promises - we are talking to
/// something that is not `xodus-service`.
const STATUS_INVALID_NETWORK_RESPONSE: NtStatus = 0xC000_0C01;

/// Mirrors `xodus-service::connection::MAX_MESSAGE_SIZE`, so a bogus declared size can
/// never make us allocate wildly on the unix side.
const MAX_MESSAGE_SIZE: u64 = 16 * 1024 * 1024;

/// `XML_MAGIC_V2` - the v2 framing (`u32` payload size) both halves speak.
const XML_MAGIC_V2: u32 = 0x5944_5358;

/// One request/response round trip. See [`ExchangeParams`].
pub const CALL_EXCHANGE: u32 = 0;
/// Copy out (and release) the reply an [`CALL_EXCHANGE`] parked. See [`FetchReplyParams`].
pub const CALL_FETCH_REPLY: u32 = 1;

/// Arguments to [`CALL_EXCHANGE`].
///
/// The reply is not copied out here: its size is unknown until the service answers, and
/// sizing a PE-side buffer for the worst case ([`MAX_MESSAGE_SIZE`]) to hold a typical
/// few-hundred-byte token would be absurd. Instead the reply is parked on this side and
/// `reply_handle`/`reply_len` describe it, for a following [`CALL_FETCH_REPLY`].
#[repr(C)]
pub struct ExchangeParams {
    /// `*const c_char` - NUL-terminated absolute path to `xodus.sock`.
    pub socket_path: u64,
    /// `*const u8` - the XML request body, without framing.
    pub request: u64,
    pub request_len: u64,
    /// Applied to both directions as `SO_SNDTIMEO`/`SO_RCVTIMEO`. Zero means no timeout,
    /// which no caller should ask for.
    pub timeout_ms: u64,
    pub msg_type: u32,
    /// Out: the reply's message type.
    pub reply_type: u32,
    /// Out: the reply body's length.
    pub reply_len: u64,
    /// Out: opaque owner of the parked reply. Non-zero on success, and the caller **must**
    /// pass it to [`CALL_FETCH_REPLY`] exactly once or leak it.
    pub reply_handle: u64,
}

/// Arguments to [`CALL_FETCH_REPLY`]. Always releases `reply_handle`, including on error,
/// so a PE-side caller cannot leak the parked reply by mishandling a failure.
#[repr(C)]
pub struct FetchReplyParams {
    pub reply_handle: u64,
    /// `*mut u8` - must have room for `ExchangeParams::reply_len` bytes.
    pub buffer: u64,
    pub buffer_len: u64,
}

/// A reply parked between [`CALL_EXCHANGE`] and [`CALL_FETCH_REPLY`]. Allocated and freed
/// entirely on this side - the PE's allocator never sees it.
struct ParkedReply {
    body: Vec<u8>,
}

/// # Safety
/// `args` must point to a valid [`ExchangeParams`].
unsafe extern "C" fn exchange(args: *mut c_void) -> NtStatus {
    // SAFETY: `args` is `exchange`'s own `# Safety` precondition.
    let Some(params) = (unsafe { (args as *mut ExchangeParams).as_mut() }) else {
        return STATUS_INVALID_PARAMETER;
    };
    params.reply_handle = 0;
    params.reply_len = 0;
    params.reply_type = 0;

    if params.socket_path == 0 || params.timeout_ms == 0 {
        return STATUS_INVALID_PARAMETER;
    }
    if params.request_len > MAX_MESSAGE_SIZE || (params.request_len > 0 && params.request == 0) {
        return STATUS_INVALID_PARAMETER;
    }
    // SAFETY: `params.socket_path` was checked non-zero above, and `ExchangeParams`'s own
    // contract is that it points to a nul-terminated path.
    let path = unsafe { CStr::from_ptr(params.socket_path as *const c_char) };
    // SAFETY: `params.request`/`request_len` were checked consistent (null iff zero-length,
    // within `MAX_MESSAGE_SIZE`) above.
    let request =
        unsafe { std::slice::from_raw_parts(params.request as *const u8, params.request_len as _) };

    let socket = match Socket::connect(path, params.timeout_ms) {
        Ok(socket) => socket,
        Err(status) => return status,
    };

    let mut framed = Vec::with_capacity(request.len() + 10);
    framed.extend_from_slice(&XML_MAGIC_V2.to_le_bytes());
    framed.extend_from_slice(&(params.msg_type as u16).to_le_bytes());
    framed.extend_from_slice(&(request.len() as u32).to_le_bytes());
    framed.extend_from_slice(request);
    if let Err(status) = socket.write_all(&framed) {
        return status;
    }

    let mut header = [0u8; 10];
    if let Err(status) = socket.read_exact(&mut header) {
        return status;
    }
    if u32::from_le_bytes([header[0], header[1], header[2], header[3]]) != XML_MAGIC_V2 {
        return STATUS_INVALID_NETWORK_RESPONSE;
    }
    let reply_type = u16::from_le_bytes([header[4], header[5]]);
    let size = u32::from_le_bytes([header[6], header[7], header[8], header[9]]) as u64;
    if size > MAX_MESSAGE_SIZE {
        return STATUS_INVALID_NETWORK_RESPONSE;
    }

    let mut body = vec![0u8; size as usize];
    if let Err(status) = socket.read_exact(&mut body) {
        return status;
    }

    params.reply_type = reply_type as u32;
    params.reply_len = size;
    params.reply_handle = Box::into_raw(Box::new(ParkedReply { body })) as u64;
    STATUS_SUCCESS
}

/// # Safety
/// `args` must point to a valid [`FetchReplyParams`] whose `reply_handle` came from a
/// successful [`exchange`] and has not been fetched already.
unsafe extern "C" fn fetch_reply(args: *mut c_void) -> NtStatus {
    // SAFETY: `args` is `fetch_reply`'s own `# Safety` precondition.
    let Some(params) = (unsafe { (args as *mut FetchReplyParams).as_mut() }) else {
        return STATUS_INVALID_PARAMETER;
    };
    if params.reply_handle == 0 {
        return STATUS_INVALID_PARAMETER;
    }
    // SAFETY: `params.reply_handle` is `fetch_reply`'s own `# Safety` precondition - it
    // must come from a successful `exchange` and not have been fetched already.
    // Reclaimed unconditionally: every path below this point frees it, so a caller that
    // sized its buffer wrong retries with a fresh exchange rather than leaking this one.
    let parked = unsafe { Box::from_raw(params.reply_handle as *mut ParkedReply) };
    params.reply_handle = 0;

    if params.buffer_len < parked.body.len() as u64 {
        return STATUS_BUFFER_TOO_SMALL;
    }
    if parked.body.is_empty() {
        return STATUS_SUCCESS;
    }
    if params.buffer == 0 {
        return STATUS_INVALID_PARAMETER;
    }
    // SAFETY: `params.buffer_len >= parked.body.len()` was checked above.
    unsafe {
        std::ptr::copy_nonoverlapping(
            parked.body.as_ptr(),
            params.buffer as *mut u8,
            parked.body.len(),
        );
    }
    STATUS_SUCCESS
}

/// The dispatch table `__wine_unix_call` indexes. Its address *is* the unixlib handle.
///
/// Order is ABI: [`CALL_EXCHANGE`] and [`CALL_FETCH_REPLY`] are indices into this array, so
/// entries may only be appended.
#[unsafe(no_mangle)]
pub static __wine_unix_call_funcs: [unsafe extern "C" fn(*mut c_void) -> NtStatus; 2] =
    [exchange, fetch_reply];

/// Wow64 thunks. A 32-bit PE would need its own table (the `u64` pointer fields would be
/// half-width on that side); this runtime is x86_64-only, so pointing at the same table is
/// correct rather than a shortcut.
#[unsafe(no_mangle)]
pub static __wine_unix_call_wow64_funcs: [unsafe extern "C" fn(*mut c_void) -> NtStatus; 2] =
    [exchange, fetch_reply];

/// Env var carrying [`__wine_unix_call_funcs`]'s address, as lowercase hex, for the
/// `LD_PRELOAD` discovery path.
const ENV_UNIXLIB_HANDLE: &str = "XODUS_UNIXLIB_HANDLE";

/// Publishes the unixlib handle into the environment at load time.
///
/// When Wine loads this as a builtin's unixlib, the PE side gets the handle from
/// `NtQueryVirtualMemory(MemoryWineUnixFuncs)` and this is redundant. When it is instead
/// `LD_PRELOAD`ed - which is how this ships, since making the PE a Wine builtin would mean
/// stamping a builtin signature into it and installing it on `WINEDLLPATH` - that query
/// fails, because it only answers for modules on Wine's `builtin_modules` list. Publishing
/// the address here gives the PE side something to find.
///
/// This runs before Wine's own init (ELF constructors run before `main`), so the value is
/// in `environ` by the time ntdll snapshots it into the PE environment block.
extern "C" fn publish_handle() {
    let address = format!("{:x}", (&raw const __wine_unix_call_funcs) as usize);
    let (Ok(name), Ok(value)) = (CString::new(ENV_UNIXLIB_HANDLE), CString::new(address)) else {
        return;
    };
    // SAFETY: `name`/`value` are valid, nul-terminated `CString`s and this runs
    // single-threaded (an ELF constructor, before any other code in the process).
    unsafe { libc::setenv(name.as_ptr(), value.as_ptr(), 1) };
}

#[used]
#[unsafe(link_section = ".init_array")]
static PUBLISH_HANDLE: extern "C" fn() = publish_handle;

/// An owned AF_UNIX socket with timeouts applied, and EINTR-tolerant I/O.
///
/// Wine leans on signals heavily (its suspend/APC machinery is signal-driven), so a plain
/// `read`/`write` here would see `EINTR` routinely rather than exceptionally.
struct Socket(c_int);

impl Socket {
    fn connect(path: &CStr, timeout_ms: u64) -> Result<Self, NtStatus> {
        let bytes = path.to_bytes();
        // SAFETY: an all-zero `sockaddr_un` is a valid value of that type.
        let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        // The final byte must stay NUL, hence the strict `>=`.
        if bytes.len() >= addr.sun_path.len() {
            return Err(STATUS_INVALID_PARAMETER);
        }
        addr.sun_family = libc::AF_UNIX as _;
        for (slot, byte) in addr.sun_path.iter_mut().zip(bytes) {
            *slot = *byte as c_char;
        }

        // SAFETY: `libc::socket` has no pointer arguments to uphold invariants for.
        let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
        if fd < 0 {
            return Err(STATUS_UNEXPECTED_IO_ERROR);
        }
        let socket = Socket(fd);

        let timeout = libc::timeval {
            tv_sec: (timeout_ms / 1000) as _,
            tv_usec: ((timeout_ms % 1000) * 1000) as _,
        };
        for option in [libc::SO_RCVTIMEO, libc::SO_SNDTIMEO] {
            // SAFETY: `timeout` is a valid `libc::timeval` and its size matches the `optlen`
            // passed alongside it.
            unsafe {
                libc::setsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    option,
                    (&raw const timeout) as *const c_void,
                    size_of::<libc::timeval>() as libc::socklen_t,
                );
            }
        }

        loop {
            // SAFETY: `addr` is a valid `sockaddr_un` and its size matches the `addrlen`
            // passed alongside it.
            let ret = unsafe {
                libc::connect(
                    fd,
                    (&raw const addr) as *const libc::sockaddr,
                    size_of::<libc::sockaddr_un>() as libc::socklen_t,
                )
            };
            if ret == 0 {
                return Ok(socket);
            }
            // Distinguishing these three matters more than it looks. The PE side only ever
            // sees the status, and folding them together produces "connection refused" for a
            // socket nobody is refusing - which reads as "the service is down" when the real
            // answer is that the game cannot see the path at all. Games launch inside umu's
            // pressure-vessel container, with its own mount namespace, so "the socket is right
            // there" on the host and `ENOENT` in the title are entirely consistent.
            return Err(match errno() {
                libc::EINTR => continue,
                libc::EAGAIN | libc::EINPROGRESS | libc::ETIMEDOUT => STATUS_IO_TIMEOUT,
                libc::ENOENT => STATUS_OBJECT_NAME_NOT_FOUND,
                libc::EACCES | libc::EPERM => STATUS_ACCESS_DENIED,
                _ => STATUS_CONNECTION_REFUSED,
            });
        }
    }

    fn write_all(&self, mut buf: &[u8]) -> Result<(), NtStatus> {
        while !buf.is_empty() {
            // SAFETY: `buf` is a valid, live `&[u8]` for the duration of this call.
            let written = unsafe {
                libc::send(
                    self.0,
                    buf.as_ptr() as *const c_void,
                    buf.len(),
                    libc::MSG_NOSIGNAL,
                )
            };
            if written > 0 {
                buf = &buf[written as usize..];
                continue;
            }
            return Err(match errno() {
                libc::EINTR => continue,
                // `SO_*TIMEO` reports a timeout as EAGAIN (== EWOULDBLOCK on Linux).
                libc::EAGAIN => STATUS_IO_TIMEOUT,
                _ => STATUS_UNEXPECTED_IO_ERROR,
            });
        }
        Ok(())
    }

    fn read_exact(&self, mut buf: &mut [u8]) -> Result<(), NtStatus> {
        while !buf.is_empty() {
            // SAFETY: `buf` is a valid, live `&mut [u8]` for the duration of this call.
            let read = unsafe { libc::recv(self.0, buf.as_mut_ptr() as *mut c_void, buf.len(), 0) };
            if read > 0 {
                buf = &mut buf[read as usize..];
                continue;
            }
            // A clean close mid-message is a truncated reply, not a successful read.
            if read == 0 {
                return Err(STATUS_UNEXPECTED_IO_ERROR);
            }
            return Err(match errno() {
                libc::EINTR => continue,
                // `SO_*TIMEO` reports a timeout as EAGAIN (== EWOULDBLOCK on Linux).
                libc::EAGAIN => STATUS_IO_TIMEOUT,
                _ => STATUS_UNEXPECTED_IO_ERROR,
            });
        }
        Ok(())
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        // SAFETY: `self.0` is `Socket`'s only owner and closed exactly once, here.
        unsafe { libc::close(self.0) };
    }
}

fn errno() -> c_int {
    // SAFETY: `__errno_location` always returns a valid pointer to the calling thread's
    // errno.
    unsafe { *libc::__errno_location() }
}

#[cfg(test)]
mod tests {
    use super::{ExchangeParams, FetchReplyParams};

    /// The mirror of `src/ipc/unixlib.rs`'s layout test. Both halves assert the same numbers
    /// independently, so a change to either one fails on its own side rather than only
    /// showing up as corruption at runtime.
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
}
