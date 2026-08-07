//! The XUser sign-in engine: `UserState`, the user handle table, the live-user registry,
//! and the change-event machinery that the COM objects in [`super::r#impl`] drive. This is
//! the non-COM state layer - no `#[implement]` here.

use super::*;
use crate::E_FAIL;
use crate::com::handle_table::HandleTable;

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

pub(crate) struct UserState {
    pub(crate) local_id: XUserLocalId,
    pub(crate) user_id: u64,
    pub(crate) is_guest: bool,
    pub(crate) state: Mutex<XUserStateValue>,
    /// Cached at sign-in (`XUserAddAsync`), same as real GDK - `XUserGetGamertag`/
    /// `XUserGetAgeGroup` are synchronous, so they can't do a network round trip per call.
    pub(crate) gamertag: String,
    /// Empty when Xbox Live's `mgt` claim wasn't present for this account; `XUserGetGamertag`
    /// falls back to `gamertag` in that case.
    pub(crate) gamertag_modern: String,
    /// `XUserAgeGroup` (`Unknown`=0, `Child`=1, `Teen`=2, `Adult`=3), mapped from Xbox Live's
    /// `agg` claim once at sign-in.
    pub(crate) age_group: u32,
}

/// Users known well enough to answer `XUserFindUserByLocalId`/`XUserFindUserById`.
/// Populated by `XUserAddAsync` once it produces a real user. `Weak` so a fully-closed user
/// (every handle dropped) falls out of the registry on its own rather than needing an
/// explicit sign-out path to clean it up.
pub(crate) static USER_REGISTRY: Mutex<Vec<Weak<UserState>>> = Mutex::new(Vec::new());

pub(crate) fn register_user(user: &Arc<UserState>) {
    let mut registry = USER_REGISTRY.lock().expect("user registry poisoned");
    registry.retain(|entry| entry.strong_count() > 0);
    registry.push(Arc::downgrade(user));
}

/// Xbox Live's `agg` claim (`"Adult"`/`"Teen"`/`"Child"`) mapped to `XUserAgeGroup`
/// (`wine/include/xuser.h`); anything else (including a missing claim) is `Unknown`=0.
pub(crate) fn parse_age_group(agg: &str) -> u32 {
    match agg {
        "Adult" => 3,
        "Teen" => 2,
        "Child" => 1,
        _ => 0,
    }
}

/// Shared by `XUserAddAsync`'s silent and interactive paths (and `XUserAddByIdWithUiAsync`):
/// finds or creates the registered [`UserState`] for a `(xuid, gamertag, gamertag_modern,
/// age_group)` tuple, firing the sign-in change event only for a genuinely new user, and
/// returns a fresh handle onto it.
pub(crate) fn user_handle_from_info(
    info: (String, String, String, String),
) -> Result<u64, HRESULT> {
    let (xuid, gamertag, gamertag_modern, age_group) = info;
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
}

/// Handles are `u64` on the wire, checked against [`HandleTable`] - see its doc comment
/// for why. `XUserDuplicateHandle`/`XUserCloseHandle` are the game's refcounting, distinct
/// from (and in addition to) the `Arc`'s own.
pub(crate) struct UserHandleTable;

static USER_HANDLES: HandleTable<Arc<UserState>> = HandleTable::new();

impl UserHandleTable {
    pub(crate) fn create(user: Arc<UserState>) -> u64 {
        USER_HANDLES.create(user)
    }

    pub(crate) fn get(handle: u64) -> Option<Arc<UserState>> {
        USER_HANDLES.get(handle)
    }

    pub(crate) fn close(handle: u64) {
        USER_HANDLES.close(handle);
    }
}

// ---------------------------------------------------------------------------------------
// Change-event registration
// ---------------------------------------------------------------------------------------

pub(crate) struct ChangeEventRegistration {
    context: *mut c_void,
    callback: XUserChangeEventCallback,
}

// SAFETY: the context and callback are only ever read back and invoked on whatever thread
// fires the event, exactly like `XAsyncWaker` elsewhere in this crate - the game is the one
// asserting these are safe to move by handing them to a registration API in the first
// place.
unsafe impl Send for ChangeEventRegistration {}
// SAFETY: same reasoning as the `Send` impl above - the context/callback pair is only ever
// read and invoked, never mutated concurrently in a way that would require exclusion.
unsafe impl Sync for ChangeEventRegistration {}

/// Accepted registrations for `XUserChangeEvent` notifications. `XUserAddAsync` fires
/// `SignedInAgain` (below) on a successful sign-in; nothing else produces a change yet
/// (there is no sign-out or gamertag-change path).
pub(crate) static CHANGE_EVENT_REGISTRY: Mutex<Option<HashMap<u64, ChangeEventRegistration>>> =
    Mutex::new(None);
pub(crate) static NEXT_CHANGE_EVENT_TOKEN: AtomicU64 = AtomicU64::new(1);

/// `XUserChangeEvent` variants (`wine/include/xuser.h`) this crate actually fires.
/// `SignedInAgain` is fired for both a first sign-in and a repeat one. `SigningOut`/
/// `SignedOut` are fired back to back by `XUserSignOutAsync` - there is no real deferral
/// window to hold between them.
pub(crate) const CHANGE_EVENT_SIGNED_IN_AGAIN: u32 = 0;
pub(crate) const CHANGE_EVENT_SIGNING_OUT: u32 = 1;
pub(crate) const CHANGE_EVENT_SIGNED_OUT: u32 = 2;

pub(crate) fn fire_change_event(local_id: XUserLocalId, event: u32) {
    let registry = CHANGE_EVENT_REGISTRY
        .lock()
        .expect("change registry poisoned");
    if let Some(registry) = registry.as_ref() {
        for registration in registry.values() {
            // SAFETY: `callback`/`context` were handed to us as a pair by the GDK caller
            // via `XUserRegisterForChangeEvent`, which contracts them to remain valid and
            // callable for as long as the registration is held.
            unsafe { (registration.callback)(registration.context, local_id, event) };
        }
    }
}

pub(crate) fn register_change_event(
    context: *mut c_void,
    callback: XUserChangeEventCallback,
) -> u64 {
    let token = NEXT_CHANGE_EVENT_TOKEN.fetch_add(1, Ordering::Relaxed);
    let mut registry = CHANGE_EVENT_REGISTRY
        .lock()
        .expect("change registry poisoned");
    registry
        .get_or_insert_with(HashMap::new)
        .insert(token, ChangeEventRegistration { context, callback });
    token
}

pub(crate) fn unregister_change_event(token: u64) -> bool {
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
pub(crate) unsafe fn read_utf16_cstr(ptr: *const u16) -> String {
    let mut len = 0usize;
    // SAFETY: this fn's own doc comment is the precondition callers must uphold - `ptr` is
    // non-null and points at a valid nul-terminated `u16` sequence.
    unsafe {
        while *ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
    }
}
