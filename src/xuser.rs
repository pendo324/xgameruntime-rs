//! Native `IXUserImpl1-6` / `IXUserGamertagImpl` / `IXUserDeviceImpl1-2`, ported from
//! WineGDK's `XUser.c` vtable shape (`wine/include/xuser.idl` has the authoritative slot
//! order, including the `__PADDING__` at slot 12 of the base interface).
//!
//! This lands the handle table and every slot that can be answered honestly with no
//! external state: duplicate/close/compare, local-id and global-id lookup, guest/state
//! queries, the RemoteConnect plumbing `lib.rs` already wires up. `XUserGetMsaTokenSilentlyAsync`,
//! `XUserGetTokenAndSignature(Utf16)Async`, and `XUserAddAsync` are wired to real
//! `xodus-service` IPC calls (`crate::ipc`); `XUserAddAsync` caches the gamertag/age-group
//! claims it gets back on the resulting `UserState` so the synchronous
//! `XUserGetGamertag`/`XUserGetAgeGroup` getters (real GDK never gives them a network round
//! trip) can answer from it, and `XUserSignOutAsync` transitions that same `UserState` and
//! fires the matching change events. Everything else that needs a real signed-in identity -
//! `XUserAddByIdWithUiAsync`, `XUserResolveIssueWithUi(Utf16)Async` - still returns
//! `E_NOTIMPL` until there's a webview/UI flow to drive it. Faking a signed-in user here
//! would be actively wrong, unlike the XStore license placeholder: a game that thinks nobody
//! is signed in behaves correctly, a game that thinks the wrong person is signed in does not.

use std::collections::HashMap;
use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use windows_core::{GUID, HRESULT, IUnknown, Interface, implement, interface};

use crate::results::*;
use crate::xasync::{self, XAsyncBlock};
use crate::{E_FAIL, E_NOTIMPL};

/// Also `IXUserImpl`'s own IID - Wine's idl reuses it as the coclass id.
pub const CLSID_XUSER: GUID = GUID::from_u128(0x01acd177_91f9_4763_a38e_ccbb55ce32e0);
/// Also `IXUserDeviceImpl`'s own IID, for the same reason.
pub const CLSID_XUSER_DEVICE: GUID = GUID::from_u128(0x7d824997_10dc_45ab_86b7_2737767c0bf1);

/// Win32 `BOOLEAN` - one byte. Distinct from the four-byte `BOOL` used elsewhere in this
/// crate; `xuser.idl` uses `BOOLEAN` throughout, so getting the width wrong would corrupt
/// whatever field follows it across the FFI boundary.
type Boolean = u8;

const TRUE: Boolean = 1;
const FALSE: Boolean = 0;

pub type XTaskQueueHandle = u64;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct XTaskQueueRegistrationToken {
    pub token: u64,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct XUserLocalId {
    pub value: u64,
}

#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum XUserStateValue {
    SignedIn = 0,
    SigningOut = 1,
    SignedOut = 2,
}

#[repr(C)]
pub struct AppLocalDeviceId {
    pub value: [u8; 32],
}

#[repr(C)]
pub struct XUserDeviceAssociationChange {
    pub device_id: AppLocalDeviceId,
    pub old_user: XUserLocalId,
    pub new_user: XUserLocalId,
}

#[repr(C)]
pub struct XUserGetTokenAndSignatureData {
    pub token_size: usize,
    pub signature_size: usize,
    pub token: *const c_char,
    pub signature: *const c_char,
}

#[repr(C)]
pub struct XUserGetTokenAndSignatureHttpHeader {
    pub name: *const c_char,
    pub value: *const c_char,
}

#[repr(C)]
pub struct XUserGetTokenAndSignatureUtf16Data {
    pub token_count: usize,
    pub signature_count: usize,
    pub token: *const u16,
    pub signature: *const u16,
}

#[repr(C)]
pub struct XUserGetTokenAndSignatureUtf16HttpHeader {
    pub name: *const u16,
    pub value: *const u16,
}

pub type XUserChangeEventCallback =
    unsafe extern "system" fn(context: *mut c_void, user_local_id: XUserLocalId, event: u32);
pub type XUserDeviceAssociationChangedCallback =
    unsafe extern "system" fn(context: *mut c_void, change: *const XUserDeviceAssociationChange);
pub type XUserDefaultAudioEndpointUtf16ChangedCallback = unsafe extern "system" fn(
    context: *mut c_void,
    user: XUserLocalId,
    kind: u32,
    endpoint_id_utf16: *const u16,
);
pub type XUserPlatformRemoteConnectShowPromptEventHandler = unsafe extern "system" fn(
    context: *const c_void,
    user_identifier: u32,
    operation: u32,
    url: *const c_char,
    code: *const c_char,
    qr_code_size: usize,
    qr_code: *const c_char,
);
pub type XUserPlatformRemoteConnectClosePromptEventHandler = unsafe extern "system" fn();
pub type XUserPlatformSpopPromptEventHandler = unsafe extern "system" fn(
    context: *mut c_void,
    user_identifier: u32,
    operation: u32,
    modern_gamertag: *const c_char,
    modern_gamertag_suffix: *const c_char,
);

