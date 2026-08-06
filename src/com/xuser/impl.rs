//! The `IXUser*`/`IXUserDevice*` COM objects: the `XUserObject` and `XUserDeviceObject`
//! plus their vtable implementations. These are thin COM faces over the sign-in engine in
//! [`super::core`].

use super::core::{read_utf16_cstr, *};
use super::{
    AppLocalDeviceId, Boolean, FALSE, TRUE, XTaskQueueHandle, XTaskQueueRegistrationToken,
    XUserChangeEventCallback, XUserDefaultAudioEndpointUtf16ChangedCallback,
    XUserDeviceAssociationChangedCallback, XUserGetTokenAndSignatureData,
    XUserGetTokenAndSignatureHttpHeader, XUserGetTokenAndSignatureUtf16Data,
    XUserGetTokenAndSignatureUtf16HttpHeader, XUserLocalId,
    XUserPlatformRemoteConnectClosePromptEventHandler, XUserPlatformRemoteConnectEventHandlers,
    XUserPlatformRemoteConnectShowPromptEventHandler, XUserPlatformSpopPromptEventHandler,
    XUserStateValue,
};
use crate::com::xasync::{self, XAsyncBlock};
use crate::results::*;

use std::collections::HashMap;
use std::ffi::{c_char, c_void};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, Weak};

use windows_core::{HRESULT, IUnknown, implement, interface};

use crate::E_NOTIMPL;
use crate::com::hresult_stub;
use crate::com::singleton;
use crate::diag::{diag, stub};

macro_rules! boolean_stub {
    ($(unsafe fn $name:ident (&self $(, $arg:ident : $ty:ty)*) -> Boolean;)*) => {
        $(unsafe fn $name(&self $(, $arg: $ty)*) -> Boolean { $(let _ = $arg;)* FALSE })*
    };
}

#[interface("01acd177-91f9-4763-a38e-ccbb55ce32e0")]
pub unsafe trait IXUserImpl: IUnknown {
    pub unsafe fn XUserDuplicateHandle(&self, handle: u64, duplicated_handle: *mut u64) -> HRESULT;
    pub unsafe fn XUserCloseHandle(&self, user: u64) -> ();
    pub unsafe fn XUserCompare(&self, user1: u64, user2: u64) -> i32;
    pub unsafe fn XUserGetMaxUsers(&self, max_users: *mut u32) -> HRESULT;
    pub unsafe fn XUserAddAsync(&self, options: u32, async_: *mut XAsyncBlock) -> HRESULT;
    pub unsafe fn XUserAddResult(&self, async_: *mut XAsyncBlock, new_user: *mut u64) -> HRESULT;
    pub unsafe fn XUserGetLocalId(&self, user: u64, user_local_id: *mut XUserLocalId) -> HRESULT;
    pub unsafe fn XUserFindUserByLocalId(
        &self,
        user_local_id: XUserLocalId,
        handle: *mut u64,
    ) -> HRESULT;
    pub unsafe fn XUserGetId(&self, user: u64, user_id: *mut u64) -> HRESULT;
    pub unsafe fn XUserFindUserById(&self, user_id: u64, handle: *mut u64) -> HRESULT;
    pub unsafe fn XUserGetIsGuest(&self, user: u64, is_guest: *mut Boolean) -> HRESULT;
    pub unsafe fn XUserGetState(&self, user: u64, state: *mut u32) -> HRESULT;
    pub unsafe fn __padding__(&self) -> HRESULT;
    pub unsafe fn XUserGetGamerPictureAsync(
        &self,
        user: u64,
        picture_size: u32,
        async_: *mut XAsyncBlock,
    ) -> HRESULT;
    pub unsafe fn XUserGetGamerPictureResultSize(
        &self,
        async_: *mut XAsyncBlock,
        buffer_size: *mut usize,
    ) -> HRESULT;
    pub unsafe fn XUserGetGamerPictureResult(
        &self,
        async_: *mut XAsyncBlock,
        buffer_size: usize,
        buffer: *mut c_void,
        buffer_used: *mut usize,
    ) -> HRESULT;
    pub unsafe fn XUserGetAgeGroup(&self, user: u64, age_group: *mut u32) -> HRESULT;
    pub unsafe fn XUserCheckPrivilege(
        &self,
        user: u64,
        options: u32,
        privilege: u32,
        has_privilege: *mut Boolean,
        reason: *mut u32,
    ) -> HRESULT;
    pub unsafe fn XUserResolvePrivilegeWithUiAsync(
        &self,
        user: u64,
        options: u32,
        privilege: u32,
        async_: *mut XAsyncBlock,
    ) -> HRESULT;
    pub unsafe fn XUserResolvePrivilegeWithUiResult(&self, async_: *mut XAsyncBlock) -> HRESULT;
    pub unsafe fn XUserGetTokenAndSignatureAsync(
        &self,
        user: u64,
        options: u32,
        method: *const c_char,
        url: *const c_char,
        header_count: usize,
        headers: *const XUserGetTokenAndSignatureHttpHeader,
        body_size: usize,
        body_buffer: *const c_void,
        async_: *mut XAsyncBlock,
    ) -> HRESULT;
    pub unsafe fn XUserGetTokenAndSignatureResultSize(
        &self,
        async_: *mut XAsyncBlock,
        buffer_size: *mut usize,
    ) -> HRESULT;
    pub unsafe fn XUserGetTokenAndSignatureResult(
        &self,
        async_: *mut XAsyncBlock,
        buffer_size: usize,
        buffer: *mut c_void,
        ptr_to_buffer: *mut *mut XUserGetTokenAndSignatureData,
        buffer_used: *mut usize,
    ) -> HRESULT;
    pub unsafe fn XUserGetTokenAndSignatureUtf16Async(
        &self,
        user: u64,
        options: u32,
        method: *const u16,
        url: *const u16,
        header_count: usize,
        headers: *const XUserGetTokenAndSignatureUtf16HttpHeader,
        body_size: usize,
        body_buffer: *const c_void,
        async_: *mut XAsyncBlock,
    ) -> HRESULT;
    pub unsafe fn XUserGetTokenAndSignatureUtf16ResultSize(
        &self,
        async_: *mut XAsyncBlock,
        buffer_size: *mut usize,
    ) -> HRESULT;
    pub unsafe fn XUserGetTokenAndSignatureUtf16Result(
        &self,
        async_: *mut XAsyncBlock,
        buffer_size: usize,
        buffer: *mut c_void,
        ptr_to_buffer: *mut *mut XUserGetTokenAndSignatureUtf16Data,
        buffer_used: *mut usize,
    ) -> HRESULT;
    pub unsafe fn XUserResolveIssueWithUiAsync(
        &self,
        user: u64,
        url: *const c_char,
        async_: *mut XAsyncBlock,
    ) -> HRESULT;
    pub unsafe fn XUserResolveIssueWithUiResult(&self, async_: *mut XAsyncBlock) -> HRESULT;
    pub unsafe fn XUserResolveIssueWithUiUtf16Async(
        &self,
        user: u64,
        url: *const u16,
        async_: *mut XAsyncBlock,
    ) -> HRESULT;
    pub unsafe fn XUserResolveIssueWithUiUtf16Result(&self, async_: *mut XAsyncBlock) -> HRESULT;
    pub unsafe fn XUserRegisterForChangeEvent(
        &self,
        queue: XTaskQueueHandle,
        context: *mut c_void,
        callback: Option<XUserChangeEventCallback>,
        token: *mut XTaskQueueRegistrationToken,
    ) -> HRESULT;
    pub unsafe fn XUserUnregisterForChangeEvent(
        &self,
        token: XTaskQueueRegistrationToken,
        wait: Boolean,
    ) -> Boolean;
    pub unsafe fn XUserGetSignOutDeferral(&self, deferral: *mut u64) -> HRESULT;
    pub unsafe fn XUserCloseSignOutDeferralHandle(&self, deferral: u64) -> ();
}

