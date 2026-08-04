use super::E_NOTIMPL;
use std::collections::HashMap;
use std::env::temp_dir;
use std::ffi::{CStr, CString, c_char, c_void};
use std::mem::size_of;
use std::ptr::null_mut;
use std::sync::{Mutex, OnceLock};
use windows_core::{GUID, HRESULT, IUnknown, Interface, implement, interface};
use windows_sys::core::BOOL;

const CLSID_XSTORE: GUID = GUID::from_u128(0x0dd112ac_7c24_448c_b92b_3960fb5bd30c);
const CLSID_XNETWORKING: GUID = GUID::from_u128(0x37e56907_2f10_41e8_b72f_36edb185331a);
const CLSID_XPACKAGE: GUID = GUID::from_u128(0xaf406016_e850_4aa8_a88d_2f3dcb9dac7e);
const CLSID_XPERSISTENT_LOCAL_STORAGE: GUID =
    GUID::from_u128(0xf4faf4d4_2d04_4fce_b3e0_474a713a3e84);
const STORE_SKU_ID_SIZE: usize = 18;
const TRIAL_UNIQUE_ID_MAX_SIZE: usize = 64;

#[allow(dead_code)] // GDK handle type kept for the XStore API surface.
type XStoreContextHandle = u64;

use crate::xasync::{XAsyncBlock, get_result};
use crate::{E_FAIL, results::*, xasync};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct XStoreGameLicense {
    pub skuStoreId: [c_char; STORE_SKU_ID_SIZE],
    pub isActive: bool,
    pub isTrialOwnedByThisUser: bool,
    pub isDiscLicense: bool,
    pub isTrial: bool,
    pub trialTimeRemainingInSeconds: u32,
    pub trialUniqueId: [c_char; TRIAL_UNIQUE_ID_MAX_SIZE],
    pub expirationDate: i64,
}

impl Default for XStoreGameLicense {
    fn default() -> Self {
        Self {
            skuStoreId: [0; STORE_SKU_ID_SIZE],
            isActive: false,
            isTrialOwnedByThisUser: false,
            isDiscLicense: false,
            isTrial: false,
            trialTimeRemainingInSeconds: 0,
            trialUniqueId: [0; TRIAL_UNIQUE_ID_MAX_SIZE],
            expirationDate: 0,
        }
    }
}

/// A permanent, non-trial license for the running title.
///
/// Fallback only, for when [`query_game_license`] has no answer to give: not running under
/// `xodus-cli run` (no `ContentId` published) or `xodus-service` unreachable. In both cases
/// we cannot tell "not entitled" apart from "cannot check right now", and a zeroed struct
/// reads as the former, making a game refuse to start for a reason that has nothing to do
/// with the account - so an active license is the safer default here.
fn build_full_game_license() -> XStoreGameLicense {
    XStoreGameLicense {
        isActive: true,
        isTrialOwnedByThisUser: false,
        isDiscLicense: false,
        isTrial: false,
        trialTimeRemainingInSeconds: 0,
        // Trials expire; a full license does not.
        expirationDate: 0,
        ..XStoreGameLicense::default()
    }
}

/// `XStoreQueryGameLicenseAsync`'s real backing, via `xodus-service`'s `LicenseRequest`
/// handler (`ipc::get_game_license`) - a real fetch-and-decrypt against
/// `licensing.mp.microsoft.com`, gated on the same `ContentId` `xodus-cli run` used to
/// decide whether to launch the game at all. Falls back to [`build_full_game_license`] when
/// that isn't available (see its doc comment for why "active" is the honest default there,
/// not "not licensed").
///
/// `isTrial`/`isTrialOwnedByThisUser`/`trialTimeRemainingInSeconds`/`isDiscLicense` are not
/// derivable from the fields `SPLicense` currently decodes (its `LicenseInformation` block
/// is parsed but discarded - see `xodus::licensing::splicense`), so a real answer always
/// reports them as `false`/`0` rather than guessing.
fn query_game_license() -> XStoreGameLicense {
    match crate::ipc::get_game_license() {
        Ok((is_active, expiration_date)) => XStoreGameLicense {
            isActive: is_active,
            expirationDate: expiration_date,
            ..XStoreGameLicense::default()
        },
        Err(_) => build_full_game_license(),
    }
}

#[repr(C)]
struct XStoreQueryGameLicenseAsyncResultPayload {
    license: XStoreGameLicense,
}

// ---------------------------------------------------------------------------------------
// XStoreProduct / entitled-products query (`XStoreQueryEntitledProductsAsync`,
// `XStoreEnumerateProductsQuery`) - ABI per `wine/include/xstore.idl`, the authoritative
// struct layout for this project (WineGDK's own `XStore.c` is not a behavior oracle here,
// see PLAN.md).
// ---------------------------------------------------------------------------------------

const XSTORE_PRODUCT_KIND_NONE: u32 = 0x00;
const XSTORE_PRODUCT_KIND_GAME: u32 = 0x04;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct XStorePrice {
    basePrice: f32,
    price: f32,
    recurrencePrice: f32,
    currencyCode: *const c_char,
    formattedBasePrice: [c_char; 16],
    formattedPrice: [c_char; 16],
    formattedRecurrencePrice: [c_char; 16],
    isOnSale: bool,
    saleEndDate: i64,
}

/// Raw-pointer fields (`storeId`/`title`/...), unlike `XStoreGameLicense`'s fixed-size
/// arrays - the backing `CString`s live in [`ProductQueryEntry`] alongside each instance and
/// must outlive it.
#[repr(C)]
#[derive(Clone, Copy)]
struct XStoreProduct {
    storeId: *const c_char,
    title: *const c_char,
    description: *const c_char,
    language: *const c_char,
    inAppOfferToken: *const c_char,
    linkUri: *mut c_char,
    productKind: u32,
    price: XStorePrice,
    hasDigitalDownload: bool,
    isInUserCollection: bool,
    keywordsCount: u32,
    keywords: *mut *const c_char,
    skusCount: u32,
    skus: *mut c_void,
    imagesCount: u32,
    images: *mut c_void,
    videosCount: u32,
    videos: *mut c_void,
}

/// `wine/include/xstore.idl`'s `XStoreProductQueryCallback` - `BOOLEAN __stdcall(const
/// XStoreProduct*, void*)`. Returning `FALSE` (0) stops enumeration early, mirroring every
/// other GDK enumeration callback of this shape.
type XStoreProductQueryCallback =
    unsafe extern "system" fn(product: *const XStoreProduct, context: *mut c_void) -> u8;

/// One entitled product's owned strings plus the [`XStoreProduct`] whose raw pointers point
/// into them. Self-referential but sound: a `CString`'s data lives in its own heap
/// allocation, so moving this struct (e.g. growing the `Vec` in [`ProductQuery`]) relocates
/// the `CString` handle, not the buffer `XStoreProduct::store_id`/`title` point at.
struct ProductQueryEntry {
    _store_id: CString,
    _title: CString,
    product: XStoreProduct,
}

struct ProductQuery {
    entries: Vec<ProductQueryEntry>,
}

/// Handle table for `XStoreQueryEntitledProductsAsync`/`XStoreEnumerateProductsQuery`/
/// `XStoreCloseProductsQueryHandle`, same leaked-`Box` scheme as `xuser.rs`'s
/// `UserHandleTable`.
struct ProductQueryHandleTable;

impl ProductQueryHandleTable {
    fn create(query: ProductQuery) -> u64 {
        Box::into_raw(Box::new(query)) as u64
    }

    /// # Safety
    /// `handle` must be zero or a handle from [`Self::create`] that has not been closed.
    unsafe fn get<'a>(handle: u64) -> Option<&'a ProductQuery> {
        if handle == 0 {
            return None;
        }
        Some(unsafe { &*(handle as *const ProductQuery) })
    }

    /// # Safety
    /// `handle` must be an open handle from [`Self::create`]; it is invalid afterwards.
    unsafe fn close(handle: u64) {
        if handle == 0 {
            return;
        }
        drop(unsafe { Box::from_raw(handle as *mut ProductQuery) });
    }
}

/// Handle table for `XPersistentLocalStorageMountForPackage`/`XPackageGetMountPath`/
/// `XPackageCloseMountHandle` - same leaked-`Box` scheme as `ProductQueryHandleTable`, storing
/// the mounted directory's path.
struct XPackageMountHandleTable;

impl XPackageMountHandleTable {
    fn create(path: String) -> u64 {
        Box::into_raw(Box::new(path)) as u64
    }

    /// # Safety
    /// `handle` must be zero or a handle from [`Self::create`] that has not been closed.
    unsafe fn get<'a>(handle: u64) -> Option<&'a String> {
        if handle == 0 {
            return None;
        }
        Some(unsafe { &*(handle as *const String) })
    }

    /// # Safety
    /// `handle` must be an open handle from [`Self::create`]; it is invalid afterwards.
    unsafe fn close(handle: u64) {
        if handle == 0 {
            return;
        }
        drop(unsafe { Box::from_raw(handle as *mut String) });
    }
}

/// Maps `EntitledProduct`/`AssociatedProduct`'s freeform `product_kind` string onto
/// `XStoreProductKind`'s bitmask (`wine/include/xstore.idl`). Only "Game" is derivable from
/// what `xodus-service` actually returns - anything else honestly reports as none rather
/// than guessing a specific DLC/consumable/durable kind.
fn product_kind_flag(kind: &str) -> u32 {
    if kind.eq_ignore_ascii_case("game") {
        XSTORE_PRODUCT_KIND_GAME
    } else {
        XSTORE_PRODUCT_KIND_NONE
    }
}

/// `XStoreQueryEntitledProductsAsync`'s real backing, via `xodus-service`'s
/// `EntitledProductsRequest` handler (`ipc::get_entitled_products`) - the "My games"
/// library for whichever account is signed in. A failed/unreachable fetch reports an empty
/// list rather than a fabricated one: unlike `query_game_license`, there is no launch
/// decision riding on this answer, so the honest-absence default is the right one here.
///
/// `description`/`language`/`inAppOfferToken`/`linkUri`/`price`/keywords/skus/images/videos
/// are not derivable from the "My games" summary payload, so they are always empty/zeroed
/// rather than guessed at.
fn query_entitled_products() -> u64 {
    let products = crate::ipc::get_entitled_products("").unwrap_or_default();
    let entries = products
        .into_iter()
        .map(|product| {
            let store_id = CString::new(product.store_id).unwrap_or_default();
            let title = CString::new(product.title).unwrap_or_default();
            let product_kind = product_kind_flag(&product.product_kind);
            let entry = XStoreProduct {
                storeId: store_id.as_ptr(),
                title: title.as_ptr(),
                description: std::ptr::null(),
                language: std::ptr::null(),
                inAppOfferToken: std::ptr::null(),
                linkUri: std::ptr::null_mut(),
                productKind: product_kind,
                price: XStorePrice::default(),
                hasDigitalDownload: true,
                isInUserCollection: true,
                keywordsCount: 0,
                keywords: std::ptr::null_mut(),
                skusCount: 0,
                skus: std::ptr::null_mut(),
                imagesCount: 0,
                images: std::ptr::null_mut(),
                videosCount: 0,
                videos: std::ptr::null_mut(),
            };
            ProductQueryEntry {
                _store_id: store_id,
                _title: title,
                product: entry,
            }
        })
        .collect();
    ProductQueryHandleTable::create(ProductQuery { entries })
}

/// `XStoreQueryAssociatedProductsAsync`'s real backing, via `xodus-service`'s
/// `AssociatedProductsRequest` handler (`ipc::get_associated_products`) - products "sellable
/// by" (DLC/add-ons for) the running game's own catalog entry, resolved server-side from the
/// `PackageFamilyName` `xodus-cli run` computed at launch (see `ipc::get_associated_products`'s
/// docs). No PFN available (not running under `xodus-cli run`, or no `AppxManifest.xml`
/// found/parsed) or a failed catalog fetch both report an empty list - same honest-absence
/// stance as [`query_entitled_products`]. `productKinds` is not filtered against here for the
/// same reason `XStoreQueryEntitledProductsAsync` doesn't: `xodus-service`'s answer only
/// carries a freeform `product_kind` string, not the bitmask GDK titles pass in.
///
/// Unlike `query_entitled_products`, `isInUserCollection` is always `false`: associated
/// products are catalog entries this account may not own yet (that is the point of the
/// query), not confirmed entitlements.
fn query_associated_products(max_items_to_retrieve_per_page: u32) -> u64 {
    let products =
        crate::ipc::get_associated_products(max_items_to_retrieve_per_page).unwrap_or_default();
    let entries = products
        .into_iter()
        .map(|product| {
            let store_id = CString::new(product.store_id).unwrap_or_default();
            let title = CString::new(product.title).unwrap_or_default();
            let product_kind = product_kind_flag(&product.product_kind);
            let entry = XStoreProduct {
                storeId: store_id.as_ptr(),
                title: title.as_ptr(),
                description: std::ptr::null(),
                language: std::ptr::null(),
                inAppOfferToken: std::ptr::null(),
                linkUri: std::ptr::null_mut(),
                productKind: product_kind,
                price: XStorePrice::default(),
                hasDigitalDownload: true,
                isInUserCollection: false,
                keywordsCount: 0,
                keywords: std::ptr::null_mut(),
                skusCount: 0,
                skus: std::ptr::null_mut(),
                imagesCount: 0,
                images: std::ptr::null_mut(),
                videosCount: 0,
                videos: std::ptr::null_mut(),
            };
            ProductQueryEntry {
                _store_id: store_id,
                _title: title,
                product: entry,
            }
        })
        .collect();
    ProductQueryHandleTable::create(ProductQuery { entries })
}