#[repr(C)]
pub struct XUserPlatformRemoteConnectEventHandlers {
    pub show: Option<XUserPlatformRemoteConnectShowPromptEventHandler>,
    pub close: Option<XUserPlatformRemoteConnectClosePromptEventHandler>,
    pub context: *mut c_void,
}

// ---------------------------------------------------------------------------------------
// Handle table
// ---------------------------------------------------------------------------------------

struct UserState {
    local_id: XUserLocalId,
    user_id: u64,
    is_guest: bool,
    state: Mutex<XUserStateValue>,
    /// Cached at sign-in (`XUserAddAsync`), same as real GDK - `XUserGetGamertag`/
    /// `XUserGetAgeGroup` are synchronous, so they can't do a network round trip per call.
    gamertag: String,
    /// Empty when Xbox Live's `mgt` claim wasn't present for this account; `XUserGetGamertag`
    /// falls back to `gamertag` in that case.
    gamertag_modern: String,
    /// `XUserAgeGroup` (`Unknown`=0, `Child`=1, `Teen`=2, `Adult`=3), mapped from Xbox Live's
    /// `agg` claim once at sign-in.
    age_group: u32,
}

/// Users known well enough to answer `XUserFindUserByLocalId`/`XUserFindUserById`.
/// Populated by `XUserAddAsync` once it produces a real user. `Weak` so a fully-closed user
/// (every handle dropped) falls out of the registry on its own rather than needing an
/// explicit sign-out path to clean it up.
static USER_REGISTRY: Mutex<Vec<Weak<UserState>>> = Mutex::new(Vec::new());

fn register_user(user: &Arc<UserState>) {
    let mut registry = USER_REGISTRY.lock().expect("user registry poisoned");
    registry.retain(|entry| entry.strong_count() > 0);
    registry.push(Arc::downgrade(user));
}

/// Xbox Live's `agg` claim (`"Adult"`/`"Teen"`/`"Child"`) mapped to `XUserAgeGroup`
/// (`wine/include/xuser.h`); anything else (including a missing claim) is `Unknown`=0.
fn parse_age_group(agg: &str) -> u32 {
    match agg {
        "Adult" => 3,
        "Teen" => 2,
        "Child" => 1,
        _ => 0,
    }
}

/// A handle table keyed by leaked `Box<Arc<UserState>>` pointers - the same scheme as
/// `task_queue::QueueHandle`. `XUserDuplicateHandle`/`XUserCloseHandle` are the game's
/// refcounting, distinct from (and in addition to) the `Arc`'s own.
struct UserHandleTable;

impl UserHandleTable {
    fn create(user: Arc<UserState>) -> u64 {
        Box::into_raw(Box::new(user)) as u64
    }

    /// # Safety
    /// `handle` must be zero or a handle from [`Self::create`] that has not been closed.
    unsafe fn get(handle: u64) -> Option<Arc<UserState>> {
        if handle == 0 {
            return None;
        }
        Some(unsafe { (*(handle as *const Arc<UserState>)).clone() })
    }

    /// # Safety
    /// `handle` must be an open handle from [`Self::create`]; it is invalid afterwards.
    unsafe fn close(handle: u64) {
        if handle == 0 {
            return;
        }
        drop(unsafe { Box::from_raw(handle as *mut Arc<UserState>) });
    }
}

// ---------------------------------------------------------------------------------------
// Change-event registration
// ---------------------------------------------------------------------------------------

struct ChangeEventRegistration {
    context: *mut c_void,
    callback: XUserChangeEventCallback,
}

// The context and callback are only ever read back and invoked on whatever thread fires
// the event, exactly like `XAsyncWaker` elsewhere in this crate - the game is the one
// asserting these are safe to move by handing them to a registration API in the first
// place.
unsafe impl Send for ChangeEventRegistration {}
unsafe impl Sync for ChangeEventRegistration {}

/// Accepted registrations for `XUserChangeEvent` notifications. `XUserAddAsync` fires
/// `SignedInAgain` (below) on a successful sign-in; nothing else produces a change yet
/// (there is no sign-out or gamertag-change path).
static CHANGE_EVENT_REGISTRY: Mutex<Option<HashMap<u64, ChangeEventRegistration>>> =
    Mutex::new(None);
static NEXT_CHANGE_EVENT_TOKEN: AtomicU64 = AtomicU64::new(1);

/// `XUserChangeEvent` variants (`wine/include/xuser.h`) this crate actually fires.
/// `SignedInAgain` is the only one WineGDK's own `XUserAddAsync` fires, for both a first
/// sign-in and a repeat one. `SigningOut`/`SignedOut` are fired back to back by
/// `XUserSignOutAsync` - there is no real deferral window to hold between them.
const CHANGE_EVENT_SIGNED_IN_AGAIN: u32 = 0;
const CHANGE_EVENT_SIGNING_OUT: u32 = 1;
const CHANGE_EVENT_SIGNED_OUT: u32 = 2;

fn fire_change_event(local_id: XUserLocalId, event: u32) {
    let registry = CHANGE_EVENT_REGISTRY
        .lock()
        .expect("change registry poisoned");
    if let Some(registry) = registry.as_ref() {
        for registration in registry.values() {
            unsafe { (registration.callback)(registration.context, local_id, event) };
        }
    }
}