#[interface("eb9bf948-18dc-4d82-bbcc-40e0a809c4c0")]
pub unsafe trait IXUserImpl2: IXUserImpl {
    pub unsafe fn XUserAddByIdWithUiAsync(&self, user_id: u64, async_: *mut XAsyncBlock)
    -> HRESULT;
    pub unsafe fn XUserAddByIdWithUiResult(
        &self,
        async_: *mut XAsyncBlock,
        new_user: *mut u64,
    ) -> HRESULT;
}

#[interface("1bf2f8c5-d507-4e52-bb05-f726d0e71161")]
pub unsafe trait IXUserImpl3: IXUserImpl2 {
    pub unsafe fn XUserGetMsaTokenSilentlyAsync(
        &self,
        user: u64,
        options: u32,
        scope: *const c_char,
        async_: *mut XAsyncBlock,
    ) -> HRESULT;
    pub unsafe fn XUserGetMsaTokenSilentlyResult(
        &self,
        async_: *mut XAsyncBlock,
        result_token_size: usize,
        result_token: *mut c_char,
        result_token_used: *mut usize,
    ) -> HRESULT;
    pub unsafe fn XUserGetMsaTokenSilentlyResultSize(
        &self,
        async_: *mut XAsyncBlock,
        token_size: *mut usize,
    ) -> HRESULT;
}

#[interface("079415e3-6727-437f-8e9d-8f8f9b2439f7")]
pub unsafe trait IXUserImpl4: IXUserImpl3 {
    pub unsafe fn XUserIsStoreUser(&self, user: u64) -> Boolean;
}

#[interface("26f3c674-a2fe-44fa-b6c4-a323bc94ff53")]
pub unsafe trait IXUserImpl5: IXUserImpl4 {
    pub unsafe fn XUserPlatformRemoteConnectSetEventHandlers(
        &self,
        queue: XTaskQueueHandle,
        handlers: *const XUserPlatformRemoteConnectEventHandlers,
    ) -> HRESULT;
    pub unsafe fn XUserPlatformRemoteConnectCancelPrompt(&self, operation: u64) -> HRESULT;
    pub unsafe fn XUserPlatformSpopPromptSetEventHandlers(
        &self,
        queue: XTaskQueueHandle,
        handler: Option<XUserPlatformSpopPromptEventHandler>,
        context: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XUserPlatformSpopPromptComplete(&self, operation: u64, result: u32) -> HRESULT;
}

#[interface("5131d685-4394-4ee6-8c18-bfb5d4aef1ff")]
pub unsafe trait IXUserImpl6: IXUserImpl5 {
    pub unsafe fn XUserIsSignOutPresent(&self) -> Boolean;
    pub unsafe fn XUserSignOutAsync(&self, user: u64, async_: *mut XAsyncBlock) -> HRESULT;
    pub unsafe fn XUserSignOutResult(&self, async_: *mut XAsyncBlock) -> HRESULT;
}

#[interface("cef4fac0-7676-4a94-a119-4c43f9eb5b74")]
pub unsafe trait IXUserGamertagImpl: IUnknown {
    pub unsafe fn XUserGetGamertag(
        &self,
        user: u64,
        component: u32,
        gamertag_size: usize,
        gamertag: *mut c_char,
        gamertag_used: *mut usize,
    ) -> HRESULT;
}

#[allow(clippy::too_many_arguments)]
#[implement(
    IXUserImpl,
    IXUserImpl2,
    IXUserImpl3,
    IXUserImpl4,
    IXUserImpl5,
    IXUserImpl6,
    IXUserGamertagImpl
)]
pub struct XUserObject;

/// Result tables keyed by the caller's `XAsyncBlock` pointer. The `X*ResultSize`/`X*Result`
/// GDK contract requires answering before any buffer size is known, so the variable-length
/// payload can't ride in `xasync::run_sync`'s fixed-size `T`.
type PendingMap<T> = Option<HashMap<usize, Result<T, HRESULT>>>;

/// Keyed by the caller's `XAsyncBlock` pointer, same rationale and lifetime tradeoff as
/// `MSA_TOKEN_RESULTS` below - the `(authorization, signature)` pair has no size known
/// until after the IPC round trip, so it can't ride in `xasync::run_sync`'s fixed-size `T`.
static TOKEN_AND_SIGNATURE_RESULTS: Mutex<PendingMap<(String, String)>> = Mutex::new(None);

/// Same as `TOKEN_AND_SIGNATURE_RESULTS`, kept separate for `XUserGetTokenAndSignatureUtf16*`
/// even though the stored `String`s are UTF-8 either way - `ResultSize`/`Result` for this
/// pair report sizes in UTF-16 code units, not bytes, so mixing the two tables would make
/// the key space (`XAsyncBlock` pointer) ambiguous about which encoding a lookup wants.
pub(crate) static TOKEN_AND_SIGNATURE_UTF16_RESULTS: Mutex<PendingMap<(String, String)>> =
    Mutex::new(None);

/// Keyed by the caller's `XAsyncBlock` pointer, same rationale as `TOKEN_AND_SIGNATURE_RESULTS`
/// - the picture is a variable-length raw byte buffer with no size known until after the IPC
///   round trip. An account with no gamer picture set stores `Ok(vec![])` here, not an `Err` -
///   `ipc::get_gamer_picture`'s `Ok(None)` is an absence, and an empty buffer is the
///   equivalent answer at this layer.
pub(crate) static GAMER_PICTURE_RESULTS: Mutex<PendingMap<Vec<u8>>> = Mutex::new(None);

impl IXUserImpl_Impl for XUserObject_Impl {
    unsafe fn XUserDuplicateHandle(&self, handle: u64, duplicated_handle: *mut u64) -> HRESULT {
        if duplicated_handle.is_null() {
            return E_POINTER;
        }
        let Some(user) = (unsafe { UserHandleTable::get(handle) }) else {
            return E_INVALIDARG;
        };
        unsafe { *duplicated_handle = UserHandleTable::create(user) };
        S_OK
    }

