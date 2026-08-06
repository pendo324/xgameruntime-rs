use super::singleton;
use super::{
    BOOLEAN, SIZE_T, UINT32, UINT64, XAppCaptureLocalStreamHandle,
    XAppCaptureScreenshotStreamHandle, XUserHandle,
};
use crate::E_NOTIMPL;
use std::ffi::{c_char, c_void};
use windows_core::{GUID, HRESULT, IUnknown, implement, interface};
// ---------------------------------------------------------------------------------------
// XAppCaptureImpl / XAppCaptureImpl2 / XAppCaptureImpl3 / XAppCaptureImpl4
// (`xappcapture.idl`)
// ---------------------------------------------------------------------------------------

/// `coclass XAppCaptureImpl` (`a4f1aee2-...`), also the `IXAppCaptureImpl` IID.
pub(crate) const CLSID_XAPPCAPTURE: GUID = GUID::from_u128(0xa4f1aee2_4bf1_4485_b008_a7c26d52ac27);

#[interface("a4f1aee2-4bf1-4485-b008-a7c26d52ac27")]
pub(crate) unsafe trait IXAppCaptureImpl: IUnknown {
    unsafe fn XAppCaptureTakeDiagnosticScreenshot(
        &self,
        gamescreenOnly: BOOLEAN,
        captureFlags: UINT32,
        filenamePrefix: *const c_char,
        result: *mut c_void,
    ) -> HRESULT;
    unsafe fn XAppCaptureRecordDiagnosticClip(
        &self,
        startTime: i64,
        durationInMs: UINT32,
        filenamePrefix: *const c_char,
        result: *mut c_void,
    ) -> HRESULT;
    unsafe fn XAppCaptureTakeScreenshot(
        &self,
        requestingUser: XUserHandle,
        result: *mut c_void,
    ) -> HRESULT;
    unsafe fn XAppCaptureOpenScreenshotStream(
        &self,
        localId: *const c_char,
        screenshotFormat: UINT32,
        handle: *mut XAppCaptureScreenshotStreamHandle,
        totalBytes: *mut UINT64,
    ) -> HRESULT;
    unsafe fn XAppCaptureReadScreenshotStream(
        &self,
        handle: XAppCaptureScreenshotStreamHandle,
        startPosition: UINT64,
        bytesToRead: UINT32,
        buffer: *mut u8,
        bytesWritten: *mut UINT32,
    ) -> HRESULT;
    unsafe fn XAppCaptureCloseScreenshotStream(
        &self,
        handle: XAppCaptureScreenshotStreamHandle,
    ) -> HRESULT;
    unsafe fn XAppCaptureEnableRecord(&self) -> HRESULT;
    unsafe fn XAppCaptureDisableRecord(&self) -> HRESULT;
}

#[interface("3a949778-772e-4799-bdea-0a6639e96baa")]
pub(crate) unsafe trait IXAppCaptureImpl2: IXAppCaptureImpl {
    unsafe fn XAppCaptureGetVideoCaptureSettings(&self, settings: *mut c_void) -> HRESULT;
    unsafe fn XAppCaptureRecordTimespan(
        &self,
        startTimestamp: *mut c_void,
        durationInMilliseconds: UINT64,
        result: *mut c_void,
    ) -> HRESULT;
    unsafe fn XAppCaptureReadLocalStream(
        &self,
        handle: XAppCaptureLocalStreamHandle,
        startPosition: SIZE_T,
        bytesToRead: UINT32,
        buffer: *mut u8,
        bytesWritten: *mut UINT32,
    ) -> HRESULT;
    unsafe fn XAppCaptureCloseLocalStream(&self, handle: XAppCaptureLocalStreamHandle) -> HRESULT;
}

