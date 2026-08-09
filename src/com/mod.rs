//! COM surface shared infrastructure: the `query_api_impl` dispatch, the per-class
//! singleton registry, the `GlobalInterface`/`query` helpers, and ABI types shared across
//! classes. Each GDK class lives in its own submodule (see the `mod` declarations below).
//! The shared ABI type aliases keep Windows/GDK names (`SIZE_T`, `UINT32`, `BOOLEAN`, ...)
//! verbatim rather than renaming them out of step with the ABI they describe.
#![allow(non_camel_case_types)]
#![allow(clippy::upper_case_acronyms)]

pub(crate) mod handle_table;
pub(crate) mod pickers;

pub mod xaccessibility;
pub mod xappcapture;
pub mod xappcapturemetadata;
pub mod xasync;
pub mod xdisplay;
pub mod xerror;
pub mod xfeature;
pub mod xgame;
pub mod xgameactivation;
pub mod xgameevent;
pub mod xgameinvite;
pub mod xgameprotocol;
pub mod xgamesave;
pub mod xgamestreaming;
pub mod xgameui;
pub mod xlauncher;
pub mod xnetworking;
pub mod xpackage;
pub mod xpersistent_local_storage;
pub mod xstore;
pub mod xsystem;
pub mod xsystemanalytics;
pub mod xuser;

use crate::diag::{diag, stub};
pub(crate) use xaccessibility::*;
pub(crate) use xappcapture::*;
pub(crate) use xappcapturemetadata::*;
pub(crate) use xdisplay::*;
pub use xerror::*;
pub use xfeature::*;
pub use xgame::*;
pub(crate) use xgameactivation::*;
pub(crate) use xgameevent::*;
pub use xgameinvite::*;
pub use xgameprotocol::*;
pub(crate) use xgamestreaming::*;
pub(crate) use xgameui::*;
pub(crate) use xlauncher::*;
pub use xnetworking::*;
pub use xpackage::*;
pub use xpersistent_local_storage::*;
pub use xstore::*;
pub use xsystem::*;
pub use xsystemanalytics::*;

use super::E_NOTIMPL;
use crate::results::*;
use std::env::temp_dir;
use std::ffi::c_void;
use windows_core::{GUID, HRESULT, Interface};

/// GDK handle/primitive types shared by the stub classes, taken verbatim from the real GDK
/// headers (see the `*.idl` files) rather than renamed out of step with the ABI they describe.
pub(crate) type XUserHandle = u64;
pub(crate) type XTaskQueueHandle = u64;
pub(crate) type XTaskQueueRegistrationToken = u64;
pub(crate) type XAppCaptureScreenshotStreamHandle = u64;
pub(crate) type XAppCaptureLocalStreamHandle = u64;
pub(crate) type XGameStreamingClientId = u64;
pub(crate) type XGameUiTextEntryHandle = u64;
pub(crate) type XGameUiCallbackHandle = u64;
pub(crate) type XDisplayTimeoutDeferralHandle = u64;
pub(crate) type XSpeechSynthesizerHandle = u64;
pub(crate) type XSpeechSynthesizerStreamHandle = u64;
pub(crate) type SIZE_T = usize;
pub(crate) type UINT32 = u32;
pub(crate) type UINT64 = u64;
pub(crate) type INT32 = i32;
/// Every GDK entry point that takes or returns a boolean declares it as C++ `bool`, which is
/// one byte - never Win32 `BOOL`, which is four. Use this alias for all of them, and never
/// `windows_sys::core::BOOL`, which is `i32`.
///
/// The width is not cosmetic in either direction. As a *parameter*, the caller only sets the
/// low byte of the register and leaves whatever was above it alone, so reading four bytes
/// turns a `false` the title passed into a garbage non-zero number - see
/// [`crate::com::xasync::IXAsyncProvider::XTaskQueueTerminate`], where that misread parked
/// Minecraft's only task-queue pump thread and silently killed all of its networking. As an
/// *out-pointer*, the caller sized the destination for one byte, so writing four overruns it
/// by three.
pub(crate) type BOOLEAN = u8;
pub(crate) type FLOAT = f32;
pub(crate) type DOUBLE = f64;
pub(crate) const FALSE: BOOLEAN = 0;