fn register_change_event(context: *mut c_void, callback: XUserChangeEventCallback) -> u64 {
    let token = NEXT_CHANGE_EVENT_TOKEN.fetch_add(1, Ordering::Relaxed);
    let mut registry = CHANGE_EVENT_REGISTRY
        .lock()
        .expect("change registry poisoned");
    registry
        .get_or_insert_with(HashMap::new)
        .insert(token, ChangeEventRegistration { context, callback });
    token
}

fn unregister_change_event(token: u64) -> bool {
    let mut registry = CHANGE_EVENT_REGISTRY
        .lock()
        .expect("change registry poisoned");
    registry
        .get_or_insert_with(HashMap::new)
        .remove(&token)
        .is_some()
}

/// Reads a null-terminated UTF-16 string from a raw pointer. `ptr` must be non-null and
/// point at a valid null-terminated `u16` sequence - callers check `is_null()` first.
unsafe fn read_utf16_cstr(ptr: *const u16) -> String {
    let mut len = 0usize;
    unsafe {
        while *ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
    }
}

// ---------------------------------------------------------------------------------------
// Stub macros - same shape as com.rs's, but for BOOLEAN (u8) rather than BOOL (i32).
// ---------------------------------------------------------------------------------------

macro_rules! hresult_stub {
    ($(unsafe fn $name:ident (&self $(, $arg:ident : $ty:ty)*) -> HRESULT;)*) => {
        $(unsafe fn $name(&self $(, $arg: $ty)*) -> HRESULT { $(let _ = $arg;)* E_NOTIMPL })*
    };
}

macro_rules! boolean_stub {
    ($(unsafe fn $name:ident (&self $(, $arg:ident : $ty:ty)*) -> Boolean;)*) => {
        $(unsafe fn $name(&self $(, $arg: $ty)*) -> Boolean { $(let _ = $arg;)* FALSE })*
    };
}

// ---------------------------------------------------------------------------------------
// IXUserImpl1-6
// ---------------------------------------------------------------------------------------

