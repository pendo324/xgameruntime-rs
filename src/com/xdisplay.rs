use super::UINT32;
use super::singleton;
use crate::E_NOTIMPL;
use std::ffi::c_void;
use windows_core::{GUID, HRESULT, IUnknown, implement, interface};
// ---------------------------------------------------------------------------------------
// XDisplayImpl + XLauncherImpl (both in `xdisplay.idl`)
// ---------------------------------------------------------------------------------------

pub(crate) const CLSID_XDISPLAY: GUID = GUID::from_u128(0x03f0fe74_fdd9_4e5c_b630_f9339c47acc5);

#[interface("35f07670-706e-4bfb-9476-090798c5ebf3")]
pub(crate) unsafe trait IXDisplayImpl: IUnknown {
    /// Reserved vtable slot - `__PADDING__()` in `xdisplay.idl`.
    unsafe fn __PaddingSlot4(&self) -> HRESULT;
    unsafe fn XDisplayTryEnableHdrMode(
        &self,
        displayModePreference: UINT32,
        displayHdrModeInfo: *mut c_void,
    ) -> UINT32;
}

#[implement(IXDisplayImpl)]
pub(crate) struct XDisplay;

impl IXDisplayImpl_Impl for XDisplay_Impl {
    unsafe fn __PaddingSlot4(&self) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XDisplayTryEnableHdrMode(
        &self,
        _displayModePreference: UINT32,
        _displayHdrModeInfo: *mut c_void,
    ) -> UINT32 {
        0 // XDisplayHdrModeResult_Unknown
    }
}

singleton! {
    pub(crate) fn xdisplay_singleton() -> IXDisplayImpl = XDisplay;
}