pub type XPackageMountHandle = u64;

struct XPackageMountHandleTable;

static XPACKAGE_MOUNT_HANDLES: handle_table::HandleTable<String> = handle_table::HandleTable::new();

impl XPackageMountHandleTable {
    fn create(path: String) -> u64 {
        XPACKAGE_MOUNT_HANDLES.create(path)
    }

    fn get(handle: u64) -> Option<String> {
        XPACKAGE_MOUNT_HANDLES.get(handle)
    }

    fn close(handle: u64) {
        XPACKAGE_MOUNT_HANDLES.close(handle);
    }
}

macro_rules! hresult_stub {
    ($(unsafe fn $name:ident (&self $(, $arg:ident : $ty:ty)*) -> HRESULT;)*) => {
        $(unsafe fn $name(&self $(, $arg: $ty)*) -> HRESULT {
            $(let _ = $arg;)*
            $crate::diag::stub!("{} -> E_NOTIMPL", stringify!($name));
            E_NOTIMPL
        })*
    };
}

macro_rules! hresult_stub_panic {
    ($(unsafe fn $name:ident (&self $(, $arg:ident : $ty:ty)*) -> HRESULT;)*) => {
        $(unsafe fn $name(&self $(, $arg: $ty)*) -> HRESULT { $(let _ = $arg;)* unimplemented!() })*
    };
}

macro_rules! bool_stub {
    ($(unsafe fn $name:ident (&self $(, $arg:ident : $ty:ty)*) -> BOOLEAN;)*) => {
        $(unsafe fn $name(&self $(, $arg: $ty)*) -> BOOLEAN {
            $(let _ = $arg;)*
            $crate::diag::stub!("{} -> false", stringify!($name));
            false.into()
        })*
    };
}

macro_rules! void_stub {
    ($(unsafe fn $name:ident (&self $(, $arg:ident : $ty:ty)*) -> ();)*) => {
        $(unsafe fn $name(&self $(, $arg: $ty)*) -> () {
            $(let _ = $arg;)*
            $crate::diag::stub!("{}", stringify!($name));
        })*
    };
}

pub(crate) use bool_stub;
pub(crate) use hresult_stub;
pub(crate) use hresult_stub_panic;
pub(crate) use void_stub;

/// A COM interface pointer held in a `static`.
///
/// `windows-core` interfaces are neither `Send` nor `Sync`, because in general a COM object
/// may be apartment-bound. Every object this crate hands out is free-threaded - they are
/// plain Rust types behind a vtable, with their own interior synchronization - so a single
/// process-wide instance shared across threads is exactly the GDK contract, and the unsafe
/// impls below assert that.
pub(crate) struct GlobalInterface<T>(T);

// SAFETY: every object this crate hands out is free-threaded (plain Rust behind a vtable,
// with its own interior synchronization), per the doc comment above.
unsafe impl<T> Send for GlobalInterface<T> {}
// SAFETY: same reasoning as the `Send` impl above.
unsafe impl<T> Sync for GlobalInterface<T> {}

