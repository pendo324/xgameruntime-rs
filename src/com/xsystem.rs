use super::E_NOTIMPL;
use crate::results::*;
use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::OnceLock;
use windows_core::{GUID, HRESULT, IUnknown, implement, interface};
/// `IXSystemImpl`'s own IID, reused as the coclass id (same pattern as `CLSID_XUSER`) -
/// see `xsystem.idl`. Confirmed as the class GDK/XSAPI queries at startup: traced in Wine
/// logs as one of the unimplemented `query_api_impl` classes before this was added. The
/// console id string and "RETAIL" sandbox below are fixed values, since Xodus has no
/// sandbox concept of its own to source them from and titles only test their presence.
pub const CLSID_XSYSTEM: GUID = GUID::from_u128(0xe349bd1a_fc20_4e40_b99c_4178cc6b409f);

const X_SYSTEM_CONSOLE_ID_BYTES: i32 = 39;
const X_SYSTEM_SANDBOX_ID_MAX_BYTES: i32 = 16;

/// `XSystemHandle`, opaque GDK handle (`typedef void *XSystemHandle` in xsystem.idl).
pub type XSystemHandle = *mut c_void;

/// `XSystemHandleCallbackReason` / `XSystemHandleType` are `UINT32` enums; they arrive as
/// plain integer args to the callback and this crate never inspects them.
pub type XSystemHandleType = u32;
pub type XSystemHandleCallbackReason = u32;

/// `void __stdcall XSystemHandleCallback(XSystemHandle, XSystemHandleType,
/// XSystemHandleCallbackReason, void *context)` - see xsystem.idl.
pub type XSystemHandleCallback = Option<
    unsafe extern "system" fn(
        XSystemHandle,
        XSystemHandleType,
        XSystemHandleCallbackReason,
        *mut c_void,
    ),
>;

// IXSystemImpl / 2 / 3 / 4 / 5. XSAPI (statically linked into titles that bundle it,
// e.g. Minecraft Bedrock) queries `CLSID_XSYSTEM` and asks for the *newer* interface IIDs
// (observed live: `IXSystemImpl4`, IID dadc2895-34b0-4ef5-a83e-45114d629b80), not just the
// base `IXSystemImpl`. Each interface tier in `xsystem.idl` has its own IID, and windows-rs
// needs each IID as its own `#[interface]` (same pattern as `xuser.rs`'s IXUserImpl1-6), so
// the whole chain is declared here. The two empty tiers
// (`IXSystemImpl2`/`IXSystemImpl5`, no new methods in the IDL) exist purely so their IIDs QI
// successfully.

#[interface("e349bd1a-fc20-4e40-b99c-4178cc6b409f")]
pub unsafe trait IXSystem: IUnknown {
    pub unsafe fn XSystemGetConsoleId(
        &self,
        consoleIdSize: i32,
        consoleId: *mut c_char,
        consoleIdUsed: *mut usize,
    ) -> HRESULT;
    pub unsafe fn XSystemGetXboxLiveSandboxId(
        &self,
        sandboxIdSize: i32,
        sandboxId: *mut c_char,
        sandboxIdUsed: *mut usize,
    ) -> HRESULT;
    pub unsafe fn XSystemGetAppSpecificDeviceId(
        &self,
        appSpecificDeviceIdSize: i32,
        appSpecificDeviceId: *mut c_char,
        appSpecificDeviceIdUsed: *mut usize,
    ) -> HRESULT;
}

#[interface("6fd71f09-7513-49f0-89bc-bfaf5df6f852")]
pub unsafe trait IXSystem2: IXSystem {}

#[interface("67ce4bfc-b1d1-4ac7-bc3a-cb9219a97a85")]
pub unsafe trait IXSystem3: IXSystem2 {
    pub unsafe fn XSystemHandleTrack(
        &self,
        callback: XSystemHandleCallback,
        context: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XSystemIsHandleValid(&self, handle: XSystemHandle) -> u8;
}

#[interface("dadc2895-34b0-4ef5-a83e-45114d629b80")]
pub unsafe trait IXSystem4: IXSystem3 {
    pub unsafe fn XSystemAllowFullDownloadBandwidth(&self, enable: u8);
}

#[interface("1861cf2e-e18b-4834-a9f5-b4a4e6efb4cf")]
pub unsafe trait IXSystem5: IXSystem4 {}

/// `IXSystemImpl` - the GDK console-identity interface. Without this, `XblInitialize`
/// (statically linked into titles that bundle XSAPI, e.g. Minecraft Bedrock) queries
/// `CLSID_XSYSTEM` for the sandbox/console/device id it needs before constructing any Xbox
/// Live request, gets `E_NOTIMPL`, and silently bails - the title never attempts an XSTS
/// exchange for `http://xboxlive.com` at all, which otherwise looks identical to "networking
/// is broken" (zero relevant traffic, no error) rather than "this CLSID was never handled".
/// `RETAIL` sandbox and the always-zero console id are fixed values - Xodus has no sandbox
/// concept of its own to source these from, and titles do not vary behavior on the console
/// id's contents, only its presence.
#[implement(IXSystem, IXSystem2, IXSystem3, IXSystem4, IXSystem5)]
pub struct XSystem;

impl IXSystem_Impl for XSystem_Impl {
    unsafe fn XSystemGetConsoleId(
        &self,
        console_id_size: i32,
        console_id: *mut c_char,
        console_id_used: *mut usize,
    ) -> HRESULT {
        const ID: &CStr = c"00000000.00000000.00000000.00000000.00";
        if console_id_used.is_null() {
            return E_POINTER;
        }
        // SAFETY: `console_id_used` was checked non-null above.
        unsafe {
            *console_id_used = ID.count_bytes() + 1;
        }
        if console_id.is_null() {
            return E_POINTER;
        }
        if console_id_size < X_SYSTEM_CONSOLE_ID_BYTES {
            return E_NOT_SUFFICIENT_BUFFER;
        }
        // SAFETY: `console_id_size >= X_SYSTEM_CONSOLE_ID_BYTES` was checked above, which
        // covers `ID.count_bytes() + 1`.
        unsafe {
            std::ptr::copy_nonoverlapping(ID.as_ptr(), console_id, ID.count_bytes() + 1);
        }
        S_OK
    }