    unsafe fn XUserCloseHandle(&self, user: u64) {
        unsafe { UserHandleTable::close(user) };
    }

    unsafe fn XUserCompare(&self, user1: u64, user2: u64) -> i32 {
        let a = unsafe { UserHandleTable::get(user1) };
        let b = unsafe { UserHandleTable::get(user2) };
        match (a, b) {
            (None, None) => 0,
            (Some(a), Some(b)) if Arc::ptr_eq(&a, &b) => 0,
            _ => 1,
        }
    }

    unsafe fn XUserGetMaxUsers(&self, max_users: *mut u32) -> HRESULT {
        if max_users.is_null() {
            return E_POINTER;
        }
        // One local, non-guest user at a time - matches the "single local user" scope
        // this milestone targets.
        unsafe { *max_users = 1 };
        S_OK
    }

    unsafe fn XUserAddAsync(&self, options: u32, async_: *mut XAsyncBlock) -> HRESULT {
        // XUserAddOptions: None=0, AddDefaultUserSilently=1, AllowGuests=2,
        // AddDefaultUserAllowingUI=4 (wine/include/xuser.h).
        diag!(
            "XUserAddAsync called options={options:#x} async_block={:p} callback_set={} queue={:?}",
            async_,
            (unsafe { async_.as_ref() }).is_some_and(|b| b.callback.is_some()),
            (unsafe { async_.as_ref() }).map(|b| b.queue)
        );
        if options & 0b101 == 0 {
            diag!("XUserAddAsync rejecting options={options:#x} -> E_ABORT");
            return E_ABORT;
        }
        let allow_ui = options & 0x04 != 0;

        let hr = unsafe {
            xasync::run_sync(async_.cast(), move || -> Result<u64, HRESULT> {
                match crate::ipc::get_user_info() {
                    Ok(info) => {
                        let r = user_handle_from_info(info);
                        diag!("XUserAddAsync silent path result: {r:?}");
                        r
                    }
                    // A game that only asked for Silently should behave as if nobody is
                    // signed in, not pop a window it didn't ask for.
                    Err(err) if !allow_ui => {
                        diag!("XUserAddAsync get_user_info failed and allow_ui=false: {err:?}");
                        Err(err)
                    }
                    Err(err) => {
                        diag!(
                            "XUserAddAsync get_user_info failed ({err:?}), falling back to interactive_sign_in"
                        );
                        match crate::ipc::interactive_sign_in()? {
                            Some(info) => user_handle_from_info(info),
                            // The human closed the sign-in window without completing it - a
                            // "declined", matching real GDK's behavior when the user backs out
                            // of the account picker.
                            None => Err(E_ABORT),
                        }
                    }
                }
            })
        };
        diag!("XUserAddAsync run_sync (XAsyncBegin) returned {hr:?}");
        hr
    }

    unsafe fn XUserAddResult(&self, async_: *mut XAsyncBlock, new_user: *mut u64) -> HRESULT {
        diag!("XUserAddResult called async_block={:p}", async_);
        if new_user.is_null() {
            diag!("XUserAddResult: new_user is null -> E_POINTER");
            return E_POINTER;
        }
        let hr = match unsafe { xasync::get_result(async_, std::ptr::null(), new_user) } {
            Ok(()) => S_OK,
            Err(hr) => hr,
        };
        diag!("XUserAddResult returning {hr:?}");
        hr
    }

    unsafe fn XUserGetLocalId(&self, user: u64, user_local_id: *mut XUserLocalId) -> HRESULT {
        if user_local_id.is_null() {
            return E_POINTER;
        }
        let Some(user) = (unsafe { UserHandleTable::get(user) }) else {
            return E_INVALIDARG;
        };
        unsafe { *user_local_id = user.local_id };
        S_OK
    }

    unsafe fn XUserFindUserByLocalId(
        &self,
        user_local_id: XUserLocalId,
        handle: *mut u64,
    ) -> HRESULT {
        if handle.is_null() {
            return E_POINTER;
        }
        let registry = USER_REGISTRY.lock().expect("user registry poisoned");
        let Some(user) = registry
            .iter()
            .filter_map(Weak::upgrade)
            .find(|user| user.local_id == user_local_id)
        else {
            unsafe { *handle = 0 };
            return E_INVALIDARG;
        };
        unsafe { *handle = UserHandleTable::create(user) };
        S_OK
    }

    unsafe fn XUserGetId(&self, user: u64, user_id: *mut u64) -> HRESULT {
        if user_id.is_null() {
            return E_POINTER;
        }
        let Some(user) = (unsafe { UserHandleTable::get(user) }) else {
            return E_INVALIDARG;
        };
        unsafe { *user_id = user.user_id };
        S_OK
    }

    unsafe fn XUserFindUserById(&self, user_id: u64, handle: *mut u64) -> HRESULT {
        if handle.is_null() {
            return E_POINTER;
        }
        let registry = USER_REGISTRY.lock().expect("user registry poisoned");
        let Some(user) = registry
            .iter()
            .filter_map(Weak::upgrade)
            .find(|user| user.user_id == user_id)
        else {
            unsafe { *handle = 0 };
            return E_INVALIDARG;
        };
        unsafe { *handle = UserHandleTable::create(user) };
        S_OK
    }

    unsafe fn XUserGetIsGuest(&self, user: u64, is_guest: *mut Boolean) -> HRESULT {
        if is_guest.is_null() {
            return E_POINTER;
        }
        let Some(user) = (unsafe { UserHandleTable::get(user) }) else {
            return E_INVALIDARG;
        };
        unsafe { *is_guest = if user.is_guest { TRUE } else { FALSE } };
        S_OK
    }

    unsafe fn XUserGetState(&self, user: u64, state: *mut u32) -> HRESULT {
        if state.is_null() {
            return E_POINTER;
        }
        let Some(user) = (unsafe { UserHandleTable::get(user) }) else {
            return E_INVALIDARG;
        };
        let value = *user.state.lock().expect("user state poisoned");
        unsafe { *state = value as u32 };
        S_OK
    }

    unsafe fn __padding__(&self) -> HRESULT {
        E_NOTIMPL
    }

    unsafe fn XUserGetGamerPictureAsync(
        &self,
        user: u64,
        picture_size: u32,
        async_: *mut XAsyncBlock,
    ) -> HRESULT {
        if (unsafe { UserHandleTable::get(user) }).is_none() {
            return E_INVALIDARG;
        }
        // XUserGamerPictureSize (Small/Medium/Large/ExtraLarge) is accepted but not
        // forwarded - xodus-service returns the one canonical GameDisplayPicRaw image for
        // every size (see xodus::api::xbox::profile::get_gamer_picture's docs in the xodus
        // repo for why: no static-analysis evidence of which CDN query variant the real
        // client requests per size).
        let _ = picture_size;

        let key = async_ as usize;
        unsafe {
            xasync::run_sync(async_.cast(), move || -> Result<(), HRESULT> {
                let result =
                    crate::ipc::get_gamer_picture().map(|picture| picture.unwrap_or_default());
                let outcome = match &result {
                    Ok(_) => Ok(()),
                    Err(hr) => Err(*hr),
                };
                GAMER_PICTURE_RESULTS
                    .lock()
                    .expect("gamer picture results poisoned")
                    .get_or_insert_with(HashMap::new)
                    .insert(key, result);
                outcome
            })
        }
    }