#[interface("8836fe87-edb9-4fe3-8dad-05f0d2cd5b40")]
pub unsafe trait IXFeature: IUnknown {
    unsafe fn XGameRuntimeIsFeatureAvailable(&self, feature: u32) -> bool;
}

#[implement(IXFeature)]
pub struct XFeature;

impl IXFeature_Impl for XFeature_Impl {
    /// Every feature reports available.
    ///
    /// A title that is told a feature is missing takes a fallback path we have not
    /// implemented either, so claiming absence buys nothing. The honest "not implemented"
    /// lives at the individual API, which returns `E_NOTIMPL`.
    unsafe fn XGameRuntimeIsFeatureAvailable(&self, _feature: u32) -> bool {
        true
    }
}

#[repr(C)]
pub struct XPersistentLocalStorageSpaceInfo {
    availableFreeBytes: u64,
    totalFreeBytes: u64,
    usedBytes: u64,
    totalBytes: u64,
}

pub type XPackageMountHandle = u64;

#[interface("41a4e10c-5a7e-41d9-8c37-37bde62a07d6")]
pub unsafe trait IXPersistentLocalStorage: IUnknown {
    pub unsafe fn x_persistent_local_storage_get_path_size(self: &Self, path_size: *mut usize);
    pub unsafe fn x_persistent_local_storage_get_path(
        self: &Self,
        path_size: usize,
        path: *mut c_char,
        path_used: *mut usize,
    );
    pub unsafe fn x_persistent_local_storage_get_space_info(
        self: &Self,
        info: *mut XPersistentLocalStorageSpaceInfo,
    );
    pub unsafe fn x_persistent_local_storage_prompt_user_for_space_async(
        self: &Self,
        requested_bytes: u64,
        async_block: *mut XAsyncBlock,
    );
    pub unsafe fn x_persistent_local_storage_prompt_user_for_space_result(
        self: &Self,
        async_block: *mut XAsyncBlock,
    );
    /// Unlike this trait's other methods (kept `()`-returning as a pre-existing, out-of-scope
    /// quirk), this one returns `HRESULT` for real: the caller needs to be able to observe
    /// `E_INVALIDARG` for a `packageIdentifier` that isn't this title's own or a declared
    /// `RelatedProducts` entry, matching the real GDK signature.
    pub unsafe fn x_persistent_local_storage_mount_for_package(
        self: &Self,
        package_identifier: *const c_char,
        mount_handle: *mut XPackageMountHandle,
    ) -> HRESULT;
}

#[implement(IXPersistentLocalStorage)]
pub struct XPersistentLocalStorage {
    tmp_path: String,
}

impl IXPersistentLocalStorage_Impl for XPersistentLocalStorage_Impl {
    unsafe fn x_persistent_local_storage_get_path_size(&self, path_size: *mut usize) {
        unsafe {
            *path_size = self.tmp_path.len() + 1;
        }
    }

    unsafe fn x_persistent_local_storage_get_path(
        &self,
        path_size: usize,
        path: *mut c_char,
        path_used: *mut usize,
    ) {
        let bytes = self.tmp_path.as_bytes();
        let len = bytes.len().min(path_size.saturating_sub(1));
        for (index, byte) in bytes.iter().copied().take(len).enumerate() {
            unsafe {
                *path.add(index) = byte as c_char;
            }
        }
        if path_size != 0 {
            unsafe {
                *path.add(len) = 0;
            }
        }
        unsafe {
            *path_used = len + 1;
        }
    }

    /// Real numbers when `xodus-cli run` found a `<PersistentLocalStorage>` element in
    /// `MicrosoftGame.config` (`ipc::persistent_local_storage_space`); the old placeholder
    /// otherwise (not running under `xodus-cli run`, or the title didn't declare one) - an
    /// honest "can't tell" fallback, not a claim this title has no storage need.
    unsafe fn x_persistent_local_storage_get_space_info(
        &self,
        info: *mut XPersistentLocalStorageSpaceInfo,
    ) {
        let (total_bytes, growable_to_bytes) = crate::ipc::persistent_local_storage_space()
            .unwrap_or((1024 * 1024 * 1024, 2 * 1024 * 1024 * 1024));
        unsafe {
            *info = XPersistentLocalStorageSpaceInfo {
                availableFreeBytes: growable_to_bytes.saturating_sub(total_bytes / 2),
                totalFreeBytes: growable_to_bytes,
                usedBytes: total_bytes / 2,
                totalBytes: total_bytes,
            };
        }
    }

    /// No real UI surface exists to show a space-request prompt, so this completes
    /// immediately as an approval - the honest alternative (failing every call) would break
    /// titles that gate on this succeeding before writing to persistent local storage, for a
    /// prompt Xodus has no way to present anyway.
    unsafe fn x_persistent_local_storage_prompt_user_for_space_async(
        &self,
        _requested_bytes: u64,
        async_block: *mut XAsyncBlock,
    ) {
        let _ = unsafe { xasync::run_sync(async_block, || Ok(())) };
    }

    unsafe fn x_persistent_local_storage_prompt_user_for_space_result(
        &self,
        async_block: *mut XAsyncBlock,
    ) {
        let _ = unsafe { get_result::<()>(async_block, null_mut(), &mut ()) };
    }

    /// `packageIdentifier` is a `PackageFamilyName` (confirmed via strings recovered from the
    /// real `xgameruntime.dll` - `XPackageGetCurrentProcessPackageIdentifier` is implemented on
    /// top of Win32's own `GetCurrentPackageFamilyName`). Two cases are honestly answerable:
    /// self-mount (the running title's own PFN, `ENV_PACKAGE_FAMILY_NAME`) maps to this
    /// title's own persistent-storage root, and any other PFN is resolved to a `StoreId` via
    /// `xodus-service` and checked against this title's own declared `RelatedProducts`
    /// (`MicrosoftGame.config`, published through `ENV_RELATED_PRODUCTS`). Mounting storage
    /// for a product that's neither has no real meaning to grant, so that case honestly fails
    /// (`E_INVALIDARG`) rather than mounting anyway. On success, the handle resolves via
    /// `IXPackageImpl::XPackageGetMountPath` to a directory under this title's own
    /// persistent-storage root, created on first mount.
    unsafe fn x_persistent_local_storage_mount_for_package(
        &self,
        package_identifier: *const c_char,
        mount_handle: *mut XPackageMountHandle,
    ) -> HRESULT {
        if package_identifier.is_null() || mount_handle.is_null() {
            return E_POINTER;
        }
        let Ok(package_identifier) = unsafe { CStr::from_ptr(package_identifier) }.to_str() else {
            return E_INVALIDARG;
        };

        let is_self = std::env::var(crate::ipc::ENV_PACKAGE_FAMILY_NAME)
            .map(|pfn| pfn == package_identifier)
            .unwrap_or(false);

        let path = if is_self {
            std::path::PathBuf::from(&self.tmp_path)
        } else {
            let store_id = crate::ipc::resolve_product_id(package_identifier)
                .ok()
                .flatten();
            let Some(store_id) = store_id else {
                return E_INVALIDARG;
            };
            if !crate::ipc::is_related_product(&store_id) {
                return E_INVALIDARG;
            }
            std::path::Path::new(&self.tmp_path)
                .join("related")
                .join(&store_id)
        };

        if std::fs::create_dir_all(&path).is_err() {
            return E_FAIL;
        }

        let handle = XPackageMountHandleTable::create(path.to_string_lossy().into_owned());
        unsafe {
            *mount_handle = handle;
        }
        S_OK
    }
}

/// `IXSystemImpl`'s own IID, reused as the coclass id (same pattern as `CLSID_XUSER`).
/// Confirmed as the class GDK/XSAPI queries at startup - traced in Wine logs as one of the
/// unimplemented `query_api_impl` classes before this was added, and matches WineGDK's own
/// `dlls/xgameruntime/GDKComponent/System/XSystem.c` reference implementation, which this
/// mirrors byte-for-byte on the string constants (console id, "RETAIL" sandbox).
pub const CLSID_XSYSTEM: GUID = GUID::from_u128(0xe349bd1a_fc20_4e40_b99c_4178cc6b409f);

const X_SYSTEM_CONSOLE_ID_BYTES: i32 = 39;
const X_SYSTEM_SANDBOX_ID_MAX_BYTES: i32 = 16;

/// `XSystemHandle`, opaque GDK handle (`typedef void *XSystemHandle` in xsystem.idl).
pub type XSystemHandle = *mut c_void;

/// `XSystemHandleCallbackReason` / `XSystemHandleType` are `UINT32` enums; they arrive as
/// plain integer args to the callback and this crate never inspects them.
pub type XSystemHandleType = u32;
pub type XSystemHandleCallbackReason = u32;

/// `void __stdcall XSystemHandleCallback(XSystemHandle, XSystemHandleType,
/// XSystemHandleCallbackReason, void *context)` - see xsystem.idl.
pub type XSystemHandleCallback =
    Option<unsafe extern "system" fn(XSystemHandle, XSystemHandleType, XSystemHandleCallbackReason, *mut c_void)>;

// IXSystemImpl / 2 / 3 / 4 / 5. XSAPI (statically linked into titles that bundle it,
// e.g. Minecraft Bedrock) queries `CLSID_XSYSTEM` and asks for the *newer* interface IIDs
// (observed live: `IXSystemImpl4`, IID dadc2895-34b0-4ef5-a83e-45114d629b80), not just the
// base `IXSystemImpl`. Wine's own reference `xsystem.c` returns the same flat vtable for all
// of these, and windows-rs needs each IID as its own `#[interface]` (same pattern as
// `xuser.rs`'s IXUserImpl1-6), so the whole chain is declared here. The two empty tiers
// (`IXSystemImpl2`/`IXSystemImpl5`, no new methods in the IDL) exist purely so their IIDs QI
// successfully.

#[interface("e349bd1a-fc20-4e40-b99c-4178cc6b409f")]
pub unsafe trait IXSystem: IUnknown {
    unsafe fn XSystemGetConsoleId(
        &self,
        consoleIdSize: i32,
        consoleId: *mut c_char,
        consoleIdUsed: *mut usize,
    ) -> HRESULT;
    unsafe fn XSystemGetXboxLiveSandboxId(
        &self,
        sandboxIdSize: i32,
        sandboxId: *mut c_char,
        sandboxIdUsed: *mut usize,
    ) -> HRESULT;
    unsafe fn XSystemGetAppSpecificDeviceId(
        &self,
        appSpecificDeviceIdSize: i32,
        appSpecificDeviceId: *mut c_char,
        appSpecificDeviceIdUsed: *mut usize,
    ) -> HRESULT;
}

#[interface("6fd71f09-7513-49f0-89bc-bfaf5df6f852")]
pub unsafe trait IXSystem2: IXSystem {}

#[interface("67ce4bfc-b1d1-4ac7-bc3a-cb9219a97a85")]
pub unsafe trait IXSystem3: IXSystem2 {
    unsafe fn XSystemHandleTrack(
        &self,
        callback: XSystemHandleCallback,
        context: *mut c_void,
    ) -> HRESULT;
    unsafe fn XSystemIsHandleValid(&self, handle: XSystemHandle) -> u8;
}

#[interface("dadc2895-34b0-4ef5-a83e-45114d629b80")]
pub unsafe trait IXSystem4: IXSystem3 {
    unsafe fn XSystemAllowFullDownloadBandwidth(&self, enable: u8);
}

#[interface("1861cf2e-e18b-4834-a9f5-b4a4e6efb4cf")]
pub unsafe trait IXSystem5: IXSystem4 {}

/// `IXSystemImpl` - the GDK console-identity interface. Without this, `XblInitialize`
/// (statically linked into titles that bundle XSAPI, e.g. Minecraft Bedrock) queries
/// `CLSID_XSYSTEM` for the sandbox/console/device id it needs before constructing any Xbox
/// Live request, gets `E_NOTIMPL`, and silently bails - the title never attempts an XSTS
/// exchange for `http://xboxlive.com` at all, which otherwise looks identical to "networking
/// is broken" (zero relevant traffic, no error) rather than "this CLSID was never handled".
/// `RETAIL` sandbox and the always-zero console id mirror WineGDK's own `x_system.c`, which
/// documents both as fixed values on real Windows too - Xodus has no sandbox concept of its
/// own to source these from, and titles do not vary behavior on the console id's contents,
/// only its presence.
#[implement(IXSystem, IXSystem2, IXSystem3, IXSystem4, IXSystem5)]
pub struct XSystem;

impl IXSystem_Impl for XSystem_Impl {
    unsafe fn XSystemGetConsoleId(
        &self,
        console_id_size: i32,
        console_id: *mut c_char,
        console_id_used: *mut usize,
    ) -> HRESULT {
        const ID: &CStr = c"00000000.00000000.00000000.00000000.00";
        if console_id_used.is_null() {
            return E_POINTER;
        }
        unsafe {
            *console_id_used = ID.count_bytes() + 1;
        }
        if console_id.is_null() {
            return E_POINTER;
        }
        if console_id_size < X_SYSTEM_CONSOLE_ID_BYTES {
            return E_NOT_SUFFICIENT_BUFFER;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(
                ID.as_ptr(),
                console_id,
                ID.count_bytes() + 1,
            );
        }
        S_OK
    }

