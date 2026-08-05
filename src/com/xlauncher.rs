use super::GlobalInterface;
use super::{XDisplayTimeoutDeferralHandle, XUserHandle};
use crate::E_NOTIMPL;
use std::ffi::c_char;
use std::sync::OnceLock;
use windows_core::{GUID, HRESULT, IUnknown, implement, interface};
pub(crate) const CLSID_XLAUNCHER: GUID = GUID::from_u128(0x1b339674_328d_4283_a200_3171f18d3639);

#[interface("1b339674-328d-4283-a200-3171f18d3639")]
pub(crate) unsafe trait IXLauncherImpl: IUnknown {
    unsafe fn XLaunchUri(&self, user: XUserHandle, uri: *const c_char) -> HRESULT;
    unsafe fn XDisplayAcquireTimeoutDeferral(
        &self,
        handle: *mut XDisplayTimeoutDeferralHandle,
    ) -> HRESULT;
    unsafe fn XDisplayCloseTimeoutDeferralHandle(&self, handle: XDisplayTimeoutDeferralHandle);
}

#[implement(IXLauncherImpl)]
pub(crate) struct XLauncher;

impl IXLauncherImpl_Impl for XLauncher_Impl {
    unsafe fn XLaunchUri(&self, _user: XUserHandle, _uri: *const c_char) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XDisplayAcquireTimeoutDeferral(
        &self,
        _handle: *mut XDisplayTimeoutDeferralHandle,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XDisplayCloseTimeoutDeferralHandle(&self, _handle: XDisplayTimeoutDeferralHandle) {}
}

static XLAUNCHER_SINGLETON: OnceLock<GlobalInterface<IXLauncherImpl>> = OnceLock::new();

pub(crate) fn xlauncher_singleton() -> &'static IXLauncherImpl {
    &XLAUNCHER_SINGLETON
        .get_or_init(|| GlobalInterface(XLauncher.into()))
        .0
}
