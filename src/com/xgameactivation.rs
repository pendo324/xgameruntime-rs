use super::GlobalInterface;
use super::{BOOLEAN, FALSE, XTaskQueueHandle, XTaskQueueRegistrationToken};
use crate::E_NOTIMPL;
use std::ffi::{c_char, c_void};
use std::sync::OnceLock;
use windows_core::{GUID, HRESULT, IUnknown, implement, interface};
// ---------------------------------------------------------------------------------------
// XGameActivationImpl (`xgameactivation.idl`)
// ---------------------------------------------------------------------------------------

/// `coclass XGameActivationImpl` (`7f0fe8b8-...`).
pub(crate) const CLSID_XGAME_ACTIVATION: GUID =
    GUID::from_u128(0x7f0fe8b8_e075_49ab_9aa7_a1e065489a9e);

#[interface("2e4f76fe-0fc7-461e-ab4d-a4499434c3cf")]
pub(crate) unsafe trait IXGameActivationImpl: IUnknown {
    unsafe fn XGameActivationRegisterForEvent(
        &self,
        queue: XTaskQueueHandle,
        context: *mut c_void,
        callback: *mut c_void,
        token: *mut XTaskQueueRegistrationToken,
    ) -> HRESULT;
    unsafe fn XGameActivationUnregisterForEvent(
        &self,
        token: XTaskQueueRegistrationToken,
        wait: BOOLEAN,
    ) -> BOOLEAN;
    unsafe fn XGameActivationAcceptPendingInvite(&self, inviteUri: *const c_char) -> HRESULT;
}

#[implement(IXGameActivationImpl)]
pub(crate) struct XGameActivation;

impl IXGameActivationImpl_Impl for XGameActivation_Impl {
    unsafe fn XGameActivationRegisterForEvent(
        &self,
        _queue: XTaskQueueHandle,
        _context: *mut c_void,
        _callback: *mut c_void,
        _token: *mut XTaskQueueRegistrationToken,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameActivationUnregisterForEvent(
        &self,
        _token: XTaskQueueRegistrationToken,
        _wait: BOOLEAN,
    ) -> BOOLEAN {
        FALSE
    }
    unsafe fn XGameActivationAcceptPendingInvite(&self, _inviteUri: *const c_char) -> HRESULT {
        E_NOTIMPL
    }
}

static XGAMEACTIVATION_SINGLETON: OnceLock<GlobalInterface<IXGameActivationImpl>> = OnceLock::new();

pub(crate) fn xgameactivation_singleton() -> &'static IXGameActivationImpl {
    &XGAMEACTIVATION_SINGLETON
        .get_or_init(|| GlobalInterface(XGameActivation.into()))
        .0
}
