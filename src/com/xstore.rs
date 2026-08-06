use super::E_NOTIMPL;
use crate::com::xasync::{self, get_result};
use crate::results::*;
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::ptr::null_mut;
use std::sync::Mutex;
use windows_core::{GUID, HRESULT, IUnknown, implement, interface};
use windows_sys::core::BOOL;

use super::bool_stub;
use super::hresult_stub;
use super::void_stub;
use crate::diag::diag;
pub const CLSID_XSTORE: GUID = GUID::from_u128(0x0dd112ac_7c24_448c_b92b_3960fb5bd30c);
const STORE_SKU_ID_SIZE: usize = 18;
const TRIAL_UNIQUE_ID_MAX_SIZE: usize = 64;

#[allow(dead_code)] // GDK handle type kept for the XStore API surface.
type XStoreContextHandle = u64;

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
/// that isn't available (see its doc comment for why "active" is the default there, not
/// "not licensed").
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
// struct layout for this project.
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
pub struct XStoreProduct {
    storeId: *const c_char,
    pub title: *const c_char,
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

/// Maps `EntitledProduct`/`AssociatedProductEntry`'s freeform `product_kind` string onto
/// `XStoreProductKind`'s bitmask (`wine/include/xstore.idl`). Only "Game" is derivable from
/// what `xodus-service` actually returns - anything else reports as none rather than guessing
/// a specific DLC/consumable/durable kind.
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
/// decision riding on this answer, so the absence default is the right one here.
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
/// found/parsed) or a failed catalog fetch both report an empty list - same absence
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

#[interface("2d42fea5-e71d-4b76-97cd-c50afbb3ae5d")]
pub unsafe trait IXStore: IUnknown {
    pub unsafe fn XStoreCreateContext(&self, user: u64, storeContextHandle: *mut u64) -> HRESULT;
    pub unsafe fn XStoreCloseContextHandle(&self, storeContextHandle: u64) -> ();
    pub unsafe fn XStoreQueryAssociatedProductsAsync(
        &self,
        storeContextHandle: u64,
        productKinds: u64,
        maxItemsToRetrievePerPage: u32,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryAssociatedProductsResult(
        &self,
        async_: *mut c_void,
        productQueryHandle: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryProductsAsync(
        &self,
        storeContextHandle: u64,
        productKinds: u64,
        storeIds: *mut *mut c_char,
        storeIdsCount: u64,
        actionFilters: *mut *mut c_char,
        actionFiltersCount: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryProductsResult(
        &self,
        async_: *mut c_void,
        productQueryHandle: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryEntitledProductsAsync(
        &self,
        storeContextHandle: u64,
        productKinds: u64,
        maxItemsToRetrievePerPage: u32,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryEntitledProductsResult(
        &self,
        async_: *mut c_void,
        productQueryHandle: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryProductForCurrentGameAsync(
        &self,
        storeContextHandle: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryProductForCurrentGameResult(
        &self,
        async_: *mut c_void,
        productQueryHandle: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryProductForPackageAsync(
        &self,
        storeContextHandle: u64,
        productKinds: u64,
        packageIdentifier: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryProductForPackageResult(
        &self,
        async_: *mut c_void,
        productQueryHandle: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreEnumerateProductsQuery(
        &self,
        productQueryHandle: u64,
        context: *mut c_void,
        callback: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreProductsQueryHasMorePages(&self, productQueryHandle: u64) -> BOOL;
    pub unsafe fn XStoreProductsQueryNextPageAsync(
        &self,
        productQueryHandle: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreProductsQueryNextPageResult(
        &self,
        async_: *mut c_void,
        productQueryHandle: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreCloseProductsQueryHandle(&self, productQueryHandle: u64) -> ();
    pub unsafe fn XStoreAcquireLicenseForPackageAsync(
        &self,
        storeContextHandle: u64,
        packageIdentifier: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreAcquireLicenseForPackageResult(
        &self,
        async_: *mut c_void,
        storeLicenseHandle: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreIsLicenseValid(&self, storeLicenseHandle: u64) -> BOOL;
    pub unsafe fn XStoreCloseLicenseHandle(&self, storeLicenseHandle: u64) -> ();
    pub unsafe fn XStoreCanAcquireLicenseForStoreIdAsync(
        &self,
        storeContextHandle: u64,
        storeProductId: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreCanAcquireLicenseForStoreIdResult(
        &self,
        async_: *mut c_void,
        storeCanAcquireLicense: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreCanAcquireLicenseForPackageAsync(
        &self,
        storeContextHandle: u64,
        packageIdentifier: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreCanAcquireLicenseForPackageResult(
        &self,
        async_: *mut c_void,
        storeCanAcquireLicense: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryGameLicenseAsync(
        &self,
        storeContextHandle: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryGameLicenseResult(
        &self,
        async_: *mut c_void,
        license: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryAddOnLicensesAsync(
        &self,
        storeContextHandle: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryAddOnLicensesResultCount(
        &self,
        async_: *mut c_void,
        count: *mut u32,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryAddOnLicensesResult(
        &self,
        async_: *mut c_void,
        count: u32,
        addOnLicenses: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryConsumableBalanceRemainingAsync(
        &self,
        storeContextHandle: u64,
        storeProductId: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryConsumableBalanceRemainingResult(
        &self,
        async_: *mut c_void,
        consumableResult: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn __ReservedSlot35(&self) -> HRESULT;
    pub unsafe fn XStoreReportConsumableFulfillmentResult(
        &self,
        async_: *mut c_void,
        consumableResult: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreGetUserCollectionsIdAsync(
        &self,
        storeContextHandle: u64,
        serviceTicket: *mut c_char,
        publisherUserId: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreGetUserCollectionsIdResultSize(
        &self,
        async_: *mut c_void,
        size: *mut usize,
    ) -> HRESULT;
    pub unsafe fn XStoreGetUserCollectionsIdResult(
        &self,
        async_: *mut c_void,
        size: u64,
        result: *mut c_char,
    ) -> HRESULT;
    pub unsafe fn XStoreGetUserPurchaseIdAsync(
        &self,
        storeContextHandle: u64,
        serviceTicket: *mut c_char,
        publisherUserId: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreGetUserPurchaseIdResultSize(
        &self,
        async_: *mut c_void,
        size: *mut usize,
    ) -> HRESULT;
    pub unsafe fn XStoreGetUserPurchaseIdResult(
        &self,
        async_: *mut c_void,
        size: u64,
        result: *mut c_char,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryLicenseTokenAsync(
        &self,
        storeContextHandle: u64,
        productIds: *mut *mut c_char,
        productIdsCount: u64,
        customDeveloperString: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryLicenseTokenResultSize(
        &self,
        async_: *mut c_void,
        size: *mut usize,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryLicenseTokenResult(
        &self,
        async_: *mut c_void,
        size: u64,
        result: *mut c_char,
    ) -> HRESULT;
    pub unsafe fn __ReservedSlot46(&self) -> HRESULT;
    pub unsafe fn __ReservedSlot47(&self) -> HRESULT;
    pub unsafe fn __ReservedSlot48(&self) -> HRESULT;
    pub unsafe fn XStoreShowPurchaseUIAsync(
        &self,
        storeContextHandle: u64,
        storeId: *mut c_char,
        name: *mut c_char,
        extendedJsonData: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreShowPurchaseUIResult(&self, async_: *mut c_void) -> HRESULT;
    pub unsafe fn XStoreShowRateAndReviewUIAsync(
        &self,
        storeContextHandle: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreShowRateAndReviewUIResult(
        &self,
        async_: *mut c_void,
        result: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreShowRedeemTokenUIAsync(
        &self,
        storeContextHandle: u64,
        token: *mut c_char,
        allowedStoreIds: *mut *mut c_char,
        allowedStoreIdsCount: u64,
        disallowCsvRedemption: BOOL,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreShowRedeemTokenUIResult(&self, async_: *mut c_void) -> HRESULT;
    pub unsafe fn XStoreQueryGameAndDlcPackageUpdatesAsync(
        &self,
        storeContextHandle: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryGameAndDlcPackageUpdatesResultCount(
        &self,
        async_: *mut c_void,
        count: *mut u32,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryGameAndDlcPackageUpdatesResult(
        &self,
        async_: *mut c_void,
        count: u32,
        packageUpdates: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreDownloadPackageUpdatesAsync(
        &self,
        storeContextHandle: u64,
        packageIdentifiers: *mut *mut c_char,
        packageIdentifiersCount: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreDownloadPackageUpdatesResult(&self, async_: *mut c_void) -> HRESULT;
    pub unsafe fn XStoreDownloadAndInstallPackageUpdatesAsync(
        &self,
        storeContextHandle: u64,
        packageIdentifiers: *mut *mut c_char,
        packageIdentifiersCount: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreDownloadAndInstallPackageUpdatesResult(
        &self,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreDownloadAndInstallPackagesAsync(
        &self,
        storeContextHandle: u64,
        storeIds: *mut *mut c_char,
        storeIdsCount: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreDownloadAndInstallPackagesResultCount(
        &self,
        async_: *mut c_void,
        count: *mut u32,
    ) -> HRESULT;
    pub unsafe fn XStoreDownloadAndInstallPackagesResult(
        &self,
        async_: *mut c_void,
        count: u32,
        packageIdentifiers: c_char,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryPackageIdentifier(
        &self,
        storeId: *mut c_char,
        size: u64,
        packageIdentifier: *mut c_char,
    ) -> HRESULT;
    pub unsafe fn XStoreRegisterGameLicenseChanged(
        &self,
        storeContextHandle: u64,
        queue: u64,
        context: *mut c_void,
        callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreUnregisterGameLicenseChanged(
        &self,
        storeContextHandle: u64,
        token: u64,
        wait: BOOL,
    ) -> BOOL;
    pub unsafe fn XStoreRegisterPackageLicenseLost(
        &self,
        licenseHandle: u64,
        queue: u64,
        context: *mut c_void,
        callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreUnregisterPackageLicenseLost(
        &self,
        licenseHandle: u64,
        token: u64,
        wait: BOOL,
    ) -> BOOL;
    pub unsafe fn __ReservedSlot70(&self) -> HRESULT;
    pub unsafe fn XStoreAcquireLicenseForDurablesAsync(
        &self,
        storeContextHandle: u64,
        storeId: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreAcquireLicenseForDurablesResult(
        &self,
        async_: *mut c_void,
        storeLicenseHandle: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreShowAssociatedProductsUIAsync(
        &self,
        storeContextHandle: u64,
        storeId: *mut c_char,
        productKinds: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreShowAssociatedProductsUIResult(&self, async_: *mut c_void) -> HRESULT;
    pub unsafe fn XStoreShowProductPageUIAsync(
        &self,
        storeContextHandle: u64,
        storeId: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreShowProductPageUIResult(&self, async_: *mut c_void) -> HRESULT;
    pub unsafe fn XStoreQueryAssociatedProductsForStoreIdAsync(
        &self,
        storeContextHandle: u64,
        storeProductId: *mut c_char,
        productKinds: u64,
        maxItemsToRetrievePerPage: u32,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryAssociatedProductsForStoreIdResult(
        &self,
        async_: *mut c_void,
        productQueryHandle: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryPackageUpdatesAsync(
        &self,
        storeContextHandle: u64,
        packageIdentifiers: *mut *mut c_char,
        packageIdentifiersCount: u64,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryPackageUpdatesResultCount(
        &self,
        async_: *mut c_void,
        count: *mut u32,
    ) -> HRESULT;
    pub unsafe fn XStoreQueryPackageUpdatesResult(
        &self,
        async_: *mut c_void,
        count: u32,
        packageUpdates: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreShowGiftingUIAsync(
        &self,
        storeContextHandle: u64,
        storeId: *mut c_char,
        name: *mut c_char,
        extendedJsonData: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XStoreShowGiftingUIResult(&self, async_: *mut c_void) -> HRESULT;
}

#[interface("5c48dedf-0b67-4492-a4b5-6829b8e796e1")]
pub unsafe trait IXStoreAlias1: IXStore {}

#[interface("b09d803c-2414-4a05-82c6-66dfdc9e9a44")]
pub unsafe trait IXStoreAlias2: IXStore {}

#[interface("0dd112ac-7c24-448c-b92b-3960fb5bd30c")]
pub unsafe trait IXStoreAlias3: IXStore {}

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
        diag!("XStoreCreateContext(user={_user}) -> handle=1");
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
        diag!("XStoreQueryGameLicenseAsync(context={storeContextHandle})");
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
    /// for the endpoint rationale.
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
    /// confirmed via static analysis of the real `xgameruntime.dll`. `serviceTicket`/
    /// `publisherUserId` are the caller's own opaque values, forwarded verbatim.
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
