use std::ffi::c_char;
use windows_core::{GUID, IUnknown, implement, interface};
/// `IXSystemAnalyticsImpl`'s own IID, reused as the coclass id - confirmed via Wine trace logs
/// as one of the classes this title queries and previously got `E_NOTIMPL` for. The real GDK
/// sources its values from Windows' `Windows.System.Profile.AnalyticsInfo` WinRT API. Xodus has
/// no WinRT host to query that from, so the fields are fixed desktop-shaped values instead -
/// same "no real console/sandbox concept, so use a stable fixed value" reasoning as
/// `CLSID_XSYSTEM`'s console/sandbox ids (see `xsystemanalytics.idl` and `xsystem.idl`).
pub const CLSID_XSYSTEM_ANALYTICS: GUID = GUID::from_u128(0xb884675d_b738_4a9c_815d_9a9a1e0c6c9b);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct XVersion {
    pub major: u16,
    pub minor: u16,
    pub build: u16,
    pub revision: u16,
}

#[repr(C)]
pub struct XSystemAnalyticsInfo {
    pub os_version: XVersion,
    pub hosting_os_version: XVersion,
    pub family: [c_char; 64],
    pub form: [c_char; 64],
}

#[interface("b884675d-b738-4a9c-815d-9a9a1e0c6c9b")]
pub unsafe trait IXSystemAnalyticsImpl: IUnknown {
    /// Mirrors the ABI the idl implies (`xsystemanalytics.idl`'s `[out, retval]` struct return):
    /// a large struct return becomes a hidden out-pointer parameter that the function also
    /// returns, per the MSVC x64 ABI.
    pub unsafe fn XSystemGetAnalyticsInfo(
        &self,
        result: *mut XSystemAnalyticsInfo,
    ) -> *mut XSystemAnalyticsInfo;
}

#[implement(IXSystemAnalyticsImpl)]
pub struct XSystemAnalytics;

fn write_fixed_cstr(dst: &mut [c_char; 64], text: &[u8]) {
    let len = text.len().min(63);
    for (slot, byte) in dst.iter_mut().zip(text[..len].iter()) {
        *slot = *byte as c_char;
    }
    dst[len] = 0;
}

impl IXSystemAnalyticsImpl_Impl for XSystemAnalytics_Impl {
    unsafe fn XSystemGetAnalyticsInfo(
        &self,
        result: *mut XSystemAnalyticsInfo,
    ) -> *mut XSystemAnalyticsInfo {
        if result.is_null() {
            return result;
        }
        // A plausible, fixed "generic Windows desktop" identity - not sourced from any real
        // device, since Xodus has no WinRT AnalyticsInfo to query. The family/form split is the
        // one real Windows reports (family "Windows", form "Desktop").
        let version = XVersion {
            major: 10,
            minor: 0,
            build: 19045,
            revision: 0,
        };
        // SAFETY: `result` was checked non-null above.
        unsafe {
            (*result).os_version = version;
            (*result).hosting_os_version = version;
            write_fixed_cstr(&mut (*result).family, b"Windows");
            write_fixed_cstr(&mut (*result).form, b"Desktop");
        }
        result
    }
}