    unsafe fn XSystemGetXboxLiveSandboxId(
        &self,
        sandbox_id_size: i32,
        sandbox_id: *mut c_char,
        sandbox_id_used: *mut usize,
    ) -> HRESULT {
        const ID: &CStr = c"RETAIL";
        if sandbox_id.is_null() {
            return E_POINTER;
        }
        if sandbox_id_size < X_SYSTEM_SANDBOX_ID_MAX_BYTES {
            return E_NOT_SUFFICIENT_BUFFER;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(
                ID.as_ptr(),
                sandbox_id,
                ID.count_bytes() + 1,
            );
        }
        if !sandbox_id_used.is_null() {
            unsafe {
                *sandbox_id_used = ID.count_bytes() + 1;
            }
        }
        S_OK
    }

    /// A random GUID, generated once per process and cached for its lifetime - matching
    /// `x_system.c`'s `CoCreateGuid`-once-then-reuse behavior. Titles use this to key local
    /// analytics/telemetry batching, not identity, so per-process stability is what matters,
    /// not cross-launch persistence.
    unsafe fn XSystemGetAppSpecificDeviceId(
        &self,
        device_id_size: i32,
        device_id: *mut c_char,
        device_id_used: *mut usize,
    ) -> HRESULT {
        static DEVICE_ID: OnceLock<CString> = OnceLock::new();
        let id = DEVICE_ID.get_or_init(|| {
            use std::hash::{Hash, Hasher};
            // No `uuid` crate dependency and no `CoCreateGuid` equivalent available here -
            // hash together entropy sources unique to this process/run (pid, start time, and
            // a stack address, which ASLR randomizes) instead. This only needs to be
            // stable-for-the-process and look GUID-shaped, not cryptographically random.
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            std::process::id().hash(&mut hasher);
            std::time::SystemTime::now().hash(&mut hasher);
            let stack_marker = 0u8;
            (&stack_marker as *const u8 as usize).hash(&mut hasher);
            let high = hasher.finish();
            std::mem::size_of::<usize>().hash(&mut hasher);
            let low = hasher.finish();
            let text = format!(
                "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
                (high >> 32) as u32,
                (high >> 16) as u16,
                high as u16,
                (low >> 48) as u16,
                low & 0xFFFF_FFFF_FFFF,
            );
            CString::new(text).expect("hex-formatted guid string has no NUL bytes")
        });
        if !device_id_used.is_null() {
            unsafe {
                *device_id_used = id.count_bytes() + 1;
            }
        }
        if device_id.is_null() || device_id_size <= 0 {
            return S_OK;
        }
        let len = (id.count_bytes() + 1).min(device_id_size as usize);
        unsafe {
            std::ptr::copy_nonoverlapping(id.as_ptr(), device_id, len);
        }
        S_OK
    }
}

impl IXSystem2_Impl for XSystem_Impl {}

impl IXSystem3_Impl for XSystem_Impl {
    /// No real handle-lifecycle notifications exist to track (no suspend/resume, no
    /// screenshot/broadcast handles under Wine) - matches `x_system.c`'s own `E_NOTIMPL` stub.
    unsafe fn XSystemHandleTrack(
        &self,
        _callback: XSystemHandleCallback,
        _context: *mut c_void,
    ) -> HRESULT {
        E_NOTIMPL
    }

    /// Matches `x_system.c`: always valid, since Xodus never invalidates a handle it never
    /// tracked in the first place.
    unsafe fn XSystemIsHandleValid(&self, _handle: XSystemHandle) -> u8 {
        1
    }
}

impl IXSystem4_Impl for XSystem_Impl {
    /// No bandwidth throttling exists to toggle; acknowledging the request (rather than
    /// `x_system.c`'s `E_NOTIMPL`) avoids failing a call titles may not check the result of.
    unsafe fn XSystemAllowFullDownloadBandwidth(&self, _enable: u8) {}
}

impl IXSystem5_Impl for XSystem_Impl {}

/// `IXGameImpl`'s own IID, reused as the coclass id (same pattern as `CLSID_XSYSTEM`) - the
/// game/title identity interface. XSAPI (via `XblInitialize`, statically linked into GDK
/// titles like Minecraft Bedrock) reads the title id here to scope its Xbox Live requests;
/// features that check "is this a genuine signed-in Microsoft account" (distinct from a
/// PlayFab-only session, which needs no title identity) appear to depend on it. Confirmed via
/// Wine trace logs as one of the classes this title queries and previously got `E_NOTIMPL` for.
pub const CLSID_XGAME: GUID = GUID::from_u128(0x973a344e_24bf_4d0f_8457_56c534892b29);

#[interface("973a344e-24bf-4d0f-8457-56c534892b29")]
pub unsafe trait IXGameImpl: IUnknown {
    unsafe fn XGameGetXboxTitleId(&self, value: *mut u32) -> HRESULT;
}

#[interface("50849859-0ad8-4f81-80e4-5bc78626f852")]
pub unsafe trait IXGameImpl2: IXGameImpl {
    unsafe fn XLaunchNewGame(
        &self,
        exe_path: *const c_char,
        args: *const c_char,
        default_user: u64,
    ) -> ();
}

#[interface("2549f142-6419-4a06-97b5-931aab7c2f34")]
pub unsafe trait IXGameImpl3: IXGameImpl2 {
    unsafe fn XLaunchRestartOnCrash(&self, args: *const c_char, reserved: u32) -> HRESULT;
}

#[implement(IXGameImpl, IXGameImpl2, IXGameImpl3)]
pub struct XGame;

/// Parses the real `<TitleId>` out of the launched title's `MicrosoftGame.Config`, walking up
/// from the game executable the same way WineGDK's own `x_game.c` does (the file lives next to
/// the exe, occasionally a parent directory) - not hardcoded, per PLAN.md's standing rule
/// against baking in title identity.
fn read_game_title_id() -> Option<u32> {
    static TITLE_ID: OnceLock<Option<u32>> = OnceLock::new();
    *TITLE_ID.get_or_init(|| {
        let exe = std::env::current_exe().ok()?;
        let mut dir = exe.parent()?.to_path_buf();
        loop {
            for name in ["MicrosoftGame.Config", "MicrosoftGame.config"] {
                let candidate = dir.join(name);
                if let Ok(contents) = std::fs::read_to_string(&candidate)
                    && let Some(id) = parse_title_id_from_config(&contents)
                {
                    return Some(id);
                }
            }
            if !dir.pop() {
                return None;
            }
        }
    })
}

fn parse_title_id_from_config(contents: &str) -> Option<u32> {
    let start = contents.find("<TitleId")?;
    let open_end = contents[start..].find('>')? + start + 1;
    let close = contents[open_end..].find("</TitleId>")? + open_end;
    let text = contents[open_end..close].trim();
    if text.len() != 8 {
        return None;
    }
    u32::from_str_radix(text, 16).ok()
}

impl IXGameImpl_Impl for XGame_Impl {
    unsafe fn XGameGetXboxTitleId(&self, value: *mut u32) -> HRESULT {
        if value.is_null() {
            return E_POINTER;
        }
        match read_game_title_id() {
            Some(id) => {
                unsafe {
                    *value = id;
                }
                S_OK
            }
            None => {
                unsafe {
                    *value = 0;
                }
                E_NOTIMPL
            }
        }
    }
}

impl IXGameImpl2_Impl for XGame_Impl {
    /// Not something Xodus can actually do under Wine (no shell to hand off to, no second
    /// process registration) - matches WineGDK's own `FIXME ... stub!`, which is also a no-op
    /// (the method has no return value to signal failure with).
    unsafe fn XLaunchNewGame(
        &self,
        _exe_path: *const c_char,
        _args: *const c_char,
        _default_user: u64,
    ) {
    }
}

impl IXGameImpl3_Impl for XGame_Impl {
    /// Matches WineGDK's own stub: not implemented there either.
    unsafe fn XLaunchRestartOnCrash(&self, _args: *const c_char, _reserved: u32) -> HRESULT {
        E_NOTIMPL
    }
}

// The five classes below have no WineGDK reference implementation (only `XSystemAnalyticsImpl`
// appears anywhere in WineGDK's source tree - the other four do not exist there at all), so
// their IIDs/method layouts come from `xgameruntime-docs` instead. That source documents some
// of these interfaces as having methods with unknown signatures (flagged inline below) - those
// slots are still given a plausible stub so the vtable's *layout* (and therefore every method
// after it) stays correct, even though the stub itself may not be what a real call expects. On
// x64, an unexpected extra/ignored argument or return value is harmless as long as the argument
// *count* and *pointer-ness* look right, so this is safe unless the title actually calls one of
// the genuinely-unknown methods, which none of these titles are expected to.

/// `IXGameInviteImpl`'s own IID, reused as the coclass id (same pattern as `CLSID_XGAME`) -
/// confirmed via Wine trace logs as one of the classes this title queries and previously got
/// `E_NOTIMPL` for. Xodus has no invite/multiplayer-activation transport, so registration always
/// succeeds (there is nothing to fail) and simply never fires a callback.
pub const CLSID_XGAME_INVITE: GUID = GUID::from_u128(0x0651aae2_4012_4077_bf84_8b9097090e2c);

#[interface("0651aae2-4012-4077-bf84-8b9097090e2c")]
pub unsafe trait IXGameInviteImpl: IUnknown {
    unsafe fn XGameInviteRegisterForEvent(
        &self,
        queue: u64,
        context: *mut c_void,
        callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT;
    unsafe fn XGameInviteUnregisterForEvent(&self, token: u64, wait: BOOL) -> ();
}

#[interface("014d1cc3-bcfe-41ff-b2f0-e1ef07155828")]
pub unsafe trait IXGameInviteImpl2: IXGameInviteImpl {
    unsafe fn XGameInviteRegisterForPendingEvent(
        &self,
        queue: u64,
        context: *mut c_void,
        callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT;
    unsafe fn XGameInviteUnregisterForPendingEvent(&self, token: u64, wait: BOOL) -> ();
    /// `xgameruntime-docs` has no documentation at all for this method (added in a GDK update
    /// alongside the "pending event" pair above) - not even a parameter list. This signature is
    /// a guess based on the name and the shape of every other invite-acceptance call in this
    /// family (an invite/activation URI string in, HRESULT out); it exists purely to keep
    /// `XGameInviteUnregisterForPendingEvent`'s vtable slot position correct, not because the
    /// guess is trusted.
    unsafe fn XGameInviteAcceptPendingInvite(&self, invite_uri: *const c_char) -> HRESULT;
}

#[implement(IXGameInviteImpl, IXGameInviteImpl2)]
pub struct XGameInvite;

impl IXGameInviteImpl_Impl for XGameInvite_Impl {
    unsafe fn XGameInviteRegisterForEvent(
        &self,
        _queue: u64,
        _context: *mut c_void,
        _callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT {
        if !token.is_null() {
            unsafe {
                *(token as *mut u64) = 0;
            }
        }
        S_OK
    }

    unsafe fn XGameInviteUnregisterForEvent(&self, _token: u64, _wait: BOOL) {}
}

impl IXGameInviteImpl2_Impl for XGameInvite_Impl {
    unsafe fn XGameInviteRegisterForPendingEvent(
        &self,
        _queue: u64,
        _context: *mut c_void,
        _callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT {
        if !token.is_null() {
            unsafe {
                *(token as *mut u64) = 0;
            }
        }
        S_OK
    }

    unsafe fn XGameInviteUnregisterForPendingEvent(&self, _token: u64, _wait: BOOL) {}

    unsafe fn XGameInviteAcceptPendingInvite(&self, _invite_uri: *const c_char) -> HRESULT {
        E_NOTIMPL
    }
}

/// Unlike `CLSID_XGAME_INVITE`, `XGameProtocolImpl`'s coclass id (`95fd18d2...`, confirmed via
/// Wine trace logs) is *not* the same value as `IXGameProtocolImpl`'s own IID
/// (`026b010c...`) - `xgameruntime-docs`' `XGameProtocolImpl/README.md` documents them as
/// distinct, so this needs its own constant rather than reusing the interface's IID.
pub const CLSID_XGAME_PROTOCOL: GUID = GUID::from_u128(0x95fd18d2_74dd_4d7c_aa1b_0b51827665d6);

#[interface("026b010c-06c3-4cdd-bbcb-43f229db1cff")]
pub unsafe trait IXGameProtocolImpl: IUnknown {
    unsafe fn XGameProtocolRegisterForActivation(
        &self,
        queue: u64,
        context: *mut c_void,
        callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT;
    unsafe fn XGameProtocolUnregisterForActivation(&self, token: u64, wait: BOOL) -> ();
}

#[implement(IXGameProtocolImpl)]
pub struct XGameProtocol;

impl IXGameProtocolImpl_Impl for XGameProtocol_Impl {
    /// No custom-protocol activation transport exists under Wine (no shell association to
    /// register against) - registration succeeds and simply never fires, matching
    /// `XGameInviteRegisterForEvent`'s reasoning above.
    unsafe fn XGameProtocolRegisterForActivation(
        &self,
        _queue: u64,
        _context: *mut c_void,
        _callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT {
        if !token.is_null() {
            unsafe {
                *(token as *mut u64) = 0;
            }
        }
        S_OK
    }

    unsafe fn XGameProtocolUnregisterForActivation(&self, _token: u64, _wait: BOOL) {}
}

/// `IXErrorImpl`'s own IID, reused as the coclass id (same pattern as `CLSID_XGAME`) - confirmed
/// via Wine trace logs as one of the classes this title queries and previously got `E_NOTIMPL`
/// for.
pub const CLSID_XERROR: GUID = GUID::from_u128(0x8ca467f7_22e8_4096_8456_bb8aa13f79d8);

#[interface("8ca467f7-22e8-4096-8456-bb8aa13f79d8")]
pub unsafe trait IXErrorImpl: IUnknown {
    /// `xgameruntime-docs` lists this vtable slot (the first method after `IUnknown`'s three) as
    /// `*unknown*` - no name, no signature, nothing derivable from WineGDK either. This stub
    /// exists only to hold the slot's position so `XErrorSetCallback`/`XErrorSetOptions` land at
    /// the right vtable offsets; if this title ever actually calls slot 4 directly, whatever this
    /// returns is not meaningful.
    unsafe fn XErrorImpl_UnknownMethod0(&self) -> HRESULT;
    unsafe fn XErrorSetCallback(&self, callback: *mut c_void, context: *mut c_void) -> ();
    unsafe fn XErrorSetOptions(&self, options: u32) -> ();
}

#[implement(IXErrorImpl)]
pub struct XError;

impl IXErrorImpl_Impl for XError_Impl {
    unsafe fn XErrorImpl_UnknownMethod0(&self) -> HRESULT {
        E_NOTIMPL
    }