    unsafe fn XSystemGetXboxLiveSandboxId(
        &self,
        sandbox_id_size: i32,
        sandbox_id: *mut c_char,
        sandbox_id_used: *mut usize,
    ) -> HRESULT {
        const ID: &CStr = c"RETAIL";
        if sandbox_id.is_null() {
            return E_POINTER;
        }
        if sandbox_id_size < X_SYSTEM_SANDBOX_ID_MAX_BYTES {
            return E_NOT_SUFFICIENT_BUFFER;
        }
        // SAFETY: `sandbox_id_size >= X_SYSTEM_SANDBOX_ID_MAX_BYTES` was checked above,
        // which covers `ID.count_bytes() + 1`.
        unsafe {
            std::ptr::copy_nonoverlapping(ID.as_ptr(), sandbox_id, ID.count_bytes() + 1);
        }
        if !sandbox_id_used.is_null() {
            // SAFETY: `sandbox_id_used` was checked non-null above.
            unsafe {
                *sandbox_id_used = ID.count_bytes() + 1;
            }
        }
        S_OK
    }

    /// A random GUID, generated once per process and cached for its lifetime, as `xsystem.idl`
    /// prescribes a device id with process-wide stability. Titles use this to key local
    /// analytics/telemetry batching, not identity, so per-process stability is what matters,
    /// not cross-launch persistence.
    unsafe fn XSystemGetAppSpecificDeviceId(
        &self,
        device_id_size: i32,
        device_id: *mut c_char,
        device_id_used: *mut usize,
    ) -> HRESULT {
        static DEVICE_ID: OnceLock<CString> = OnceLock::new();
        let id = DEVICE_ID.get_or_init(|| {
            use std::hash::{Hash, Hasher};
            // No `uuid` crate dependency and no `CoCreateGuid` equivalent available here -
            // hash together entropy sources unique to this process/run (pid, start time, and
            // a stack address, which ASLR randomizes) instead. This only needs to be
            // stable-for-the-process and look GUID-shaped, not cryptographically random.
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            std::process::id().hash(&mut hasher);
            std::time::SystemTime::now().hash(&mut hasher);
            let stack_marker = 0u8;
            (&stack_marker as *const u8 as usize).hash(&mut hasher);
            let high = hasher.finish();
            std::mem::size_of::<usize>().hash(&mut hasher);
            let low = hasher.finish();
            let text = format!(
                "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
                (high >> 32) as u32,
                (high >> 16) as u16,
                high as u16,
                (low >> 48) as u16,
                low & 0xFFFF_FFFF_FFFF,
            );
            CString::new(text).expect("hex-formatted guid string has no NUL bytes")
        });
        if !device_id_used.is_null() {
            // SAFETY: `device_id_used` was checked non-null above.
            unsafe {
                *device_id_used = id.count_bytes() + 1;
            }
        }
        if device_id.is_null() || device_id_size <= 0 {
            return S_OK;
        }
        let len = (id.count_bytes() + 1).min(device_id_size as usize);
        // SAFETY: `len` is clamped to `device_id_size`, and `device_id_size > 0` was
        // checked above.
        unsafe {
            std::ptr::copy_nonoverlapping(id.as_ptr(), device_id, len);
        }
        S_OK
    }
}

impl IXSystem2_Impl for XSystem_Impl {}

impl IXSystem3_Impl for XSystem_Impl {
    /// No real handle-lifecycle notifications exist to track (no suspend/resume, no
    /// screenshot/broadcast handles under Wine), so this is un-implemented (`E_NOTIMPL`).
    unsafe fn XSystemHandleTrack(
        &self,
        _callback: XSystemHandleCallback,
        _context: *mut c_void,
    ) -> HRESULT {
        E_NOTIMPL
    }

    /// Always valid, since Xodus never invalidates a handle it never tracked in the first
    /// place.
    unsafe fn XSystemIsHandleValid(&self, _handle: XSystemHandle) -> u8 {
        1
    }
}

impl IXSystem4_Impl for XSystem_Impl {
    /// No bandwidth throttling exists to toggle; acknowledging the request (rather than
    /// failing it) avoids failing a call titles may not check the result of.
    unsafe fn XSystemAllowFullDownloadBandwidth(&self, _enable: u8) {}
}

impl IXSystem5_Impl for XSystem_Impl {}