    unsafe fn XUserGetGamerPictureResultSize(
        &self,
        async_: *mut XAsyncBlock,
        buffer_size: *mut usize,
    ) -> HRESULT {
        if buffer_size.is_null() {
            return E_POINTER;
        }
        let key = async_ as usize;
        let results = GAMER_PICTURE_RESULTS
            .lock()
            .expect("gamer picture results poisoned");
        match results.as_ref().and_then(|results| results.get(&key)) {
            Some(Ok(picture)) => {
                unsafe { *buffer_size = picture.len() };
                S_OK
            }
            Some(Err(hr)) => *hr,
            None => E_ILLEGAL_METHOD_CALL,
        }
    }

    unsafe fn XUserGetGamerPictureResult(
        &self,
        async_: *mut XAsyncBlock,
        buffer_size: usize,
        buffer: *mut c_void,
        buffer_used: *mut usize,
    ) -> HRESULT {
        let key = async_ as usize;
        let results = GAMER_PICTURE_RESULTS
            .lock()
            .expect("gamer picture results poisoned");
        match results.as_ref().and_then(|results| results.get(&key)) {
            Some(Ok(picture)) => {
                if picture.len() > buffer_size {
                    return E_NOT_SUFFICIENT_BUFFER;
                }
                if buffer.is_null() && !picture.is_empty() {
                    return E_POINTER;
                }
                unsafe {
                    if !picture.is_empty() {
                        std::ptr::copy_nonoverlapping(
                            picture.as_ptr(),
                            buffer.cast::<u8>(),
                            picture.len(),
                        );
                    }
                    if !buffer_used.is_null() {
                        *buffer_used = picture.len();
                    }
                }
                S_OK
            }
            Some(Err(hr)) => *hr,
            None => E_ILLEGAL_METHOD_CALL,
        }
    }

    unsafe fn XUserResolveIssueWithUiAsync(
        &self,
        user: u64,
        url: *const c_char,
        async_: *mut XAsyncBlock,
    ) -> HRESULT {
        let _ = (user, async_);
        let url = if url.is_null() {
            "<null>".to_string()
        } else {
            unsafe { std::ffi::CStr::from_ptr(url) }
                .to_string_lossy()
                .into_owned()
        };
        stub!("XUserResolveIssueWithUiAsync(url={url:?}) -> E_NOTIMPL");
        E_NOTIMPL
    }

    unsafe fn XUserResolveIssueWithUiResult(&self, async_: *mut XAsyncBlock) -> HRESULT {
        let _ = async_;
        stub!("XUserResolveIssueWithUiResult -> E_NOTIMPL");
        E_NOTIMPL
    }

