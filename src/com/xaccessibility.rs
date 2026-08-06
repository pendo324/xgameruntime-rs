use super::singleton;
use super::{BOOLEAN, SIZE_T, UINT32, XSpeechSynthesizerHandle, XSpeechSynthesizerStreamHandle};
use crate::E_NOTIMPL;
use std::ffi::{c_char, c_void};
use windows_core::{GUID, HRESULT, IUnknown, implement, interface};
// ---------------------------------------------------------------------------------------
// XAccessibilityImpl (`xaccessibility.idl`)
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

singleton! {
    pub(crate) fn xaccessibility_singleton() -> IXAccessibilityImpl2 = XAccessibility;
}