#[interface("01acd177-91f9-4763-a38e-ccbb55ce32e0")]
pub unsafe trait IXUserImpl: IUnknown {
    unsafe fn XUserDuplicateHandle(&self, handle: u64, duplicated_handle: *mut u64) -> HRESULT;
    unsafe fn XUserCloseHandle(&self, user: u64) -> ();
    unsafe fn XUserCompare(&self, user1: u64, user2: u64) -> i32;
    unsafe fn XUserGetMaxUsers(&self, max_users: *mut u32) -> HRESULT;
    unsafe fn XUserAddAsync(&self, options: u32, async_: *mut XAsyncBlock) -> HRESULT;
    unsafe fn XUserAddResult(&self, async_: *mut XAsyncBlock, new_user: *mut u64) -> HRESULT;
    unsafe fn XUserGetLocalId(&self, user: u64, user_local_id: *mut XUserLocalId) -> HRESULT;
    unsafe fn XUserFindUserByLocalId(
        &self,
        user_local_id: XUserLocalId,
        handle: *mut u64,
    ) -> HRESULT;
    unsafe fn XUserGetId(&self, user: u64, user_id: *mut u64) -> HRESULT;
    unsafe fn XUserFindUserById(&self, user_id: u64, handle: *mut u64) -> HRESULT;
    unsafe fn XUserGetIsGuest(&self, user: u64, is_guest: *mut Boolean) -> HRESULT;
    unsafe fn XUserGetState(&self, user: u64, state: *mut u32) -> HRESULT;
    unsafe fn __padding__(&self) -> HRESULT;
    unsafe fn XUserGetGamerPictureAsync(
        &self,
        user: u64,
        picture_size: u32,
        async_: *mut XAsyncBlock,
    ) -> HRESULT;
    unsafe fn XUserGetGamerPictureResultSize(
        &self,
        async_: *mut XAsyncBlock,
        buffer_size: *mut usize,
    ) -> HRESULT;
    unsafe fn XUserGetGamerPictureResult(
        &self,
        async_: *mut XAsyncBlock,
        buffer_size: usize,
        buffer: *mut c_void,
        buffer_used: *mut usize,
    ) -> HRESULT;
    unsafe fn XUserGetAgeGroup(&self, user: u64, age_group: *mut u32) -> HRESULT;
    unsafe fn XUserCheckPrivilege(
        &self,
        user: u64,
        options: u32,
        privilege: u32,
        has_privilege: *mut Boolean,
        reason: *mut u32,
    ) -> HRESULT;
    unsafe fn XUserResolvePrivilegeWithUiAsync(
        &self,
        user: u64,
        options: u32,
        privilege: u32,
        async_: *mut XAsyncBlock,
    ) -> HRESULT;
    unsafe fn XUserResolvePrivilegeWithUiResult(&self, async_: *mut XAsyncBlock) -> HRESULT;
    unsafe fn XUserGetTokenAndSignatureAsync(
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
    unsafe fn XUserGetTokenAndSignatureResultSize(
        &self,
        async_: *mut XAsyncBlock,
        buffer_size: *mut usize,
    ) -> HRESULT;
    unsafe fn XUserGetTokenAndSignatureResult(
        &self,
        async_: *mut XAsyncBlock,
        buffer_size: usize,
        buffer: *mut c_void,
        ptr_to_buffer: *mut *mut XUserGetTokenAndSignatureData,
        buffer_used: *mut usize,
    ) -> HRESULT;
    unsafe fn XUserGetTokenAndSignatureUtf16Async(
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
    unsafe fn XUserGetTokenAndSignatureUtf16ResultSize(
        &self,
        async_: *mut XAsyncBlock,
        buffer_size: *mut usize,
    ) -> HRESULT;
    unsafe fn XUserGetTokenAndSignatureUtf16Result(
        &self,
        async_: *mut XAsyncBlock,
        buffer_size: usize,
        buffer: *mut c_void,
        ptr_to_buffer: *mut *mut XUserGetTokenAndSignatureUtf16Data,
        buffer_used: *mut usize,
    ) -> HRESULT;
    unsafe fn XUserResolveIssueWithUiAsync(
        &self,
        user: u64,
        url: *const c_char,
        async_: *mut XAsyncBlock,
    ) -> HRESULT;
    unsafe fn XUserResolveIssueWithUiResult(&self, async_: *mut XAsyncBlock) -> HRESULT;
    unsafe fn XUserResolveIssueWithUiUtf16Async(
        &self,
        user: u64,
        url: *const u16,
        async_: *mut XAsyncBlock,
    ) -> HRESULT;
    unsafe fn XUserResolveIssueWithUiUtf16Result(&self, async_: *mut XAsyncBlock) -> HRESULT;
    unsafe fn XUserRegisterForChangeEvent(
        &self,
        queue: XTaskQueueHandle,
        context: *mut c_void,
        callback: Option<XUserChangeEventCallback>,
        token: *mut XTaskQueueRegistrationToken,
    ) -> HRESULT;
    unsafe fn XUserUnregisterForChangeEvent(
        &self,
        token: XTaskQueueRegistrationToken,
        wait: Boolean,
    ) -> Boolean;
    unsafe fn XUserGetSignOutDeferral(&self, deferral: *mut u64) -> HRESULT;
    unsafe fn XUserCloseSignOutDeferralHandle(&self, deferral: u64) -> ();
}

#[interface("eb9bf948-18dc-4d82-bbcc-40e0a809c4c0")]
pub unsafe trait IXUserImpl2: IXUserImpl {
    unsafe fn XUserAddByIdWithUiAsync(&self, user_id: u64, async_: *mut XAsyncBlock) -> HRESULT;
    unsafe fn XUserAddByIdWithUiResult(
        &self,
        async_: *mut XAsyncBlock,
        new_user: *mut u64,
    ) -> HRESULT;
}

#[interface("1bf2f8c5-d507-4e52-bb05-f726d0e71161")]
pub unsafe trait IXUserImpl3: IXUserImpl2 {
    unsafe fn XUserGetMsaTokenSilentlyAsync(
        &self,
        user: u64,
        options: u32,
        scope: *const c_char,
        async_: *mut XAsyncBlock,
    ) -> HRESULT;
    unsafe fn XUserGetMsaTokenSilentlyResult(
        &self,
        async_: *mut XAsyncBlock,
        result_token_size: usize,
        result_token: *mut c_char,
        result_token_used: *mut usize,
    ) -> HRESULT;
    unsafe fn XUserGetMsaTokenSilentlyResultSize(
        &self,
        async_: *mut XAsyncBlock,
        token_size: *mut usize,
    ) -> HRESULT;
}

#[interface("079415e3-6727-437f-8e9d-8f8f9b2439f7")]
pub unsafe trait IXUserImpl4: IXUserImpl3 {
    unsafe fn XUserIsStoreUser(&self, user: u64) -> Boolean;
}

#[interface("26f3c674-a2fe-44fa-b6c4-a323bc94ff53")]
pub unsafe trait IXUserImpl5: IXUserImpl4 {
    pub unsafe fn XUserPlatformRemoteConnectSetEventHandlers(
        &self,
        queue: XTaskQueueHandle,
        handlers: *const XUserPlatformRemoteConnectEventHandlers,
    ) -> HRESULT;
    unsafe fn XUserPlatformRemoteConnectCancelPrompt(&self, operation: u64) -> HRESULT;
    unsafe fn XUserPlatformSpopPromptSetEventHandlers(
        &self,
        queue: XTaskQueueHandle,
        handler: Option<XUserPlatformSpopPromptEventHandler>,
        context: *mut c_void,
    ) -> HRESULT;
    unsafe fn XUserPlatformSpopPromptComplete(&self, operation: u64, result: u32) -> HRESULT;
}

#[interface("5131d685-4394-4ee6-8c18-bfb5d4aef1ff")]
pub unsafe trait IXUserImpl6: IXUserImpl5 {
    unsafe fn XUserIsSignOutPresent(&self) -> Boolean;
    unsafe fn XUserSignOutAsync(&self, user: u64, async_: *mut XAsyncBlock) -> HRESULT;
    unsafe fn XUserSignOutResult(&self, async_: *mut XAsyncBlock) -> HRESULT;
}

#[interface("cef4fac0-7676-4a94-a119-4c43f9eb5b74")]
pub unsafe trait IXUserGamertagImpl: IUnknown {
    unsafe fn XUserGetGamertag(
        &self,
        user: u64,
        component: u32,
        gamertag_size: usize,
        gamertag: *mut c_char,
        gamertag_used: *mut usize,
    ) -> HRESULT;
}

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

/// Keyed by the caller's `XAsyncBlock` pointer, same rationale and lifetime tradeoff as
/// `MSA_TOKEN_RESULTS` below - the `(authorization, signature)` pair has no size known
/// until after the IPC round trip, so it can't ride in `xasync::run_sync`'s fixed-size `T`.
static TOKEN_AND_SIGNATURE_RESULTS: Mutex<
    Option<HashMap<usize, Result<(String, String), HRESULT>>>,
> = Mutex::new(None);

/// Same as `TOKEN_AND_SIGNATURE_RESULTS`, kept separate for `XUserGetTokenAndSignatureUtf16*`
/// even though the stored `String`s are UTF-8 either way - `ResultSize`/`Result` for this
/// pair report sizes in UTF-16 code units, not bytes, so mixing the two tables would make
/// the key space (`XAsyncBlock` pointer) ambiguous about which encoding a lookup wants.
static TOKEN_AND_SIGNATURE_UTF16_RESULTS: Mutex<
    Option<HashMap<usize, Result<(String, String), HRESULT>>>,
> = Mutex::new(None);

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
        // WineGDK's XUser.c hardcodes the same value: one local, non-guest user at a
        // time. Matches the "single local user" scope this milestone targets.
        unsafe { *max_users = 1 };
        S_OK
    }

