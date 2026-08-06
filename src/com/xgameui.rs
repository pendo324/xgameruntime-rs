use super::singleton;
use super::{
    BOOLEAN, SIZE_T, UINT32, UINT64, XGameUiCallbackHandle, XGameUiTextEntryHandle, XUserHandle,
};
use crate::E_NOTIMPL;
use std::ffi::{c_char, c_void};
use windows_core::{GUID, HRESULT, IUnknown, implement, interface};
// ---------------------------------------------------------------------------------------
// XGameUiImpl / XGameUiImpl2 / XGameUiImpl3 / XGameUiImpl4 (`xgameui.idl`)
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

singleton! {
    pub(crate) fn xgameui_singleton() -> IXGameUiImpl4 = XGameUi;
}
