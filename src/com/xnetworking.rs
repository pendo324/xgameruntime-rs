use crate::E_FAIL;
use crate::com::xasync::{self, get_result};
use crate::results::*;
use std::ffi::{CStr, c_char, c_void};

pub const CLSID_XNETWORKING: GUID = GUID::from_u128(0x37e56907_2f10_41e8_b72f_36edb185331a);
use std::mem::size_of;
use std::ptr::null_mut;
use windows_core::{GUID, HRESULT, IUnknown, implement, interface};
use windows_sys::core::BOOL;

use super::bool_stub;
use super::hresult_stub_panic;
#[interface("bf2346b2-39af-4658-b5ea-44713c7e83b3")]
pub unsafe trait IXNetworking: IUnknown {
    pub unsafe fn XNetworkingQueryPreferredLocalUdpMultiplayerPort(
        &self,
        preferredLocalUdpMultiplayerPort: *mut u16,
    ) -> HRESULT;
    pub unsafe fn XNetworkingQueryPreferredLocalUdpMultiplayerPortAsync(
        &self,
        asyncBlock: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XNetworkingQueryPreferredLocalUdpMultiplayerPortAsyncResult(
        &self,
        asyncBlock: *mut c_void,
        preferredLocalUdpMultiplayerPort: *mut u16,
    ) -> HRESULT;
    pub unsafe fn XNetworkingRegisterPreferredLocalUdpMultiplayerPortChanged(
        &self,
        queue: u64,
        context: *mut c_void,
        callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XNetworkingUnregisterPreferredLocalUdpMultiplayerPortChanged(
        &self,
        token: u64,
        wait: BOOL,
    ) -> BOOL;
    pub unsafe fn XNetworkingQuerySecurityInformationForUrlAsync(
        &self,
        url: *mut c_char,
        asyncBlock: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XNetworkingQuerySecurityInformationForUrlAsyncResultSize(
        &self,
        asyncBlock: *mut c_void,
        securityInformationBufferByteCount: *mut usize,
    ) -> HRESULT;
    pub unsafe fn XNetworkingQuerySecurityInformationForUrlAsyncResult(
        &self,
        asyncBlock: *mut c_void,
        securityInformationBufferByteCount: u64,
        securityInformationBufferByteCountUsed: *mut usize,
        securityInformationBuffer: *mut u8,
        securityInformation: *mut *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XNetworkingQuerySecurityInformationForUrlUtf16Async(
        &self,
        url: *mut u16,
        asyncBlock: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XNetworkingQuerySecurityInformationForUrlUtf16AsyncResultSize(
        &self,
        asyncBlock: *mut c_void,
        securityInformationBufferByteCount: *mut usize,
    ) -> HRESULT;
    pub unsafe fn XNetworkingQuerySecurityInformationForUrlUtf16AsyncResult(
        &self,
        asyncBlock: *mut c_void,
        securityInformationBufferByteCount: u64,
        securityInformationBufferByteCountUsed: *mut usize,
        securityInformationBuffer: *mut u8,
        securityInformation: *mut *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XNetworkingVerifyServerCertificate(
        &self,
        requestHandle: *mut c_void,
        securityInformation: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XNetworkingGetConnectivityHint(
        &self,
        connectivityHint: *mut XNetworkingConnectivityHint,
    ) -> HRESULT;
    pub unsafe fn XNetworkingRegisterConnectivityHintChanged(
        &self,
        queue: *mut c_void,
        context: *mut c_void,
        callback: Option<OnChanged>,
        token: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XNetworkingUnregisterConnectivityHintChanged(
        &self,
        token: u64,
        wait: BOOL,
    ) -> BOOL;
    pub unsafe fn XNetworkingQueryConfigurationSetting(
        &self,
        configurationSetting: u64,
        value: *mut u64,
    ) -> HRESULT;
    pub unsafe fn XNetworkingSetConfigurationSetting(
        &self,
        configurationSetting: u64,
        value: u64,
    ) -> HRESULT;
    pub unsafe fn XNetworkingQueryStatistics(
        &self,
        statisticsType: u64,
        statisticsBuffer: *mut c_void,
    ) -> HRESULT;
}

#[interface("37e56907-2f10-41e8-b72f-36edb185331a")]
pub unsafe trait IXNetworking2: IXNetworking {}

#[implement(IXNetworking, IXNetworking2)]
pub struct XNetworkingObject;

#[repr(u32)]
#[allow(dead_code)] // Complete set of GDK connectivity hint values; not all are produced by the runtime yet.
pub enum XNetworkingConnectivityCostHint {
    Unknown = 0,
    Unrestricted = 1,
    Fixed = 2,
    Variable = 3,
}
#[repr(u32)]
#[allow(dead_code)] // Complete set of GDK connectivity hint values; not all are produced by the runtime yet.
pub enum XNetworkingConnectivityLevelHint {
    Unknown = 0,
    None = 1,
    LocalAccess = 2,
    InternetAccess = 3,
    ConstrainedInternetAccess = 4,
}

#[repr(C)]
pub struct XNetworkingConnectivityHint {
    pub connectivity_level: XNetworkingConnectivityLevelHint,
    pub connectivity_cost: XNetworkingConnectivityCostHint,
    pub iana_interface_type: u32,
    pub network_initialized: bool,
    pub approaching_data_limit: bool,
    pub over_data_limit: bool,
    pub roaming: bool,
}

#[repr(C)]
pub struct XNetworkingSecurityInformation {
    enabledHttpSecurityProtocolFlags: u32,
    thumbprintCount: usize,
    thumbprints: *const c_void,
}

// `run_sync` produces its result on a worker thread, so the result type has to cross a
// thread boundary. Sound here because both producers below report zero thumbprints and a
// null `thumbprints` pointer - there is no pointee to race over. Should this ever return a
// real thumbprint array, revisit: the array's ownership would have to cross with it.
// SAFETY: see above - `thumbprints` is always null with a zero count, so there is no
// pointee to race over when this crosses to the run_sync worker thread.
unsafe impl Send for XNetworkingSecurityInformation {}

type OnChanged =
    unsafe extern "system" fn(context: *mut c_void, hint: *const XNetworkingConnectivityHint);

impl IXNetworking_Impl for XNetworkingObject_Impl {
    hresult_stub_panic! {
        unsafe fn XNetworkingQueryPreferredLocalUdpMultiplayerPort(&self, preferredLocalUdpMultiplayerPort: *mut u16) -> HRESULT;
        unsafe fn XNetworkingQueryPreferredLocalUdpMultiplayerPortAsync(&self, asyncBlock: *mut c_void) -> HRESULT;
        unsafe fn XNetworkingQueryPreferredLocalUdpMultiplayerPortAsyncResult(&self, asyncBlock: *mut c_void, preferredLocalUdpMultiplayerPort: *mut u16) -> HRESULT;
        unsafe fn XNetworkingRegisterPreferredLocalUdpMultiplayerPortChanged(&self, queue: u64, context: *mut c_void, callback: *mut c_void, token: *mut c_void) -> HRESULT;
        unsafe fn XNetworkingQueryConfigurationSetting(&self, configurationSetting: u64, value: *mut u64) -> HRESULT;
        unsafe fn XNetworkingSetConfigurationSetting(&self, configurationSetting: u64, value: u64) -> HRESULT;
        unsafe fn XNetworkingQueryStatistics(&self, statisticsType: u64, statisticsBuffer: *mut c_void) -> HRESULT;
    }
    bool_stub! {
        unsafe fn XNetworkingUnregisterPreferredLocalUdpMultiplayerPortChanged(&self, token: u64, wait: BOOL) -> BOOL;
        unsafe fn XNetworkingUnregisterConnectivityHintChanged(&self, token: u64, wait: BOOL) -> BOOL;
    }

    unsafe fn XNetworkingGetConnectivityHint(
        &self,
        connectivityHint: *mut XNetworkingConnectivityHint,
    ) -> HRESULT {
        if connectivityHint.is_null() {
            return E_POINTER;
        }
        // SAFETY: connectivityHint was null-checked above and is a valid
        // XNetworkingConnectivityHint out-pointer per the GDK contract.
        unsafe {
            *connectivityHint = XNetworkingConnectivityHint {
                connectivity_level: XNetworkingConnectivityLevelHint::InternetAccess,
                connectivity_cost: XNetworkingConnectivityCostHint::Unrestricted,
                iana_interface_type: 6,
                network_initialized: true,
                approaching_data_limit: false,
                over_data_limit: false,
                roaming: false,
            };
        }
        S_OK
    }

    unsafe fn XNetworkingVerifyServerCertificate(
        &self,
        _requestHandle: *mut c_void,
        _securityInformation: *mut c_void,
    ) -> HRESULT {
        S_OK
    }

    unsafe fn XNetworkingRegisterConnectivityHintChanged(
        &self,
        _queue: *mut c_void,
        context: *mut c_void,
        callback: Option<OnChanged>,
        _token: *mut c_void,
    ) -> HRESULT {
        if let Some(callback) = callback {
            // SAFETY: callback is a caller-supplied function pointer registered
            // via XNetworkingRegisterConnectivityHintChanged, matching OnChanged's
            // signature per that registration's contract.
            unsafe {
                callback(
                    context,
                    &XNetworkingConnectivityHint {
                        connectivity_level: XNetworkingConnectivityLevelHint::InternetAccess,
                        connectivity_cost: XNetworkingConnectivityCostHint::Unrestricted,
                        iana_interface_type: 6,
                        network_initialized: true,
                        approaching_data_limit: false,
                        over_data_limit: false,
                        roaming: false,
                    },
                )
            };
        }
        S_OK
    }

    unsafe fn XNetworkingQuerySecurityInformationForUrlAsync(
        &self,
        url: *mut c_char,
        asyncBlock: *mut c_void,
    ) -> HRESULT {
        // SAFETY: url is a GDK-caller-supplied, NUL-terminated C string per the
        // XNetworkingQuerySecurityInformationForUrlAsync contract.
        let _url = unsafe { CStr::from_ptr(url) };
        // SAFETY: asyncBlock is the caller's XAsyncBlock pointer per the XAsync
        // GDK contract; run_sync itself no-ops on a null pointer.
        unsafe {
            xasync::run_sync(asyncBlock.cast(), move || {
                Ok(XNetworkingSecurityInformation {
                    enabledHttpSecurityProtocolFlags: 0x00000080
                        | 0x00000200
                        | 0x00000800
                        | 0x00002000,
                    thumbprintCount: 0,
                    thumbprints: null_mut(),
                })
            })
        }
    }

    unsafe fn XNetworkingQuerySecurityInformationForUrlAsyncResultSize(
        &self,
        asyncBlock: *mut c_void,
        securityInformationBufferByteCount: *mut usize,
    ) -> HRESULT {
        // SAFETY: asyncBlock is the caller's live XAsyncBlock pointer per the
        // XAsync GDK contract for this ResultSize call.
        let r = unsafe { xasync::get_result_size(asyncBlock.cast()) };
        match r {
            // SAFETY: securityInformationBufferByteCount is a valid usize
            // out-pointer per the GDK contract.
            Ok(size) => unsafe {
                *securityInformationBufferByteCount = size;
                S_OK
            },
            Err(hr) => hr,
        }
    }

    unsafe fn XNetworkingQuerySecurityInformationForUrlAsyncResult(
        &self,
        asyncBlock: *mut c_void,
        securityInformationBufferByteCount: u64,
        securityInformationBufferByteCountUsed: *mut usize,
        securityInformationBuffer: *mut u8,
        securityInformation: *mut *mut c_void,
    ) -> HRESULT {
        if securityInformationBufferByteCount < size_of::<XNetworkingSecurityInformation>() as u64 {
            return E_FAIL;
        }
        if !securityInformationBufferByteCountUsed.is_null() {
            // SAFETY: securityInformationBufferByteCountUsed was null-checked
            // above and is a valid usize out-pointer per the GDK contract.
            unsafe { *securityInformationBufferByteCountUsed = 0 };
        }
        // SAFETY: securityInformationBufferByteCount was checked above to be at
        // least size_of::<XNetworkingSecurityInformation>(), so
        // securityInformationBuffer is large enough for get_result's write.
        match unsafe {
            get_result(
                asyncBlock.cast(),
                null_mut(),
                securityInformationBuffer.cast::<XNetworkingSecurityInformation>(),
            )
        } {
            Ok(_) => {
                if !securityInformationBufferByteCountUsed.is_null() {
                    // SAFETY: securityInformationBufferByteCountUsed was
                    // null-checked above and is a valid usize out-pointer per the
                    // GDK contract.
                    unsafe {
                        *securityInformationBufferByteCountUsed =
                            size_of::<XNetworkingSecurityInformation>()
                    };
                }
                // SAFETY: securityInformation is a valid out-pointer per the GDK
                // contract, and securityInformationBuffer was just populated above.
                unsafe { *securityInformation = securityInformationBuffer.cast() };
                S_OK
            }
            Err(hr) => hr,
        }
    }

    unsafe fn XNetworkingQuerySecurityInformationForUrlUtf16Async(
        &self,
        _url: *mut u16,
        asyncBlock: *mut c_void,
    ) -> HRESULT {
        // SAFETY: asyncBlock is the caller's XAsyncBlock pointer per the XAsync
        // GDK contract; run_sync itself no-ops on a null pointer.
        unsafe {
            xasync::run_sync(asyncBlock.cast(), move || {
                Ok(XNetworkingSecurityInformation {
                    enabledHttpSecurityProtocolFlags: 0x00000080
                        | 0x00000200
                        | 0x00000800
                        | 0x00002000,
                    thumbprintCount: 0,
                    thumbprints: null_mut(),
                })
            })
        }
    }

    unsafe fn XNetworkingQuerySecurityInformationForUrlUtf16AsyncResultSize(
        &self,
        asyncBlock: *mut c_void,
        securityInformationBufferByteCount: *mut usize,
    ) -> HRESULT {
        // SAFETY: asyncBlock is the caller's live XAsyncBlock pointer per the
        // XAsync GDK contract for this ResultSize call.
        let r = unsafe { xasync::get_result_size(asyncBlock.cast()) };
        match r {
            // SAFETY: securityInformationBufferByteCount is a valid usize
            // out-pointer per the GDK contract.
            Ok(size) => unsafe {
                *securityInformationBufferByteCount = size;
                S_OK
            },
            Err(hr) => hr,
        }
    }

    unsafe fn XNetworkingQuerySecurityInformationForUrlUtf16AsyncResult(
        &self,
        asyncBlock: *mut c_void,
        securityInformationBufferByteCount: u64,
        securityInformationBufferByteCountUsed: *mut usize,
        securityInformationBuffer: *mut u8,
        securityInformation: *mut *mut c_void,
    ) -> HRESULT {
        if securityInformationBufferByteCount < size_of::<XNetworkingSecurityInformation>() as u64 {
            return E_FAIL;
        }
        if !securityInformationBufferByteCountUsed.is_null() {
            // SAFETY: securityInformationBufferByteCountUsed was null-checked
            // above and is a valid usize out-pointer per the GDK contract.
            unsafe { *securityInformationBufferByteCountUsed = 0 };
        }
        // SAFETY: securityInformationBufferByteCount was checked above to be at
        // least size_of::<XNetworkingSecurityInformation>(), so
        // securityInformationBuffer is large enough for get_result's write.
        match unsafe {
            get_result(
                asyncBlock.cast(),
                null_mut(),
                securityInformationBuffer.cast::<XNetworkingSecurityInformation>(),
            )
        } {
            Ok(_) => {
                if !securityInformationBufferByteCountUsed.is_null() {
                    // SAFETY: securityInformationBufferByteCountUsed was
                    // null-checked above and is a valid usize out-pointer per the
                    // GDK contract.
                    unsafe {
                        *securityInformationBufferByteCountUsed =
                            size_of::<XNetworkingSecurityInformation>()
                    };
                }
                // SAFETY: securityInformation is a valid out-pointer per the GDK
                // contract, and securityInformationBuffer was just populated above.
                unsafe { *securityInformation = securityInformationBuffer.cast() };
                S_OK
            }
            Err(hr) => hr,
        }
    }
}

impl IXNetworking2_Impl for XNetworkingObject_Impl {}