    unsafe fn XUserAddAsync(&self, options: u32, async_: *mut XAsyncBlock) -> HRESULT {
        // XUserAddOptions: None=0, AddDefaultUserSilently=1, AllowGuests=2,
        // AddDefaultUserAllowingUI=4 (wine/include/xuser.h). WineGDK's own XUserAddAsync
        // handles Silently and AllowingUI identically (both just load stored credentials);
        // we do the same, since there is no webview/UI architecture in xodus-service to
        // drive an AllowingUI-only interactive sign-in yet - it degrades to the same
        // "fail if nothing is stored" behavior as Silently, rather than fabricating a UI
        // flow that doesn't exist.
        if options & 0b101 == 0 {
            return E_ABORT;
        }

        unsafe {
            xasync::run_sync(async_.cast(), move || -> Result<u64, HRESULT> {
                let (xuid, gamertag, gamertag_modern, age_group) = crate::ipc::get_user_info()?;
                let user_id: u64 = xuid.parse().map_err(|_| E_FAIL)?;

                let existing = {
                    let registry = USER_REGISTRY.lock().expect("user registry poisoned");
                    registry
                        .iter()
                        .filter_map(Weak::upgrade)
                        .find(|user| user.user_id == user_id)
                };

                let user = match existing {
                    Some(user) => user,
                    None => {
                        let user = Arc::new(UserState {
                            local_id: XUserLocalId { value: user_id },
                            user_id,
                            is_guest: false,
                            state: Mutex::new(XUserStateValue::SignedIn),
                            gamertag,
                            gamertag_modern,
                            age_group: parse_age_group(&age_group),
                        });
                        register_user(&user);
                        fire_change_event(user.local_id, CHANGE_EVENT_SIGNED_IN_AGAIN);
                        user
                    }
                };

                Ok(UserHandleTable::create(user))
            })
        }
    }

