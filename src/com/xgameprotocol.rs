use crate::results::*;
use std::ffi::c_void;
use windows_core::{GUID, HRESULT, IUnknown, implement, interface};
use windows_sys::core::BOOL;
/// Unlike `CLSID_XGAME_INVITE`, `XGameProtocolImpl`'s coclass id (`95fd18d2...`, confirmed via
/// Wine trace logs) is *not* the same value as `IXGameProtocolImpl`'s own IID
/// (`026b010c...`) - `xgameruntime-docs`' `XGameProtocolImpl/README.md` documents them as
/// distinct, so this needs its own constant rather than reusing the interface's IID.
pub const CLSID_XGAME_PROTOCOL: GUID = GUID::from_u128(0x95fd18d2_74dd_4d7c_aa1b_0b51827665d6);

#[interface("026b010c-06c3-4cdd-bbcb-43f229db1cff")]
pub unsafe trait IXGameProtocolImpl: IUnknown {
    pub unsafe fn XGameProtocolRegisterForActivation(
        &self,
        queue: u64,
        context: *mut c_void,
        callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XGameProtocolUnregisterForActivation(&self, token: u64, wait: BOOL) -> ();
}

#[implement(IXGameProtocolImpl)]
pub struct XGameProtocol;

impl IXGameProtocolImpl_Impl for XGameProtocol_Impl {
    /// No custom-protocol activation transport exists under Wine (no shell association to
    /// register against) - registration succeeds and simply never fires, matching
    /// `XGameInviteRegisterForEvent`'s reasoning above.
    unsafe fn XGameProtocolRegisterForActivation(
        &self,
        _queue: u64,
        _context: *mut c_void,
        _callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT {
        if !token.is_null() {
            unsafe {
                *(token as *mut u64) = 0;
            }
        }
        S_OK
    }

    unsafe fn XGameProtocolUnregisterForActivation(&self, _token: u64, _wait: BOOL) {}
}