    unsafe fn XUserResolveIssueWithUiUtf16Async(
        &self,
        user: u64,
        url: *const u16,
        async_: *mut XAsyncBlock,
    ) -> HRESULT {
        let _ = (user, async_);
        let url = if url.is_null() {
            "<null>".to_string()
        } else {
            let len = (0..).take_while(|&i| unsafe { *url.add(i) } != 0).count();
            String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(url, len) })
        };
        stub!("XUserResolveIssueWithUiUtf16Async(url={url:?}) -> E_NOTIMPL");
        E_NOTIMPL
    }

    unsafe fn XUserResolveIssueWithUiUtf16Result(&self, async_: *mut XAsyncBlock) -> HRESULT {
        let _ = async_;
        stub!("XUserResolveIssueWithUiUtf16Result -> E_NOTIMPL");
        E_NOTIMPL
    }

    unsafe fn XUserGetAgeGroup(&self, user: u64, age_group: *mut u32) -> HRESULT {
        if age_group.is_null() {
            return E_POINTER;
        }
        let Some(user) = (unsafe { UserHandleTable::get(user) }) else {
            return E_INVALIDARG;
        };
        unsafe { *age_group = user.age_group };
        S_OK
    }

    unsafe fn XUserGetTokenAndSignatureAsync(
        &self,
        user: u64,
        _options: u32,
        method: *const c_char,
        url: *const c_char,
        _header_count: usize,
        _headers: *const XUserGetTokenAndSignatureHttpHeader,
        body_size: usize,
        body_buffer: *const c_void,
        async_: *mut XAsyncBlock,
    ) -> HRESULT {
        if (unsafe { UserHandleTable::get(user) }).is_none() {
            return E_INVALIDARG;
        }
        if method.is_null() || url.is_null() {
            return E_POINTER;
        }
        let method = unsafe { std::ffi::CStr::from_ptr(method) }
            .to_string_lossy()
            .into_owned();
        let url = unsafe { std::ffi::CStr::from_ptr(url) }
            .to_string_lossy()
            .into_owned();
        let body = if body_size == 0 || body_buffer.is_null() {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(body_buffer.cast::<u8>(), body_size) }.to_vec()
        };

        diag!("XUserGetTokenAndSignatureAsync(method={method:?}, url={url:?})");
        let key = async_ as usize;
        unsafe {
            xasync::run_sync(async_.cast(), move || -> Result<(), HRESULT> {
                let result = crate::ipc::get_token_and_signature(&method, &url, &body);
                diag!(
                    "XUserGetTokenAndSignatureAsync(url={url:?}) -> {:?}",
                    result.as_ref().map(|_| ()).map_err(|hr: &HRESULT| *hr)
                );
                let outcome = match &result {
                    Ok(_) => Ok(()),
                    Err(hr) => Err(*hr),
                };
                TOKEN_AND_SIGNATURE_RESULTS
                    .lock()
                    .expect("token and signature results poisoned")
                    .get_or_insert_with(HashMap::new)
                    .insert(key, result);
                outcome
            })
        }
    }

    unsafe fn XUserGetTokenAndSignatureResultSize(
        &self,
        async_: *mut XAsyncBlock,
        buffer_size: *mut usize,
    ) -> HRESULT {
        if buffer_size.is_null() {
            return E_POINTER;
        }
        let key = async_ as usize;
        let results = TOKEN_AND_SIGNATURE_RESULTS
            .lock()
            .expect("token and signature results poisoned");
        match results.as_ref().and_then(|results| results.get(&key)) {
            Some(Ok((token, signature))) => {
                unsafe {
                    *buffer_size = size_of::<XUserGetTokenAndSignatureData>()
                        + token.len()
                        + 1
                        + signature.len()
                        + 1
                };
                S_OK
            }
            Some(Err(hr)) => *hr,
            None => E_ILLEGAL_METHOD_CALL,
        }
    }

    unsafe fn XUserGetTokenAndSignatureResult(
        &self,
        async_: *mut XAsyncBlock,
        buffer_size: usize,
        buffer: *mut c_void,
        ptr_to_buffer: *mut *mut XUserGetTokenAndSignatureData,
        buffer_used: *mut usize,
    ) -> HRESULT {
        let key = async_ as usize;
        let results = TOKEN_AND_SIGNATURE_RESULTS
            .lock()
            .expect("token and signature results poisoned");
        match results.as_ref().and_then(|results| results.get(&key)) {
            Some(Ok((token, signature))) => {
                let header_size = size_of::<XUserGetTokenAndSignatureData>();
                let token_size = token.len() + 1;
                let signature_size = signature.len() + 1;
                let needed = header_size + token_size + signature_size;
                if needed > buffer_size {
                    return E_NOT_SUFFICIENT_BUFFER;
                }
                if buffer.is_null() || ptr_to_buffer.is_null() {
                    return E_POINTER;
                }

                unsafe {
                    let base = buffer.cast::<u8>();
                    let token_ptr = base.add(header_size);
                    let signature_ptr = token_ptr.add(token_size);

                    std::ptr::copy_nonoverlapping(token.as_ptr(), token_ptr, token.len());
                    *token_ptr.add(token.len()) = 0;
                    std::ptr::copy_nonoverlapping(
                        signature.as_ptr(),
                        signature_ptr,
                        signature.len(),
                    );
                    *signature_ptr.add(signature.len()) = 0;

                    let data = buffer.cast::<XUserGetTokenAndSignatureData>();
                    (*data).token_size = token_size;
                    (*data).signature_size = signature_size;
                    (*data).token = token_ptr.cast();
                    (*data).signature = signature_ptr.cast();
                    *ptr_to_buffer = data;

                    if !buffer_used.is_null() {
                        *buffer_used = needed;
                    }
                }
                S_OK
            }
            Some(Err(hr)) => *hr,
            None => E_ILLEGAL_METHOD_CALL,
        }
    }

    unsafe fn XUserGetTokenAndSignatureUtf16Async(
        &self,
        user: u64,
        _options: u32,
        method: *const u16,
        url: *const u16,
        _header_count: usize,
        _headers: *const XUserGetTokenAndSignatureUtf16HttpHeader,
        body_size: usize,
        body_buffer: *const c_void,
        async_: *mut XAsyncBlock,
    ) -> HRESULT {
        if (unsafe { UserHandleTable::get(user) }).is_none() {
            return E_INVALIDARG;
        }
        if method.is_null() || url.is_null() {
            return E_POINTER;
        }
        let method = unsafe { read_utf16_cstr(method) };
        let url = unsafe { read_utf16_cstr(url) };
        let body = if body_size == 0 || body_buffer.is_null() {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(body_buffer.cast::<u8>(), body_size) }.to_vec()
        };

        let key = async_ as usize;
        unsafe {
            xasync::run_sync(async_.cast(), move || -> Result<(), HRESULT> {
                let result = crate::ipc::get_token_and_signature(&method, &url, &body);
                let outcome = match &result {
                    Ok(_) => Ok(()),
                    Err(hr) => Err(*hr),
                };
                TOKEN_AND_SIGNATURE_UTF16_RESULTS
                    .lock()
                    .expect("token and signature utf16 results poisoned")
                    .get_or_insert_with(HashMap::new)
                    .insert(key, result);
                outcome
            })
        }
    }

    unsafe fn XUserGetTokenAndSignatureUtf16ResultSize(
        &self,
        async_: *mut XAsyncBlock,
        buffer_size: *mut usize,
    ) -> HRESULT {
        if buffer_size.is_null() {
            return E_POINTER;
        }
        let key = async_ as usize;
        let results = TOKEN_AND_SIGNATURE_UTF16_RESULTS
            .lock()
            .expect("token and signature utf16 results poisoned");
        match results.as_ref().and_then(|results| results.get(&key)) {
            Some(Ok((token, signature))) => {
                let token_count = token.encode_utf16().count() + 1;
                let signature_count = signature.encode_utf16().count() + 1;
                unsafe {
                    *buffer_size = size_of::<XUserGetTokenAndSignatureUtf16Data>()
                        + (token_count + signature_count) * size_of::<u16>();
                };
                S_OK
            }
            Some(Err(hr)) => *hr,
            None => E_ILLEGAL_METHOD_CALL,
        }
    }

    unsafe fn XUserGetTokenAndSignatureUtf16Result(
        &self,
        async_: *mut XAsyncBlock,
        buffer_size: usize,
        buffer: *mut c_void,
        ptr_to_buffer: *mut *mut XUserGetTokenAndSignatureUtf16Data,
        buffer_used: *mut usize,
    ) -> HRESULT {
        let key = async_ as usize;
        let results = TOKEN_AND_SIGNATURE_UTF16_RESULTS
            .lock()
            .expect("token and signature utf16 results poisoned");
        match results.as_ref().and_then(|results| results.get(&key)) {
            Some(Ok((token, signature))) => {
                let header_size = size_of::<XUserGetTokenAndSignatureUtf16Data>();
                let token_units: Vec<u16> =
                    token.encode_utf16().chain(std::iter::once(0)).collect();
                let signature_units: Vec<u16> =
                    signature.encode_utf16().chain(std::iter::once(0)).collect();
                let token_bytes = token_units.len() * size_of::<u16>();
                let signature_bytes = signature_units.len() * size_of::<u16>();
                let needed = header_size + token_bytes + signature_bytes;
                if needed > buffer_size {
                    return E_NOT_SUFFICIENT_BUFFER;
                }
                if buffer.is_null() || ptr_to_buffer.is_null() {
                    return E_POINTER;
                }

                unsafe {
                    let base = buffer.cast::<u8>();
                    let token_ptr = base.add(header_size).cast::<u16>();
                    let signature_ptr = base.add(header_size + token_bytes).cast::<u16>();

                    std::ptr::copy_nonoverlapping(
                        token_units.as_ptr(),
                        token_ptr,
                        token_units.len(),
                    );
                    std::ptr::copy_nonoverlapping(
                        signature_units.as_ptr(),
                        signature_ptr,
                        signature_units.len(),
                    );

                    let data = buffer.cast::<XUserGetTokenAndSignatureUtf16Data>();
                    (*data).token_count = token_units.len();
                    (*data).signature_count = signature_units.len();
                    (*data).token = token_ptr;
                    (*data).signature = signature_ptr;
                    *ptr_to_buffer = data;

                    if !buffer_used.is_null() {
                        *buffer_used = needed;
                    }
                }
                S_OK
            }
            Some(Err(hr)) => *hr,
            None => E_ILLEGAL_METHOD_CALL,
        }
    }

    unsafe fn XUserCheckPrivilege(
        &self,
        user: u64,
        options: u32,
        privilege: u32,
        has_privilege: *mut Boolean,
        reason: *mut u32,
    ) -> HRESULT {
        diag!("XUserCheckPrivilege(user={user}, options={options:#x}, privilege={privilege:#x})");
        if has_privilege.is_null() {
            return E_POINTER;
        }
        if (unsafe { UserHandleTable::get(user) }).is_none() {
            return E_INVALIDARG;
        }
        // Grant the common privileges initially. Real per-privilege answers need XSTS
        // display claims, which need the IPC client.
        unsafe { *has_privilege = TRUE };
        if !reason.is_null() {
            unsafe { *reason = 0 }; // XUserPrivilegeDenyReason_None
        }
        S_OK
    }

    unsafe fn XUserResolvePrivilegeWithUiAsync(
        &self,
        user: u64,
        options: u32,
        privilege: u32,
        async_: *mut XAsyncBlock,
    ) -> HRESULT {
        diag!(
            "XUserResolvePrivilegeWithUiAsync(user={user}, options={options:#x}, privilege={privilege:#x})"
        );
        // Every privilege is already granted, so there is nothing to resolve.
        unsafe { xasync::run_sync(async_.cast(), move || -> Result<(), HRESULT> { Ok(()) }) }
    }

    unsafe fn XUserResolvePrivilegeWithUiResult(&self, async_: *mut XAsyncBlock) -> HRESULT {
        diag!("XUserResolvePrivilegeWithUiResult");
        match unsafe { xasync::get_result::<()>(async_, std::ptr::null(), &mut () as *mut ()) } {
            Ok(()) => S_OK,
            Err(hr) => hr,
        }
    }

    unsafe fn XUserRegisterForChangeEvent(
        &self,
        _queue: XTaskQueueHandle,
        context: *mut c_void,
        callback: Option<XUserChangeEventCallback>,
        token: *mut XTaskQueueRegistrationToken,
    ) -> HRESULT {
        let Some(callback) = callback else {
            return E_POINTER;
        };
        if token.is_null() {
            return E_POINTER;
        }
        let raw = register_change_event(context, callback);
        unsafe { *token = XTaskQueueRegistrationToken { token: raw } };
        S_OK
    }

    unsafe fn XUserUnregisterForChangeEvent(
        &self,
        token: XTaskQueueRegistrationToken,
        _wait: Boolean,
    ) -> Boolean {
        if unregister_change_event(token.token) {
            TRUE
        } else {
            FALSE
        }
    }

    unsafe fn XUserGetSignOutDeferral(&self, deferral: *mut u64) -> HRESULT {
        if deferral.is_null() {
            return E_POINTER;
        }
        // XUserSignOutAsync (below) runs the whole SigningOut -> SignedOut transition
        // synchronously inside run_sync, so there is no real window to defer - a listener
        // asking for a deferral just gets a handle it can close once it's done, same as if
        // it had actually held one up. A distinct nonzero handle, not zero, so a caller that
        // checks the handle for validity before closing it isn't misled.
        unsafe { *deferral = 1 };
        S_OK
    }

    unsafe fn XUserCloseSignOutDeferralHandle(&self, _deferral: u64) {}
}

