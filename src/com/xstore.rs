use super::E_NOTIMPL;
use crate::com::handle_table;
use crate::com::xasync::{self, get_result};
use crate::results::*;
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use windows_core::{GUID, HRESULT, IUnknown, implement, interface};
use crate::com::BOOLEAN;
use xodus_ipc_models::xstore::{CatalogProductEntry, StoreUiKind};

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
const XSTORE_PRODUCT_KIND_CONSUMABLE: u32 = 0x01;
const XSTORE_PRODUCT_KIND_DURABLE: u32 = 0x02;
const XSTORE_PRODUCT_KIND_GAME: u32 = 0x04;
const XSTORE_PRODUCT_KIND_PASS: u32 = 0x08;
const XSTORE_PRODUCT_KIND_UNMANAGED_CONSUMABLE: u32 = 0x10;

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
    _currency_code: CString,
    product: XStoreProduct,
}

/// Fills in an [`XStorePrice`] from what `xodus-service`'s catalog lookup returned.
///
/// `is_on_sale` isn't carried over the wire because it isn't a catalog field - it's the
/// comparison between the list price and the undiscounted one, made here so both sides can't
/// disagree about it. `sale_end_date` is passed through untouched; the catalog fills it with a
/// far-future sentinel outside a sale, which is harmless because nothing reads it unless
/// `isOnSale`.
///
/// The `currencyCode` pointer borrows `currency_code`, which the caller must keep alive in the
/// same [`ProductQueryEntry`] as the returned price - the same arrangement as `storeId`/`title`.
fn store_price(entry: &CatalogProductEntry, currency_code: &CStr) -> XStorePrice {
    XStorePrice {
        basePrice: entry.base_price,
        price: entry.price,
        recurrencePrice: entry.recurrence_price,
        currencyCode: currency_code.as_ptr(),
        formattedBasePrice: format_price(entry.base_price, &entry.currency_code),
        formattedPrice: format_price(entry.price, &entry.currency_code),
        formattedRecurrencePrice: format_price(entry.recurrence_price, &entry.currency_code),
        isOnSale: entry.price < entry.base_price,
        saleEndDate: entry.sale_end_date,
    }
}

/// Renders an amount for the `formatted*` fields, which is what a store page actually displays.
///
/// The real GDK gets these strings from the Store client, already formatted for the account's
/// market - symbol placement, decimal separator, digit grouping and all. We have only the amount
/// and an ISO 4217 code, and the field is 16 *bytes* of ANSI, so the best available answer is a
/// plain ASCII rendering: `$7.99` where the symbol is unambiguous and ASCII, `7.99 EUR`
/// otherwise. Deliberately not a locale-formatting attempt - a wrong separator reads as a wrong
/// price, whereas a currency code beside the number is merely plain.
///
/// An empty `currency_code` means the catalog listed no purchasable availability, and yields an
/// empty string rather than a bare `0.00` that would read as free.
fn format_price(amount: f32, currency_code: &str) -> [c_char; 16] {
    let text = match currency_code {
        "" => String::new(),
        // The only symbol that is both ASCII and unambiguous. `$` alone would be wrong for the
        // eight other dollar currencies, which get the code treatment below.
        "USD" => format!("${amount:.2}"),
        code => format!("{amount:.2} {code}"),
    };

    let mut out = [0 as c_char; 16];
    // Truncating at a byte boundary is safe for content this function produced: it is ASCII by
    // construction. The last slot stays zero so the result is always terminated.
    for (slot, byte) in out.iter_mut().zip(text.bytes().take(15)) {
        *slot = byte as c_char;
    }
    out
}

struct ProductQuery {
    entries: Vec<ProductQueryEntry>,
}

// Raw pointers in `XStoreProduct` only ever point into `CString`s owned by the same
// `ProductQueryEntry`, are read-only after construction, and the `Arc` in
// `ProductQueryHandleTable` never hands out anything but shared access - safe to move or
// share across threads on the same footing as the `CString`/`String` data behind them.
// SAFETY: see above - the raw pointers are read-only and always outlived by their
// owning `CString`s, so moving a `ProductQuery` across threads is sound.
unsafe impl Send for ProductQuery {}
// SAFETY: see above - the raw pointers are read-only after construction, so
// shared (`&ProductQuery`) access across threads is sound.
unsafe impl Sync for ProductQuery {}

/// Handle table for `XStoreQueryEntitledProductsAsync`/`XStoreEnumerateProductsQuery`/
/// `XStoreCloseProductsQueryHandle`. Stores an `Arc` rather than the bare `ProductQuery`
/// because [`HandleTable::get`] hands out clones - `ProductQuery`'s entries are
/// self-referential (each `XStoreProduct`'s raw pointers point into its own `CString`
/// storage), so a deep clone would leave those pointers dangling into the original's
/// allocations.
struct ProductQueryHandleTable;

/// Token source for `XStoreRegisterGameLicenseChanged`. Starts at 1 so 0 stays available
/// as "not a token this crate issued".
static LICENSE_CHANGE_TOKENS: AtomicU64 = AtomicU64::new(1);

static PRODUCT_QUERY_HANDLES: handle_table::HandleTable<Arc<ProductQuery>> =
    handle_table::HandleTable::new();

impl ProductQueryHandleTable {
    fn create(query: ProductQuery) -> u64 {
        PRODUCT_QUERY_HANDLES.create(Arc::new(query))
    }

    fn get(handle: u64) -> Option<Arc<ProductQuery>> {
        PRODUCT_QUERY_HANDLES.get(handle)
    }

    fn close(handle: u64) {
        PRODUCT_QUERY_HANDLES.close(handle);
    }
}

