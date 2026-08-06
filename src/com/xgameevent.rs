use super::XUserHandle;
use super::singleton;
use crate::E_NOTIMPL;
use std::ffi::c_char;
use windows_core::{GUID, HRESULT, IUnknown, implement, interface};
// ---------------------------------------------------------------------------------------
// XGameEventImpl (`xgameevent.idl`)
// ---------------------------------------------------------------------------------------

/// `coclass XGameEventImpl` (`bbfbdcc7-...`), also the `IXGameEventImpl` IID.
pub(crate) const CLSID_XGAME_EVENT: GUID = GUID::from_u128(0xbbfbdcc7_bfe7_409b_a5ca_edf054960b4d);

#[interface("bbfbdcc7-bfe7-409b-a5ca-edf054960b4d")]
pub(crate) unsafe trait IXGameEventImpl: IUnknown {
    unsafe fn XGameEventWrite(
        &self,
        user: XUserHandle,
        serviceConfigId: *const c_char,
        playSessionId: *const c_char,
        eventName: *const c_char,
        dimensionsJson: *const c_char,
        measurementsJson: *const c_char,
    ) -> HRESULT;
}

#[implement(IXGameEventImpl)]
pub(crate) struct XGameEvent;

impl IXGameEventImpl_Impl for XGameEvent_Impl {
    unsafe fn XGameEventWrite(
        &self,
        _user: XUserHandle,
        _serviceConfigId: *const c_char,
        _playSessionId: *const c_char,
        _eventName: *const c_char,
        _dimensionsJson: *const c_char,
        _measurementsJson: *const c_char,
    ) -> HRESULT {
        E_NOTIMPL
    }
}

singleton! {
    pub(crate) fn xgameevent_singleton() -> IXGameEventImpl = XGameEvent;
}
