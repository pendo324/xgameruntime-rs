//! Stub COM classes that complete the `query_api_impl` dispatch surface.
//!
//! These nine classes exist in WineGDK's C `xgameruntime` (and in the real GDK) but have
//! nothing backing them here. Their C counterparts are themselves honest `E_NOTIMPL`
//! stubs - see `xaccessibility.c`, `xappcapture.c`, `xdisplay.c`, `xgameactivation.c`,
//! `xgameevent.c`, `xgamestreaming.c`, `xgameui.c` - so no real implementation is being
//! skipped: the ABI-shaped vtable is registered so a title's `QueryApiImpl` succeeds with
//! a real object rather than crashing on an unresolved class, and every method returns
//! `E_NOTIMPL` (or the same stub value WineGDK returns for non-HRESULT methods).
//!
//! Interface IIDs and the coclass ids (CLSID = coclass uuid) are taken from WineGDK's
//! `include/*.idl`; vtable slot order follows the `.idl` `interface` blocks verbatim,
//! including `__PADDING__` reserved slots, so the generated vtables line up with what a
//! title compiled against the real GDK headers expects. `XThreadingImpl` is deliberately
//! absent: its coclass uuid (`073b7dcb-...`) is the crate's `CLSID_XASYNC`, and
//! `crate::xasync::IXAsync` already *is* the IXThreadingImpl vtable with a real
//! `XTaskQueue`/`XAsync` implementation behind it.

use super::E_NOTIMPL;
use crate::results::{E_NOINTERFACE, E_POINTER, S_OK};
use std::ffi::{c_char, c_void};
use std::sync::OnceLock;
use windows_core::{GUID, HRESULT, IUnknown, Interface, implement, interface};

/// GDK handle types, opaque here because every method is a stub. Kept as `u64` (pointer
/// width) to match the crate's handle convention (`xuser.rs` passes `XUserHandle`s as
/// `u64`), and `*mut c_void` for opaque object pointers.
type XUserHandle = u64;
type XTaskQueueHandle = u64;
type XTaskQueueRegistrationToken = u64;
type XAppCaptureScreenshotStreamHandle = u64;
type XAppCaptureLocalStreamHandle = u64;
type XGameStreamingClientId = u64;
type XGameUiTextEntryHandle = u64;
type XGameUiCallbackHandle = u64;
type XDisplayTimeoutDeferralHandle = u64;
type XSpeechSynthesizerHandle = u64;
type XSpeechSynthesizerStreamHandle = u64;
type SIZE_T = usize;
type UINT32 = u32;
type UINT64 = u64;
type INT32 = i32;
type BOOLEAN = u8;
type FLOAT = f32;
type DOUBLE = f64;
const FALSE: BOOLEAN = 0;

// ---------------------------------------------------------------------------------------
// XAccessibilityImpl (`XAccessibilityImpl.c`)
// ---------------------------------------------------------------------------------------

/// `coclass XAccessibilityImpl` (also the `IXAccessibilityImpl` IID) in `xaccessibility.idl`.
pub(crate) const CLSID_XACCESSIBILITY: GUID =
    GUID::from_u128(0x3e241977_6237_41e9_8559_102c2d9983f1);

#[interface("3e241977-6237-41e9-8559-102c2d9983f1")]
pub(crate) unsafe trait IXAccessibilityImpl: IUnknown {
    unsafe fn XClosedCaptionGetProperties(&self, props: *mut c_void) -> HRESULT;
    unsafe fn XClosedCaptionSetEnabled(&self, enabled: BOOLEAN) -> HRESULT;
    unsafe fn XHighContrastGetMode(&self, mode: *mut UINT32) -> HRESULT;
    unsafe fn XSpeechToTextSetPositionHint(&self, position: UINT32) -> HRESULT;
    unsafe fn XSpeechToTextSendString(
        &self,
        speakerName: *const c_char,
        content: *const c_char,
        ty: UINT32,
    ) -> HRESULT;
    unsafe fn XSpeechSynthesizerEnumerateInstalledVoices(
        &self,
        context: *mut c_void,
        callback: *mut c_void,
    ) -> HRESULT;
    unsafe fn XSpeechSynthesizerCreate(
        &self,
        speechSynthesizer: *mut XSpeechSynthesizerHandle,
    ) -> HRESULT;
    unsafe fn XSpeechSynthesizerCloseHandle(
        &self,
        speechSynthesizer: XSpeechSynthesizerHandle,
    ) -> HRESULT;
    unsafe fn XSpeechSynthesizerSetDefaultVoice(
        &self,
        speechSynthesizer: XSpeechSynthesizerHandle,
    ) -> HRESULT;
    unsafe fn XSpeechSynthesizerSetCustomVoice(
        &self,
        speechSynthesizer: XSpeechSynthesizerHandle,
        voiceId: *const c_char,
    ) -> HRESULT;
    unsafe fn XSpeechSynthesizerCreateStreamFromText(
        &self,
        speechSynthesizer: XSpeechSynthesizerHandle,
        text: *const c_char,
        speechSynthesisStream: *mut XSpeechSynthesizerStreamHandle,
    ) -> HRESULT;
    unsafe fn XSpeechSynthesizerCloseStreamHandle(
        &self,
        speechSynthesisStream: XSpeechSynthesizerStreamHandle,
    ) -> HRESULT;
    unsafe fn XSpeechSynthesizerGetStreamDataSize(
        &self,
        speechSynthesisStream: XSpeechSynthesizerStreamHandle,
        bufferSize: *mut SIZE_T,
    ) -> HRESULT;
    unsafe fn XSpeechSynthesizerGetStreamData(
        &self,
        speechSynthesisStream: XSpeechSynthesizerStreamHandle,
        bufferSize: SIZE_T,
        buffer: *mut c_void,
        bufferUsed: *mut SIZE_T,
    ) -> HRESULT;
    unsafe fn XSpeechToTextBeginHypothesisString(
        &self,
        speakerName: *const c_char,
        content: *const c_char,
        ty: UINT32,
        hypothesisId: *mut UINT32,
    ) -> HRESULT;
    unsafe fn XSpeechToTextUpdateHypothesisString(
        &self,
        hypothesisId: UINT32,
        content: *const c_char,
    ) -> HRESULT;
    unsafe fn XSpeechToTextFinalizeHypothesisString(
        &self,
        hypothesisId: UINT32,
        content: *const c_char,
    ) -> HRESULT;
    unsafe fn XSpeechToTextCancelHypothesisString(&self, hypothesisId: UINT32) -> HRESULT;
    unsafe fn XSpeechSynthesizerCreateStreamFromSsml(
        &self,
        speechSynthesizer: XSpeechSynthesizerHandle,
        ssml: *const c_char,
        speechSynthesisStream: *mut XSpeechSynthesizerStreamHandle,
    ) -> HRESULT;
}

