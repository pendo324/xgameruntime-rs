use super::GlobalInterface;
use super::{
    BOOLEAN, DOUBLE, FALSE, INT32, UINT32, UINT64, XTaskQueueHandle, XTaskQueueRegistrationToken,
    XUserHandle,
};
use crate::E_NOTIMPL;
use std::ffi::{c_char, c_void};
use std::sync::OnceLock;
use windows_core::{GUID, HRESULT, IUnknown, implement, interface};
// ---------------------------------------------------------------------------------------
// XAppCaptureMetadataImpl (`xappcapture.idl`)
// ---------------------------------------------------------------------------------------

/// `coclass XAppCaptureMetadataImpl` (`186d5592-...`), also the `IXAppCaptureMetadataImpl` IID.
pub(crate) const CLSID_XAPPCAPTURE_METADATA: GUID =
    GUID::from_u128(0x186d5592_a72d_45fb_9560_11aed0e6647a);

#[interface("186d5592-a72d-45fb-9560-11aed0e6647a")]
pub(crate) unsafe trait IXAppCaptureMetadataImpl: IUnknown {
    unsafe fn XAppBroadcastIsAppBroadcasting(&self) -> BOOLEAN;
    unsafe fn XAppBroadcastShowUI(&self, requestingUser: XUserHandle) -> HRESULT;
    unsafe fn XAppBroadcastGetStatus(
        &self,
        requestingUser: XUserHandle,
        appBroadcastStatus: *mut c_void,
    ) -> HRESULT;
    unsafe fn XAppBroadcastRegisterIsAppBroadcastingChanged(
        &self,
        queue: XTaskQueueHandle,
        context: *mut c_void,
        callback: *mut c_void,
        token: *mut XTaskQueueRegistrationToken,
    ) -> HRESULT;
    unsafe fn XAppBroadcastUnregisterIsAppBroadcastingChanged(
        &self,
        token: XTaskQueueRegistrationToken,
        wait: BOOLEAN,
    ) -> BOOLEAN;
    unsafe fn XAppCaptureMetadataAddStringEvent(
        &self,
        name: *const c_char,
        value: *const c_char,
        priority: UINT32,
    ) -> HRESULT;
    unsafe fn XAppCaptureMetadataAddInt32Event(
        &self,
        name: *const c_char,
        value: INT32,
        priority: UINT32,
    ) -> HRESULT;
    unsafe fn XAppCaptureMetadataAddDoubleEvent(
        &self,
        name: *const c_char,
        value: DOUBLE,
        priority: UINT32,
    ) -> HRESULT;
    unsafe fn XAppCaptureMetadataStartStringState(
        &self,
        name: *const c_char,
        value: *const c_char,
        priority: UINT32,
    ) -> HRESULT;
    unsafe fn XAppCaptureMetadataStartInt32State(
        &self,
        name: *const c_char,
        value: INT32,
        priority: UINT32,
    ) -> HRESULT;
    unsafe fn XAppCaptureMetadataStartDoubleState(
        &self,
        name: *const c_char,
        value: DOUBLE,
        priority: UINT32,
    ) -> HRESULT;
    unsafe fn XAppCaptureMetadataStopState(&self, name: *const c_char) -> HRESULT;
    unsafe fn XAppCaptureMetadataStopAllStates(&self) -> HRESULT;
    unsafe fn XAppCaptureMetadataRemainingStorageBytesAvailable(
        &self,
        value: *mut UINT64,
    ) -> HRESULT;
    unsafe fn XAppCaptureRegisterMetadataPurged(
        &self,
        queue: XTaskQueueHandle,
        context: *mut c_void,
        callback: *mut c_void,
        token: *mut XTaskQueueRegistrationToken,
    ) -> HRESULT;
    unsafe fn XAppCaptureUnRegisterMetadataPurged(
        &self,
        token: XTaskQueueRegistrationToken,
        wait: BOOLEAN,
    ) -> BOOLEAN;
}

#[implement(IXAppCaptureMetadataImpl)]
pub(crate) struct XAppCaptureMetadata;

impl IXAppCaptureMetadataImpl_Impl for XAppCaptureMetadata_Impl {
    unsafe fn XAppBroadcastIsAppBroadcasting(&self) -> BOOLEAN {
        FALSE
    }
    unsafe fn XAppBroadcastShowUI(&self, _requestingUser: XUserHandle) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppBroadcastGetStatus(
        &self,
        _requestingUser: XUserHandle,
        _appBroadcastStatus: *mut c_void,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppBroadcastRegisterIsAppBroadcastingChanged(
        &self,
        _queue: XTaskQueueHandle,
        _context: *mut c_void,
        _callback: *mut c_void,
        _token: *mut XTaskQueueRegistrationToken,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppBroadcastUnregisterIsAppBroadcastingChanged(
        &self,
        _token: XTaskQueueRegistrationToken,
        _wait: BOOLEAN,
    ) -> BOOLEAN {
        FALSE
    }
    unsafe fn XAppCaptureMetadataAddStringEvent(
        &self,
        _name: *const c_char,
        _value: *const c_char,
        _priority: UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppCaptureMetadataAddInt32Event(
        &self,
        _name: *const c_char,
        _value: INT32,
        _priority: UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppCaptureMetadataAddDoubleEvent(
        &self,
        _name: *const c_char,
        _value: DOUBLE,
        _priority: UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppCaptureMetadataStartStringState(
        &self,
        _name: *const c_char,
        _value: *const c_char,
        _priority: UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppCaptureMetadataStartInt32State(
        &self,
        _name: *const c_char,
        _value: INT32,
        _priority: UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppCaptureMetadataStartDoubleState(
        &self,
        _name: *const c_char,
        _value: DOUBLE,
        _priority: UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppCaptureMetadataStopState(&self, _name: *const c_char) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppCaptureMetadataStopAllStates(&self) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppCaptureMetadataRemainingStorageBytesAvailable(
        &self,
        _value: *mut UINT64,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppCaptureRegisterMetadataPurged(
        &self,
        _queue: XTaskQueueHandle,
        _context: *mut c_void,
        _callback: *mut c_void,
        _token: *mut XTaskQueueRegistrationToken,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppCaptureUnRegisterMetadataPurged(
        &self,
        _token: XTaskQueueRegistrationToken,
        _wait: BOOLEAN,
    ) -> BOOLEAN {
        FALSE
    }
}

static XAPPCAPTUREMETADATA_SINGLETON: OnceLock<GlobalInterface<IXAppCaptureMetadataImpl>> =
    OnceLock::new();

pub(crate) fn xappcapturemetadata_singleton() -> &'static IXAppCaptureMetadataImpl {
    &XAPPCAPTUREMETADATA_SINGLETON
        .get_or_init(|| GlobalInterface(XAppCaptureMetadata.into()))
        .0
}