    unsafe fn XUserAddResult(&self, async_: *mut XAsyncBlock, new_user: *mut u64) -> HRESULT {
        if new_user.is_null() {
            return E_POINTER;
        }
        match unsafe { xasync::get_result(async_, std::ptr::null(), new_user) } {
            Ok(()) => S_OK,
            Err(hr) => hr,
        }
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

    hresult_stub! {
        unsafe fn XUserGetGamerPictureAsync(&self, user: u64, picture_size: u32, async_: *mut XAsyncBlock) -> HRESULT;
        unsafe fn XUserGetGamerPictureResultSize(&self, async_: *mut XAsyncBlock, buffer_size: *mut usize) -> HRESULT;
        unsafe fn XUserGetGamerPictureResult(&self, async_: *mut XAsyncBlock, buffer_size: usize, buffer: *mut c_void, buffer_used: *mut usize) -> HRESULT;
        unsafe fn XUserResolveIssueWithUiAsync(&self, user: u64, url: *const c_char, async_: *mut XAsyncBlock) -> HRESULT;
        unsafe fn XUserResolveIssueWithUiResult(&self, async_: *mut XAsyncBlock) -> HRESULT;
        unsafe fn XUserResolveIssueWithUiUtf16Async(&self, user: u64, url: *const u16, async_: *mut XAsyncBlock) -> HRESULT;
        unsafe fn XUserResolveIssueWithUiUtf16Result(&self, async_: *mut XAsyncBlock) -> HRESULT;
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

        let key = async_ as usize;
        unsafe {
            xasync::run_sync(async_.cast(), move || -> Result<(), HRESULT> {
                let result = crate::ipc::get_token_and_signature(&method, &url, &body);
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
        _options: u32,
        _privilege: u32,
        has_privilege: *mut Boolean,
        reason: *mut u32,
    ) -> HRESULT {
        if has_privilege.is_null() {
            return E_POINTER;
        }
        if (unsafe { UserHandleTable::get(user) }).is_none() {
            return E_INVALIDARG;
        }
        // PLAN.md Phase 3: "grant the common privileges initially". Real per-privilege
        // answers need XSTS display claims, which need the IPC client.
        unsafe { *has_privilege = TRUE };
        if !reason.is_null() {
            unsafe { *reason = 0 }; // XUserPrivilegeDenyReason_None
        }
        S_OK
    }

    unsafe fn XUserResolvePrivilegeWithUiAsync(
        &self,
        _user: u64,
        _options: u32,
        _privilege: u32,
        async_: *mut XAsyncBlock,
    ) -> HRESULT {
        // Every privilege is already granted, so there is nothing to resolve.
        unsafe { xasync::run_sync(async_.cast(), move || -> Result<(), HRESULT> { Ok(()) }) }
    }

    unsafe fn XUserResolvePrivilegeWithUiResult(&self, async_: *mut XAsyncBlock) -> HRESULT {
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
    hresult_stub! {
        unsafe fn XUserAddByIdWithUiAsync(&self, user_id: u64, async_: *mut XAsyncBlock) -> HRESULT;
        unsafe fn XUserAddByIdWithUiResult(&self, async_: *mut XAsyncBlock, new_user: *mut u64) -> HRESULT;
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
static MSA_TOKEN_RESULTS: Mutex<Option<HashMap<usize, Result<(String, i64), HRESULT>>>> =
    Mutex::new(None);

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
    boolean_stub! {
        unsafe fn XUserIsStoreUser(&self, user: u64) -> Boolean;
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
/// WineGDK's own `x_user_gt_XUserGetGamertag` ignores `component` entirely and always
/// returns the one cached classic gamertag; unlike Wine, we do have a `gamertag_modern`
/// claim available, so honor the modern-vs-classic distinction where we can.
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
    unsafe fn XUserFindForDevice(
        &self,
        device_id: *const AppLocalDeviceId,
        handle: *mut u64,
    ) -> HRESULT;
    unsafe fn XUserRegisterForDeviceAssociationChanged(
        &self,
        queue: XTaskQueueHandle,
        context: *mut c_void,
        callback: Option<XUserDeviceAssociationChangedCallback>,
        token: *mut XTaskQueueRegistrationToken,
    ) -> HRESULT;
    unsafe fn XUserUnregisterForDeviceAssociationChanged(
        &self,
        token: XTaskQueueRegistrationToken,
        wait: Boolean,
    ) -> Boolean;
    unsafe fn XUserGetDefaultAudioEndpointUtf16(
        &self,
        user: XUserLocalId,
        kind: u32,
        endpoint_id_utf16_count: usize,
        endpoint_id_utf16: *mut u16,
        endpoint_id_utf16_used: *mut usize,
    ) -> HRESULT;
    unsafe fn XUserRegisterForDefaultAudioEndpointUtf16Changed(
        &self,
        queue: XTaskQueueHandle,
        context: *mut c_void,
        callback: Option<XUserDefaultAudioEndpointUtf16ChangedCallback>,
        token: *mut XTaskQueueRegistrationToken,
    ) -> HRESULT;
    unsafe fn XUserUnregisterForDefaultAudioEndpointUtf16Changed(
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
        // No controller-to-user association is modeled (Linux/Wine has no
        // XInputGetControllerCapabilities-style user binding), so no device is ever
        // associated with a user yet.
        unsafe { *handle = 0 };
        E_NOTIMPL
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

    // Deliberately not implemented: WineGDK's own NOTES forbid stubbing audio-endpoint
    // functionality (NDA'd), and these are the audio-device-selection slots.
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

struct GlobalInterface<T>(T);

unsafe impl<T> Send for GlobalInterface<T> {}
unsafe impl<T> Sync for GlobalInterface<T> {}

static XUSER_SINGLETON: OnceLock<GlobalInterface<IXUserImpl6>> = OnceLock::new();
static XUSER_DEVICE_SINGLETON: OnceLock<GlobalInterface<IXUserDeviceImpl2>> = OnceLock::new();

pub fn xuser_singleton() -> &'static IXUserImpl6 {
    &XUSER_SINGLETON
        .get_or_init(|| GlobalInterface(XUserObject.into()))
        .0
}

pub fn xuser_device_singleton() -> &'static IXUserDeviceImpl2 {
    &XUSER_DEVICE_SINGLETON
        .get_or_init(|| GlobalInterface(XUserDeviceObject.into()))
        .0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_user(local_id: u64, user_id: u64) -> Arc<UserState> {
        Arc::new(UserState {
            local_id: XUserLocalId { value: local_id },
            user_id,
            is_guest: false,
            state: Mutex::new(XUserStateValue::SignedIn),
            gamertag: "TestGamer".to_string(),
            gamertag_modern: String::new(),
            age_group: 3,
        })
    }

    #[test]
    fn max_users_is_one() {
        let mut max = 0u32;
        let hr = unsafe { xuser_singleton().XUserGetMaxUsers(&mut max) };
        assert_eq!(hr, S_OK);
        assert_eq!(max, 1);
    }

    #[test]
    fn duplicate_and_close_share_the_same_underlying_user() {
        let user = make_user(1, 100);
        let handle = UserHandleTable::create(user);

        let mut dup = 0u64;
        let hr = unsafe { xuser_singleton().XUserDuplicateHandle(handle, &mut dup) };
        assert_eq!(hr, S_OK);
        assert_ne!(dup, 0);
        assert_ne!(dup, handle);
        assert_eq!(unsafe { xuser_singleton().XUserCompare(handle, dup) }, 0);

        unsafe { xuser_singleton().XUserCloseHandle(handle) };

        // Closing one duplicate must not invalidate the other.
        let mut id = 0u64;
        let hr = unsafe { xuser_singleton().XUserGetId(dup, &mut id) };
        assert_eq!(hr, S_OK);
        assert_eq!(id, 100);

        unsafe { xuser_singleton().XUserCloseHandle(dup) };
    }

    #[test]
    fn find_by_local_id_and_by_id_return_a_registered_user() {
        let user = make_user(42, 4242);
        register_user(&user);
        let handle = UserHandleTable::create(user);

        let mut found = 0u64;
        let hr = unsafe {
            xuser_singleton().XUserFindUserByLocalId(XUserLocalId { value: 42 }, &mut found)
        };
        assert_eq!(hr, S_OK);
        assert_ne!(found, 0);
        unsafe { xuser_singleton().XUserCloseHandle(found) };

        let mut found_by_id = 0u64;
        let hr = unsafe { xuser_singleton().XUserFindUserById(4242, &mut found_by_id) };
        assert_eq!(hr, S_OK);
        assert_ne!(found_by_id, 0);
        unsafe { xuser_singleton().XUserCloseHandle(found_by_id) };

        unsafe { xuser_singleton().XUserCloseHandle(handle) };
    }

    #[test]
    fn is_guest_and_state_reflect_the_backing_user() {
        let user = Arc::new(UserState {
            local_id: XUserLocalId { value: 7 },
            user_id: 700,
            is_guest: true,
            state: Mutex::new(XUserStateValue::SigningOut),
            gamertag: "TestGamer".to_string(),
            gamertag_modern: String::new(),
            age_group: 3,
        });
        let handle = UserHandleTable::create(user);

        let mut is_guest = 0u8;
        let hr = unsafe { xuser_singleton().XUserGetIsGuest(handle, &mut is_guest) };
        assert_eq!(hr, S_OK);
        assert_eq!(is_guest, TRUE);

        let mut state = 0u32;
        let hr = unsafe { xuser_singleton().XUserGetState(handle, &mut state) };
        assert_eq!(hr, S_OK);
        assert_eq!(state, XUserStateValue::SigningOut as u32);

        unsafe { xuser_singleton().XUserCloseHandle(handle) };
    }

    #[test]
    fn change_event_registration_round_trips() {
        unsafe extern "system" fn cb(_context: *mut c_void, _local_id: XUserLocalId, _event: u32) {}

        let mut token = XTaskQueueRegistrationToken::default();
        let hr = unsafe {
            xuser_singleton().XUserRegisterForChangeEvent(
                0,
                std::ptr::null_mut(),
                Some(cb),
                &mut token,
            )
        };
        assert_eq!(hr, S_OK);
        assert_ne!(token.token, 0);

        assert_eq!(
            unsafe { xuser_singleton().XUserUnregisterForChangeEvent(token, FALSE) },
            TRUE
        );
        // Unregistering an already-removed token reports failure, not a duplicate success.
        assert_eq!(
            unsafe { xuser_singleton().XUserUnregisterForChangeEvent(token, FALSE) },
            FALSE
        );
    }

    #[test]
    fn a_zero_handle_is_rejected_not_silently_accepted() {
        // Matches task_queue::QueueHandle: a nonzero handle is trusted as one the game
        // actually got back from us, but zero ("no handle") must not be treated as valid.
        let mut id = 0u64;
        assert_eq!(
            unsafe { xuser_singleton().XUserGetId(0, &mut id) },
            E_INVALIDARG
        );
    }

    #[test]
    fn get_gamertag_and_age_group_reflect_the_backing_user() {
        let user = make_user(9, 900);
        let handle = UserHandleTable::create(user);

        let mut age_group = 0u32;
        let hr = unsafe { xuser_singleton().XUserGetAgeGroup(handle, &mut age_group) };
        assert_eq!(hr, S_OK);
        assert_eq!(age_group, 3);

        let gamertag_iface = xuser_singleton().cast::<IXUserGamertagImpl>().unwrap();

        let mut buf = [0i8; 32];
        let mut used = 0usize;
        let hr = unsafe {
            gamertag_iface.XUserGetGamertag(handle, 0, buf.len(), buf.as_mut_ptr(), &mut used)
        };
        assert_eq!(hr, S_OK);
        assert_eq!(used, "TestGamer".len() + 1);
        let gamertag = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }.to_string_lossy();
        assert_eq!(gamertag, "TestGamer");

        // A too-small buffer reports the needed size and errors rather than truncating.
        let mut tiny = [0i8; 2];
        let hr = unsafe {
            gamertag_iface.XUserGetGamertag(
                handle,
                0,
                tiny.len(),
                tiny.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(hr, E_NOT_SUFFICIENT_BUFFER);

        unsafe { xuser_singleton().XUserCloseHandle(handle) };
    }

    #[test]
    fn sign_out_transitions_state_and_fires_signing_out_then_signed_out() {
        static EVENTS: Mutex<Vec<u32>> = Mutex::new(Vec::new());
        unsafe extern "system" fn cb(_context: *mut c_void, _local_id: XUserLocalId, event: u32) {
            EVENTS.lock().expect("events poisoned").push(event);
        }

        let mut token = XTaskQueueRegistrationToken::default();
        let hr = unsafe {
            xuser_singleton().XUserRegisterForChangeEvent(
                0,
                std::ptr::null_mut(),
                Some(cb),
                &mut token,
            )
        };
        assert_eq!(hr, S_OK);

        let user = make_user(11, 1100);
        let handle = UserHandleTable::create(user);

        let mut async_block = XAsyncBlock {
            queue: std::ptr::null_mut(),
            context: std::ptr::null_mut(),
            callback: None,
            internal: [0; std::mem::size_of::<*mut c_void>() * 4],
        };
        let hr = unsafe { xuser_singleton().XUserSignOutAsync(handle, &mut async_block) };
        assert_eq!(hr, S_OK);

        let mut state = 0u32;
        let hr = unsafe { xuser_singleton().XUserGetState(handle, &mut state) };
        assert_eq!(hr, S_OK);
        assert_eq!(state, XUserStateValue::SignedOut as u32);

        assert_eq!(
            *EVENTS.lock().expect("events poisoned"),
            vec![CHANGE_EVENT_SIGNING_OUT, CHANGE_EVENT_SIGNED_OUT]
        );

        assert_eq!(
            unsafe { xuser_singleton().XUserUnregisterForChangeEvent(token, FALSE) },
            TRUE
        );
        unsafe { xuser_singleton().XUserCloseHandle(handle) };
    }

    #[test]
    fn token_and_signature_utf16_result_round_trips_via_the_side_table() {
        let async_block = XAsyncBlock {
            queue: std::ptr::null_mut(),
            context: std::ptr::null_mut(),
            callback: None,
            internal: [0; std::mem::size_of::<*mut c_void>() * 4],
        };
        let key = &async_block as *const XAsyncBlock as usize;
        TOKEN_AND_SIGNATURE_UTF16_RESULTS
            .lock()
            .expect("token and signature utf16 results poisoned")
            .get_or_insert_with(HashMap::new)
            .insert(key, Ok(("tok3n".to_string(), "s1g".to_string())));

        let async_ptr = &async_block as *const XAsyncBlock as *mut XAsyncBlock;

        let mut needed = 0usize;
        let hr = unsafe {
            xuser_singleton().XUserGetTokenAndSignatureUtf16ResultSize(async_ptr, &mut needed)
        };
        assert_eq!(hr, S_OK);
        assert_eq!(
            needed,
            size_of::<XUserGetTokenAndSignatureUtf16Data>() + (6 + 4) * size_of::<u16>()
        );

        let mut buf = vec![0u8; needed];
        let mut data_ptr: *mut XUserGetTokenAndSignatureUtf16Data = std::ptr::null_mut();
        let mut used = 0usize;
        let hr = unsafe {
            xuser_singleton().XUserGetTokenAndSignatureUtf16Result(
                async_ptr,
                buf.len(),
                buf.as_mut_ptr().cast(),
                &mut data_ptr,
                &mut used,
            )
        };
        assert_eq!(hr, S_OK);
        assert_eq!(used, needed);
        assert!(!data_ptr.is_null());

        unsafe {
            let data = &*data_ptr;
            assert_eq!(data.token_count, 6);
            assert_eq!(data.signature_count, 4);
            let token = String::from_utf16_lossy(std::slice::from_raw_parts(
                data.token,
                data.token_count - 1,
            ));
            let signature = String::from_utf16_lossy(std::slice::from_raw_parts(
                data.signature,
                data.signature_count - 1,
            ));
            assert_eq!(token, "tok3n");
            assert_eq!(signature, "s1g");
        }

        // A too-small buffer reports the error rather than writing a truncated struct.
        let mut tiny = vec![0u8; needed - 1];
        let hr = unsafe {
            xuser_singleton().XUserGetTokenAndSignatureUtf16Result(
                async_ptr,
                tiny.len(),
                tiny.as_mut_ptr().cast(),
                &mut data_ptr,
                std::ptr::null_mut(),
            )
        };
        assert_eq!(hr, E_NOT_SUFFICIENT_BUFFER);

        TOKEN_AND_SIGNATURE_UTF16_RESULTS
            .lock()
            .expect("token and signature utf16 results poisoned")
            .get_or_insert_with(HashMap::new)
            .remove(&key);
    }

    #[test]
    fn max_devices_lookup_reports_not_implemented_without_a_binding() {
        let device_id = AppLocalDeviceId { value: [0u8; 32] };
        let mut handle = 0u64;
        let hr = unsafe { xuser_device_singleton().XUserFindForDevice(&device_id, &mut handle) };
        assert_eq!(hr, E_NOTIMPL);
        assert_eq!(handle, 0);
    }
}