impl IXUserImpl2_Impl for XUserObject_Impl {
    unsafe fn XUserAddByIdWithUiAsync(&self, user_id: u64, async_: *mut XAsyncBlock) -> HRESULT {
        // `user_id` is a hint for which account to preselect - real GDK uses it to skip the
        // account picker when that account's credentials are already cached. There is no
        // way to preselect an account in an MSA login page opened fresh (it always starts
        // at "choose an account"/"sign in"), so this crate ignores the hint and always
        // drives the same interactive flow XUserAddAsync(AddDefaultUserAllowingUI) does,
        // rather than fabricating an account-preselection feature that doesn't exist here.
        let _ = user_id;

        unsafe {
            xasync::run_sync(async_.cast(), move || -> Result<u64, HRESULT> {
                match crate::ipc::interactive_sign_in()? {
                    Some(info) => user_handle_from_info(info),
                    // The human closed the sign-in window without completing it - a
                    // "declined", matching real GDK's behavior when the user backs out of
                    // the account picker.
                    None => Err(E_ABORT),
                }
            })
        }
    }

    unsafe fn XUserAddByIdWithUiResult(
        &self,
        async_: *mut XAsyncBlock,
        new_user: *mut u64,
    ) -> HRESULT {
        if new_user.is_null() {
            return E_POINTER;
        }
        match unsafe { xasync::get_result(async_, std::ptr::null(), new_user) } {
            Ok(()) => S_OK,
            Err(hr) => hr,
        }
    }
}

/// Keyed by the caller's `XAsyncBlock` pointer, since the token (unlike the fixed-size
/// payloads `xasync::run_sync`'s generic `T` is built for) has no size known in advance -
/// `XUserGetMsaTokenSilentlyResultSize` has to answer before `Result` is called at all.
/// Entries are not removed on read: a caller is allowed to call `ResultSize` then `Result`
/// with a too-small buffer and retry, and the real GDK contract has no separate "I am
/// done with this async block" signal short of the block going out of scope. This leaks
/// one small entry per call that never asks for its result, same tradeoff already made by
/// `CHANGE_EVENT_REGISTRY` and the user handle table.
static MSA_TOKEN_RESULTS: Mutex<PendingMap<(String, i64)>> = Mutex::new(None);

impl IXUserImpl3_Impl for XUserObject_Impl {
    unsafe fn XUserGetMsaTokenSilentlyAsync(
        &self,
        user: u64,
        _options: u32,
        scope: *const c_char,
        async_: *mut XAsyncBlock,
    ) -> HRESULT {
        if (unsafe { UserHandleTable::get(user) }).is_none() {
            return E_INVALIDARG;
        }
        let scope = if scope.is_null() {
            None
        } else {
            Some(
                unsafe { std::ffi::CStr::from_ptr(scope) }
                    .to_string_lossy()
                    .into_owned(),
            )
        };
        let key = async_ as usize;
        unsafe {
            xasync::run_sync(async_.cast(), move || -> Result<(), HRESULT> {
                let result = crate::ipc::get_msa_token_silently(scope.as_deref());
                let outcome = match &result {
                    Ok(_) => Ok(()),
                    Err(hr) => Err(*hr),
                };
                MSA_TOKEN_RESULTS
                    .lock()
                    .expect("msa token results poisoned")
                    .get_or_insert_with(HashMap::new)
                    .insert(key, result);
                outcome
            })
        }
    }

    unsafe fn XUserGetMsaTokenSilentlyResultSize(
        &self,
        async_: *mut XAsyncBlock,
        token_size: *mut usize,
    ) -> HRESULT {
        if token_size.is_null() {
            return E_POINTER;
        }
        let key = async_ as usize;
        let results = MSA_TOKEN_RESULTS
            .lock()
            .expect("msa token results poisoned");
        match results.as_ref().and_then(|results| results.get(&key)) {
            Some(Ok((token, _))) => {
                unsafe { *token_size = token.len() + 1 };
                S_OK
            }
            Some(Err(hr)) => *hr,
            None => E_ILLEGAL_METHOD_CALL,
        }
    }