#[interface("2bbca60a-619c-4fe1-812e-fb5c1dbdcf51")]
pub(crate) unsafe trait IXAppCaptureImpl3: IXAppCaptureImpl2 {
    unsafe fn XAppCaptureStartUserRecord(
        &self,
        requestingUser: XUserHandle,
        localIdBufferLength: UINT32,
        localIdBuffer: *mut c_char,
    ) -> HRESULT;
    unsafe fn XAppCaptureStopUserRecord(
        &self,
        localId: *const c_char,
        result: *mut c_void,
    ) -> HRESULT;
}

#[interface("22e672d7-b4e3-406c-bd50-8f0d25236f9e")]
pub(crate) unsafe trait IXAppCaptureImpl4: IXAppCaptureImpl3 {
    unsafe fn XAppCaptureCancelUserRecord(&self, localId: *const c_char) -> HRESULT;
}

#[implement(
    IXAppCaptureImpl,
    IXAppCaptureImpl2,
    IXAppCaptureImpl3,
    IXAppCaptureImpl4
)]
pub(crate) struct XAppCapture;

impl IXAppCaptureImpl_Impl for XAppCapture_Impl {
    unsafe fn XAppCaptureTakeDiagnosticScreenshot(
        &self,
        _gamescreenOnly: BOOLEAN,
        _captureFlags: UINT32,
        _filenamePrefix: *const c_char,
        _result: *mut c_void,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppCaptureRecordDiagnosticClip(
        &self,
        _startTime: i64,
        _durationInMs: UINT32,
        _filenamePrefix: *const c_char,
        _result: *mut c_void,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppCaptureTakeScreenshot(
        &self,
        _requestingUser: XUserHandle,
        _result: *mut c_void,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppCaptureOpenScreenshotStream(
        &self,
        _localId: *const c_char,
        _screenshotFormat: UINT32,
        _handle: *mut XAppCaptureScreenshotStreamHandle,
        _totalBytes: *mut UINT64,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppCaptureReadScreenshotStream(
        &self,
        _handle: XAppCaptureScreenshotStreamHandle,
        _startPosition: UINT64,
        _bytesToRead: UINT32,
        _buffer: *mut u8,
        _bytesWritten: *mut UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppCaptureCloseScreenshotStream(
        &self,
        _handle: XAppCaptureScreenshotStreamHandle,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppCaptureEnableRecord(&self) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppCaptureDisableRecord(&self) -> HRESULT {
        E_NOTIMPL
    }
}

impl IXAppCaptureImpl2_Impl for XAppCapture_Impl {
    unsafe fn XAppCaptureGetVideoCaptureSettings(&self, _settings: *mut c_void) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppCaptureRecordTimespan(
        &self,
        _startTimestamp: *mut c_void,
        _durationInMilliseconds: UINT64,
        _result: *mut c_void,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppCaptureReadLocalStream(
        &self,
        _handle: XAppCaptureLocalStreamHandle,
        _startPosition: SIZE_T,
        _bytesToRead: UINT32,
        _buffer: *mut u8,
        _bytesWritten: *mut UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppCaptureCloseLocalStream(&self, _handle: XAppCaptureLocalStreamHandle) -> HRESULT {
        E_NOTIMPL
    }
}

impl IXAppCaptureImpl3_Impl for XAppCapture_Impl {
    unsafe fn XAppCaptureStartUserRecord(
        &self,
        _requestingUser: XUserHandle,
        _localIdBufferLength: UINT32,
        _localIdBuffer: *mut c_char,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XAppCaptureStopUserRecord(
        &self,
        _localId: *const c_char,
        _result: *mut c_void,
    ) -> HRESULT {
        E_NOTIMPL
    }
}

impl IXAppCaptureImpl4_Impl for XAppCapture_Impl {
    unsafe fn XAppCaptureCancelUserRecord(&self, _localId: *const c_char) -> HRESULT {
        E_NOTIMPL
    }
}

singleton! {
    pub(crate) fn xappcapture_singleton() -> IXAppCaptureImpl4 = XAppCapture;
}
