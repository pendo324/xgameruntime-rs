use super::E_NOTIMPL;
use crate::results::*;
use std::ffi::{c_char, c_void};
use windows_core::{GUID, HRESULT, IUnknown, implement, interface};
use crate::com::BOOLEAN;
// The five classes below have no `.idl` entry in Wine's `xgameruntime` include tree (only
// `xsystemanalytics.idl` exists there - the other four have no idl at all), so their
// IIDs/method layouts come from `xgameruntime-docs` instead. That source documents some
// of these interfaces as having methods with unknown signatures (flagged inline below) - those
// slots are still given a plausible stub so the vtable's *layout* (and therefore every method
// after it) stays correct, even though the stub itself may not be what a real call expects. On
// x64, an unexpected extra/ignored argument or return value is harmless as long as the argument
// *count* and *pointer-ness* look right, so this is safe unless the title actually calls one of
// the genuinely-unknown methods, which none of these titles are expected to.

/// `IXGameInviteImpl`'s own IID, reused as the coclass id (same pattern as `CLSID_XGAME`) -
/// confirmed via Wine trace logs as one of the classes this title queries and previously got
/// `E_NOTIMPL` for. Xodus has no invite/multiplayer-activation transport, so registration always
/// succeeds (there is nothing to fail) and simply never fires a callback.
pub const CLSID_XGAME_INVITE: GUID = GUID::from_u128(0x0651aae2_4012_4077_bf84_8b9097090e2c);

#[interface("0651aae2-4012-4077-bf84-8b9097090e2c")]
pub unsafe trait IXGameInviteImpl: IUnknown {
    pub unsafe fn XGameInviteRegisterForEvent(
        &self,
        queue: u64,
        context: *mut c_void,
        callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XGameInviteUnregisterForEvent(&self, token: u64, wait: BOOLEAN) -> ();
}

#[interface("014d1cc3-bcfe-41ff-b2f0-e1ef07155828")]
pub unsafe trait IXGameInviteImpl2: IXGameInviteImpl {
    pub unsafe fn XGameInviteRegisterForPendingEvent(
        &self,
        queue: u64,
        context: *mut c_void,
        callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XGameInviteUnregisterForPendingEvent(&self, token: u64, wait: BOOLEAN) -> ();
    /// `xgameruntime-docs` has no documentation at all for this method (added in a GDK update
    /// alongside the "pending event" pair above) - not even a parameter list. This signature is
    /// a guess based on the name and the shape of every other invite-acceptance call in this
    /// family (an invite/activation URI string in, HRESULT out); it exists purely to keep
    /// `XGameInviteUnregisterForPendingEvent`'s vtable slot position correct, not because the
    /// guess is trusted.
    pub unsafe fn XGameInviteAcceptPendingInvite(&self, invite_uri: *const c_char) -> HRESULT;
}

#[implement(IXGameInviteImpl, IXGameInviteImpl2)]
pub struct XGameInvite;

impl IXGameInviteImpl_Impl for XGameInvite_Impl {
    unsafe fn XGameInviteRegisterForEvent(
        &self,
        _queue: u64,
        _context: *mut c_void,
        _callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT {
        if !token.is_null() {
            // SAFETY: `token` was checked non-null above; XGameInviteRegisterForEvent's GDK
            // contract declares it a `*mut u64` out-param.
            unsafe {
                *(token as *mut u64) = 0;
            }
        }
        S_OK
    }

    unsafe fn XGameInviteUnregisterForEvent(&self, _token: u64, _wait: BOOLEAN) {}
}

impl IXGameInviteImpl2_Impl for XGameInvite_Impl {
    unsafe fn XGameInviteRegisterForPendingEvent(
        &self,
        _queue: u64,
        _context: *mut c_void,
        _callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT {
        if !token.is_null() {
            // SAFETY: `token` was checked non-null above; XGameInviteRegisterForPendingEvent's
            // GDK contract declares it a `*mut u64` out-param.
            unsafe {
                *(token as *mut u64) = 0;
            }
        }
        S_OK
    }

    unsafe fn XGameInviteUnregisterForPendingEvent(&self, _token: u64, _wait: BOOLEAN) {}

    unsafe fn XGameInviteAcceptPendingInvite(&self, _invite_uri: *const c_char) -> HRESULT {
        E_NOTIMPL
    }
}