/// Maps `EntitledProduct`/`CatalogProductEntry`'s freeform `product_kind` string onto
/// `XStoreProductKind`'s bitmask (`wine/include/xstore.idl`). The strings are displaycatalog's
/// `ProductKind` verbatim, which uses exactly the same names as the enum. An unrecognized kind
/// reports as none rather than being guessed at - but the recognized ones matter: a title that
/// asked for `Durable` and gets back `None` will filter the product straight back out, which is
/// how a correctly-priced Realms plan still renders blank.
fn product_kind_flag(kind: &str) -> u32 {
    const KINDS: [(&str, u32); 5] = [
        ("Consumable", XSTORE_PRODUCT_KIND_CONSUMABLE),
        ("Durable", XSTORE_PRODUCT_KIND_DURABLE),
        ("Game", XSTORE_PRODUCT_KIND_GAME),
        ("Pass", XSTORE_PRODUCT_KIND_PASS),
        (
            "UnmanagedConsumable",
            XSTORE_PRODUCT_KIND_UNMANAGED_CONSUMABLE,
        ),
    ];

    KINDS
        .iter()
        .find(|(name, _)| kind.eq_ignore_ascii_case(name))
        .map_or(XSTORE_PRODUCT_KIND_NONE, |(_, flag)| *flag)
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
    let products =
        crate::ipc::get_entitled_products(&crate::ipc::store_market()).unwrap_or_default();

    let entries = products
        .into_iter()
        .map(|product| {
            let store_id = CString::new(product.store_id).unwrap_or_default();

            let title = CString::new(product.title).unwrap_or_default();

            let product_kind = product_kind_flag(&product.product_kind);

            // No price in this payload, but an empty string rather than a null `currencyCode`:
            // a title that reads the field before checking the amount finds "", not a crash.
            let currency_code = CString::default();

            let entry = XStoreProduct {
                storeId: store_id.as_ptr(),

                title: title.as_ptr(),

                description: std::ptr::null(),

                language: std::ptr::null(),

                inAppOfferToken: std::ptr::null(),

                linkUri: std::ptr::null_mut(),

                productKind: product_kind,

                price: XStorePrice {
                    currencyCode: currency_code.as_ptr(),
                    ..XStorePrice::default()
                },

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

                _currency_code: currency_code,

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
///
/// `maxItemsToRetrievePerPage` is deliberately not forwarded as a limit. It is a page size, and
/// `XStoreProductsQueryHasMorePages` here always answers "no more pages", so whatever comes back
/// from this one call is everything the title will ever see - capping it at the title's page size
/// would silently hide the rest of the catalog. Minecraft asks for a page of 25 and its Realms
/// subscriptions sit several hundred products into the associated-product list, which is exactly
/// how the "Choose your plan" screen ends up with no price to show.
fn query_associated_products(_max_items_to_retrieve_per_page: u32) -> u64 {
    let products = crate::ipc::get_associated_products(0).unwrap_or_default();

    build_product_query(products)
}

/// `XStoreQueryProductsAsync`'s real backing, via `xodus-service`'s `ProductsRequest` handler
/// (`ipc::get_products`) - a catalog lookup for `StoreId`s the title names outright. This is what
/// an in-game storefront runs on, so an empty answer is a page with no prices on it rather than
/// no page: Minecraft's "Choose your plan" screen asks here for the Realms Core and Realms Plus
/// subscriptions and renders whatever comes back beside "/month".
///
/// `productKinds` and `actionFilters` are not applied: the caller already chose the exact
/// products by id, and `xodus-service` has no per-kind or per-action facet to filter on beyond
/// the freeform `product_kind` string it returns. Filtering on a guess would drop products the
/// title explicitly asked for - see `query_associated_products` for the same reasoning.
///
/// `isInUserCollection` is `false` for the same reason as `query_associated_products`: these are
/// catalog entries, not confirmed entitlements.
fn query_products(store_ids: &[String]) -> u64 {
    let products = crate::ipc::get_products(store_ids).unwrap_or_default();

    build_product_query(products)
}

/// Copies a GDK `char**` argument into owned strings.
///
/// # Safety
/// `array` must either be null or point to `count` nul-terminated strings that stay valid for
/// the duration of the call. Null elements are skipped rather than dereferenced.
unsafe fn read_string_array(array: *mut *mut c_char, count: u64) -> Vec<String> {
    if array.is_null() {
        return Vec::new();
    }
    (0..count as usize)
        .filter_map(|i| {
            // SAFETY: `i` is within `count`, which the caller promises the array holds.
            let entry = unsafe { *array.add(i) };
            if entry.is_null() {
                return None;
            }
            // SAFETY: a non-null element is a nul-terminated string per the caller's contract.
            Some(
                unsafe { CStr::from_ptr(entry) }
                    .to_string_lossy()
                    .into_owned(),
            )
        })
        .collect()
}

/// Turns `xodus-service`'s catalog answer into a handle the title can enumerate.
///
/// Shared by [`query_associated_products`] and [`query_products`], which differ only in how the
/// products were chosen. The fields left empty (`description`/`language`/`inAppOfferToken`/
/// `linkUri`/keywords/skus/images/videos) are not in the `fieldsTemplate=StoreSDK` subset
/// `xodus-service` asks the catalog for, so they are zeroed rather than guessed at.
fn build_product_query(products: Vec<CatalogProductEntry>) -> u64 {
    // Catalog listings, not credentials: the store id, kind and asking price are exactly
    // what the storefront is meant to render, and an empty `currency_code` here is what
    // [`format_price`] turns into the blank price the plan picker rejects.
    for product in &products {
        diag!(
            "catalog product: store_id={} kind={} price={} base={} currency={:?}",
            product.store_id,
            product.product_kind,
            product.price,
            product.base_price,
            product.currency_code
        );
    }

    let entries = products
        .into_iter()
        .map(|product| {
            let currency_code = CString::new(product.currency_code.as_str()).unwrap_or_default();

            // Built before `product`'s strings are moved out into `CString`s below.
            let price = store_price(&product, &currency_code);

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

                price,

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

                _currency_code: currency_code,

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
    pub unsafe fn XStoreProductsQueryHasMorePages(&self, productQueryHandle: u64) -> BOOLEAN;
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
    pub unsafe fn XStoreIsLicenseValid(&self, storeLicenseHandle: u64) -> BOOLEAN;
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
        disallowCsvRedemption: BOOLEAN,
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
        wait: BOOLEAN,
    ) -> BOOLEAN;
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
        wait: BOOLEAN,
    ) -> BOOLEAN;
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
        unsafe fn __ReservedSlot46(&self) -> HRESULT;
        unsafe fn __ReservedSlot47(&self) -> HRESULT;
        unsafe fn __ReservedSlot48(&self) -> HRESULT;
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
        unsafe fn XStoreRegisterPackageLicenseLost(&self, licenseHandle: u64, queue: u64, context: *mut c_void, callback: *mut c_void, token: *mut c_void) -> HRESULT;
        unsafe fn __ReservedSlot70(&self) -> HRESULT;
        unsafe fn XStoreAcquireLicenseForDurablesAsync(&self, storeContextHandle: u64, storeId: *mut c_char, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreAcquireLicenseForDurablesResult(&self, async_: *mut c_void, storeLicenseHandle: *mut c_void) -> HRESULT;
        unsafe fn XStoreQueryAssociatedProductsForStoreIdAsync(&self, storeContextHandle: u64, storeProductId: *mut c_char, productKinds: u64, maxItemsToRetrievePerPage: u32, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreQueryAssociatedProductsForStoreIdResult(&self, async_: *mut c_void, productQueryHandle: *mut c_void) -> HRESULT;
        unsafe fn XStoreQueryPackageUpdatesAsync(&self, storeContextHandle: u64, packageIdentifiers: *mut *mut c_char, packageIdentifiersCount: u64, async_: *mut c_void) -> HRESULT;
        unsafe fn XStoreQueryPackageUpdatesResultCount(&self, async_: *mut c_void, count: *mut u32) -> HRESULT;
        unsafe fn XStoreQueryPackageUpdatesResult(&self, async_: *mut c_void, count: u32, packageUpdates: *mut c_void) -> HRESULT;
    }
    bool_stub! {
        unsafe fn XStoreIsLicenseValid(&self, storeLicenseHandle: u64) -> BOOLEAN;
        unsafe fn XStoreUnregisterPackageLicenseLost(&self, licenseHandle: u64, token: u64, wait: BOOLEAN) -> BOOLEAN;
    }
    void_stub! {
        unsafe fn XStoreCloseContextHandle(&self, storeContextHandle: u64) -> ();
        unsafe fn XStoreCloseLicenseHandle(&self, storeLicenseHandle: u64) -> ();
    }

    unsafe fn XStoreCreateContext(&self, _user: u64, storeContextHandle: *mut u64) -> HRESULT {
        diag!("XStoreCreateContext(user={_user}) -> handle=1");
        // SAFETY: storeContextHandle is a valid u64 out-pointer per the
        // XStoreCreateContext GDK contract.
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
        // SAFETY: async_ is the caller's XAsyncBlock pointer per the XAsync GDK
        // contract; run_sync itself no-ops on a null pointer.
        unsafe { xasync::run_sync(async_.cast(), move || Ok(query_game_license())) }
    }

    unsafe fn XStoreQueryGameLicenseResult(
        &self,
        async_: *mut c_void,
        license: *mut c_void,
    ) -> HRESULT {
        if async_.is_null() || license.is_null() {
            return E_POINTER;
        }

        let mut payload = XStoreQueryGameLicenseAsyncResultPayload {
            license: XStoreGameLicense::default(),
        };
        // SAFETY: async_ was null-checked above and payload's type matches the T
        // that XStoreQueryGameLicenseAsync's run_sync closure stored.
        match unsafe { get_result(async_.cast(), null_mut(), &mut payload) } {
            Ok(_) => {
                // SAFETY: license was null-checked above and is a valid
                // XStoreGameLicense out-pointer per the GDK contract.
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
        // SAFETY: async_ is the caller's XAsyncBlock pointer per the XAsync GDK
        // contract; run_sync itself no-ops on a null pointer.
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
        // SAFETY: async_ was null-checked above and handle's type (u64) matches
        // what query_entitled_products's run_sync closure stored.
        match unsafe { get_result(async_.cast(), null_mut(), &mut handle) } {
            Ok(_) => {
                // SAFETY: productQueryHandle was null-checked above and is a valid
                // u64 out-pointer per the GDK contract.
                unsafe {
                    *(productQueryHandle as *mut u64) = handle;
                }
                S_OK
            }
            Err(hr) => hr,
        }
    }

    /// `XStoreQueryProductsAsync`'s real backing - see [`query_products`] for the endpoint
    /// rationale and for why `productKinds`/`actionFilters` are not applied.
    unsafe fn XStoreQueryProductsAsync(
        &self,
        storeContextHandle: u64,
        _productKinds: u64,
        storeIds: *mut *mut c_char,
        storeIdsCount: u64,
        _actionFilters: *mut *mut c_char,
        _actionFiltersCount: u64,
        async_: *mut c_void,
    ) -> HRESULT {
        if storeContextHandle == 0 {
            return E_POINTER;
        }
        // SAFETY: `storeIds` is the caller's array of `storeIdsCount` nul-terminated strings
        // per the GDK contract, read here and not retained - `read_string_array` copies each
        // one and tolerates a null array or null elements.
        let store_ids = unsafe { read_string_array(storeIds, storeIdsCount) };
        diag!("XStoreQueryProductsAsync(context={storeContextHandle}, ids={store_ids:?})");
        // SAFETY: async_ is the caller's XAsyncBlock pointer per the XAsync GDK
        // contract; run_sync itself no-ops on a null pointer.
        unsafe { xasync::run_sync(async_.cast(), move || Ok(query_products(&store_ids))) }
    }

    unsafe fn XStoreQueryProductsResult(
        &self,
        async_: *mut c_void,
        productQueryHandle: *mut c_void,
    ) -> HRESULT {
        if async_.is_null() || productQueryHandle.is_null() {
            return E_POINTER;
        }

        let mut handle = 0u64;
        // SAFETY: async_ was null-checked above and handle's type (u64) matches
        // what query_products's run_sync closure stored.
        match unsafe { get_result(async_.cast(), null_mut(), &mut handle) } {
            Ok(_) => {
                // SAFETY: productQueryHandle was null-checked above and is a valid
                // u64 out-pointer per the GDK contract.
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
        diag!("XStoreQueryAssociatedProductsAsync(context={storeContextHandle})");
        // SAFETY: async_ is the caller's XAsyncBlock pointer per the XAsync GDK
        // contract; run_sync itself no-ops on a null pointer.
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
        // SAFETY: async_ was null-checked above and handle's type (u64) matches
        // what query_associated_products's run_sync closure stored.
        match unsafe { get_result(async_.cast(), null_mut(), &mut handle) } {
            Ok(_) => {
                // SAFETY: productQueryHandle was null-checked above and is a valid
                // u64 out-pointer per the GDK contract.
                unsafe {
                    *(productQueryHandle as *mut u64) = handle;
                }
                S_OK
            }
            Err(hr) => hr,
        }
    }

    /// Accepts a game-license-change listener and hands back a token, without ever calling
    /// the listener.
    ///
    /// The registration itself has to succeed: titles call this while *building* their store
    /// object, so returning `E_NOTIMPL` here can leave a title without a usable store at all.
    /// (It does not, on its own, produce Minecraft's "Couldn't access platform store" dialog -
    /// that is the plan picker's empty-price state, reached with the store object fully built.
    /// See `docs/xodus/store.md`.)
    ///
    /// Never invoking the callback is honest rather than lazy: a game license changes when
    /// the store grants or revokes one out from under a running title, and nothing here can
    /// observe that. `XStoreQueryGameLicenseAsync` remains the real source of license state.
    unsafe fn XStoreRegisterGameLicenseChanged(
        &self,
        storeContextHandle: u64,
        _queue: u64,
        _context: *mut c_void,
        callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT {
        if storeContextHandle == 0 || callback.is_null() || token.is_null() {
            return E_POINTER;
        }
        // `token` is an `XTaskQueueRegistrationToken*`, a single u64 field. Distinct and
        // nonzero so a caller that validates the token before unregistering isn't misled.
        let raw = LICENSE_CHANGE_TOKENS.fetch_add(1, Ordering::Relaxed);
        diag!("XStoreRegisterGameLicenseChanged(context={storeContextHandle}) -> token={raw}");
        // SAFETY: `token` was null-checked above and is a valid u64 out-pointer per the
        // GDK contract.
        unsafe { *token.cast::<u64>() = raw };
        S_OK
    }

    /// The unregister half of [`XStoreRegisterGameLicenseChanged`]. There is nothing to tear
    /// down - no callback was ever scheduled - so any token this crate handed out
    /// unregisters successfully.
    unsafe fn XStoreUnregisterGameLicenseChanged(
        &self,
        _storeContextHandle: u64,
        token: u64,
        _wait: BOOLEAN,
    ) -> BOOLEAN {
        if token == 0 || token >= LICENSE_CHANGE_TOKENS.load(Ordering::Relaxed) {
            return false.into();
        }
        true.into()
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
        let Some(query) = ProductQueryHandleTable::get(productQueryHandle) else {
            diag!("XStoreEnumerateProductsQuery(handle={productQueryHandle}) -> no such handle");
            return E_INVALIDARG;
        };
        // The count is the whole point: a storefront that enumerates zero products renders
        // a page with no prices on it, which is indistinguishable from never having asked.
        diag!(
            "XStoreEnumerateProductsQuery(handle={productQueryHandle}) -> {} products",
            query.entries.len()
        );
        // SAFETY: GDK guarantees `callback` matches `XStoreProductQueryCallback` when
        // calling `XStoreEnumerateProductsQuery`.
        let callback: XStoreProductQueryCallback =
            unsafe { crate::ffi_util::fn_ptr_cast(callback) };
        for entry in &query.entries {
            // SAFETY: callback was validated as XStoreProductQueryCallback above;
            // XStoreEnumerateProductsQuery's contract is that it's valid to invoke
            // for the duration of this enumeration loop.
            let keep_going = unsafe { callback(&entry.product as *const XStoreProduct, context) };
            if keep_going == 0 {
                break;
            }
        }
        S_OK
    }

    unsafe fn XStoreProductsQueryHasMorePages(&self, _productQueryHandle: u64) -> BOOLEAN {
        // "My games" doesn't paginate - every entitled product comes back in one page.
        false.into()
    }

    unsafe fn XStoreCloseProductsQueryHandle(&self, productQueryHandle: u64) {
        ProductQueryHandleTable::close(productQueryHandle);
    }

    /// `XStoreGetUserCollectionsIdAsync`'s real backing, via `xodus-service`'s
    /// `CollectionsIdRequest` handler (`ipc::get_user_collections_id`) - a real call
    /// against `collections.mp.microsoft.com/v7.0/beneficiaries/me/keys`. `serviceTicket`/
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
        // SAFETY: serviceTicket is a GDK-caller-supplied pointer that's null or
        // NUL-terminated per the XStoreGetUserCollectionsIdAsync contract.
        let service_ticket = unsafe { c_string_or_empty(serviceTicket) };
        // SAFETY: publisherUserId is a GDK-caller-supplied pointer that's null or
        // NUL-terminated per the XStoreGetUserCollectionsIdAsync contract.
        let publisher_user_id = unsafe { c_string_or_empty(publisherUserId) };
        diag!("XStoreGetUserCollectionsIdAsync(context={storeContextHandle})");
        let key = async_ as usize;
        // SAFETY: async_ is the caller's XAsyncBlock pointer per the XAsync GDK
        // contract; run_sync itself no-ops on a null pointer.
        unsafe {
            xasync::run_sync(async_.cast(), move || -> Result<(), HRESULT> {
                let result =
                    crate::ipc::get_user_collections_id(&service_ticket, &publisher_user_id);
                store_opaque_result(&COLLECTIONS_ID_RESULTS, key, result)
            })
        }
    }

    unsafe fn XStoreGetUserCollectionsIdResultSize(
        &self,
        async_: *mut c_void,
        size: *mut usize,
    ) -> HRESULT {
        // SAFETY: size is the caller's out-pointer per the GDK contract; the helper
        // null-checks it before writing.
        unsafe { opaque_result_size("collections-id", &COLLECTIONS_ID_RESULTS, async_, size) }
    }

    unsafe fn XStoreGetUserCollectionsIdResult(
        &self,
        async_: *mut c_void,
        size: u64,
        result: *mut c_char,
    ) -> HRESULT {
        // SAFETY: result is the caller's buffer of `size` bytes per the GDK contract; the
        // helper null-checks it and verifies the buffer is large enough before writing.
        unsafe {
            opaque_result(
                "collections-id",
                &COLLECTIONS_ID_RESULTS,
                async_,
                size,
                result,
            )
        }
    }

    /// `XStoreGetUserPurchaseIdAsync`'s real backing, via `xodus-service`'s
    /// `PurchaseIdRequest` handler (`ipc::get_user_purchase_id`) - the purchase-side twin of
    /// `XStoreGetUserCollectionsIdAsync` above. The two are not spelled alike - purchase is
    /// `users/me/keys` where collections is `beneficiaries/me/keys`, and each 404s on the
    /// other's path. See `xodus::licensing::content::get_purchase_id`.
    unsafe fn XStoreGetUserPurchaseIdAsync(
        &self,
        storeContextHandle: u64,
        serviceTicket: *mut c_char,
        publisherUserId: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT {
        if storeContextHandle == 0 {
            return E_POINTER;
        }
        // SAFETY: serviceTicket is a GDK-caller-supplied pointer that's null or
        // NUL-terminated per the XStoreGetUserPurchaseIdAsync contract.
        let service_ticket = unsafe { c_string_or_empty(serviceTicket) };
        // SAFETY: publisherUserId is a GDK-caller-supplied pointer that's null or
        // NUL-terminated per the XStoreGetUserPurchaseIdAsync contract.
        let publisher_user_id = unsafe { c_string_or_empty(publisherUserId) };
        diag!("XStoreGetUserPurchaseIdAsync(context={storeContextHandle})");
        let key = async_ as usize;
        // SAFETY: async_ is the caller's XAsyncBlock pointer per the XAsync GDK
        // contract; run_sync itself no-ops on a null pointer.
        unsafe {
            xasync::run_sync(async_.cast(), move || -> Result<(), HRESULT> {
                let result = crate::ipc::get_user_purchase_id(&service_ticket, &publisher_user_id);
                store_opaque_result(&PURCHASE_ID_RESULTS, key, result)
            })
        }
    }

    unsafe fn XStoreGetUserPurchaseIdResultSize(
        &self,
        async_: *mut c_void,
        size: *mut usize,
    ) -> HRESULT {
        // SAFETY: size is the caller's out-pointer per the GDK contract; the helper
        // null-checks it before writing.
        unsafe { opaque_result_size("purchase-id", &PURCHASE_ID_RESULTS, async_, size) }
    }

    unsafe fn XStoreGetUserPurchaseIdResult(
        &self,
        async_: *mut c_void,
        size: u64,
        result: *mut c_char,
    ) -> HRESULT {
        // SAFETY: result is the caller's buffer of `size` bytes per the GDK contract; the
        // helper null-checks it and verifies the buffer is large enough before writing.
        unsafe { opaque_result("purchase-id", &PURCHASE_ID_RESULTS, async_, size, result) }
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
            // SAFETY: productIds is non-null (checked above) and productIdsCount is
            // the caller's contractual element count for it per the
            // XStoreQueryLicenseTokenAsync GDK contract.
            unsafe { std::slice::from_raw_parts(productIds, productIdsCount as usize) }
                .iter()
                // SAFETY: each ptr is a GDK-caller-supplied pointer that's null or
                // NUL-terminated per XStoreQueryLicenseTokenAsync's productIds contract.
                .map(|&ptr| unsafe { c_string_or_empty(ptr) })
                .collect()
        };
        // SAFETY: customDeveloperString is a GDK-caller-supplied pointer that's
        // null or NUL-terminated per the XStoreQueryLicenseTokenAsync contract.
        let custom_developer_string = unsafe { c_string_or_empty(customDeveloperString) };
        let key = async_ as usize;
        // SAFETY: async_ is the caller's XAsyncBlock pointer per the XAsync GDK
        // contract; run_sync itself no-ops on a null pointer.
        unsafe {
            xasync::run_sync(async_.cast(), move || -> Result<(), HRESULT> {
                let result = crate::ipc::get_license_token(&product_ids, &custom_developer_string);
                store_opaque_result(&LICENSE_TOKEN_RESULTS, key, result)
            })
        }
    }

    unsafe fn XStoreQueryLicenseTokenResultSize(
        &self,
        async_: *mut c_void,
        size: *mut usize,
    ) -> HRESULT {
        // SAFETY: size is the caller's out-pointer per the GDK contract; the helper
        // null-checks it before writing.
        unsafe { opaque_result_size("license-token", &LICENSE_TOKEN_RESULTS, async_, size) }
    }

    unsafe fn XStoreQueryLicenseTokenResult(
        &self,
        async_: *mut c_void,
        size: u64,
        result: *mut c_char,
    ) -> HRESULT {
        // SAFETY: result is the caller's buffer of `size` bytes per the GDK contract; the
        // helper null-checks it and verifies the buffer is large enough before writing.
        unsafe {
            opaque_result(
                "license-token",
                &LICENSE_TOKEN_RESULTS,
                async_,
                size,
                result,
            )
        }
    }

    /// `XStoreShowPurchaseUIAsync`'s real backing - see [`show_store_ui_async`].
    unsafe fn XStoreShowPurchaseUIAsync(
        &self,
        storeContextHandle: u64,
        storeId: *mut c_char,
        name: *mut c_char,
        extendedJsonData: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT {
        if storeContextHandle == 0 {
            return E_POINTER;
        }
        // SAFETY: storeId/name/extendedJsonData are GDK-caller-supplied pointers that are
        // null or NUL-terminated per the XStoreShowPurchaseUIAsync contract.
        let (store_id, name, extended_json_data) = unsafe {
            (
                c_string_or_empty(storeId),
                c_string_or_empty(name),
                c_string_or_empty(extendedJsonData),
            )
        };
        // SAFETY: async_ is the caller's XAsyncBlock pointer per the XAsync GDK contract;
        // show_store_ui_async itself no-ops on a null pointer via run_sync.
        unsafe {
            show_store_ui_async(
                async_,
                StoreUiKind::Purchase,
                store_id,
                name,
                extended_json_data,
                String::new(),
                Vec::new(),
            )
        }
    }

    unsafe fn XStoreShowPurchaseUIResult(&self, async_: *mut c_void) -> HRESULT {
        // SAFETY: async_ is the caller's XAsyncBlock pointer per the XAsync GDK contract.
        unsafe { show_store_ui_result(async_) }
    }

    /// `XStoreShowRateAndReviewUIAsync`'s real backing - see [`show_store_ui_async`].
    unsafe fn XStoreShowRateAndReviewUIAsync(
        &self,
        storeContextHandle: u64,
        async_: *mut c_void,
    ) -> HRESULT {
        if storeContextHandle == 0 {
            return E_POINTER;
        }
        // SAFETY: async_ is the caller's XAsyncBlock pointer per the XAsync GDK contract;
        // show_store_ui_async itself no-ops on a null pointer via run_sync.
        unsafe {
            show_store_ui_async(
                async_,
                StoreUiKind::RateAndReview,
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                Vec::new(),
            )
        }
    }

    /// `result` is `XStoreRateAndReviewResult { wasUpdated: bool }`. Nothing here can observe
    /// whether the page the human saw actually submitted a review - `false` is the honest
    /// "unknown, not a guess at success" answer, same stance as [`crate::ipc::show_store_ui`]'s
    /// `completed` flag never claiming a transaction outcome.
    unsafe fn XStoreShowRateAndReviewUIResult(
        &self,
        async_: *mut c_void,
        result: *mut c_void,
    ) -> HRESULT {
        if result.is_null() {
            return E_POINTER;
        }
        // SAFETY: async_ is the caller's XAsyncBlock pointer per the XAsync GDK contract.
        let hr = unsafe { show_store_ui_result(async_) };
        if hr.is_ok() {
            // SAFETY: result was null-checked above and is a valid
            // XStoreRateAndReviewResult out-pointer per the GDK contract.
            unsafe {
                *result.cast::<BOOLEAN>() = false.into();
            }
        }
        hr
    }

    /// `XStoreShowRedeemTokenUIAsync`'s real backing - see [`show_store_ui_async`].
    unsafe fn XStoreShowRedeemTokenUIAsync(
        &self,
        storeContextHandle: u64,
        token: *mut c_char,
        allowedStoreIds: *mut *mut c_char,
        allowedStoreIdsCount: u64,
        _disallowCsvRedemption: BOOLEAN,
        async_: *mut c_void,
    ) -> HRESULT {
        if storeContextHandle == 0 {
            return E_POINTER;
        }
        // SAFETY: token is a GDK-caller-supplied pointer that's null or NUL-terminated per
        // the XStoreShowRedeemTokenUIAsync contract.
        let token_value = unsafe { c_string_or_empty(token) };
        // SAFETY: allowedStoreIds is the caller's array of allowedStoreIdsCount
        // nul-terminated strings per the GDK contract, copied here and not retained.
        let allowed_store_ids = unsafe { read_string_array(allowedStoreIds, allowedStoreIdsCount) };
        // SAFETY: async_ is the caller's XAsyncBlock pointer per the XAsync GDK contract;
        // show_store_ui_async itself no-ops on a null pointer via run_sync.
        unsafe {
            show_store_ui_async(
                async_,
                StoreUiKind::RedeemToken,
                String::new(),
                String::new(),
                String::new(),
                token_value,
                allowed_store_ids,
            )
        }
    }

    unsafe fn XStoreShowRedeemTokenUIResult(&self, async_: *mut c_void) -> HRESULT {
        // SAFETY: async_ is the caller's XAsyncBlock pointer per the XAsync GDK contract.
        unsafe { show_store_ui_result(async_) }
    }

    /// `XStoreShowAssociatedProductsUIAsync`'s real backing - see [`show_store_ui_async`].
    unsafe fn XStoreShowAssociatedProductsUIAsync(
        &self,
        storeContextHandle: u64,
        storeId: *mut c_char,
        _productKinds: u64,
        async_: *mut c_void,
    ) -> HRESULT {
        if storeContextHandle == 0 {
            return E_POINTER;
        }
        // SAFETY: storeId is a GDK-caller-supplied pointer that's null or NUL-terminated
        // per the XStoreShowAssociatedProductsUIAsync contract.
        let store_id = unsafe { c_string_or_empty(storeId) };
        // SAFETY: async_ is the caller's XAsyncBlock pointer per the XAsync GDK contract;
        // show_store_ui_async itself no-ops on a null pointer via run_sync.
        unsafe {
            show_store_ui_async(
                async_,
                StoreUiKind::AssociatedProducts,
                store_id,
                String::new(),
                String::new(),
                String::new(),
                Vec::new(),
            )
        }
    }

    unsafe fn XStoreShowAssociatedProductsUIResult(&self, async_: *mut c_void) -> HRESULT {
        // SAFETY: async_ is the caller's XAsyncBlock pointer per the XAsync GDK contract.
        unsafe { show_store_ui_result(async_) }
    }

    /// `XStoreShowProductPageUIAsync`'s real backing - see [`show_store_ui_async`].
    unsafe fn XStoreShowProductPageUIAsync(
        &self,
        storeContextHandle: u64,
        storeId: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT {
        if storeContextHandle == 0 {
            return E_POINTER;
        }
        // SAFETY: storeId is a GDK-caller-supplied pointer that's null or NUL-terminated
        // per the XStoreShowProductPageUIAsync contract.
        let store_id = unsafe { c_string_or_empty(storeId) };
        // SAFETY: async_ is the caller's XAsyncBlock pointer per the XAsync GDK contract;
        // show_store_ui_async itself no-ops on a null pointer via run_sync.
        unsafe {
            show_store_ui_async(
                async_,
                StoreUiKind::ProductPage,
                store_id,
                String::new(),
                String::new(),
                String::new(),
                Vec::new(),
            )
        }
    }

    unsafe fn XStoreShowProductPageUIResult(&self, async_: *mut c_void) -> HRESULT {
        // SAFETY: async_ is the caller's XAsyncBlock pointer per the XAsync GDK contract.
        unsafe { show_store_ui_result(async_) }
    }

    /// `XStoreShowGiftingUIAsync`'s real backing - see [`show_store_ui_async`].
    unsafe fn XStoreShowGiftingUIAsync(
        &self,
        storeContextHandle: u64,
        storeId: *mut c_char,
        name: *mut c_char,
        extendedJsonData: *mut c_char,
        async_: *mut c_void,
    ) -> HRESULT {
        if storeContextHandle == 0 {
            return E_POINTER;
        }
        // SAFETY: storeId/name/extendedJsonData are GDK-caller-supplied pointers that are
        // null or NUL-terminated per the XStoreShowGiftingUIAsync contract.
        let (store_id, name, extended_json_data) = unsafe {
            (
                c_string_or_empty(storeId),
                c_string_or_empty(name),
                c_string_or_empty(extendedJsonData),
            )
        };
        // SAFETY: async_ is the caller's XAsyncBlock pointer per the XAsync GDK contract;
        // show_store_ui_async itself no-ops on a null pointer via run_sync.
        unsafe {
            show_store_ui_async(
                async_,
                StoreUiKind::Gifting,
                store_id,
                name,
                extended_json_data,
                String::new(),
                Vec::new(),
            )
        }
    }

    unsafe fn XStoreShowGiftingUIResult(&self, async_: *mut c_void) -> HRESULT {
        // SAFETY: async_ is the caller's XAsyncBlock pointer per the XAsync GDK contract.
        unsafe { show_store_ui_result(async_) }
    }
}

/// Keyed by the caller's `async_` pointer, same rationale and same leak-on-unread
/// tradeoff as `xuser.rs`'s `MSA_TOKEN_RESULTS` - each of these APIs returns an opaque,
/// variable-length string whose size has to be answered by a separate `*ResultSize` call
/// before `*Result` is called at all.
type OpaqueResults = Mutex<Option<HashMap<usize, Result<String, HRESULT>>>>;

static COLLECTIONS_ID_RESULTS: OpaqueResults = Mutex::new(None);
static LICENSE_TOKEN_RESULTS: OpaqueResults = Mutex::new(None);
static PURCHASE_ID_RESULTS: OpaqueResults = Mutex::new(None);

/// Files an opaque-string fetch's outcome under the caller's `async_` key and reports
/// whether it succeeded, which is all `xasync::run_sync` needs to complete the block - the
/// string itself is picked up later by the matching `*ResultSize`/`*Result` pair.
fn store_opaque_result(
    results: &OpaqueResults,
    key: usize,
    result: Result<String, HRESULT>,
) -> Result<(), HRESULT> {
    let outcome = match &result {
        Ok(_) => Ok(()),
        Err(hr) => Err(*hr),
    };
    results
        .lock()
        .expect("opaque store results poisoned")
        .get_or_insert_with(HashMap::new)
        .insert(key, result);
    outcome
}

/// The shared body of every `*ResultSize` in this file: the NUL-terminated length of the
/// string filed under `async_`.
///
/// # Safety
/// `size` must be null or a valid `usize` out-pointer, per the GDK `*ResultSize` contract.
unsafe fn opaque_result_size(
    label: &str,
    results: &OpaqueResults,
    async_: *mut c_void,
    size: *mut usize,
) -> HRESULT {
    if size.is_null() {
        return E_POINTER;
    }
    let results = results.lock().expect("opaque store results poisoned");
    match results.as_ref().and_then(|map| map.get(&(async_ as usize))) {
        Some(Ok(value)) => {
            diag!("{label} ResultSize -> {} bytes", value.len() + 1);
            // SAFETY: size was null-checked above and is a valid usize out-pointer per
            // this function's own `# Safety` contract.
            unsafe { *size = value.len() + 1 };
            S_OK
        }
        Some(Err(hr)) => {
            diag!("{label} ResultSize -> {hr:?}");
            *hr
        }
        None => {
            diag!("{label} ResultSize -> no result filed for this async block");
            E_ILLEGAL_METHOD_CALL
        }
    }
}

/// The shared body of every `*Result` in this file: copies the string filed under `async_`
/// into the caller's buffer, NUL-terminated.
///
/// # Safety
/// `result` must be null or valid for writes of `size` bytes, per the GDK `*Result`
/// contract.
unsafe fn opaque_result(
    label: &str,
    results: &OpaqueResults,
    async_: *mut c_void,
    size: u64,
    result: *mut c_char,
) -> HRESULT {
    let results = results.lock().expect("opaque store results poisoned");
    match results.as_ref().and_then(|map| map.get(&(async_ as usize))) {
        Some(Ok(value)) => {
            let bytes = value.as_bytes();
            if bytes.len() + 1 > size as usize {
                return E_NOT_SUFFICIENT_BUFFER;
            }
            if result.is_null() {
                return E_POINTER;
            }
            // The value is a bearer credential, so log only its length - enough to tell an
            // empty key (what a failed key fetch leaves behind) from a real one.
            diag!("{label} Result -> {} bytes read by the title", bytes.len());
            // SAFETY: `result` is non-null and valid for `size` bytes per this function's
            // own `# Safety` contract, and `size` covers `bytes.len() + 1` as checked above.
            unsafe {
                crate::ffi_util::write_out_bytes(bytes, result.cast::<u8>());
                *result.cast::<u8>().add(bytes.len()) = 0;
            }
            S_OK
        }
        Some(Err(hr)) => {
            diag!("{label} Result -> {hr:?}");
            *hr
        }
        None => {
            diag!("{label} Result -> no result filed for this async block");
            E_ILLEGAL_METHOD_CALL
        }
    }
}

/// Shared body of every `XStoreShow*UIAsync` entry point - builds a `StoreUiRequest` and
/// blocks on [`crate::ipc::show_store_ui`]'s webview round trip via [`xasync::run_sync`].
/// Never logs `token`/`extended_json_data`: the former can carry a redeemable code, the
/// latter is caller-defined data with no guaranteed-safe shape to print.
///
/// # Safety
/// `async_` must be null or the caller's live `XAsyncBlock*` per the XAsync GDK contract;
/// `run_sync` itself no-ops on a null pointer.
unsafe fn show_store_ui_async(
    async_: *mut c_void,
    kind: StoreUiKind,
    store_id: String,
    name: String,
    extended_json_data: String,
    token: String,
    allowed_store_ids: Vec<String>,
) -> HRESULT {
    diag!("Show{kind:?}UIAsync(store_id={store_id:?})");
    let market = crate::ipc::store_market();
    // SAFETY: async_ is the caller's XAsyncBlock pointer per this function's own `# Safety`
    // contract; run_sync itself no-ops on a null pointer.
    unsafe {
        xasync::run_sync(async_.cast(), move || {
            crate::ipc::show_store_ui(
                kind,
                &store_id,
                &name,
                &extended_json_data,
                &token,
                &allowed_store_ids,
                &market,
            )
        })
    }
}

/// Shared body of every plain `XStoreShow*UIResult` (no output beyond success/failure): did
/// the webview [`show_store_ui_async`] launched run and close normally. The `bool` itself
/// isn't surfaced to the title here - only whether the async op resolved - since none of
/// these calls have a payload to report beyond "the UI ran."
///
/// # Safety
/// `async_` must be null or the caller's live `XAsyncBlock*` per the XAsync GDK contract.
unsafe fn show_store_ui_result(async_: *mut c_void) -> HRESULT {
    if async_.is_null() {
        return E_POINTER;
    }
    let mut completed = false;
    // SAFETY: async_ was null-checked above and completed's type (bool) matches what
    // show_store_ui_async's run_sync closure stored.
    match unsafe { get_result(async_.cast(), null_mut(), &mut completed) } {
        Ok(_) => S_OK,
        Err(hr) => hr,
    }
}

/// # Safety
/// `ptr` must be null or a valid, NUL-terminated C string for the duration of this call.
unsafe fn c_string_or_empty(ptr: *mut c_char) -> String {
    if ptr.is_null() {
        String::new()
    } else {
        // SAFETY: ptr is non-null here (checked above) and NUL-terminated per
        // this function's own `# Safety` contract.
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }
}

impl IXStoreAlias1_Impl for XStoreObject_Impl {}
impl IXStoreAlias2_Impl for XStoreObject_Impl {}
impl IXStoreAlias3_Impl for XStoreObject_Impl {}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(price: [c_char; 16]) -> String {
        // SAFETY: `format_price` always leaves at least the last slot zero, so the array
        // holds a terminated string.
        unsafe { CStr::from_ptr(price.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn usd_gets_its_symbol_and_everything_else_gets_its_code() {
        assert_eq!(rendered(format_price(7.99, "USD")), "$7.99");
        assert_eq!(rendered(format_price(7.99, "EUR")), "7.99 EUR");
        assert_eq!(rendered(format_price(46.0, "JPY")), "46.00 JPY");
    }

    /// A missing currency means the catalog had no purchasable availability. Rendering `0.00`
    /// there would tell the player the item is free.
    #[test]
    fn no_currency_renders_nothing_rather_than_zero() {
        assert_eq!(rendered(format_price(0.0, "")), "");
    }

    /// The field is 16 bytes including the terminator; an amount that overruns it has to stay
    /// a readable, terminated string rather than run off the end.
    #[test]
    fn an_overlong_amount_is_truncated_and_still_terminated() {
        let price = format_price(100_000_000.0, "EUR");
        assert_eq!(price[15], 0);
        assert_eq!(rendered(price), "100000000.00 EU");
    }

    #[test]
    fn a_discounted_entry_is_on_sale_and_an_undiscounted_one_is_not() {
        let currency = CString::new("USD").unwrap();
        let entry = CatalogProductEntry {
            currency_code: "USD".to_string(),
            base_price: 7.99,
            price: 5.99,
            recurrence_price: 5.99,
            sale_end_date: 1788220800,
            ..CatalogProductEntry::default()
        };
        let price = store_price(&entry, &currency);
        assert!(price.isOnSale);
        assert_eq!(price.saleEndDate, 1788220800);
        assert_eq!(rendered(price.formattedBasePrice), "$7.99");
        assert_eq!(rendered(price.formattedPrice), "$5.99");
        assert_eq!(rendered(price.formattedRecurrencePrice), "$5.99");

        let full = CatalogProductEntry {
            base_price: 7.99,
            price: 7.99,
            ..entry
        };
        assert!(!store_price(&full, &currency).isOnSale);
    }

    #[test]
    fn every_catalog_product_kind_maps_onto_its_flag() {
        assert_eq!(product_kind_flag("Durable"), XSTORE_PRODUCT_KIND_DURABLE);
        assert_eq!(product_kind_flag("game"), XSTORE_PRODUCT_KIND_GAME);
        assert_eq!(product_kind_flag("Pass"), XSTORE_PRODUCT_KIND_PASS);
        assert_eq!(
            product_kind_flag("Consumable"),
            XSTORE_PRODUCT_KIND_CONSUMABLE
        );
        assert_eq!(
            product_kind_flag("UnmanagedConsumable"),
            XSTORE_PRODUCT_KIND_UNMANAGED_CONSUMABLE
        );
        assert_eq!(product_kind_flag("Application"), XSTORE_PRODUCT_KIND_NONE);
        assert_eq!(product_kind_flag(""), XSTORE_PRODUCT_KIND_NONE);
    }
}