/// `IXAccessibilityImpl2` (`d722b373-...`) adds nothing - vtable is identical to
/// `IXAccessibilityImpl`, kept as a distinct IID for QueryInterface parity with the coclass's
/// `[default] interface IXAccessibilityImpl2`.
#[interface("d722b373-8c4d-4692-9e51-d6ad9b37aa7d")]
pub(crate) unsafe trait IXAccessibilityImpl2: IXAccessibilityImpl {}

#[implement(IXAccessibilityImpl, IXAccessibilityImpl2)]
pub(crate) struct XAccessibility;

impl IXAccessibilityImpl_Impl for XAccessibility_Impl {
    unsafe fn XClosedCaptionGetProperties(&self, _props: *mut c_void) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XClosedCaptionSetEnabled(&self, _enabled: BOOLEAN) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XHighContrastGetMode(&self, _mode: *mut UINT32) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XSpeechToTextSetPositionHint(&self, _position: UINT32) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XSpeechToTextSendString(
        &self,
        _speakerName: *const c_char,
        _content: *const c_char,
        _ty: UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XSpeechSynthesizerEnumerateInstalledVoices(
        &self,
        _context: *mut c_void,
        _callback: *mut c_void,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XSpeechSynthesizerCreate(
        &self,
        _speechSynthesizer: *mut XSpeechSynthesizerHandle,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XSpeechSynthesizerCloseHandle(
        &self,
        _speechSynthesizer: XSpeechSynthesizerHandle,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XSpeechSynthesizerSetDefaultVoice(
        &self,
        _speechSynthesizer: XSpeechSynthesizerHandle,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XSpeechSynthesizerSetCustomVoice(
        &self,
        _speechSynthesizer: XSpeechSynthesizerHandle,
        _voiceId: *const c_char,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XSpeechSynthesizerCreateStreamFromText(
        &self,
        _speechSynthesizer: XSpeechSynthesizerHandle,
        _text: *const c_char,
        _speechSynthesisStream: *mut XSpeechSynthesizerStreamHandle,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XSpeechSynthesizerCloseStreamHandle(
        &self,
        _speechSynthesisStream: XSpeechSynthesizerStreamHandle,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XSpeechSynthesizerGetStreamDataSize(
        &self,
        _speechSynthesisStream: XSpeechSynthesizerStreamHandle,
        _bufferSize: *mut SIZE_T,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XSpeechSynthesizerGetStreamData(
        &self,
        _speechSynthesisStream: XSpeechSynthesizerStreamHandle,
        _bufferSize: SIZE_T,
        _buffer: *mut c_void,
        _bufferUsed: *mut SIZE_T,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XSpeechToTextBeginHypothesisString(
        &self,
        _speakerName: *const c_char,
        _content: *const c_char,
        _ty: UINT32,
        _hypothesisId: *mut UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XSpeechToTextUpdateHypothesisString(
        &self,
        _hypothesisId: UINT32,
        _content: *const c_char,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XSpeechToTextFinalizeHypothesisString(
        &self,
        _hypothesisId: UINT32,
        _content: *const c_char,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XSpeechToTextCancelHypothesisString(&self, _hypothesisId: UINT32) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XSpeechSynthesizerCreateStreamFromSsml(
        &self,
        _speechSynthesizer: XSpeechSynthesizerHandle,
        _ssml: *const c_char,
        _speechSynthesisStream: *mut XSpeechSynthesizerStreamHandle,
    ) -> HRESULT {
        E_NOTIMPL
    }
}

impl IXAccessibilityImpl2_Impl for XAccessibility_Impl {}

// ---------------------------------------------------------------------------------------
// XAppCaptureImpl / XAppCaptureImpl2 / XAppCaptureImpl3 / XAppCaptureImpl4
// (`XAppCaptureImpl.c`)
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

// ---------------------------------------------------------------------------------------
// XAppCaptureMetadataImpl (`XAppCaptureMetadataImpl.c`)
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

// ---------------------------------------------------------------------------------------
// XDisplayImpl + XLauncherImpl (`XDisplayImpl.c` - one coclass pair in one file)
// ---------------------------------------------------------------------------------------

/// `coclass XDisplayImpl` (`03f0fe74-...`).
pub(crate) const CLSID_XDISPLAY: GUID = GUID::from_u128(0x03f0fe74_fdd9_4e5c_b630_f9339c47acc5);

/// `coclass XLauncherImpl` (`1b339674-...`), also the `IXLauncherImpl` IID.
pub(crate) const CLSID_XLAUNCHER: GUID = GUID::from_u128(0x1b339674_328d_4283_a200_3171f18d3639);

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

#[interface("1b339674-328d-4283-a200-3171f18d3639")]
pub(crate) unsafe trait IXLauncherImpl: IUnknown {
    unsafe fn XLaunchUri(&self, user: XUserHandle, uri: *const c_char) -> HRESULT;
    unsafe fn XDisplayAcquireTimeoutDeferral(
        &self,
        handle: *mut XDisplayTimeoutDeferralHandle,
    ) -> HRESULT;
    unsafe fn XDisplayCloseTimeoutDeferralHandle(&self, handle: XDisplayTimeoutDeferralHandle);
}

#[implement(IXDisplayImpl)]
pub(crate) struct XDisplay;

#[implement(IXLauncherImpl)]
pub(crate) struct XLauncher;

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

// ---------------------------------------------------------------------------------------
// XGameActivationImpl (`XGameActivationImpl.c`)
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

// ---------------------------------------------------------------------------------------
// XGameEventImpl (`XGameEventImpl.c`)
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

// ---------------------------------------------------------------------------------------
// XGameStreamingImpl / XGameStreamingImpl2 / XGameStreamingImpl3 (`XGameStreamingImpl.c`)
// ---------------------------------------------------------------------------------------

/// `coclass XGameStreamingImpl` (`0a2192aa-...`).
pub(crate) const CLSID_XGAME_STREAMING: GUID =
    GUID::from_u128(0x0a2192aa_b2d5_4d58_83be_383b6d80799e);

#[interface("8aff07f5-a1bf-4db8-80a5-31cca0de51b7")]
pub(crate) unsafe trait IXGameStreamingImpl: IUnknown {
    unsafe fn XGameStreamingInitialize(&self) -> HRESULT;
    unsafe fn XGameStreamingUninitialize(&self);
    unsafe fn XGameStreamingIsStreaming(&self) -> BOOLEAN;
    unsafe fn XGameStreamingRegisterClientPropertiesChanged(
        &self,
        client: XGameStreamingClientId,
        queue: XTaskQueueHandle,
        context: *mut c_void,
        callback: *mut c_void,
        token: *mut XTaskQueueRegistrationToken,
    ) -> HRESULT;
    unsafe fn XGameStreamingUnregisterClientPropertiesChanged(
        &self,
        client: XGameStreamingClientId,
        token: XTaskQueueRegistrationToken,
        wait: BOOLEAN,
    ) -> BOOLEAN;
    unsafe fn XGameStreamingGetStreamPhysicalDimensions(
        &self,
        client: XGameStreamingClientId,
        horizontalMm: *mut UINT32,
        verticalMm: *mut UINT32,
    ) -> HRESULT;
    unsafe fn XGameStreamingGetClientCount(&self) -> UINT32;
    unsafe fn XGameStreamingGetClients(
        &self,
        clientCount: UINT32,
        clients: *mut XGameStreamingClientId,
        clientsUsed: *mut UINT32,
    ) -> HRESULT;
    unsafe fn XGameStreamingGetConnectionState(&self, client: XGameStreamingClientId) -> UINT32;
    unsafe fn XGameStreamingRegisterConnectionStateChanged(
        &self,
        queue: XTaskQueueHandle,
        context: *mut c_void,
        callback: *mut c_void,
        token: *mut XTaskQueueRegistrationToken,
    ) -> HRESULT;
    unsafe fn XGameStreamingUnregisterConnectionStateChanged(
        &self,
        token: XTaskQueueRegistrationToken,
        wait: BOOLEAN,
    ) -> BOOLEAN;
    unsafe fn XGameStreamingGetStreamAddedLatency(
        &self,
        client: XGameStreamingClientId,
        averageInputLatencyUs: *mut UINT32,
        averageOutputLatencyUs: *mut UINT32,
        standardDeviationUs: *mut UINT32,
    ) -> HRESULT;
    unsafe fn XGameStreamingGetServerLocationNameSize(&self) -> SIZE_T;
    unsafe fn XGameStreamingGetServerLocationName(
        &self,
        serverLocationNameSize: SIZE_T,
        serverLocationName: *mut c_char,
    ) -> HRESULT;
    unsafe fn XGameStreamingHideTouchControls(&self);
    unsafe fn XGameStreamingShowTouchControlLayout(&self, layout: *const c_char);
    unsafe fn XGameStreamingHideTouchControlsOnClient(&self, client: XGameStreamingClientId);
    unsafe fn XGameStreamingShowTouchControlLayoutOnClient(
        &self,
        client: XGameStreamingClientId,
        layout: *const c_char,
    );
    unsafe fn XGameStreamingIsTouchInputEnabled(
        &self,
        client: XGameStreamingClientId,
        touchInputEnabled: *mut BOOLEAN,
    ) -> HRESULT;
    unsafe fn XGameStreamingGetLastFrameDisplayed(
        &self,
        client: XGameStreamingClientId,
        framePipelineToken: *mut c_void,
    ) -> HRESULT;
    unsafe fn XGameStreamingGetAssociatedFrame(
        &self,
        gamepadReading: *mut c_void,
        framePipelineToken: *mut c_void,
    ) -> HRESULT;
    unsafe fn XGameStreamingGetGamepadPhysicality(
        &self,
        gamepadReading: *mut c_void,
        gamepadPhysicality: *mut c_void,
    ) -> HRESULT;
    unsafe fn XGameStreamingUpdateTouchControlsState(
        &self,
        operationCount: SIZE_T,
        operations: *const c_void,
    ) -> HRESULT;
    unsafe fn XGameStreamingUpdateTouchControlsStateOnClient(
        &self,
        client: XGameStreamingClientId,
        operationCount: SIZE_T,
        operations: *const c_void,
    ) -> HRESULT;
    unsafe fn XGameStreamingShowTouchControlsWithStateUpdate(
        &self,
        layout: *const c_char,
        operationCount: SIZE_T,
        operations: *const c_void,
    ) -> HRESULT;
    unsafe fn XGameStreamingShowTouchControlsWithStateUpdateOnClient(
        &self,
        client: XGameStreamingClientId,
        layout: *const c_char,
        operationCount: SIZE_T,
        operations: *const c_void,
    ) -> HRESULT;
    unsafe fn XGameStreamingGetTouchBundleVersionNameSize(
        &self,
        client: XGameStreamingClientId,
    ) -> SIZE_T;
    unsafe fn XGameStreamingGetTouchBundleVersion(
        &self,
        client: XGameStreamingClientId,
        version: *mut c_void,
        versionNameSize: SIZE_T,
        versionName: *mut c_char,
    ) -> HRESULT;
    unsafe fn XGameStreamingGetClientIPAddress(
        &self,
        client: XGameStreamingClientId,
        ipAddressSize: SIZE_T,
        ipAddress: *mut c_char,
    ) -> HRESULT;
}

#[interface("5f5e5169-746c-4001-ad1c-da728d01c9eb")]
pub(crate) unsafe trait IXGameStreamingImpl2: IXGameStreamingImpl {
    unsafe fn XGameStreamingGetSessionId(
        &self,
        client: XGameStreamingClientId,
        sessionIdSize: SIZE_T,
        sessionId: *mut c_char,
        sessionIdUsed: *mut SIZE_T,
    ) -> HRESULT;
}

#[interface("57786622-6605-46d0-b917-0f22bbcd9c52")]
pub(crate) unsafe trait IXGameStreamingImpl3: IXGameStreamingImpl2 {
    unsafe fn XGameStreamingGetDisplayDetails(
        &self,
        client: XGameStreamingClientId,
        maxSupportedPixels: UINT32,
        widestSupportedAspectRatio: FLOAT,
        tallestSupportedAspectRatio: FLOAT,
        displayDetails: *mut c_void,
    ) -> HRESULT;
    unsafe fn XGameStreamingSetResolution(&self, width: UINT32, height: UINT32) -> HRESULT;
}

#[implement(IXGameStreamingImpl, IXGameStreamingImpl2, IXGameStreamingImpl3)]
pub(crate) struct XGameStreaming;

impl IXGameStreamingImpl_Impl for XGameStreaming_Impl {
    unsafe fn XGameStreamingInitialize(&self) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameStreamingUninitialize(&self) {}
    unsafe fn XGameStreamingIsStreaming(&self) -> BOOLEAN {
        FALSE
    }
    unsafe fn XGameStreamingRegisterClientPropertiesChanged(
        &self,
        _client: XGameStreamingClientId,
        _queue: XTaskQueueHandle,
        _context: *mut c_void,
        _callback: *mut c_void,
        _token: *mut XTaskQueueRegistrationToken,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameStreamingUnregisterClientPropertiesChanged(
        &self,
        _client: XGameStreamingClientId,
        _token: XTaskQueueRegistrationToken,
        _wait: BOOLEAN,
    ) -> BOOLEAN {
        FALSE
    }
    unsafe fn XGameStreamingGetStreamPhysicalDimensions(
        &self,
        _client: XGameStreamingClientId,
        _horizontalMm: *mut UINT32,
        _verticalMm: *mut UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameStreamingGetClientCount(&self) -> UINT32 {
        0
    }
    unsafe fn XGameStreamingGetClients(
        &self,
        _clientCount: UINT32,
        _clients: *mut XGameStreamingClientId,
        _clientsUsed: *mut UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameStreamingGetConnectionState(&self, _client: XGameStreamingClientId) -> UINT32 {
        0 // XGameStreamingConnectionState_Disconnected
    }
    unsafe fn XGameStreamingRegisterConnectionStateChanged(
        &self,
        _queue: XTaskQueueHandle,
        _context: *mut c_void,
        _callback: *mut c_void,
        _token: *mut XTaskQueueRegistrationToken,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameStreamingUnregisterConnectionStateChanged(
        &self,
        _token: XTaskQueueRegistrationToken,
        _wait: BOOLEAN,
    ) -> BOOLEAN {
        FALSE
    }
    unsafe fn XGameStreamingGetStreamAddedLatency(
        &self,
        _client: XGameStreamingClientId,
        _averageInputLatencyUs: *mut UINT32,
        _averageOutputLatencyUs: *mut UINT32,
        _standardDeviationUs: *mut UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameStreamingGetServerLocationNameSize(&self) -> SIZE_T {
        0
    }
    unsafe fn XGameStreamingGetServerLocationName(
        &self,
        _serverLocationNameSize: SIZE_T,
        _serverLocationName: *mut c_char,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameStreamingHideTouchControls(&self) {}
    unsafe fn XGameStreamingShowTouchControlLayout(&self, _layout: *const c_char) {}
    unsafe fn XGameStreamingHideTouchControlsOnClient(&self, _client: XGameStreamingClientId) {}
    unsafe fn XGameStreamingShowTouchControlLayoutOnClient(
        &self,
        _client: XGameStreamingClientId,
        _layout: *const c_char,
    ) {
    }
    unsafe fn XGameStreamingIsTouchInputEnabled(
        &self,
        _client: XGameStreamingClientId,
        _touchInputEnabled: *mut BOOLEAN,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameStreamingGetLastFrameDisplayed(
        &self,
        _client: XGameStreamingClientId,
        _framePipelineToken: *mut c_void,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameStreamingGetAssociatedFrame(
        &self,
        _gamepadReading: *mut c_void,
        _framePipelineToken: *mut c_void,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameStreamingGetGamepadPhysicality(
        &self,
        _gamepadReading: *mut c_void,
        _gamepadPhysicality: *mut c_void,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameStreamingUpdateTouchControlsState(
        &self,
        _operationCount: SIZE_T,
        _operations: *const c_void,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameStreamingUpdateTouchControlsStateOnClient(
        &self,
        _client: XGameStreamingClientId,
        _operationCount: SIZE_T,
        _operations: *const c_void,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameStreamingShowTouchControlsWithStateUpdate(
        &self,
        _layout: *const c_char,
        _operationCount: SIZE_T,
        _operations: *const c_void,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameStreamingShowTouchControlsWithStateUpdateOnClient(
        &self,
        _client: XGameStreamingClientId,
        _layout: *const c_char,
        _operationCount: SIZE_T,
        _operations: *const c_void,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameStreamingGetTouchBundleVersionNameSize(
        &self,
        _client: XGameStreamingClientId,
    ) -> SIZE_T {
        0
    }
    unsafe fn XGameStreamingGetTouchBundleVersion(
        &self,
        _client: XGameStreamingClientId,
        _version: *mut c_void,
        _versionNameSize: SIZE_T,
        _versionName: *mut c_char,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameStreamingGetClientIPAddress(
        &self,
        _client: XGameStreamingClientId,
        _ipAddressSize: SIZE_T,
        _ipAddress: *mut c_char,
    ) -> HRESULT {
        E_NOTIMPL
    }
}

impl IXGameStreamingImpl2_Impl for XGameStreaming_Impl {
    unsafe fn XGameStreamingGetSessionId(
        &self,
        _client: XGameStreamingClientId,
        _sessionIdSize: SIZE_T,
        _sessionId: *mut c_char,
        _sessionIdUsed: *mut SIZE_T,
    ) -> HRESULT {
        E_NOTIMPL
    }
}

impl IXGameStreamingImpl3_Impl for XGameStreaming_Impl {
    unsafe fn XGameStreamingGetDisplayDetails(
        &self,
        _client: XGameStreamingClientId,
        _maxSupportedPixels: UINT32,
        _widestSupportedAspectRatio: FLOAT,
        _tallestSupportedAspectRatio: FLOAT,
        _displayDetails: *mut c_void,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameStreamingSetResolution(&self, _width: UINT32, _height: UINT32) -> HRESULT {
        E_NOTIMPL
    }
}

// ---------------------------------------------------------------------------------------
// XGameUiImpl / XGameUiImpl2 / XGameUiImpl3 / XGameUiImpl4 (`XGameUiImpl.c`)
// ---------------------------------------------------------------------------------------

/// `coclass XGameUiImpl` (`dfcd4649-...`).
pub(crate) const CLSID_XGAME_UI: GUID = GUID::from_u128(0xdfcd4649_4ff8_4043_ba07_35d607df98b0);

#[interface("6eeaa73e-9669-43ad-a2c7-d0da4e1f50a1")]
pub(crate) unsafe trait IXGameUiImpl: IUnknown {
    unsafe fn XGameUiShowMessageDialogAsync(
        &self,
        async_: *mut c_void,
        titleText: *const c_char,
        contentText: *const c_char,
        firstButtonText: *const c_char,
        secondButtonText: *const c_char,
        thirdButtonText: *const c_char,
        defaultButton: UINT32,
        cancelButton: UINT32,
    ) -> HRESULT;
    unsafe fn XGameUiShowMessageDialogResult(
        &self,
        async_: *mut c_void,
        resultButton: *mut UINT32,
    ) -> HRESULT;
    unsafe fn XGameUiShowSendGameInviteAsync(
        &self,
        async_: *mut c_void,
        requestingUser: XUserHandle,
        sessionConfigurationId: *const c_char,
        sessionTemplateName: *const c_char,
        sessionId: *const c_char,
        invitationText: *const c_char,
        customActivationContext: *const c_char,
    ) -> HRESULT;
    unsafe fn XGameUiShowSendGameInviteResult(&self, async_: *mut c_void) -> HRESULT;
    unsafe fn XGameUiShowPlayerProfileCardAsync(
        &self,
        async_: *mut c_void,
        requestingUser: XUserHandle,
        targetPlayer: UINT64,
    ) -> HRESULT;
    unsafe fn XGameUiShowPlayerProfileCardResult(&self, async_: *mut c_void) -> HRESULT;
    unsafe fn XGameUiShowAchievementsAsync(
        &self,
        async_: *mut c_void,
        requestingUser: XUserHandle,
        titleId: UINT32,
    ) -> HRESULT;
    unsafe fn XGameUiShowAchievementsResult(&self, async_: *mut c_void) -> HRESULT;
    unsafe fn XGameUiShowPlayerPickerAsync(
        &self,
        async_: *mut c_void,
        requestingUser: XUserHandle,
        promptText: *const c_char,
        selectFromPlayersCount: UINT32,
        selectFromPlayers: *const UINT64,
        preSelectedPlayersCount: UINT32,
        preSelectedPlayers: *const UINT64,
        minSelectionCount: UINT32,
        maxSelectionCount: UINT32,
    ) -> HRESULT;
    unsafe fn XGameUiShowPlayerPickerResultCount(
        &self,
        async_: *mut c_void,
        resultPlayersCount: *mut UINT32,
    ) -> HRESULT;
    unsafe fn XGameUiShowPlayerPickerResult(
        &self,
        async_: *mut c_void,
        resultPlayersCount: UINT32,
        resultPlayers: *mut UINT64,
        resultPlayersUsed: *mut UINT32,
    ) -> HRESULT;
    unsafe fn XGameUiShowErrorDialogAsync(
        &self,
        async_: *mut c_void,
        errorCode: i32,
        context: *const c_char,
    ) -> HRESULT;
    unsafe fn XGameUiShowErrorDialogResult(&self, async_: *mut c_void) -> HRESULT;
    unsafe fn XGameUiSetNotificationPositionHint(&self, position: UINT32) -> HRESULT;
    unsafe fn XGameUiShowTextEntryAsync(
        &self,
        async_: *mut c_void,
        titleText: *const c_char,
        descriptionText: *const c_char,
        defaultText: *const c_char,
        inputScope: UINT32,
        maxTextLength: UINT32,
    ) -> HRESULT;
    unsafe fn XGameUiShowTextEntryResultSize(
        &self,
        async_: *mut c_void,
        resultTextBufferSize: *mut UINT32,
    ) -> HRESULT;
    unsafe fn XGameUiShowTextEntryResult(
        &self,
        async_: *mut c_void,
        resultTextBufferSize: UINT32,
        resultTextBuffer: *mut c_char,
        resultTextBufferUsed: *mut UINT32,
    ) -> HRESULT;
    /// Reserved vtable slot - `__PADDING__()` etc. in `xgameui.idl`.
    unsafe fn __PaddingSlot21(&self) -> HRESULT;
    unsafe fn __PaddingSlot22(&self) -> HRESULT;
    unsafe fn __PaddingSlot23(&self) -> HRESULT;
    unsafe fn __PaddingSlot24(&self) -> HRESULT;
    unsafe fn XGameUiShowWebAuthenticationAsync(
        &self,
        async_: *mut c_void,
        requestingUser: XUserHandle,
        requestUri: *const c_char,
        completionUri: *const c_char,
    ) -> HRESULT;
    unsafe fn XGameUiShowWebAuthenticationResultSize(
        &self,
        async_: *mut c_void,
        bufferSize: *mut SIZE_T,
    ) -> HRESULT;
    unsafe fn XGameUiShowWebAuthenticationResult(
        &self,
        async_: *mut c_void,
        bufferSize: SIZE_T,
        buffer: *mut c_void,
        ptrToBuffer: *mut *mut c_void,
        bufferUsed: *mut SIZE_T,
    ) -> HRESULT;
    unsafe fn XGameUiShowWebAuthenticationWithOptionsAsync(
        &self,
        async_: *mut c_void,
        requestingUser: XUserHandle,
        requestUri: *const c_char,
        completionUri: *const c_char,
        options: UINT32,
    ) -> HRESULT;
    /// Reserved vtable slot - `__PADDING_5__()` / `__PADDING_6__()` in `xgameui.idl`.
    unsafe fn __PaddingSlot29(&self) -> HRESULT;
    unsafe fn __PaddingSlot30(&self) -> HRESULT;
}

#[interface("36a03122-9ea3-4a3a-a8a4-899cfd85d7db")]
pub(crate) unsafe trait IXGameUiImpl2: IXGameUiImpl {
    unsafe fn XGameUiShowMultiplayerActivityGameInviteAsync(
        &self,
        async_: *mut c_void,
        requestingUser: XUserHandle,
    ) -> HRESULT;
    unsafe fn XGameUiShowMultiplayerActivityGameInviteResult(&self, async_: *mut c_void)
    -> HRESULT;
    /// Reserved vtable slots - `__PADDING_7__()` / `__PADDING_8__()`.
    unsafe fn __PaddingSlot33(&self) -> HRESULT;
    unsafe fn __PaddingSlot34(&self) -> HRESULT;
    unsafe fn XGameUiTextEntryOpen(
        &self,
        options: *const c_void,
        maxLength: UINT32,
        initialText: *const c_char,
        initialCursorIndex: UINT32,
        handle: *mut XGameUiTextEntryHandle,
    ) -> HRESULT;
    unsafe fn XGameUiTextEntryClose(&self, handle: XGameUiTextEntryHandle) -> HRESULT;
    unsafe fn XGameUiTextEntryGetState(
        &self,
        handle: XGameUiTextEntryHandle,
        changeType: *mut UINT32,
        cursorIndex: *mut UINT32,
        imeClauseStartIndex: *mut UINT32,
        imeClauseEndIndex: *mut UINT32,
        bufferSize: UINT32,
        buffer: *mut c_char,
    ) -> HRESULT;
    unsafe fn XGameUiTextEntryGetExtents(
        &self,
        handle: XGameUiTextEntryHandle,
        extents: *mut c_void,
    ) -> HRESULT;
    unsafe fn XGameUiTextEntryUpdatePositionHint(
        &self,
        handle: XGameUiTextEntryHandle,
        positionHint: UINT32,
    ) -> HRESULT;
    unsafe fn XGameUiTextEntryUpdateVisibility(
        &self,
        handle: XGameUiTextEntryHandle,
        visibilityFlags: UINT32,
    ) -> HRESULT;
}

#[interface("ade7eba1-2093-42ce-a544-a523a66790e0")]
pub(crate) unsafe trait IXGameUiImpl3: IXGameUiImpl2 {
    unsafe fn XGameUiShowStateShareAsync(
        &self,
        async_: *mut c_void,
        requestingUser: XUserHandle,
        linkToken: *const c_char,
    ) -> HRESULT;
    unsafe fn XGameUiShowStateShareResult(&self, async_: *mut c_void) -> HRESULT;
}

#[interface("eaf669df-5542-4590-99a3-8dc061f837cc")]
pub(crate) unsafe trait IXGameUiImpl4: IXGameUiImpl3 {
    unsafe fn XGameUiSetUiCallbacks(
        &self,
        callbacks: *const c_void,
        useSystemUiIfAvailable: BOOLEAN,
    ) -> HRESULT;
    unsafe fn XGameUiSetMessageDialogUiResponse(
        &self,
        callbackHandle: XGameUiCallbackHandle,
        response: UINT32,
    ) -> HRESULT;
    unsafe fn XGameUiSetPlayerPickerUiResponse(
        &self,
        callbackHandle: XGameUiCallbackHandle,
        playerCount: UINT32,
        players: *const UINT64,
    ) -> HRESULT;
    unsafe fn XGameUiSetTextEntryUiResponse(
        &self,
        callbackHandle: XGameUiCallbackHandle,
        response: *const c_char,
    ) -> HRESULT;
    unsafe fn XGameUiSetPlayerProfileCardUiResponse(
        &self,
        callbackHandle: XGameUiCallbackHandle,
    ) -> HRESULT;
    unsafe fn XGameUiSetSendGameInviteUiResponse(
        &self,
        callbackHandle: XGameUiCallbackHandle,
    ) -> HRESULT;
    unsafe fn XGameUiSetAchievementsUiResponse(
        &self,
        callbackHandle: XGameUiCallbackHandle,
    ) -> HRESULT;
    unsafe fn XGameUiSetMultiplayerActivityGameInviteUiResponse(
        &self,
        callbackHandle: XGameUiCallbackHandle,
    ) -> HRESULT;
    unsafe fn XGameUiSetErrorDialogUiResponse(
        &self,
        callbackHandle: XGameUiCallbackHandle,
    ) -> HRESULT;
}

#[implement(IXGameUiImpl, IXGameUiImpl2, IXGameUiImpl3, IXGameUiImpl4)]
pub(crate) struct XGameUi;

impl IXGameUiImpl_Impl for XGameUi_Impl {
    unsafe fn XGameUiShowMessageDialogAsync(
        &self,
        _async_: *mut c_void,
        _titleText: *const c_char,
        _contentText: *const c_char,
        _firstButtonText: *const c_char,
        _secondButtonText: *const c_char,
        _thirdButtonText: *const c_char,
        _defaultButton: UINT32,
        _cancelButton: UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiShowMessageDialogResult(
        &self,
        _async_: *mut c_void,
        _resultButton: *mut UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiShowSendGameInviteAsync(
        &self,
        _async_: *mut c_void,
        _requestingUser: XUserHandle,
        _sessionConfigurationId: *const c_char,
        _sessionTemplateName: *const c_char,
        _sessionId: *const c_char,
        _invitationText: *const c_char,
        _customActivationContext: *const c_char,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiShowSendGameInviteResult(&self, _async_: *mut c_void) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiShowPlayerProfileCardAsync(
        &self,
        _async_: *mut c_void,
        _requestingUser: XUserHandle,
        _targetPlayer: UINT64,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiShowPlayerProfileCardResult(&self, _async_: *mut c_void) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiShowAchievementsAsync(
        &self,
        _async_: *mut c_void,
        _requestingUser: XUserHandle,
        _titleId: UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiShowAchievementsResult(&self, _async_: *mut c_void) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiShowPlayerPickerAsync(
        &self,
        _async_: *mut c_void,
        _requestingUser: XUserHandle,
        _promptText: *const c_char,
        _selectFromPlayersCount: UINT32,
        _selectFromPlayers: *const UINT64,
        _preSelectedPlayersCount: UINT32,
        _preSelectedPlayers: *const UINT64,
        _minSelectionCount: UINT32,
        _maxSelectionCount: UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiShowPlayerPickerResultCount(
        &self,
        _async_: *mut c_void,
        _resultPlayersCount: *mut UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiShowPlayerPickerResult(
        &self,
        _async_: *mut c_void,
        _resultPlayersCount: UINT32,
        _resultPlayers: *mut UINT64,
        _resultPlayersUsed: *mut UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiShowErrorDialogAsync(
        &self,
        _async_: *mut c_void,
        _errorCode: i32,
        _context: *const c_char,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiShowErrorDialogResult(&self, _async_: *mut c_void) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiSetNotificationPositionHint(&self, _position: UINT32) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiShowTextEntryAsync(
        &self,
        _async_: *mut c_void,
        _titleText: *const c_char,
        _descriptionText: *const c_char,
        _defaultText: *const c_char,
        _inputScope: UINT32,
        _maxTextLength: UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiShowTextEntryResultSize(
        &self,
        _async_: *mut c_void,
        _resultTextBufferSize: *mut UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiShowTextEntryResult(
        &self,
        _async_: *mut c_void,
        _resultTextBufferSize: UINT32,
        _resultTextBuffer: *mut c_char,
        _resultTextBufferUsed: *mut UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn __PaddingSlot21(&self) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn __PaddingSlot22(&self) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn __PaddingSlot23(&self) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn __PaddingSlot24(&self) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiShowWebAuthenticationAsync(
        &self,
        _async_: *mut c_void,
        _requestingUser: XUserHandle,
        _requestUri: *const c_char,
        _completionUri: *const c_char,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiShowWebAuthenticationResultSize(
        &self,
        _async_: *mut c_void,
        _bufferSize: *mut SIZE_T,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiShowWebAuthenticationResult(
        &self,
        _async_: *mut c_void,
        _bufferSize: SIZE_T,
        _buffer: *mut c_void,
        _ptrToBuffer: *mut *mut c_void,
        _bufferUsed: *mut SIZE_T,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiShowWebAuthenticationWithOptionsAsync(
        &self,
        _async_: *mut c_void,
        _requestingUser: XUserHandle,
        _requestUri: *const c_char,
        _completionUri: *const c_char,
        _options: UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn __PaddingSlot29(&self) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn __PaddingSlot30(&self) -> HRESULT {
        E_NOTIMPL
    }
}

impl IXGameUiImpl2_Impl for XGameUi_Impl {
    unsafe fn XGameUiShowMultiplayerActivityGameInviteAsync(
        &self,
        _async_: *mut c_void,
        _requestingUser: XUserHandle,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiShowMultiplayerActivityGameInviteResult(
        &self,
        _async_: *mut c_void,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn __PaddingSlot33(&self) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn __PaddingSlot34(&self) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiTextEntryOpen(
        &self,
        _options: *const c_void,
        _maxLength: UINT32,
        _initialText: *const c_char,
        _initialCursorIndex: UINT32,
        _handle: *mut XGameUiTextEntryHandle,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiTextEntryClose(&self, _handle: XGameUiTextEntryHandle) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiTextEntryGetState(
        &self,
        _handle: XGameUiTextEntryHandle,
        _changeType: *mut UINT32,
        _cursorIndex: *mut UINT32,
        _imeClauseStartIndex: *mut UINT32,
        _imeClauseEndIndex: *mut UINT32,
        _bufferSize: UINT32,
        _buffer: *mut c_char,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiTextEntryGetExtents(
        &self,
        _handle: XGameUiTextEntryHandle,
        _extents: *mut c_void,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiTextEntryUpdatePositionHint(
        &self,
        _handle: XGameUiTextEntryHandle,
        _positionHint: UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiTextEntryUpdateVisibility(
        &self,
        _handle: XGameUiTextEntryHandle,
        _visibilityFlags: UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
}

impl IXGameUiImpl3_Impl for XGameUi_Impl {
    unsafe fn XGameUiShowStateShareAsync(
        &self,
        _async_: *mut c_void,
        _requestingUser: XUserHandle,
        _linkToken: *const c_char,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiShowStateShareResult(&self, _async_: *mut c_void) -> HRESULT {
        E_NOTIMPL
    }
}

impl IXGameUiImpl4_Impl for XGameUi_Impl {
    unsafe fn XGameUiSetUiCallbacks(
        &self,
        _callbacks: *const c_void,
        _useSystemUiIfAvailable: BOOLEAN,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiSetMessageDialogUiResponse(
        &self,
        _callbackHandle: XGameUiCallbackHandle,
        _response: UINT32,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiSetPlayerPickerUiResponse(
        &self,
        _callbackHandle: XGameUiCallbackHandle,
        _playerCount: UINT32,
        _players: *const UINT64,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiSetTextEntryUiResponse(
        &self,
        _callbackHandle: XGameUiCallbackHandle,
        _response: *const c_char,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiSetPlayerProfileCardUiResponse(
        &self,
        _callbackHandle: XGameUiCallbackHandle,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiSetSendGameInviteUiResponse(
        &self,
        _callbackHandle: XGameUiCallbackHandle,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiSetAchievementsUiResponse(
        &self,
        _callbackHandle: XGameUiCallbackHandle,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiSetMultiplayerActivityGameInviteUiResponse(
        &self,
        _callbackHandle: XGameUiCallbackHandle,
    ) -> HRESULT {
        E_NOTIMPL
    }
    unsafe fn XGameUiSetErrorDialogUiResponse(
        &self,
        _callbackHandle: XGameUiCallbackHandle,
    ) -> HRESULT {
        E_NOTIMPL
    }
}

// ---------------------------------------------------------------------------------------
// Per-class singletons + dispatch, mirroring `com.rs`'s `OnceLock<GlobalInterface<..>>`
// ---------------------------------------------------------------------------------------

struct GlobalInterface<T>(T);

unsafe impl<T> Send for GlobalInterface<T> {}
unsafe impl<T> Sync for GlobalInterface<T> {}

static XACCESSIBILITY_SINGLETON: OnceLock<GlobalInterface<IXAccessibilityImpl2>> = OnceLock::new();
static XAPPCAPTURE_SINGLETON: OnceLock<GlobalInterface<IXAppCaptureImpl4>> = OnceLock::new();
static XAPPCAPTURE_METADATA_SINGLETON: OnceLock<GlobalInterface<IXAppCaptureMetadataImpl>> =
    OnceLock::new();
static XDISPLAY_SINGLETON: OnceLock<GlobalInterface<IXDisplayImpl>> = OnceLock::new();
static XLAUNCHER_SINGLETON: OnceLock<GlobalInterface<IXLauncherImpl>> = OnceLock::new();
static XGAME_ACTIVATION_SINGLETON: OnceLock<GlobalInterface<IXGameActivationImpl>> =
    OnceLock::new();
static XGAME_EVENT_SINGLETON: OnceLock<GlobalInterface<IXGameEventImpl>> = OnceLock::new();
static XGAME_STREAMING_SINGLETON: OnceLock<GlobalInterface<IXGameStreamingImpl3>> = OnceLock::new();
static XGAME_UI_SINGLETON: OnceLock<GlobalInterface<IXGameUiImpl4>> = OnceLock::new();

pub(crate) fn xaccessibility_singleton() -> &'static IXAccessibilityImpl2 {
    &XACCESSIBILITY_SINGLETON
        .get_or_init(|| GlobalInterface(XAccessibility.into()))
        .0
}

pub(crate) fn xappcapture_singleton() -> &'static IXAppCaptureImpl4 {
    &XAPPCAPTURE_SINGLETON
        .get_or_init(|| GlobalInterface(XAppCapture.into()))
        .0
}

pub(crate) fn xappcapture_metadata_singleton() -> &'static IXAppCaptureMetadataImpl {
    &XAPPCAPTURE_METADATA_SINGLETON
        .get_or_init(|| GlobalInterface(XAppCaptureMetadata.into()))
        .0
}

pub(crate) fn xdisplay_singleton() -> &'static IXDisplayImpl {
    &XDISPLAY_SINGLETON
        .get_or_init(|| GlobalInterface(XDisplay.into()))
        .0
}

pub(crate) fn xlauncher_singleton() -> &'static IXLauncherImpl {
    &XLAUNCHER_SINGLETON
        .get_or_init(|| GlobalInterface(XLauncher.into()))
        .0
}

pub(crate) fn xgame_activation_singleton() -> &'static IXGameActivationImpl {
    &XGAME_ACTIVATION_SINGLETON
        .get_or_init(|| GlobalInterface(XGameActivation.into()))
        .0
}

pub(crate) fn xgame_event_singleton() -> &'static IXGameEventImpl {
    &XGAME_EVENT_SINGLETON
        .get_or_init(|| GlobalInterface(XGameEvent.into()))
        .0
}

pub(crate) fn xgame_streaming_singleton() -> &'static IXGameStreamingImpl3 {
    &XGAME_STREAMING_SINGLETON
        .get_or_init(|| GlobalInterface(XGameStreaming.into()))
        .0
}

pub(crate) fn xgame_ui_singleton() -> &'static IXGameUiImpl4 {
    &XGAME_UI_SINGLETON
        .get_or_init(|| GlobalInterface(XGameUi.into()))
        .0
}

fn query<T: Interface + Clone>(
    object: &T,
    interface_id: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    if interface_id.is_null() || out.is_null() {
        return E_POINTER;
    }
    let object = object.clone();
    let interface_id = unsafe { *interface_id };
    if unsafe { object.query(&interface_id, out) }.is_ok() {
        S_OK
    } else {
        unsafe {
            *out = std::ptr::null_mut();
        }
        E_NOINTERFACE
    }
}

/// `QueryApiImpl` equivalent for the stub surface, mirroring `com.rs`'s per-class dispatch.
/// Returns `S_OK`/`E_NOINTERFACE` when `class_id` is one of ours, `None` when it isn't (so
/// `com::query_api_impl` falls through to its own arms / `E_NOTIMPL` fallback).
pub(crate) fn query_stubbed(
    class_id: GUID,
    interface_id: *const GUID,
    out: *mut *mut c_void,
) -> Option<HRESULT> {
    let result = match class_id {
        CLSID_XACCESSIBILITY => query(xaccessibility_singleton(), interface_id, out),
        CLSID_XAPPCAPTURE => query(xappcapture_singleton(), interface_id, out),
        CLSID_XAPPCAPTURE_METADATA => query(xappcapture_metadata_singleton(), interface_id, out),
        CLSID_XDISPLAY => query(xdisplay_singleton(), interface_id, out),
        CLSID_XLAUNCHER => query(xlauncher_singleton(), interface_id, out),
        CLSID_XGAME_ACTIVATION => query(xgame_activation_singleton(), interface_id, out),
        CLSID_XGAME_EVENT => query(xgame_event_singleton(), interface_id, out),
        CLSID_XGAME_STREAMING => query(xgame_streaming_singleton(), interface_id, out),
        CLSID_XGAME_UI => query(xgame_ui_singleton(), interface_id, out),
        _ => return None,
    };
    Some(result)
}