/// Defines the process-wide instance of a COM object, built on first request.
///
/// Every `QueryApiImpl` class is a singleton: a title that asks for XStore twice must get
/// the same object, since the state behind it (task queues, user handles, mount tables) is
/// the runtime's, not the caller's.
///
/// ```ignore
/// singleton! {
///     /// Doc comment, if any.
///     pub(crate) fn xdisplay_singleton() -> IXDisplayImpl = XDisplay;
/// }
/// ```
macro_rules! singleton {
    ($($(#[$meta:meta])* $vis:vis fn $name:ident() -> $iface:ty = $object:expr;)*) => {
        $(
            $(#[$meta])*
            $vis fn $name() -> &'static $iface {
                static INSTANCE: std::sync::OnceLock<$crate::com::GlobalInterface<$iface>> =
                    std::sync::OnceLock::new();
                &INSTANCE
                    .get_or_init(|| $crate::com::GlobalInterface(($object).into()))
                    .0
            }
        )*
    };
}

pub(crate) use singleton;

singleton! {
    fn xsystem_singleton() -> IXSystem = XSystem;
}

singleton! {
    fn xgame_singleton() -> IXGameImpl3 = XGame;
}

singleton! {
    fn xgame_invite_singleton() -> IXGameInviteImpl2 = XGameInvite;
}

singleton! {
    fn xgame_protocol_singleton() -> IXGameProtocolImpl = XGameProtocol;
}

singleton! {
    fn xerror_singleton() -> IXErrorImpl = XError;
}

singleton! {
    fn xsystem_analytics_singleton() -> IXSystemAnalyticsImpl = XSystemAnalytics;
}

singleton! {
    /// The async runtime is a process-wide singleton: task queues and in-flight calls have
    /// to be shared between every API that hands out an `XAsyncBlock`.
    fn xasync_singleton() -> crate::com::xasync::IXAsync = crate::com::xasync::r#impl::XAsyncObject;
}

singleton! {
    fn xfeature_singleton() -> IXFeature = XFeature;
}

singleton! {
    fn xstore_singleton() -> IXStore = XStoreObject;
}

singleton! {
    fn xnetworking_singleton() -> IXNetworking = XNetworkingObject;
}

singleton! {
    fn xpersistent_local_storage_singleton() -> IXPersistentLocalStorage =
        XPersistentLocalStorage { tmp_path: temp_dir().to_string_lossy().into_owned() };
}

singleton! {
    /// Only backs `XPackageGetMountPathSize`/`XPackageGetMountPath`/`XPackageCloseMountHandle` for
    /// real - see [`IXPackageImpl`]'s docs.
    fn xpackage_singleton() -> IXPackageImpl3 = XPackageObject;
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
    // SAFETY: `interface_id.is_null()` was just checked above.
    let interface_id = unsafe { *interface_id };
    // SAFETY: `out.is_null()` was just checked above, satisfying `Interface::query`'s
    // contract that `interface` be a non-null, valid pointer for writing.
    if unsafe { object.query(&interface_id, out) }.is_ok() {
        S_OK
    } else {
        diag!("query: no such interface {:#034x}", interface_id.to_u128());
        // SAFETY: `out.is_null()` was checked above.
        unsafe {
            *out = std::ptr::null_mut();
        }
        E_NOINTERFACE
    }
}

pub fn query_api_impl(
    runtime_class_id: *const GUID,
    interface_id: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    if runtime_class_id.is_null() || interface_id.is_null() || out.is_null() {
        return E_POINTER;
    }

    // SAFETY: `runtime_class_id.is_null()` was just checked above.
    let class_id = unsafe { *runtime_class_id };
    diag!(
        "query_api_impl: class {:#034x} interface {:#034x}",
        class_id.to_u128(),
        // SAFETY: `interface_id.is_null()` was just checked above.
        unsafe { *interface_id }.to_u128()
    );
    match class_id {
        IXFeature::IID => query(xfeature_singleton(), interface_id, out),
        CLSID_XSTORE => query(xstore_singleton(), interface_id, out),
        CLSID_XNETWORKING => query(xnetworking_singleton(), interface_id, out),
        CLSID_XPERSISTENT_LOCAL_STORAGE => {
            query(xpersistent_local_storage_singleton(), interface_id, out)
        }
        CLSID_XPACKAGE => query(xpackage_singleton(), interface_id, out),
        crate::com::xasync::CLSID_XASYNC => query(xasync_singleton(), interface_id, out),
        crate::com::xuser::CLSID_XUSER => {
            query(crate::com::xuser::xuser_singleton(), interface_id, out)
        }
        crate::com::xuser::CLSID_XUSER_DEVICE => query(
            crate::com::xuser::xuser_device_singleton(),
            interface_id,
            out,
        ),
        crate::com::xgamesave::CLSID_XGAMESAVE => query(
            crate::com::xgamesave::xgamesave_singleton(),
            interface_id,
            out,
        ),
        CLSID_XSYSTEM => query(xsystem_singleton(), interface_id, out),
        CLSID_XGAME => query(xgame_singleton(), interface_id, out),
        CLSID_XGAME_INVITE => query(xgame_invite_singleton(), interface_id, out),
        CLSID_XGAME_PROTOCOL => query(xgame_protocol_singleton(), interface_id, out),
        CLSID_XERROR => query(xerror_singleton(), interface_id, out),
        CLSID_XSYSTEM_ANALYTICS => query(xsystem_analytics_singleton(), interface_id, out),
        CLSID_XACCESSIBILITY => query(xaccessibility_singleton(), interface_id, out),
        CLSID_XAPPCAPTURE => query(xappcapture_singleton(), interface_id, out),
        CLSID_XAPPCAPTURE_METADATA => query(xappcapturemetadata_singleton(), interface_id, out),
        CLSID_XDISPLAY => query(xdisplay_singleton(), interface_id, out),
        CLSID_XLAUNCHER => query(xlauncher_singleton(), interface_id, out),
        CLSID_XGAME_ACTIVATION => query(xgameactivation_singleton(), interface_id, out),
        CLSID_XGAME_EVENT => query(xgameevent_singleton(), interface_id, out),
        CLSID_XGAME_STREAMING => query(xgamestreaming_singleton(), interface_id, out),
        CLSID_XGAME_UI => query(xgameui_singleton(), interface_id, out),
        _ => {
            // Everything this crate does not implement yet. There is no Microsoft DLL to
            // fall back to - that is the point - so say so rather than crashing the game.
            stub!(
                "query_api_impl: unimplemented class {:#034x}",
                class_id.to_u128()
            );
            // SAFETY: `out.is_null()` was checked above.
            unsafe {
                *out = std::ptr::null_mut();
            }
            E_NOTIMPL
        }
    }
}

#[cfg(test)]
// Test code exercises this crate's own already-documented internal APIs against
// synthetic, controlled inputs, not untrusted FFI callers - a per-site SAFETY comment
// here would just restate the production contract already documented at each fn.
#[allow(clippy::undocumented_unsafe_blocks)]
mod tests {
    use std::ffi::{c_char, c_void};

    use crate::com::xasync::{XAsyncBlock, get_status};
    use crate::com::xstore::IXStore;
    use crate::com::{XStoreGameLicense, query_api_impl};
    use crate::{InitializeApiImplEx2, UninitializeApiImpl};
    use windows_core::{GUID, HRESULT, Interface};

    fn read_c_string(bytes: &[c_char]) -> String {
        let len = bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(bytes.len());
        let raw: Vec<u8> = bytes[..len].iter().map(|byte| *byte as u8).collect();
        String::from_utf8(raw).expect("license string should be valid utf-8")
    }
    #[test]
    fn test() {
        let mut out: *mut c_void = std::ptr::null_mut();
        let hr = query_api_impl(
            &crate::com::CLSID_XSTORE,
            &crate::com::IXStore::IID,
            &mut out,
        );

        assert_eq!(hr, HRESULT(0));

        let store: IXStore = unsafe { IXStore::from_raw(out) };

        unsafe {
            let mut store_ctx: u64 = 0;
            let hr = store.XStoreCreateContext(0, &mut store_ctx);
            assert_eq!(hr, HRESULT(0));
            let hr = store.XStoreQueryGameLicenseAsync(store_ctx, std::ptr::null_mut());
            assert_eq!(hr, HRESULT(0));
        };
    }

    /// The end-to-end shape a game sees: initialize the runtime, ask XStore for the
    /// game license through an XAsyncBlock, then block on the result. Nothing here
    /// loads a Microsoft DLL.
    #[test]
    fn query_game_license_async_blocks_via_xasync() {
        let init_hr = InitializeApiImplEx2(2604, 100000, 10, std::ptr::null_mut());
        assert_eq!(init_hr, HRESULT(0));

        let mut out = std::ptr::null_mut();
        let hr = query_api_impl(
            &crate::com::CLSID_XSTORE,
            &crate::com::IXStore::IID,
            &mut out,
        );
        assert_eq!(hr, HRESULT(0));

        let store: IXStore = unsafe { IXStore::from_raw(out) };
        let mut store_ctx: u64 = 0;
        let hr = unsafe { store.XStoreCreateContext(0, &mut store_ctx) };
        assert_eq!(hr, HRESULT(0));

        let mut async_block = XAsyncBlock {
            queue: std::ptr::null_mut(),
            context: std::ptr::null_mut(),
            callback: None,
            internal: [0; std::mem::size_of::<*mut c_void>() * 4],
        };
        let hr = unsafe {
            store.XStoreQueryGameLicenseAsync(
                store_ctx,
                (&mut async_block as *mut XAsyncBlock).cast(),
            )
        };
        assert_eq!(hr, HRESULT(0));

        let status_hr = unsafe { get_status(&mut async_block, true) };
        assert_eq!(status_hr, Ok(()));

        let mut license = XStoreGameLicense::default();
        let result_hr = unsafe {
            store.XStoreQueryGameLicenseResult(
                (&mut async_block as *mut XAsyncBlock).cast(),
                (&mut license as *mut XStoreGameLicense).cast(),
            )
        };
        assert_eq!(result_hr, HRESULT(0));
        assert!(license.isActive);
        assert!(!license.isTrialOwnedByThisUser);
        assert!(!license.isTrial);
        assert!(!license.isDiscLicense);
        assert_eq!(license.trialTimeRemainingInSeconds, 0);
        assert_eq!(
            read_c_string(&license.trialUniqueId),
            "",
            "a full license has no trial id"
        );

        let uninit_hr = UninitializeApiImpl();
        assert_eq!(uninit_hr, HRESULT(0));
    }

    /// The stub surface must be reachable through the same `QueryApiImpl` entry point a
    /// game uses: every class the real GDK's `QueryApiImpl` dispatches that this crate stubs
    /// (rather than implements) should resolve with its default interface IID to `S_OK` and a
    /// non-null object, exactly like the real classes - so a title probing for e.g.
    /// `CLSID_XDisplayImpl` gets a live vtable, not the unresolved-class `E_NOTIMPL` a
    /// `_`-fallthrough would give.
    #[test]
    fn stub_classes_resolve_via_query_api_impl() {
        // (class id, default interface IID, singleton pointer sanity) - mirror of the
        // `gdk_extra` dispatch table. `IXThreadingImpl` is intentionally absent: its
        // coclass uuid is `CLSID_XASYNC`, served by the real `XAsync` singleton.
        let cases: &[(GUID, GUID)] = &[
            (
                crate::com::CLSID_XACCESSIBILITY,
                crate::com::IXAccessibilityImpl2::IID,
            ),
            (
                crate::com::CLSID_XAPPCAPTURE,
                crate::com::IXAppCaptureImpl4::IID,
            ),
            (
                crate::com::CLSID_XAPPCAPTURE_METADATA,
                crate::com::IXAppCaptureMetadataImpl::IID,
            ),
            (crate::com::CLSID_XDISPLAY, crate::com::IXDisplayImpl::IID),
            (crate::com::CLSID_XLAUNCHER, crate::com::IXLauncherImpl::IID),
            (
                crate::com::CLSID_XGAME_ACTIVATION,
                crate::com::IXGameActivationImpl::IID,
            ),
            (
                crate::com::CLSID_XGAME_EVENT,
                crate::com::IXGameEventImpl::IID,
            ),
            (
                crate::com::CLSID_XGAME_STREAMING,
                crate::com::IXGameStreamingImpl3::IID,
            ),
            (crate::com::CLSID_XGAME_UI, crate::com::IXGameUiImpl4::IID),
        ];

        for (class_id, interface_id) in cases {
            let mut out: *mut c_void = std::ptr::null_mut();
            let hr = query_api_impl(class_id, interface_id, &mut out);
            assert_eq!(
                hr,
                HRESULT(0),
                "QueryApiImpl for {class_id:?} with default IID {interface_id:?} should resolve"
            );
            assert!(
                !out.is_null(),
                "stub class {class_id:?} returned a null object"
            );

            // An unrelated IID must be refused with E_NOINTERFACE, not crash.
            let mut other: *mut c_void = std::ptr::null_mut();
            let hr = query_api_impl(class_id, &windows_core::GUID::zeroed(), &mut other);
            assert_eq!(
                hr,
                crate::results::E_NOINTERFACE,
                "QueryApiImpl for {class_id:?} with a bogus IID should be E_NOINTERFACE"
            );
        }
    }

    /// End-to-end shape for `XStoreQueryEntitledProductsAsync` +
    /// `XStoreEnumerateProductsQuery`: no `xodus-service` reachable in this test, so the
    /// absence fallback (empty list, see `query_entitled_products`) is what the callback
    /// should observe - not a crash, not fabricated products.
    #[test]
    fn entitled_products_async_blocks_via_xasync_and_enumerates_via_callback() {
        let mut out = std::ptr::null_mut();
        let hr = query_api_impl(
            &crate::com::CLSID_XSTORE,
            &crate::com::IXStore::IID,
            &mut out,
        );
        assert_eq!(hr, HRESULT(0));

        let store: IXStore = unsafe { IXStore::from_raw(out) };
        let mut store_ctx: u64 = 0;
        let hr = unsafe { store.XStoreCreateContext(0, &mut store_ctx) };
        assert_eq!(hr, HRESULT(0));

        let mut async_block = XAsyncBlock {
            queue: std::ptr::null_mut(),
            context: std::ptr::null_mut(),
            callback: None,
            internal: [0; std::mem::size_of::<*mut c_void>() * 4],
        };
        let hr = unsafe {
            store.XStoreQueryEntitledProductsAsync(
                store_ctx,
                0,
                0,
                (&mut async_block as *mut XAsyncBlock).cast(),
            )
        };
        assert_eq!(hr, HRESULT(0));

        let status_hr = unsafe { get_status(&mut async_block, true) };
        assert_eq!(status_hr, Ok(()));

        let mut handle: u64 = 0;
        let result_hr = unsafe {
            store.XStoreQueryEntitledProductsResult(
                (&mut async_block as *mut XAsyncBlock).cast(),
                (&mut handle as *mut u64).cast(),
            )
        };
        assert_eq!(result_hr, HRESULT(0));

        assert_eq!(
            unsafe { store.XStoreProductsQueryHasMorePages(handle) },
            super::FALSE
        );

        unsafe extern "system" fn collect(
            product: *const crate::com::XStoreProduct,
            context: *mut c_void,
        ) -> u8 {
            let seen = unsafe { &mut *(context as *mut Vec<String>) };
            let title = unsafe { std::ffi::CStr::from_ptr((*product).title) };
            seen.push(title.to_string_lossy().into_owned());
            1
        }
        let mut seen: Vec<String> = Vec::new();
        let hr = unsafe {
            store.XStoreEnumerateProductsQuery(
                handle,
                (&mut seen as *mut Vec<String>).cast(),
                collect as *mut c_void,
            )
        };
        assert_eq!(hr, HRESULT(0));
        assert!(
            seen.is_empty(),
            "no xodus-service reachable in this test - empty is the expected answer"
        );

        unsafe { store.XStoreCloseProductsQueryHandle(handle) };
    }
}