    unsafe fn XUserGetMsaTokenSilentlyResult(
        &self,
        async_: *mut XAsyncBlock,
        result_token_size: usize,
        result_token: *mut c_char,
        result_token_used: *mut usize,
    ) -> HRESULT {
        let key = async_ as usize;
        let results = MSA_TOKEN_RESULTS
            .lock()
            .expect("msa token results poisoned");
        match results.as_ref().and_then(|results| results.get(&key)) {
            Some(Ok((token, _))) => {
                let bytes = token.as_bytes();
                let needed = bytes.len() + 1;
                if needed > result_token_size {
                    return E_NOT_SUFFICIENT_BUFFER;
                }
                if result_token.is_null() {
                    return E_POINTER;
                }
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        bytes.as_ptr(),
                        result_token.cast::<u8>(),
                        bytes.len(),
                    );
                    *result_token.cast::<u8>().add(bytes.len()) = 0;
                }
                if !result_token_used.is_null() {
                    unsafe { *result_token_used = needed };
                }
                S_OK
            }
            Some(Err(hr)) => *hr,
            None => E_ILLEGAL_METHOD_CALL,
        }
    }
}

impl IXUserImpl4_Impl for XUserObject_Impl {
    unsafe fn XUserIsStoreUser(&self, user: u64) -> Boolean {
        // Every XUserHandle in this codebase comes from a genuine Xbox Live/MSA sign-in
        // (XUserAddAsync's silent or interactive path, or XUserFindForDevice) - there is no
        // second, non-store identity provider modeled here for this to distinguish against.
        // An unknown handle reports FALSE rather than a stub that returns TRUE unconditionally
        // without checking the handle at all.
        if (unsafe { UserHandleTable::get(user) }).is_some() {
            TRUE
        } else {
            FALSE
        }
    }
}

impl IXUserImpl5_Impl for XUserObject_Impl {
    unsafe fn XUserPlatformRemoteConnectSetEventHandlers(
        &self,
        _queue: XTaskQueueHandle,
        handlers: *const XUserPlatformRemoteConnectEventHandlers,
    ) -> HRESULT {
        if handlers.is_null() {
            return E_POINTER;
        }
        let mut registry = REMOTE_CONNECT_HANDLERS
            .lock()
            .expect("remote connect handlers poisoned");
        *registry = Some(unsafe {
            RemoteConnectHandlers {
                show: (*handlers).show,
                close: (*handlers).close,
                context: (*handlers).context,
            }
        });
        S_OK
    }

    unsafe fn XUserPlatformRemoteConnectCancelPrompt(&self, _operation: u64) -> HRESULT {
        if let Some(handlers) = REMOTE_CONNECT_HANDLERS
            .lock()
            .expect("remote connect handlers poisoned")
            .as_ref()
            && let Some(close) = handlers.close
        {
            unsafe { close() };
        }
        S_OK
    }

    hresult_stub! {
        unsafe fn XUserPlatformSpopPromptSetEventHandlers(&self, queue: XTaskQueueHandle, handler: Option<XUserPlatformSpopPromptEventHandler>, context: *mut c_void) -> HRESULT;
        unsafe fn XUserPlatformSpopPromptComplete(&self, operation: u64, result: u32) -> HRESULT;
    }
}

impl IXUserImpl6_Impl for XUserObject_Impl {
    unsafe fn XUserIsSignOutPresent(&self) -> Boolean {
        FALSE
    }

    unsafe fn XUserSignOutAsync(&self, user: u64, async_: *mut XAsyncBlock) -> HRESULT {
        let Some(user) = (unsafe { UserHandleTable::get(user) }) else {
            return E_INVALIDARG;
        };
        unsafe {
            xasync::run_sync(async_.cast(), move || -> Result<(), HRESULT> {
                *user.state.lock().expect("user state poisoned") = XUserStateValue::SigningOut;
                fire_change_event(user.local_id, CHANGE_EVENT_SIGNING_OUT);
                *user.state.lock().expect("user state poisoned") = XUserStateValue::SignedOut;
                fire_change_event(user.local_id, CHANGE_EVENT_SIGNED_OUT);
                Ok(())
            })
        }
    }

    unsafe fn XUserSignOutResult(&self, async_: *mut XAsyncBlock) -> HRESULT {
        match unsafe { xasync::get_result::<()>(async_, std::ptr::null(), &mut () as *mut ()) } {
            Ok(()) => S_OK,
            Err(hr) => hr,
        }
    }
}

/// `XUserGamertagComponent_Modern`/`_ModernSuffix`/`_UniqueModern` (`wine/include/xuser.h`).
/// A trivial implementation would ignore `component` and always return the one cached classic
/// gamertag; we do have a `gamertag_modern` claim available, so honor the modern-vs-classic
/// distinction where we can.
const GAMERTAG_COMPONENT_MODERN: u32 = 1;

impl IXUserGamertagImpl_Impl for XUserObject_Impl {
    unsafe fn XUserGetGamertag(
        &self,
        user: u64,
        component: u32,
        gamertag_size: usize,
        gamertag: *mut c_char,
        gamertag_used: *mut usize,
    ) -> HRESULT {
        let Some(user) = (unsafe { UserHandleTable::get(user) }) else {
            return E_INVALIDARG;
        };
        let value = if component == GAMERTAG_COMPONENT_MODERN && !user.gamertag_modern.is_empty() {
            &user.gamertag_modern
        } else {
            &user.gamertag
        };
        let bytes = value.as_bytes();
        let needed = bytes.len() + 1;
        if !gamertag_used.is_null() {
            unsafe { *gamertag_used = needed };
        }
        if gamertag.is_null() || gamertag_size == 0 {
            return S_OK;
        }
        if gamertag_size < needed {
            return E_NOT_SUFFICIENT_BUFFER;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), gamertag.cast::<u8>(), bytes.len());
            *gamertag.cast::<u8>().add(bytes.len()) = 0;
        }
        S_OK
    }
}

#[allow(dead_code)] // Handlers are retained by the platform for the remote-connect prompt.
struct RemoteConnectHandlers {
    show: Option<XUserPlatformRemoteConnectShowPromptEventHandler>,
    close: Option<XUserPlatformRemoteConnectClosePromptEventHandler>,
    context: *mut c_void,
}

unsafe impl Send for RemoteConnectHandlers {}
unsafe impl Sync for RemoteConnectHandlers {}

static REMOTE_CONNECT_HANDLERS: Mutex<Option<RemoteConnectHandlers>> = Mutex::new(None);

// ---------------------------------------------------------------------------------------
// IXUserDeviceImpl / IXUserDeviceImpl2
// ---------------------------------------------------------------------------------------