    /// No error-reporting sink to forward to (see `XErrorReport`'s own `E_NOTIMPL` in `lib.rs`) -
    /// accepting the registration without ever invoking it is honest about that absence rather
    /// than silently dropping the call as unimplemented.
    unsafe fn XErrorSetCallback(&self, _callback: *mut c_void, _context: *mut c_void) {}

    unsafe fn XErrorSetOptions(&self, _options: u32) {}
}

/// `IXSystemAnalyticsImpl`'s own IID, reused as the coclass id - confirmed via Wine trace logs
/// as one of the classes this title queries and previously got `E_NOTIMPL` for. This is the only
/// one of the five newly-added classes with a real reference implementation in WineGDK
/// (`GDKComponent/System/XSystemAnalytics.c`), which sources its values from Windows'
/// `Windows.System.Profile.AnalyticsInfo` WinRT API. Xodus has no WinRT host to query that from,
/// so the fields are fixed desktop-shaped values instead - same "no real console/sandbox concept,
/// so use a stable fixed value" reasoning as `CLSID_XSYSTEM`'s console/sandbox ids.
pub const CLSID_XSYSTEM_ANALYTICS: GUID = GUID::from_u128(0xb884675d_b738_4a9c_815d_9a9a1e0c6c9b);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct XVersion {
    pub major: u16,
    pub minor: u16,
    pub build: u16,
    pub revision: u16,
}

#[repr(C)]
pub struct XSystemAnalyticsInfo {
    pub os_version: XVersion,
    pub hosting_os_version: XVersion,
    pub family: [c_char; 64],
    pub form: [c_char; 64],
}

#[interface("b884675d-b738-4a9c-815d-9a9a1e0c6c9b")]
pub unsafe trait IXSystemAnalyticsImpl: IUnknown {
    /// Mirrors WineGDK's own C ABI for this method exactly (`XSystemAnalyticsInfo *
    /// x_system_analytics_XSystemGetAnalyticsInfo(iface, XSystemAnalyticsInfo *__ret)`): the IDL's
    /// `[out, retval]` struct return becomes a hidden out-pointer parameter that the function also
    /// returns, per the MSVC x64 ABI for large struct returns.
    unsafe fn XSystemGetAnalyticsInfo(
        &self,
        result: *mut XSystemAnalyticsInfo,
    ) -> *mut XSystemAnalyticsInfo;
}

#[implement(IXSystemAnalyticsImpl)]
pub struct XSystemAnalytics;

fn write_fixed_cstr(dst: &mut [c_char; 64], text: &[u8]) {
    let len = text.len().min(63);
    for (slot, byte) in dst.iter_mut().zip(text[..len].iter()) {
        *slot = *byte as c_char;
    }
    dst[len] = 0;
}

impl IXSystemAnalyticsImpl_Impl for XSystemAnalytics_Impl {
    unsafe fn XSystemGetAnalyticsInfo(
        &self,
        result: *mut XSystemAnalyticsInfo,
    ) -> *mut XSystemAnalyticsInfo {
        if result.is_null() {
            return result;
        }
        // A plausible, fixed "generic Windows desktop" identity - not sourced from any real
        // device, since Xodus has no WinRT AnalyticsInfo to query. Matches the family/form split
        // WineGDK's own implementation produces for real Windows (family "Windows", form
        // "Desktop").
        let version = XVersion {
            major: 10,
            minor: 0,
            build: 19045,
            revision: 0,
        };
        unsafe {
            (*result).os_version = version;
            (*result).hosting_os_version = version;
            write_fixed_cstr(&mut (*result).family, b"Windows");
            write_fixed_cstr(&mut (*result).form, b"Desktop");
        }
        result
    }
}

#[interface("2d42fea5-e71d-4b76-97cd-c50afbb3ae5d")]
pub unsafe trait IXStore: IUnknown {
    unsafe fn XStoreCreateContext(&self, user: u64, storeContextHandle: *mut u64) -> HRESULT;
    unsafe fn XStoreCloseContextHandle(&self, storeContextHandle: u64) -> ();
    unsafe fn XStoreQueryAssociatedProductsAsync(
        &self,
        storeContextHandle: u64,
        productKinds: u64,
        maxItemsToRetrievePerPage: u32,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreQueryAssociatedProductsResult(
        &self,
        async_: *mut c_void,
        productQueryHandle: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreQueryProductsAsync(
        &self,
        storeContextHandle: u64,
        productKinds: u64,
        storeIds: *mut *mut c_char,
        storeIdsCount: u64,
        actionFilters: *mut *mut c_char,
        actionFiltersCount: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreQueryProductsResult(
        &self,
        async_: *mut c_void,
        productQueryHandle: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreQueryEntitledProductsAsync(
        &self,
        storeContextHandle: u64,
        productKinds: u64,
        maxItemsToRetrievePerPage: u32,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreQueryEntitledProductsResult(
        &self,
        async_: *mut c_void,
        productQueryHandle: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreQueryProductForCurrentGameAsync(
        &self,
        storeContextHandle: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreQueryProductForCurrentGameResult(
        &self,
        async_: *mut c_void,
        productQueryHandle: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreQueryProductForPackageAsync(
        &self,
        storeContextHandle: u64,
        productKinds: u64,
        packageIdentifier: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreQueryProductForPackageResult(
        &self,
        async_: *mut c_void,
        productQueryHandle: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreEnumerateProductsQuery(
        &self,
        productQueryHandle: u64,
        context: *mut c_void,
        callback: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreProductsQueryHasMorePages(&self, productQueryHandle: u64) -> BOOL;
    unsafe fn XStoreProductsQueryNextPageAsync(
        &self,
        productQueryHandle: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreProductsQueryNextPageResult(
        &self,
        async_: *mut c_void,
        productQueryHandle: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreCloseProductsQueryHandle(&self, productQueryHandle: u64) -> ();
    unsafe fn XStoreAcquireLicenseForPackageAsync(
        &self,
        storeContextHandle: u64,
        packageIdentifier: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreAcquireLicenseForPackageResult(
        &self,
        async_: *mut c_void,
        storeLicenseHandle: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreIsLicenseValid(&self, storeLicenseHandle: u64) -> BOOL;
    unsafe fn XStoreCloseLicenseHandle(&self, storeLicenseHandle: u64) -> ();
    unsafe fn XStoreCanAcquireLicenseForStoreIdAsync(
        &self,
        storeContextHandle: u64,
        storeProductId: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreCanAcquireLicenseForStoreIdResult(
        &self,
        async_: *mut c_void,
        storeCanAcquireLicense: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreCanAcquireLicenseForPackageAsync(
        &self,
        storeContextHandle: u64,
        packageIdentifier: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreCanAcquireLicenseForPackageResult(
        &self,
        async_: *mut c_void,
        storeCanAcquireLicense: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreQueryGameLicenseAsync(
        &self,
        storeContextHandle: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreQueryGameLicenseResult(
        &self,
        async_: *mut c_void,
        license: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreQueryAddOnLicensesAsync(
        &self,
        storeContextHandle: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreQueryAddOnLicensesResultCount(
        &self,
        async_: *mut c_void,
        count: *mut u32,
    ) -> HRESULT;
    unsafe fn XStoreQueryAddOnLicensesResult(
        &self,
        async_: *mut c_void,
        count: u32,
        addOnLicenses: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreQueryConsumableBalanceRemainingAsync(
        &self,
        storeContextHandle: u64,
        storeProductId: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreQueryConsumableBalanceRemainingResult(
        &self,
        async_: *mut c_void,
        consumableResult: *mut c_void,
    ) -> HRESULT;
    unsafe fn __ReservedSlot35(&self) -> HRESULT;
    unsafe fn XStoreReportConsumableFulfillmentResult(
        &self,
        async_: *mut c_void,
        consumableResult: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreGetUserCollectionsIdAsync(
        &self,
        storeContextHandle: u64,
        serviceTicket: *mut c_char,
        publisherUserId: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreGetUserCollectionsIdResultSize(
        &self,
        async_: *mut c_void,
        size: *mut usize,
    ) -> HRESULT;
    unsafe fn XStoreGetUserCollectionsIdResult(
        &self,
        async_: *mut c_void,
        size: u64,
        result: *mut c_char,
    ) -> HRESULT;
    unsafe fn XStoreGetUserPurchaseIdAsync(
        &self,
        storeContextHandle: u64,
        serviceTicket: *mut c_char,
        publisherUserId: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreGetUserPurchaseIdResultSize(
        &self,
        async_: *mut c_void,
        size: *mut usize,
    ) -> HRESULT;
    unsafe fn XStoreGetUserPurchaseIdResult(
        &self,
        async_: *mut c_void,
        size: u64,
        result: *mut c_char,
    ) -> HRESULT;
    unsafe fn XStoreQueryLicenseTokenAsync(
        &self,
        storeContextHandle: u64,
        productIds: *mut *mut c_char,
        productIdsCount: u64,
        customDeveloperString: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreQueryLicenseTokenResultSize(
        &self,
        async_: *mut c_void,
        size: *mut usize,
    ) -> HRESULT;
    unsafe fn XStoreQueryLicenseTokenResult(
        &self,
        async_: *mut c_void,
        size: u64,
        result: *mut c_char,
    ) -> HRESULT;
    unsafe fn __ReservedSlot46(&self) -> HRESULT;
    unsafe fn __ReservedSlot47(&self) -> HRESULT;
    unsafe fn __ReservedSlot48(&self) -> HRESULT;
    unsafe fn XStoreShowPurchaseUIAsync(
        &self,
        storeContextHandle: u64,
        storeId: *mut c_char,
        name: *mut c_char,
        extendedJsonData: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreShowPurchaseUIResult(&self, async_: *mut c_void) -> HRESULT;
    unsafe fn XStoreShowRateAndReviewUIAsync(
        &self,
        storeContextHandle: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreShowRateAndReviewUIResult(
        &self,
        async_: *mut c_void,
        result: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreShowRedeemTokenUIAsync(
        &self,
        storeContextHandle: u64,
        token: *mut c_char,
        allowedStoreIds: *mut *mut c_char,
        allowedStoreIdsCount: u64,
        disallowCsvRedemption: BOOL,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreShowRedeemTokenUIResult(&self, async_: *mut c_void) -> HRESULT;
    unsafe fn XStoreQueryGameAndDlcPackageUpdatesAsync(
        &self,
        storeContextHandle: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreQueryGameAndDlcPackageUpdatesResultCount(
        &self,
        async_: *mut c_void,
        count: *mut u32,
    ) -> HRESULT;
    unsafe fn XStoreQueryGameAndDlcPackageUpdatesResult(
        &self,
        async_: *mut c_void,
        count: u32,
        packageUpdates: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreDownloadPackageUpdatesAsync(
        &self,
        storeContextHandle: u64,
        packageIdentifiers: *mut *mut c_char,
        packageIdentifiersCount: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreDownloadPackageUpdatesResult(&self, async_: *mut c_void) -> HRESULT;
    unsafe fn XStoreDownloadAndInstallPackageUpdatesAsync(
        &self,
        storeContextHandle: u64,
        packageIdentifiers: *mut *mut c_char,
        packageIdentifiersCount: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreDownloadAndInstallPackageUpdatesResult(&self, async_: *mut c_void) -> HRESULT;
    unsafe fn XStoreDownloadAndInstallPackagesAsync(
        &self,
        storeContextHandle: u64,
        storeIds: *mut *mut c_char,
        storeIdsCount: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreDownloadAndInstallPackagesResultCount(
        &self,
        async_: *mut c_void,
        count: *mut u32,
    ) -> HRESULT;
    unsafe fn XStoreDownloadAndInstallPackagesResult(
        &self,
        async_: *mut c_void,
        count: u32,
        packageIdentifiers: c_char,
    ) -> HRESULT;
    unsafe fn XStoreQueryPackageIdentifier(
        &self,
        storeId: *mut c_char,
        size: u64,
        packageIdentifier: *mut c_char,
    ) -> HRESULT;
    unsafe fn XStoreRegisterGameLicenseChanged(
        &self,
        storeContextHandle: u64,
        queue: u64,
        context: *mut c_void,
        callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreUnregisterGameLicenseChanged(
        &self,
        storeContextHandle: u64,
        token: u64,
        wait: BOOL,
    ) -> BOOL;
    unsafe fn XStoreRegisterPackageLicenseLost(
        &self,
        licenseHandle: u64,
        queue: u64,
        context: *mut c_void,
        callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreUnregisterPackageLicenseLost(
        &self,
        licenseHandle: u64,
        token: u64,
        wait: BOOL,
    ) -> BOOL;
    unsafe fn __ReservedSlot70(&self) -> HRESULT;
    unsafe fn XStoreAcquireLicenseForDurablesAsync(
        &self,
        storeContextHandle: u64,
        storeId: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreAcquireLicenseForDurablesResult(
        &self,
        async_: *mut c_void,
        storeLicenseHandle: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreShowAssociatedProductsUIAsync(
        &self,
        storeContextHandle: u64,
        storeId: *mut c_char,
        productKinds: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreShowAssociatedProductsUIResult(&self, async_: *mut c_void) -> HRESULT;
    unsafe fn XStoreShowProductPageUIAsync(
        &self,
        storeContextHandle: u64,
        storeId: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreShowProductPageUIResult(&self, async_: *mut c_void) -> HRESULT;
    unsafe fn XStoreQueryAssociatedProductsForStoreIdAsync(
        &self,
        storeContextHandle: u64,
        storeProductId: *mut c_char,
        productKinds: u64,
        maxItemsToRetrievePerPage: u32,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreQueryAssociatedProductsForStoreIdResult(
        &self,
        async_: *mut c_void,
        productQueryHandle: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreQueryPackageUpdatesAsync(
        &self,
        storeContextHandle: u64,
        packageIdentifiers: *mut *mut c_char,
        packageIdentifiersCount: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreQueryPackageUpdatesResultCount(
        &self,
        async_: *mut c_void,
        count: *mut u32,
    ) -> HRESULT;
    unsafe fn XStoreQueryPackageUpdatesResult(
        &self,
        async_: *mut c_void,
        count: u32,
        packageUpdates: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreShowGiftingUIAsync(
        &self,
        storeContextHandle: u64,
        storeId: *mut c_char,
        name: *mut c_char,
        extendedJsonData: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XStoreShowGiftingUIResult(&self, async_: *mut c_void) -> HRESULT;
}

#[interface("5c48dedf-0b67-4492-a4b5-6829b8e796e1")]
pub unsafe trait IXStoreAlias1: IXStore {}

#[interface("b09d803c-2414-4a05-82c6-66dfdc9e9a44")]
pub unsafe trait IXStoreAlias2: IXStore {}

#[interface("0dd112ac-7c24-448c-b92b-3960fb5bd30c")]
pub unsafe trait IXStoreAlias3: IXStore {}

#[interface("bf2346b2-39af-4658-b5ea-44713c7e83b3")]
pub unsafe trait IXNetworking: IUnknown {
    unsafe fn XNetworkingQueryPreferredLocalUdpMultiplayerPort(
        &self,
        preferredLocalUdpMultiplayerPort: *mut u16,
    ) -> HRESULT;
    unsafe fn XNetworkingQueryPreferredLocalUdpMultiplayerPortAsync(
        &self,
        asyncBlock: *mut c_void,
    ) -> HRESULT;
    unsafe fn XNetworkingQueryPreferredLocalUdpMultiplayerPortAsyncResult(
        &self,
        asyncBlock: *mut c_void,
        preferredLocalUdpMultiplayerPort: *mut u16,
    ) -> HRESULT;
    unsafe fn XNetworkingRegisterPreferredLocalUdpMultiplayerPortChanged(
        &self,
        queue: u64,
        context: *mut c_void,
        callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT;
    unsafe fn XNetworkingUnregisterPreferredLocalUdpMultiplayerPortChanged(
        &self,
        token: u64,
        wait: BOOL,
    ) -> BOOL;
    unsafe fn XNetworkingQuerySecurityInformationForUrlAsync(
        &self,
        url: *mut c_char,
        asyncBlock: *mut c_void,
    ) -> HRESULT;
    unsafe fn XNetworkingQuerySecurityInformationForUrlAsyncResultSize(
        &self,
        asyncBlock: *mut c_void,
        securityInformationBufferByteCount: *mut usize,
    ) -> HRESULT;
    unsafe fn XNetworkingQuerySecurityInformationForUrlAsyncResult(
        &self,
        asyncBlock: *mut c_void,
        securityInformationBufferByteCount: u64,
        securityInformationBufferByteCountUsed: *mut usize,
        securityInformationBuffer: *mut u8,
        securityInformation: *mut *mut c_void,
    ) -> HRESULT;
    unsafe fn XNetworkingQuerySecurityInformationForUrlUtf16Async(
        &self,
        url: *mut u16,
        asyncBlock: *mut c_void,
    ) -> HRESULT;
    unsafe fn XNetworkingQuerySecurityInformationForUrlUtf16AsyncResultSize(
        &self,
        asyncBlock: *mut c_void,
        securityInformationBufferByteCount: *mut usize,
    ) -> HRESULT;
    unsafe fn XNetworkingQuerySecurityInformationForUrlUtf16AsyncResult(
        &self,
        asyncBlock: *mut c_void,
        securityInformationBufferByteCount: u64,
        securityInformationBufferByteCountUsed: *mut usize,
        securityInformationBuffer: *mut u8,
        securityInformation: *mut *mut c_void,
    ) -> HRESULT;
    unsafe fn XNetworkingVerifyServerCertificate(
        &self,
        requestHandle: *mut c_void,
        securityInformation: *mut c_void,
    ) -> HRESULT;
    unsafe fn XNetworkingGetConnectivityHint(
        &self,
        connectivityHint: *mut XNetworkingConnectivityHint,
    ) -> HRESULT;
    unsafe fn XNetworkingRegisterConnectivityHintChanged(
        &self,
        queue: *mut c_void,
        context: *mut c_void,
        callback: Option<OnChanged>,
        token: *mut c_void,
    ) -> HRESULT;
    unsafe fn XNetworkingUnregisterConnectivityHintChanged(&self, token: u64, wait: BOOL) -> BOOL;
    unsafe fn XNetworkingQueryConfigurationSetting(
        &self,
        configurationSetting: u64,
        value: *mut u64,
    ) -> HRESULT;
    unsafe fn XNetworkingSetConfigurationSetting(
        &self,
        configurationSetting: u64,
        value: u64,
    ) -> HRESULT;
    unsafe fn XNetworkingQueryStatistics(
        &self,
        statisticsType: u64,
        statisticsBuffer: *mut c_void,
    ) -> HRESULT;
}

#[interface("37e56907-2f10-41e8-b72f-36edb185331a")]
pub unsafe trait IXNetworking2: IXNetworking {}

/// `wine/include/xpackage.idl`'s `IXPackageImpl`, `__PADDING__`/`__PADDING_2__`/.../`__PADDING_5__`
/// slots included in their exact positions since these are real (unnamed-in-practice) vtable
/// slots, not something to compact away. Only `XPackageGetMountPathSize`/`XPackageGetMountPath`/
/// `XPackageCloseMountHandle` have real bodies (see [`XPackageMountHandleTable`], populated by
/// `IXPersistentLocalStorage::mount_for_package`) - everything else here has no real backing
/// (package install/chunk-download management, which Xodus doesn't model) and honestly reports
/// `E_NOTIMPL`/`FALSE` rather than a guess.
#[interface("3720de07-e8e4-44a3-ad32-b359e8adbe55")]
pub unsafe trait IXPackageImpl: IUnknown {
    unsafe fn XPackageGetCurrentProcessPackageIdentifier(
        &self,
        bufferSize: usize,
        buffer: *mut c_char,
    ) -> HRESULT;
    unsafe fn XPackageIsPackagedProcess(&self) -> BOOL;
    unsafe fn XPackageCreateInstallationMonitor(
        &self,
        packageIdentifier: *const c_char,
        selectorCount: u32,
        selectors: *mut c_void,
        minimumUpdateIntervalMs: u32,
        queue: *mut c_void,
        installationMonitor: *mut c_void,
    ) -> HRESULT;
    unsafe fn XPackageCloseInstallationMonitorHandle(&self, installationMonitor: u64) -> ();
    unsafe fn XPackageGetInstallationProgress(
        &self,
        installationMonitor: u64,
        progress: *mut c_void,
    ) -> ();
    unsafe fn XPackageUpdateInstallationMonitor(&self, installationMonitor: u64) -> BOOL;
    unsafe fn XPackageRegisterInstallationProgressChanged(
        &self,
        installationMonitor: u64,
        context: *mut c_void,
        callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT;
    unsafe fn XPackageUnregisterInstallationProgressChanged(
        &self,
        installationMonitor: u64,
        token: u64,
        wait: BOOL,
    ) -> BOOL;
    unsafe fn XPackageGetUserLocale(&self, localeSize: usize, locale: *mut c_char) -> HRESULT;
    unsafe fn XPackageFindChunkAvailability(
        &self,
        packageIdentifier: *const c_char,
        selectorCount: u32,
        selectors: *mut c_void,
        availability: *mut c_void,
    ) -> HRESULT;
    unsafe fn XPackageEnumerateChunkAvailability(
        &self,
        packageIdentifier: *const c_char,
        selectorType: u32,
        context: *mut c_void,
        callback: *mut c_void,
    ) -> HRESULT;
    unsafe fn XPackageChangeChunkInstallOrder(
        &self,
        packageIdentifier: *const c_char,
        selectorCount: u32,
        selectors: *mut c_void,
    ) -> HRESULT;
    unsafe fn XPackageInstallChunks(
        &self,
        packageIdentifier: *const c_char,
        selectorCount: u32,
        selectors: *mut c_void,
        minimumUpdateIntervalMs: u32,
        suppressUserConfirmation: BOOL,
        queue: *mut c_void,
        installationMonitor: *mut c_void,
    ) -> HRESULT;
    unsafe fn XPackageInstallChunksAsync(
        &self,
        packageIdentifier: *const c_char,
        selectorCount: u32,
        selectors: *mut c_void,
        minimumUpdateIntervalMs: u32,
        suppressUserConfirmation: BOOL,
        asyncBlock: *mut c_void,
    ) -> HRESULT;
    unsafe fn XPackageInstallChunksResult(
        &self,
        asyncBlock: *mut c_void,
        installationMonitor: *mut c_void,
    ) -> HRESULT;
    unsafe fn XPackageEstimateDownloadSize(
        &self,
        packageIdentifier: *const c_char,
        selectorCount: u32,
        selectors: *mut c_void,
        downloadSize: *mut u64,
        shouldPresentUserConfirmation: *mut BOOL,
    ) -> HRESULT;
    unsafe fn XPackageUninstallChunks(
        &self,
        packageIdentifier: *const c_char,
        selectorCount: u32,
        selectors: *mut c_void,
    ) -> HRESULT;
    unsafe fn __PADDING__(&self) -> HRESULT;
    unsafe fn __PADDING_2__(&self) -> HRESULT;
    unsafe fn XPackageUnregisterPackageInstalled(&self, token: u64, wait: BOOL) -> BOOL;
    unsafe fn __PADDING_3__(&self) -> HRESULT;
    unsafe fn XPackageGetMountPathSize(
        &self,
        mount: XPackageMountHandle,
        pathSize: *mut usize,
    ) -> HRESULT;
    unsafe fn XPackageGetMountPath(
        &self,
        mount: XPackageMountHandle,
        pathSize: usize,
        path: *mut c_char,
    ) -> HRESULT;
    unsafe fn XPackageCloseMountHandle(&self, mount: XPackageMountHandle) -> ();
    unsafe fn __PADDING_4__(&self) -> HRESULT;
    unsafe fn XPackageEnumeratePackages(
        &self,
        kind: u32,
        scope: u32,
        context: *mut c_void,
        callback: *mut c_void,
    ) -> HRESULT;
    unsafe fn XPackageRegisterPackageInstalled(
        &self,
        queue: u64,
        context: *mut c_void,
        callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT;
    unsafe fn XPackageGetWriteStats(&self, writeStats: *mut c_void) -> HRESULT;
    unsafe fn __PADDING_5__(&self) -> HRESULT;
    unsafe fn XPackageUninstallUWPInstance(&self, packageName: *const c_char) -> HRESULT;
    unsafe fn XPackageEnumerateFeatures(
        &self,
        packageIdentifier: *const c_char,
        context: *mut c_void,
        callback: *mut c_void,
    ) -> HRESULT;
    unsafe fn XPackageUninstallPackage(&self, packageIdentifier: *const c_char) -> BOOL;
}

/// Adds `XPackageMountWithUiAsync`/`Result` over [`IXPackageImpl`] - not implemented, since
/// Xodus has no UI surface to show (same rationale as
/// `IXPersistentLocalStorage::prompt_user_for_space_async`, but this one has no honest
/// always-succeed answer: mounting requires actually resolving a package, unlike a storage-space
/// prompt).
#[interface("f92d8712-2b27-4d8a-bf01-11a6f8e3eb42")]
pub unsafe trait IXPackageImpl2: IXPackageImpl {
    unsafe fn XPackageMountWithUiAsync(
        &self,
        packageIdentifier: *const c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    unsafe fn XPackageMountWithUiResult(
        &self,
        async_: *mut c_void,
        mount: *mut XPackageMountHandle,
    ) -> HRESULT;
}

/// No new methods over [`IXPackageImpl2`] (`XPackageEnumeratePackages`/
/// `XPackageRegisterPackageInstalled` are re-declared in the real IDL only to mark them
/// overridden, not to add new vtable slots - same alias pattern as `IXNetworking2`/
/// `IXStoreAlias1`). This is the coclass `XPackageImpl`'s default interface.
#[interface("e2a4734b-2f4a-456d-aa8f-d065e04fb209")]
pub unsafe trait IXPackageImpl3: IXPackageImpl2 {}

macro_rules! hresult_stub {
    ($(unsafe fn $name:ident (&self $(, $arg:ident : $ty:ty)*) -> HRESULT;)*) => {
        $(unsafe fn $name(&self $(, $arg: $ty)*) -> HRESULT {
            $(let _ = $arg;)*
            eprintln!("[stub {:?}] {} -> E_NOTIMPL", std::thread::current().id(), stringify!($name));
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
    ($(unsafe fn $name:ident (&self $(, $arg:ident : $ty:ty)*) -> BOOL;)*) => {
        $(unsafe fn $name(&self $(, $arg: $ty)*) -> BOOL {
            $(let _ = $arg;)*
            eprintln!("[stub {:?}] {} -> false", std::thread::current().id(), stringify!($name));
            false.into()
        })*
    };
}

macro_rules! void_stub {
    ($(unsafe fn $name:ident (&self $(, $arg:ident : $ty:ty)*) -> ();)*) => {
        $(unsafe fn $name(&self $(, $arg: $ty)*) -> () {
            $(let _ = $arg;)*
            eprintln!("[stub {:?}] {}", std::thread::current().id(), stringify!($name));
        })*
    };
}

#[allow(clippy::too_many_arguments)]
#[implement(IXPackageImpl, IXPackageImpl2, IXPackageImpl3)]
pub struct XPackageObject;

impl IXPackageImpl_Impl for XPackageObject_Impl {
    hresult_stub! {
        unsafe fn XPackageGetCurrentProcessPackageIdentifier(&self, bufferSize: usize, buffer: *mut c_char) -> HRESULT;
        unsafe fn XPackageCreateInstallationMonitor(&self, packageIdentifier: *const c_char, selectorCount: u32, selectors: *mut c_void, minimumUpdateIntervalMs: u32, queue: *mut c_void, installationMonitor: *mut c_void) -> HRESULT;
        unsafe fn XPackageRegisterInstallationProgressChanged(&self, installationMonitor: u64, context: *mut c_void, callback: *mut c_void, token: *mut c_void) -> HRESULT;
        unsafe fn XPackageGetUserLocale(&self, localeSize: usize, locale: *mut c_char) -> HRESULT;
        unsafe fn XPackageFindChunkAvailability(&self, packageIdentifier: *const c_char, selectorCount: u32, selectors: *mut c_void, availability: *mut c_void) -> HRESULT;
        unsafe fn XPackageEnumerateChunkAvailability(&self, packageIdentifier: *const c_char, selectorType: u32, context: *mut c_void, callback: *mut c_void) -> HRESULT;
        unsafe fn XPackageChangeChunkInstallOrder(&self, packageIdentifier: *const c_char, selectorCount: u32, selectors: *mut c_void) -> HRESULT;
        unsafe fn XPackageInstallChunks(&self, packageIdentifier: *const c_char, selectorCount: u32, selectors: *mut c_void, minimumUpdateIntervalMs: u32, suppressUserConfirmation: BOOL, queue: *mut c_void, installationMonitor: *mut c_void) -> HRESULT;
        unsafe fn XPackageInstallChunksAsync(&self, packageIdentifier: *const c_char, selectorCount: u32, selectors: *mut c_void, minimumUpdateIntervalMs: u32, suppressUserConfirmation: BOOL, asyncBlock: *mut c_void) -> HRESULT;
        unsafe fn XPackageInstallChunksResult(&self, asyncBlock: *mut c_void, installationMonitor: *mut c_void) -> HRESULT;
        unsafe fn XPackageEstimateDownloadSize(&self, packageIdentifier: *const c_char, selectorCount: u32, selectors: *mut c_void, downloadSize: *mut u64, shouldPresentUserConfirmation: *mut BOOL) -> HRESULT;
        unsafe fn XPackageUninstallChunks(&self, packageIdentifier: *const c_char, selectorCount: u32, selectors: *mut c_void) -> HRESULT;
        unsafe fn __PADDING__(&self) -> HRESULT;
        unsafe fn __PADDING_2__(&self) -> HRESULT;
        unsafe fn __PADDING_3__(&self) -> HRESULT;
        unsafe fn __PADDING_4__(&self) -> HRESULT;
        unsafe fn XPackageEnumeratePackages(&self, kind: u32, scope: u32, context: *mut c_void, callback: *mut c_void) -> HRESULT;
        unsafe fn XPackageRegisterPackageInstalled(&self, queue: u64, context: *mut c_void, callback: *mut c_void, token: *mut c_void) -> HRESULT;
        unsafe fn XPackageGetWriteStats(&self, writeStats: *mut c_void) -> HRESULT;
        unsafe fn __PADDING_5__(&self) -> HRESULT;
        unsafe fn XPackageUninstallUWPInstance(&self, packageName: *const c_char) -> HRESULT;
        unsafe fn XPackageEnumerateFeatures(&self, packageIdentifier: *const c_char, context: *mut c_void, callback: *mut c_void) -> HRESULT;
    }

    bool_stub! {
        unsafe fn XPackageIsPackagedProcess(&self) -> BOOL;
        unsafe fn XPackageUpdateInstallationMonitor(&self, installationMonitor: u64) -> BOOL;
        unsafe fn XPackageUnregisterInstallationProgressChanged(&self, installationMonitor: u64, token: u64, wait: BOOL) -> BOOL;
        unsafe fn XPackageUnregisterPackageInstalled(&self, token: u64, wait: BOOL) -> BOOL;
        unsafe fn XPackageUninstallPackage(&self, packageIdentifier: *const c_char) -> BOOL;
    }

    void_stub! {
        unsafe fn XPackageCloseInstallationMonitorHandle(&self, installationMonitor: u64) -> ();
        unsafe fn XPackageGetInstallationProgress(&self, installationMonitor: u64, progress: *mut c_void) -> ();
    }

    unsafe fn XPackageGetMountPathSize(
        &self,
        mount: XPackageMountHandle,
        pathSize: *mut usize,
    ) -> HRESULT {
        let Some(path) = (unsafe { XPackageMountHandleTable::get(mount) }) else {
            return E_INVALIDARG;
        };
        if pathSize.is_null() {
            return E_POINTER;
        }
        unsafe {
            *pathSize = path.len() + 1;
        }
        S_OK
    }

    unsafe fn XPackageGetMountPath(
        &self,
        mount: XPackageMountHandle,
        pathSize: usize,
        path: *mut c_char,
    ) -> HRESULT {
        let Some(mount_path) = (unsafe { XPackageMountHandleTable::get(mount) }) else {
            return E_INVALIDARG;
        };
        if path.is_null() {
            return E_POINTER;
        }
        let bytes = mount_path.as_bytes();
        let len = bytes.len().min(pathSize.saturating_sub(1));
        for (index, byte) in bytes.iter().copied().take(len).enumerate() {
            unsafe {
                *path.add(index) = byte as c_char;
            }
        }
        if pathSize != 0 {
            unsafe {
                *path.add(len) = 0;
            }
        }
        S_OK
    }

    unsafe fn XPackageCloseMountHandle(&self, mount: XPackageMountHandle) {
        unsafe { XPackageMountHandleTable::close(mount) };
    }
}

impl IXPackageImpl2_Impl for XPackageObject_Impl {
    hresult_stub! {
        unsafe fn XPackageMountWithUiAsync(&self, packageIdentifier: *const c_char, async_: *mut c_void) -> HRESULT;
        unsafe fn XPackageMountWithUiResult(&self, async_: *mut c_void, mount: *mut XPackageMountHandle) -> HRESULT;
    }
}

impl IXPackageImpl3_Impl for XPackageObject_Impl {}

#[allow(clippy::too_many_arguments)]
#[implement(IXStore, IXStoreAlias1, IXStoreAlias2)]
pub struct XStoreObject;

impl IXStore_Impl for XStoreObject_Impl {
    hresult_stub! {
        unsafe fn XStoreQueryProductsAsync(&self, storeContextHandle: u64, productKinds: u64, storeIds: *mut *mut c_char, storeIdsCount: u64, actionFilters: *mut *mut c_char, actionFiltersCount: u64, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreQueryProductsResult(&self, async_: *mut c_void, productQueryHandle: *mut c_void) -> HRESULT;
        unsafe fn XStoreQueryProductForCurrentGameAsync(&self, storeContextHandle: u64, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreQueryProductForCurrentGameResult(&self, async_: *mut c_void, productQueryHandle: *mut c_void) -> HRESULT;
        unsafe fn XStoreQueryProductForPackageAsync(&self, storeContextHandle: u64, productKinds: u64, packageIdentifier: *mut c_char, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreQueryProductForPackageResult(&self, async_: *mut c_void, productQueryHandle: *mut c_void) -> HRESULT;
        unsafe fn XStoreProductsQueryNextPageAsync(&self, productQueryHandle: u64, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreProductsQueryNextPageResult(&self, async_: *mut c_void, productQueryHandle: *mut c_void) -> HRESULT;
        unsafe fn XStoreAcquireLicenseForPackageAsync(&self, storeContextHandle: u64, packageIdentifier: *mut c_char, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreAcquireLicenseForPackageResult(&self, async_: *mut c_void, storeLicenseHandle: *mut c_void) -> HRESULT;
        unsafe fn XStoreCanAcquireLicenseForStoreIdAsync(&self, storeContextHandle: u64, storeProductId: *mut c_char, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreCanAcquireLicenseForStoreIdResult(&self, async_: *mut c_void, storeCanAcquireLicense: *mut c_void) -> HRESULT;
        unsafe fn XStoreCanAcquireLicenseForPackageAsync(&self, storeContextHandle: u64, packageIdentifier: *mut c_char, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreCanAcquireLicenseForPackageResult(&self, async_: *mut c_void, storeCanAcquireLicense: *mut c_void) -> HRESULT;
        unsafe fn XStoreQueryAddOnLicensesAsync(&self, storeContextHandle: u64, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreQueryAddOnLicensesResultCount(&self, async_: *mut c_void, count: *mut u32) -> HRESULT;
        unsafe fn XStoreQueryAddOnLicensesResult(&self, async_: *mut c_void, count: u32, addOnLicenses: *mut c_void) -> HRESULT;
        unsafe fn XStoreQueryConsumableBalanceRemainingAsync(&self, storeContextHandle: u64, storeProductId: *mut c_char, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreQueryConsumableBalanceRemainingResult(&self, async_: *mut c_void, consumableResult: *mut c_void) -> HRESULT;
        unsafe fn __ReservedSlot35(&self) -> HRESULT;
        unsafe fn XStoreReportConsumableFulfillmentResult(&self, async_: *mut c_void, consumableResult: *mut c_void) -> HRESULT;
        unsafe fn XStoreGetUserPurchaseIdAsync(&self, storeContextHandle: u64, serviceTicket: *mut c_char, publisherUserId: *mut c_char, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreGetUserPurchaseIdResultSize(&self, async_: *mut c_void, size: *mut usize) -> HRESULT;
        unsafe fn XStoreGetUserPurchaseIdResult(&self, async_: *mut c_void, size: u64, result: *mut c_char) -> HRESULT;
        unsafe fn __ReservedSlot46(&self) -> HRESULT;
        unsafe fn __ReservedSlot47(&self) -> HRESULT;
        unsafe fn __ReservedSlot48(&self) -> HRESULT;
        unsafe fn XStoreShowPurchaseUIAsync(&self, storeContextHandle: u64, storeId: *mut c_char, name: *mut c_char, extendedJsonData: *mut c_char, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreShowPurchaseUIResult(&self, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreShowRateAndReviewUIAsync(&self, storeContextHandle: u64, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreShowRateAndReviewUIResult(&self, async_: *mut c_void, result: *mut c_void) -> HRESULT;
        unsafe fn XStoreShowRedeemTokenUIAsync(&self, storeContextHandle: u64, token: *mut c_char, allowedStoreIds: *mut *mut c_char, allowedStoreIdsCount: u64, disallowCsvRedemption: BOOL, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreShowRedeemTokenUIResult(&self, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreQueryGameAndDlcPackageUpdatesAsync(&self, storeContextHandle: u64, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreQueryGameAndDlcPackageUpdatesResultCount(&self, async_: *mut c_void, count: *mut u32) -> HRESULT;
        unsafe fn XStoreQueryGameAndDlcPackageUpdatesResult(&self, async_: *mut c_void, count: u32, packageUpdates: *mut c_void) -> HRESULT;
        unsafe fn XStoreDownloadPackageUpdatesAsync(&self, storeContextHandle: u64, packageIdentifiers: *mut *mut c_char, packageIdentifiersCount: u64, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreDownloadPackageUpdatesResult(&self, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreDownloadAndInstallPackageUpdatesAsync(&self, storeContextHandle: u64, packageIdentifiers: *mut *mut c_char, packageIdentifiersCount: u64, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreDownloadAndInstallPackageUpdatesResult(&self, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreDownloadAndInstallPackagesAsync(&self, storeContextHandle: u64, storeIds: *mut *mut c_char, storeIdsCount: u64, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreDownloadAndInstallPackagesResultCount(&self, async_: *mut c_void, count: *mut u32) -> HRESULT;
        unsafe fn XStoreDownloadAndInstallPackagesResult(&self, async_: *mut c_void, count: u32, packageIdentifiers: c_char) -> HRESULT;
        unsafe fn XStoreQueryPackageIdentifier(&self, storeId: *mut c_char, size: u64, packageIdentifier: *mut c_char) -> HRESULT;
        unsafe fn XStoreRegisterGameLicenseChanged(&self, storeContextHandle: u64, queue: u64, context: *mut c_void, callback: *mut c_void, token: *mut c_void) -> HRESULT;
        unsafe fn XStoreRegisterPackageLicenseLost(&self, licenseHandle: u64, queue: u64, context: *mut c_void, callback: *mut c_void, token: *mut c_void) -> HRESULT;
        unsafe fn __ReservedSlot70(&self) -> HRESULT;
        unsafe fn XStoreAcquireLicenseForDurablesAsync(&self, storeContextHandle: u64, storeId: *mut c_char, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreAcquireLicenseForDurablesResult(&self, async_: *mut c_void, storeLicenseHandle: *mut c_void) -> HRESULT;
        unsafe fn XStoreShowAssociatedProductsUIAsync(&self, storeContextHandle: u64, storeId: *mut c_char, productKinds: u64, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreShowAssociatedProductsUIResult(&self, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreShowProductPageUIAsync(&self, storeContextHandle: u64, storeId: *mut c_char, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreShowProductPageUIResult(&self, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreQueryAssociatedProductsForStoreIdAsync(&self, storeContextHandle: u64, storeProductId: *mut c_char, productKinds: u64, maxItemsToRetrievePerPage: u32, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreQueryAssociatedProductsForStoreIdResult(&self, async_: *mut c_void, productQueryHandle: *mut c_void) -> HRESULT;
        unsafe fn XStoreQueryPackageUpdatesAsync(&self, storeContextHandle: u64, packageIdentifiers: *mut *mut c_char, packageIdentifiersCount: u64, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreQueryPackageUpdatesResultCount(&self, async_: *mut c_void, count: *mut u32) -> HRESULT;
        unsafe fn XStoreQueryPackageUpdatesResult(&self, async_: *mut c_void, count: u32, packageUpdates: *mut c_void) -> HRESULT;
        unsafe fn XStoreShowGiftingUIAsync(&self, storeContextHandle: u64, storeId: *mut c_char, name: *mut c_char, extendedJsonData: *mut c_char, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreShowGiftingUIResult(&self, async_: *mut c_void) -> HRESULT;
    }
    bool_stub! {
        unsafe fn XStoreIsLicenseValid(&self, storeLicenseHandle: u64) -> BOOL;
        unsafe fn XStoreUnregisterGameLicenseChanged(&self, storeContextHandle: u64, token: u64, wait: BOOL) -> BOOL;
        unsafe fn XStoreUnregisterPackageLicenseLost(&self, licenseHandle: u64, token: u64, wait: BOOL) -> BOOL;
    }
    void_stub! {
        unsafe fn XStoreCloseContextHandle(&self, storeContextHandle: u64) -> ();
        unsafe fn XStoreCloseLicenseHandle(&self, storeLicenseHandle: u64) -> ();
    }

    unsafe fn XStoreCreateContext(&self, _user: u64, storeContextHandle: *mut u64) -> HRESULT {
        eprintln!(
            "[diag {:?}] XStoreCreateContext(user={_user}) -> handle=1",
            std::thread::current().id()
        );
        unsafe {
            *storeContextHandle = 1;
        };
        HRESULT(0)
    }

    unsafe fn XStoreQueryGameLicenseAsync(
        &self,
        storeContextHandle: u64,
        async_: *mut c_void,
    ) -> HRESULT {
        eprintln!(
            "[diag {:?}] XStoreQueryGameLicenseAsync(context={storeContextHandle})",
            std::thread::current().id()
        );
        if storeContextHandle == 0 {
            return E_POINTER;
        }
        unsafe { xasync::run_sync(async_.cast(), move || Ok(query_game_license())) }
    }

    unsafe fn XStoreQueryGameLicenseResult(
        &self,
        async_: *mut c_void,
        license: *mut c_void,
    ) -> HRESULT {
        // println!("XStoreQueryGameLicenseResult");
        if async_.is_null() || license.is_null() {
            return E_POINTER;
        }

        let mut payload = XStoreQueryGameLicenseAsyncResultPayload {
            license: XStoreGameLicense::default(),
        };
        match unsafe { get_result(async_.cast(), null_mut(), &mut payload) } {
            Ok(_) => {
                unsafe {
                    *(license as *mut XStoreGameLicense) = payload.license;
                }
                S_OK
            }
            Err(hr) => hr,
        }
    }

    unsafe fn XStoreQueryEntitledProductsAsync(
        &self,
        storeContextHandle: u64,
        _productKinds: u64,
        _maxItemsToRetrievePerPage: u32,
        async_: *mut c_void,
    ) -> HRESULT {
        if storeContextHandle == 0 {
            return E_POINTER;
        }
        unsafe { xasync::run_sync(async_.cast(), move || Ok(query_entitled_products())) }
    }

    unsafe fn XStoreQueryEntitledProductsResult(
        &self,
        async_: *mut c_void,
        productQueryHandle: *mut c_void,
    ) -> HRESULT {
        if async_.is_null() || productQueryHandle.is_null() {
            return E_POINTER;
        }

        let mut handle = 0u64;
        match unsafe { get_result(async_.cast(), null_mut(), &mut handle) } {
            Ok(_) => {
                unsafe {
                    *(productQueryHandle as *mut u64) = handle;
                }
                S_OK
            }
            Err(hr) => hr,
        }
    }

    /// `XStoreQueryAssociatedProductsAsync`'s real backing - see [`query_associated_products`]
    /// for the endpoint/honesty rationale.
    unsafe fn XStoreQueryAssociatedProductsAsync(
        &self,
        storeContextHandle: u64,
        _productKinds: u64,
        maxItemsToRetrievePerPage: u32,
        async_: *mut c_void,
    ) -> HRESULT {
        if storeContextHandle == 0 {
            return E_POINTER;
        }
        unsafe {
            xasync::run_sync(async_.cast(), move || {
                Ok(query_associated_products(maxItemsToRetrievePerPage))
            })
        }
    }

    unsafe fn XStoreQueryAssociatedProductsResult(
        &self,
        async_: *mut c_void,
        productQueryHandle: *mut c_void,
    ) -> HRESULT {
        if async_.is_null() || productQueryHandle.is_null() {
            return E_POINTER;
        }

        let mut handle = 0u64;
        match unsafe { get_result(async_.cast(), null_mut(), &mut handle) } {
            Ok(_) => {
                unsafe {
                    *(productQueryHandle as *mut u64) = handle;
                }
                S_OK
            }
            Err(hr) => hr,
        }
    }

    unsafe fn XStoreEnumerateProductsQuery(
        &self,
        productQueryHandle: u64,
        context: *mut c_void,
        callback: *mut c_void,
    ) -> HRESULT {
        if callback.is_null() {
            return E_POINTER;
        }
        let Some(query) = (unsafe { ProductQueryHandleTable::get(productQueryHandle) }) else {
            return E_INVALIDARG;
        };
        let callback: XStoreProductQueryCallback = unsafe { std::mem::transmute(callback) };
        for entry in &query.entries {
            let keep_going = unsafe { callback(&entry.product as *const XStoreProduct, context) };
            if keep_going == 0 {
                break;
            }
        }
        S_OK
    }

    unsafe fn XStoreProductsQueryHasMorePages(&self, _productQueryHandle: u64) -> BOOL {
        // "My games" doesn't paginate - every entitled product comes back in one page.
        false.into()
    }

    unsafe fn XStoreCloseProductsQueryHandle(&self, productQueryHandle: u64) {
        unsafe { ProductQueryHandleTable::close(productQueryHandle) };
    }

    /// `XStoreGetUserCollectionsIdAsync`'s real backing, via `xodus-service`'s
    /// `CollectionsIdRequest` handler (`ipc::get_user_collections_id`) - a real call
    /// against `collections.mp.microsoft.com/v7.0/beneficiaries/me/keys`, endpoint
    /// confirmed via static analysis of the real `xgameruntime.dll` (see PLAN.md), not
    /// guessed. `serviceTicket`/`publisherUserId` are the caller's own opaque values,
    /// forwarded verbatim.
    unsafe fn XStoreGetUserCollectionsIdAsync(
        &self,
        storeContextHandle: u64,
        serviceTicket: *mut c_char,
        publisherUserId: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT {
        if storeContextHandle == 0 {
            return E_POINTER;
        }
        let service_ticket = unsafe { c_string_or_empty(serviceTicket) };
        let publisher_user_id = unsafe { c_string_or_empty(publisherUserId) };
        let key = async_ as usize;
        unsafe {
            xasync::run_sync(async_.cast(), move || -> Result<(), HRESULT> {
                let result =
                    crate::ipc::get_user_collections_id(&service_ticket, &publisher_user_id);
                let outcome = match &result {
                    Ok(_) => Ok(()),
                    Err(hr) => Err(*hr),
                };
                COLLECTIONS_ID_RESULTS
                    .lock()
                    .expect("collections id results poisoned")
                    .get_or_insert_with(HashMap::new)
                    .insert(key, result);
                outcome
            })
        }
    }

    unsafe fn XStoreGetUserCollectionsIdResultSize(
        &self,
        async_: *mut c_void,
        size: *mut usize,
    ) -> HRESULT {
        if size.is_null() {
            return E_POINTER;
        }
        let key = async_ as usize;
        let results = COLLECTIONS_ID_RESULTS
            .lock()
            .expect("collections id results poisoned");
        match results.as_ref().and_then(|results| results.get(&key)) {
            Some(Ok(value)) => {
                unsafe { *size = value.len() + 1 };
                S_OK
            }
            Some(Err(hr)) => *hr,
            None => E_ILLEGAL_METHOD_CALL,
        }
    }

    unsafe fn XStoreGetUserCollectionsIdResult(
        &self,
        async_: *mut c_void,
        size: u64,
        result: *mut c_char,
    ) -> HRESULT {
        let key = async_ as usize;
        let results = COLLECTIONS_ID_RESULTS
            .lock()
            .expect("collections id results poisoned");
        match results.as_ref().and_then(|results| results.get(&key)) {
            Some(Ok(value)) => {
                let bytes = value.as_bytes();
                let needed = bytes.len() + 1;
                if needed > size as usize {
                    return E_NOT_SUFFICIENT_BUFFER;
                }
                if result.is_null() {
                    return E_POINTER;
                }
                unsafe {
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), result.cast::<u8>(), bytes.len());
                    *result.cast::<u8>().add(bytes.len()) = 0;
                }
                S_OK
            }
            Some(Err(hr)) => *hr,
            None => E_ILLEGAL_METHOD_CALL,
        }
    }

    /// `XStoreQueryLicenseTokenAsync`'s real backing, via `xodus-service`'s
    /// `LicenseTokenRequest` handler (`ipc::get_license_token`) - a real call against
    /// `licensing.mp.microsoft.com/v8.0/licenseToken`, endpoint confirmed the same way as
    /// `XStoreGetUserCollectionsIdAsync` above. `productIds[0]` is treated as the parent
    /// product and the rest as related products, matching the real API's flat array.
    unsafe fn XStoreQueryLicenseTokenAsync(
        &self,
        storeContextHandle: u64,
        productIds: *mut *mut c_char,
        productIdsCount: u64,
        customDeveloperString: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT {
        if storeContextHandle == 0 {
            return E_POINTER;
        }
        let product_ids: Vec<String> = if productIds.is_null() {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(productIds, productIdsCount as usize) }
                .iter()
                .map(|&ptr| unsafe { c_string_or_empty(ptr) })
                .collect()
        };
        let custom_developer_string = unsafe { c_string_or_empty(customDeveloperString) };
        let key = async_ as usize;
        unsafe {
            xasync::run_sync(async_.cast(), move || -> Result<(), HRESULT> {
                let result = crate::ipc::get_license_token(&product_ids, &custom_developer_string);
                let outcome = match &result {
                    Ok(_) => Ok(()),
                    Err(hr) => Err(*hr),
                };
                LICENSE_TOKEN_RESULTS
                    .lock()
                    .expect("license token results poisoned")
                    .get_or_insert_with(HashMap::new)
                    .insert(key, result);
                outcome
            })
        }
    }

    unsafe fn XStoreQueryLicenseTokenResultSize(
        &self,
        async_: *mut c_void,
        size: *mut usize,
    ) -> HRESULT {
        if size.is_null() {
            return E_POINTER;
        }
        let key = async_ as usize;
        let results = LICENSE_TOKEN_RESULTS
            .lock()
            .expect("license token results poisoned");
        match results.as_ref().and_then(|results| results.get(&key)) {
            Some(Ok(value)) => {
                unsafe { *size = value.len() + 1 };
                S_OK
            }
            Some(Err(hr)) => *hr,
            None => E_ILLEGAL_METHOD_CALL,
        }
    }

    unsafe fn XStoreQueryLicenseTokenResult(
        &self,
        async_: *mut c_void,
        size: u64,
        result: *mut c_char,
    ) -> HRESULT {
        let key = async_ as usize;
        let results = LICENSE_TOKEN_RESULTS
            .lock()
            .expect("license token results poisoned");
        match results.as_ref().and_then(|results| results.get(&key)) {
            Some(Ok(value)) => {
                let bytes = value.as_bytes();
                let needed = bytes.len() + 1;
                if needed > size as usize {
                    return E_NOT_SUFFICIENT_BUFFER;
                }
                if result.is_null() {
                    return E_POINTER;
                }
                unsafe {
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), result.cast::<u8>(), bytes.len());
                    *result.cast::<u8>().add(bytes.len()) = 0;
                }
                S_OK
            }
            Some(Err(hr)) => *hr,
            None => E_ILLEGAL_METHOD_CALL,
        }
    }
}

/// Keyed by the caller's `async_` pointer, same rationale and same leak-on-unread
/// tradeoff as `xuser.rs`'s `MSA_TOKEN_RESULTS` - both `XStoreGetUserCollectionsIdAsync`
/// and `XStoreQueryLicenseTokenAsync` return an opaque, variable-length string whose size
/// has to be answered by a separate `*ResultSize` call before `*Result` is called at all.
static COLLECTIONS_ID_RESULTS: Mutex<Option<HashMap<usize, Result<String, HRESULT>>>> =
    Mutex::new(None);
static LICENSE_TOKEN_RESULTS: Mutex<Option<HashMap<usize, Result<String, HRESULT>>>> =
    Mutex::new(None);

/// # Safety
/// `ptr` must be null or a valid, NUL-terminated C string for the duration of this call.
unsafe fn c_string_or_empty(ptr: *mut c_char) -> String {
    if ptr.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }
}

impl IXStoreAlias1_Impl for XStoreObject_Impl {}
impl IXStoreAlias2_Impl for XStoreObject_Impl {}
impl IXStoreAlias3_Impl for XStoreObject_Impl {}

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
            // println!("XNetworkingRegisterConnectivityHintChanged");
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
        let _url = unsafe { CStr::from_ptr(url) };
        // println!("XNetworkingQuerySecurityInformationForUrlAsync {}", url.to_string_lossy());
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
        let r = unsafe { xasync::get_result_size(asyncBlock.cast()) };
        match r {
            Ok(size) => unsafe {
                *securityInformationBufferByteCount = size;
                // println!("XNetworkingQuerySecurityInformationForUrlAsyncResultSize: OK");
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
            unsafe { *securityInformationBufferByteCountUsed = 0 };
        }
        match unsafe {
            get_result(
                asyncBlock.cast(),
                null_mut(),
                securityInformationBuffer.cast::<XNetworkingSecurityInformation>(),
            )
        } {
            Ok(_) => {
                if !securityInformationBufferByteCountUsed.is_null() {
                    unsafe {
                        *securityInformationBufferByteCountUsed =
                            size_of::<XNetworkingSecurityInformation>()
                    };
                }
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
        let r = unsafe { xasync::get_result_size(asyncBlock.cast()) };
        match r {
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
            unsafe { *securityInformationBufferByteCountUsed = 0 };
        }
        match unsafe {
            get_result(
                asyncBlock.cast(),
                null_mut(),
                securityInformationBuffer.cast::<XNetworkingSecurityInformation>(),
            )
        } {
            Ok(_) => {
                if !securityInformationBufferByteCountUsed.is_null() {
                    unsafe {
                        *securityInformationBufferByteCountUsed =
                            size_of::<XNetworkingSecurityInformation>()
                    };
                }
                unsafe { *securityInformation = securityInformationBuffer.cast() };
                S_OK
            }
            Err(hr) => hr,
        }
    }
}

impl IXNetworking2_Impl for XNetworkingObject_Impl {}

struct GlobalInterface<T>(T);

unsafe impl<T> Send for GlobalInterface<T> {}
unsafe impl<T> Sync for GlobalInterface<T> {}

static XFEATURE_SINGLETON: OnceLock<GlobalInterface<IXFeature>> = OnceLock::new();
static XSTORE_SINGLETON: OnceLock<GlobalInterface<IXStore>> = OnceLock::new();
static XNETWORKING_SINGLETON: OnceLock<GlobalInterface<IXNetworking>> = OnceLock::new();
static XPERSISTENT_LOCAL_STORAGE_SINGLETON: OnceLock<GlobalInterface<IXPersistentLocalStorage>> =
    OnceLock::new();
static XPACKAGE_SINGLETON: OnceLock<GlobalInterface<IXPackageImpl3>> = OnceLock::new();
static XASYNC_SINGLETON: OnceLock<GlobalInterface<crate::xasync::IXAsync>> = OnceLock::new();
static XSYSTEM_SINGLETON: OnceLock<GlobalInterface<IXSystem>> = OnceLock::new();
static XGAME_SINGLETON: OnceLock<GlobalInterface<IXGameImpl3>> = OnceLock::new();
static XGAME_INVITE_SINGLETON: OnceLock<GlobalInterface<IXGameInviteImpl2>> = OnceLock::new();
static XGAME_PROTOCOL_SINGLETON: OnceLock<GlobalInterface<IXGameProtocolImpl>> = OnceLock::new();
static XERROR_SINGLETON: OnceLock<GlobalInterface<IXErrorImpl>> = OnceLock::new();
static XSYSTEM_ANALYTICS_SINGLETON: OnceLock<GlobalInterface<IXSystemAnalyticsImpl>> =
    OnceLock::new();

fn xsystem_singleton() -> &'static IXSystem {
    &XSYSTEM_SINGLETON.get_or_init(|| GlobalInterface(XSystem.into())).0
}

fn xgame_singleton() -> &'static IXGameImpl3 {
    &XGAME_SINGLETON.get_or_init(|| GlobalInterface(XGame.into())).0
}

fn xgame_invite_singleton() -> &'static IXGameInviteImpl2 {
    &XGAME_INVITE_SINGLETON
        .get_or_init(|| GlobalInterface(XGameInvite.into()))
        .0
}

fn xgame_protocol_singleton() -> &'static IXGameProtocolImpl {
    &XGAME_PROTOCOL_SINGLETON
        .get_or_init(|| GlobalInterface(XGameProtocol.into()))
        .0
}

fn xerror_singleton() -> &'static IXErrorImpl {
    &XERROR_SINGLETON.get_or_init(|| GlobalInterface(XError.into())).0
}

fn xsystem_analytics_singleton() -> &'static IXSystemAnalyticsImpl {
    &XSYSTEM_ANALYTICS_SINGLETON
        .get_or_init(|| GlobalInterface(XSystemAnalytics.into()))
        .0
}

/// The async runtime is a process-wide singleton: task queues and in-flight calls have
/// to be shared between every API that hands out an `XAsyncBlock`.
fn xasync_singleton() -> &'static crate::xasync::IXAsync {
    &XASYNC_SINGLETON
        .get_or_init(|| GlobalInterface(crate::xasync_impl::XAsyncObject.into()))
        .0
}

fn xfeature_singleton() -> &'static IXFeature {
    &XFEATURE_SINGLETON
        .get_or_init(|| GlobalInterface(XFeature.into()))
        .0
}

fn xstore_singleton() -> &'static IXStore {
    &XSTORE_SINGLETON
        .get_or_init(|| GlobalInterface(XStoreObject.into()))
        .0
}

fn xnetworking_singleton() -> &'static IXNetworking {
    &XNETWORKING_SINGLETON
        .get_or_init(|| GlobalInterface(XNetworkingObject.into()))
        .0
}

fn xpersistent_local_storage_singleton() -> &'static IXPersistentLocalStorage {
    &XPERSISTENT_LOCAL_STORAGE_SINGLETON
        .get_or_init(|| {
            GlobalInterface(
                XPersistentLocalStorage {
                    tmp_path: temp_dir().to_string_lossy().into_owned(),
                }
                .into(),
            )
        })
        .0
}

/// Only backs `XPackageGetMountPathSize`/`XPackageGetMountPath`/`XPackageCloseMountHandle` for
/// real - see [`IXPackageImpl`]'s docs.
fn xpackage_singleton() -> &'static IXPackageImpl3 {
    &XPACKAGE_SINGLETON
        .get_or_init(|| GlobalInterface(XPackageObject.into()))
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
        // println!("query: ack {:#32x}", interface_id.to_u128());
        S_OK
    } else {
        println!("query: nack {:#32x}", interface_id.to_u128());
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

    let class_id = unsafe { *runtime_class_id };
    // println!("query_api_impl: {:#8x}-{:#4x}-{:#4x}-{:#4x}", class_id.data1, class_id.data2, class_id.data3, class_id.data4);
    if let Some(result) = crate::gdk_extra::query_stubbed(class_id, interface_id, out) {
        return result;
    }
    match class_id {
        IXFeature::IID => {
            // println!("query_api_impl: {:#32x} {:#32x}", class_id.to_u128(), unsafe { *interface_id }.to_u128());
            query(xfeature_singleton(), interface_id, out)
        }
        CLSID_XSTORE => {
            eprintln!(
                "[diag {:?}] query_api_impl: CLSID_XSTORE requested",
                std::thread::current().id()
            );
            query(xstore_singleton(), interface_id, out)
        }
        CLSID_XNETWORKING => {
            // println!(
            //     "query_api_impl: {:#32x} {:#32x}",
            //     class_id.to_u128(),
            //     unsafe { *interface_id }.to_u128()
            // );
            query(xnetworking_singleton(), interface_id, out)
        }
        CLSID_XPERSISTENT_LOCAL_STORAGE => {
            query(xpersistent_local_storage_singleton(), interface_id, out)
        }
        CLSID_XPACKAGE => query(xpackage_singleton(), interface_id, out),
        crate::xasync::CLSID_XASYNC => query(xasync_singleton(), interface_id, out),
        crate::xuser::CLSID_XUSER => query(crate::xuser::xuser_singleton(), interface_id, out),
        crate::xuser::CLSID_XUSER_DEVICE => {
            query(crate::xuser::xuser_device_singleton(), interface_id, out)
        }
        crate::xgamesave::CLSID_XGAMESAVE => {
            query(crate::xgamesave::xgamesave_singleton(), interface_id, out)
        }
        CLSID_XSYSTEM => {
            eprintln!(
                "[diag {:?}] query_api_impl: CLSID_XSYSTEM requested",
                std::thread::current().id()
            );
            query(xsystem_singleton(), interface_id, out)
        }
        CLSID_XGAME => query(xgame_singleton(), interface_id, out),
        CLSID_XGAME_INVITE => query(xgame_invite_singleton(), interface_id, out),
        CLSID_XGAME_PROTOCOL => query(xgame_protocol_singleton(), interface_id, out),
        CLSID_XERROR => query(xerror_singleton(), interface_id, out),
        CLSID_XSYSTEM_ANALYTICS => {
            eprintln!(
                "[diag {:?}] query_api_impl: CLSID_XSYSTEM_ANALYTICS requested",
                std::thread::current().id()
            );
            query(xsystem_analytics_singleton(), interface_id, out)
        }
        _ => {
            // Everything this crate does not implement yet. There is no Microsoft DLL to
            // fall back to - that is the point - so say so rather than crashing the game.
            println!(
                "query_api_impl: unimplemented class {:#034x}",
                class_id.to_u128()
            );
            unsafe {
                *out = std::ptr::null_mut();
            }
            E_NOTIMPL
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::{c_char, c_void};

    use crate::com::{IXStore, XStoreGameLicense, query_api_impl};
    use crate::xasync::{XAsyncBlock, get_status};
    use crate::{InitializeApiImplEx2, UninitializeApiImpl};
    use windows_core::{HRESULT, Interface};

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
    /// The stub surface (`gdk_extra.rs`) must be reachable through the same
    /// `QueryApiImpl` entry point a game uses: every class WineGDK's C `QueryApiImpl`
    /// dispatches that this crate stubs (rather than implements) should resolve with its
    /// default interface IID to `S_OK` and a non-null object, exactly like the real
    /// classes - so a title probing for e.g. `CLSID_XDisplayImpl` gets a live vtable,
    /// not the unresolved-class `E_NOTIMPL` a `_`-fallthrough would give.
    #[test]
    fn stub_classes_resolve_via_query_api_impl() {
        // (class id, default interface IID, singleton pointer sanity) - mirror of the
        // `gdk_extra` dispatch table. `IXThreadingImpl` is intentionally absent: its
        // coclass uuid is `CLSID_XASYNC`, served by the real `XAsync` singleton.
        let cases: &[(GUID, GUID)] = &[
            (
                crate::gdk_extra::CLSID_XACCESSIBILITY,
                crate::gdk_extra::IXAccessibilityImpl2::IID,
            ),
            (
                crate::gdk_extra::CLSID_XAPPCAPTURE,
                crate::gdk_extra::IXAppCaptureImpl4::IID,
            ),
            (
                crate::gdk_extra::CLSID_XAPPCAPTURE_METADATA,
                crate::gdk_extra::IXAppCaptureMetadataImpl::IID,
            ),
            (
                crate::gdk_extra::CLSID_XDISPLAY,
                crate::gdk_extra::IXDisplayImpl::IID,
            ),
            (
                crate::gdk_extra::CLSID_XLAUNCHER,
                crate::gdk_extra::IXLauncherImpl::IID,
            ),
            (
                crate::gdk_extra::CLSID_XGAME_ACTIVATION,
                crate::gdk_extra::IXGameActivationImpl::IID,
            ),
            (
                crate::gdk_extra::CLSID_XGAME_EVENT,
                crate::gdk_extra::IXGameEventImpl::IID,
            ),
            (
                crate::gdk_extra::CLSID_XGAME_STREAMING,
                crate::gdk_extra::IXGameStreamingImpl3::IID,
            ),
            (
                crate::gdk_extra::CLSID_XGAME_UI,
                crate::gdk_extra::IXGameUiImpl4::IID,
            ),
        ];

        for (class_id, interface_id) in cases {
            let mut out: *mut c_void = std::ptr::null_mut();
            let hr = query_api_impl(class_id, interface_id, &mut out);
            assert_eq!(
                hr,
                HRESULT(0),
                "QueryApiImpl for {class_id:?} with default IID {interface_id:?} should resolve"
            );
            assert!(!out.is_null(), "stub class {class_id:?} returned a null object");

            // An unrelated IID must be refused honestly, not crash.
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
    /// honest-absence fallback (empty list, see `query_entitled_products`) is what the
    /// callback should observe - not a crash, not fabricated products.
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
            0i32
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
            "no xodus-service reachable in this test - empty is the honest answer"
        );

        unsafe { store.XStoreCloseProductsQueryHandle(handle) };
    }
}
