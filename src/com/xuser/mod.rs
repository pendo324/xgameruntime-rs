//! Native `IXUserImpl1-6` / `IXUserGamertagImpl` / `IXUserDeviceImpl1-2`, following the
//! vtable shape in `wine/include/xuser.idl` (the authoritative slot order, including the
//! `__PADDING__` at slot 12 of the base interface).
//!
//! This lands the handle table and every slot answerable with no
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
//!
//! Layout: [`core`] holds the sign-in engine (handle tables, user registry, change events);
//! [`r#impl`] holds the `IXUser*`/`IXUserDevice*` COM objects and their vtables; this file
//! re-exports the ABI types, interfaces, and singletons.

pub mod core;
pub mod r#impl;

pub use r#impl::*;

use std::ffi::{c_char, c_void};

use windows_core::{GUID, HRESULT};

/// Also `IXUserImpl`'s own IID - `xuser.idl` reuses it as the coclass id.
pub const CLSID_XUSER: GUID = GUID::from_u128(0x01acd177_91f9_4763_a38e_ccbb55ce32e0);
/// Also `IXUserDeviceImpl`'s own IID, for the same reason.
pub const CLSID_XUSER_DEVICE: GUID = GUID::from_u128(0x7d824997_10dc_45ab_86b7_2737767c0bf1);

/// Win32 `BOOLEAN` - one byte. Distinct from the four-byte `BOOL` used elsewhere in this
/// crate; `xuser.idl` uses `BOOLEAN` throughout, so getting the width wrong would corrupt
/// whatever field follows it across the FFI boundary.
type Boolean = u8;

pub(crate) const TRUE: Boolean = 1;
pub(crate) const FALSE: Boolean = 0;

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

#[cfg(test)]
// Test code exercises this crate's own already-documented internal APIs against
// synthetic, controlled inputs, not untrusted FFI callers - a per-site SAFETY comment
// here would just restate the production contract already documented at each fn.
#[allow(clippy::undocumented_unsafe_blocks)]
mod tests {
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::sync::{Arc, Mutex};

    use super::core::{
        CHANGE_EVENT_SIGNED_OUT, CHANGE_EVENT_SIGNING_OUT, UserHandleTable, UserState,
        register_user,
    };
    use super::r#impl::{
        GAMER_PICTURE_RESULTS, TOKEN_AND_SIGNATURE_UTF16_RESULTS, xuser_singleton,
    };
    use super::*;
    use crate::com::xasync::XAsyncBlock;
    use crate::results::*;
    use windows_core::Interface;

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
    fn is_store_user_is_true_for_a_valid_handle_and_false_otherwise() {
        let user = make_user(2, 200);
        let handle = UserHandleTable::create(user);

        assert_eq!(unsafe { xuser_singleton().XUserIsStoreUser(handle) }, TRUE);
        assert_eq!(unsafe { xuser_singleton().XUserIsStoreUser(0) }, FALSE);

        unsafe { xuser_singleton().XUserCloseHandle(handle) };
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
        // The sign-out body runs on a worker now, so the state transition and its change
        // events have not necessarily landed by the time the call returns.
        assert_eq!(
            unsafe { crate::com::xasync::get_status(&mut async_block, true) },
            Ok(())
        );

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
    fn gamer_picture_result_round_trips_via_the_side_table() {
        let async_block = XAsyncBlock {
            queue: std::ptr::null_mut(),
            context: std::ptr::null_mut(),
            callback: None,
            internal: [0; std::mem::size_of::<*mut c_void>() * 4],
        };
        let key = &async_block as *const XAsyncBlock as usize;
        GAMER_PICTURE_RESULTS
            .lock()
            .expect("gamer picture results poisoned")
            .get_or_insert_with(HashMap::new)
            .insert(key, Ok(b"not-really-a-png".to_vec()));

        let async_ptr = &async_block as *const XAsyncBlock as *mut XAsyncBlock;

        let mut needed = 0usize;
        let hr =
            unsafe { xuser_singleton().XUserGetGamerPictureResultSize(async_ptr, &mut needed) };
        assert_eq!(hr, S_OK);
        assert_eq!(needed, b"not-really-a-png".len());

        let mut buf = vec![0u8; needed];
        let mut used = 0usize;
        let hr = unsafe {
            xuser_singleton().XUserGetGamerPictureResult(
                async_ptr,
                buf.len(),
                buf.as_mut_ptr().cast(),
                &mut used,
            )
        };
        assert_eq!(hr, S_OK);
        assert_eq!(used, needed);
        assert_eq!(buf, b"not-really-a-png");

        // A too-small buffer reports the error rather than writing a truncated picture.
        let mut tiny = vec![0u8; needed - 1];
        let hr = unsafe {
            xuser_singleton().XUserGetGamerPictureResult(
                async_ptr,
                tiny.len(),
                tiny.as_mut_ptr().cast(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(hr, E_NOT_SUFFICIENT_BUFFER);

        GAMER_PICTURE_RESULTS
            .lock()
            .expect("gamer picture results poisoned")
            .get_or_insert_with(HashMap::new)
            .remove(&key);
    }

    #[test]
    fn gamer_picture_result_reports_an_empty_buffer_when_no_picture_is_set() {
        let async_block = XAsyncBlock {
            queue: std::ptr::null_mut(),
            context: std::ptr::null_mut(),
            callback: None,
            internal: [0; std::mem::size_of::<*mut c_void>() * 4],
        };
        let key = &async_block as *const XAsyncBlock as usize;
        GAMER_PICTURE_RESULTS
            .lock()
            .expect("gamer picture results poisoned")
            .get_or_insert_with(HashMap::new)
            .insert(key, Ok(Vec::new()));

        let async_ptr = &async_block as *const XAsyncBlock as *mut XAsyncBlock;

        let mut needed = 1usize;
        let hr =
            unsafe { xuser_singleton().XUserGetGamerPictureResultSize(async_ptr, &mut needed) };
        assert_eq!(hr, S_OK);
        assert_eq!(needed, 0);

        let mut used = 1usize;
        let hr = unsafe {
            xuser_singleton().XUserGetGamerPictureResult(
                async_ptr,
                0,
                std::ptr::null_mut(),
                &mut used,
            )
        };
        assert_eq!(hr, S_OK);
        assert_eq!(used, 0);

        GAMER_PICTURE_RESULTS
            .lock()
            .expect("gamer picture results poisoned")
            .get_or_insert_with(HashMap::new)
            .remove(&key);
    }

    #[test]
    fn find_for_device_returns_a_signed_in_user_regardless_of_device_id() {
        // `USER_REGISTRY` is process-global, so (unlike most tests here) this can't
        // assert against a specific id if the suite runs multi-threaded and another
        // test's signed-in user happens to be alive at the same moment - only that
        // *some* signed-in user is found, which is the whole of what single-user mode
        // promises (there is no second user to disambiguate against).
        let user = make_user(43, 4343);
        register_user(&user);
        let keep_alive = UserHandleTable::create(user);

        let device_id = AppLocalDeviceId { value: [0xab; 32] };
        let mut handle = 0u64;
        let hr = unsafe { xuser_device_singleton().XUserFindForDevice(&device_id, &mut handle) };
        assert_eq!(hr, S_OK);
        assert_ne!(handle, 0);

        unsafe { xuser_singleton().XUserCloseHandle(handle) };
        unsafe { xuser_singleton().XUserCloseHandle(keep_alive) };
    }
}