#[interface("7d824997-10dc-45ab-86b7-2737767c0bf1")]
pub unsafe trait IXUserDeviceImpl: IUnknown {
    pub unsafe fn XUserFindForDevice(
        &self,
        device_id: *const AppLocalDeviceId,
        handle: *mut u64,
    ) -> HRESULT;
    pub unsafe fn XUserRegisterForDeviceAssociationChanged(
        &self,
        queue: XTaskQueueHandle,
        context: *mut c_void,
        callback: Option<XUserDeviceAssociationChangedCallback>,
        token: *mut XTaskQueueRegistrationToken,
    ) -> HRESULT;
    pub unsafe fn XUserUnregisterForDeviceAssociationChanged(
        &self,
        token: XTaskQueueRegistrationToken,
        wait: Boolean,
    ) -> Boolean;
    pub unsafe fn XUserGetDefaultAudioEndpointUtf16(
        &self,
        user: XUserLocalId,
        kind: u32,
        endpoint_id_utf16_count: usize,
        endpoint_id_utf16: *mut u16,
        endpoint_id_utf16_used: *mut usize,
    ) -> HRESULT;
    pub unsafe fn XUserRegisterForDefaultAudioEndpointUtf16Changed(
        &self,
        queue: XTaskQueueHandle,
        context: *mut c_void,
        callback: Option<XUserDefaultAudioEndpointUtf16ChangedCallback>,
        token: *mut XTaskQueueRegistrationToken,
    ) -> HRESULT;
    pub unsafe fn XUserUnregisterForDefaultAudioEndpointUtf16Changed(
        &self,
        token: XTaskQueueRegistrationToken,
        wait: Boolean,
    ) -> Boolean;
    unsafe fn XUserFindControllerForUserWithUiAsync(
        &self,
        user: u64,
        async_: *mut XAsyncBlock,
    ) -> HRESULT;
    unsafe fn XUserFindControllerForUserWithUiResult(
        &self,
        async_: *mut XAsyncBlock,
        device_id: *mut AppLocalDeviceId,
    ) -> HRESULT;
}

#[interface("0cc6a956-e7e1-4fdf-9341-9d5da649ebc8")]
pub unsafe trait IXUserDeviceImpl2: IXUserDeviceImpl {}

#[implement(IXUserDeviceImpl, IXUserDeviceImpl2)]
pub struct XUserDeviceObject;

impl IXUserDeviceImpl_Impl for XUserDeviceObject_Impl {
    unsafe fn XUserFindForDevice(
        &self,
        device_id: *const AppLocalDeviceId,
        handle: *mut u64,
    ) -> HRESULT {
        if device_id.is_null() || handle.is_null() {
            return E_POINTER;
        }
        // This crate caps at one signed-in local user (see `max_users_is_one`), so there
        // is no real controller-to-user routing decision to make: any device belongs to
        // the sole signed-in user, or to nobody if none is signed in. Real per-device
        // identity (evdev enumeration, hotplug tracking) would only matter for
        // disambiguating between multiple users, which can't happen here - so
        // `device_id` is intentionally not inspected.
        let registry = USER_REGISTRY.lock().expect("user registry poisoned");
        let Some(user) = registry.iter().filter_map(Weak::upgrade).find(|user| {
            *user.state.lock().expect("user state poisoned") == XUserStateValue::SignedIn
        }) else {
            unsafe { *handle = 0 };
            return E_INVALIDARG;
        };
        unsafe { *handle = UserHandleTable::create(user) };
        S_OK
    }

    unsafe fn XUserRegisterForDeviceAssociationChanged(
        &self,
        _queue: XTaskQueueHandle,
        _context: *mut c_void,
        callback: Option<XUserDeviceAssociationChangedCallback>,
        token: *mut XTaskQueueRegistrationToken,
    ) -> HRESULT {
        if callback.is_none() || token.is_null() {
            return E_POINTER;
        }
        unsafe {
            *token = XTaskQueueRegistrationToken {
                token: NEXT_CHANGE_EVENT_TOKEN.fetch_add(1, Ordering::Relaxed),
            }
        };
        S_OK
    }

    boolean_stub! {
        unsafe fn XUserUnregisterForDeviceAssociationChanged(&self, token: XTaskQueueRegistrationToken, wait: Boolean) -> Boolean;
    }

    // Deliberately not implemented: these are the audio-device-selection slots, and the
    // GDK's own notes forbid stubbing audio-endpoint functionality (NDA'd).
    hresult_stub! {
        unsafe fn XUserGetDefaultAudioEndpointUtf16(&self, user: XUserLocalId, kind: u32, endpoint_id_utf16_count: usize, endpoint_id_utf16: *mut u16, endpoint_id_utf16_used: *mut usize) -> HRESULT;
        unsafe fn XUserRegisterForDefaultAudioEndpointUtf16Changed(&self, queue: XTaskQueueHandle, context: *mut c_void, callback: Option<XUserDefaultAudioEndpointUtf16ChangedCallback>, token: *mut XTaskQueueRegistrationToken) -> HRESULT;
        unsafe fn XUserFindControllerForUserWithUiAsync(&self, user: u64, async_: *mut XAsyncBlock) -> HRESULT;
        unsafe fn XUserFindControllerForUserWithUiResult(&self, async_: *mut XAsyncBlock, device_id: *mut AppLocalDeviceId) -> HRESULT;
    }

    boolean_stub! {
        unsafe fn XUserUnregisterForDefaultAudioEndpointUtf16Changed(&self, token: XTaskQueueRegistrationToken, wait: Boolean) -> Boolean;
    }
}

impl IXUserDeviceImpl2_Impl for XUserDeviceObject_Impl {}

// ---------------------------------------------------------------------------------------
// Singletons
// ---------------------------------------------------------------------------------------

singleton! {
    pub fn xuser_singleton() -> IXUserImpl6 = XUserObject;
}

singleton! {
    pub fn xuser_device_singleton() -> IXUserDeviceImpl2 = XUserDeviceObject;
}

/// Resolves an `XUserHandle` to the xuid it was signed in with, for callers outside this
/// module (`XGameSave`'s provider initialization namespaces save containers per user).
/// `None` for a null or unrecognized handle.
///
/// # Safety
/// `handle` must be zero or a handle from [`UserHandleTable::create`] that has not been closed.
pub unsafe fn user_id_for_handle(handle: u64) -> Option<u64> {
    unsafe { UserHandleTable::get(handle) }.map(|user| user.user_id)
}

/// A real, `UserHandleTable`-backed handle for `xgamesave`'s cross-module tests to pass to
/// [`user_id_for_handle`] - unlike this crate's own tests, `xgamesave`'s can't build a
/// `UserState` directly since both the struct and `UserHandleTable::create` are private here.
#[cfg(test)]
pub(crate) fn create_test_user_handle(user_id: u64) -> u64 {
    let user = Arc::new(UserState {
        local_id: XUserLocalId { value: user_id },
        user_id,
        is_guest: false,
        state: Mutex::new(XUserStateValue::SignedIn),
        gamertag: "TestGamer".to_string(),
        gamertag_modern: String::new(),
        age_group: 3,
    });
    UserHandleTable::create(user)
}
