use super::singleton;
use super::{
    BOOLEAN, FALSE, FLOAT, SIZE_T, UINT32, XGameStreamingClientId, XTaskQueueHandle,
    XTaskQueueRegistrationToken,
};
use crate::E_NOTIMPL;
use std::ffi::{c_char, c_void};
use windows_core::{GUID, HRESULT, IUnknown, implement, interface};
// ---------------------------------------------------------------------------------------
// XGameStreamingImpl / XGameStreamingImpl2 / XGameStreamingImpl3 (`xgamestreaming.idl`)
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

singleton! {
    pub(crate) fn xgamestreaming_singleton() -> IXGameStreamingImpl3 = XGameStreaming;
}
