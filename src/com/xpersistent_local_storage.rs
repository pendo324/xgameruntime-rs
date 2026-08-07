use super::{XPackageMountHandle, XPackageMountHandleTable};
use crate::E_FAIL;
use crate::com::xasync::{self, XAsyncBlock, get_result};
use crate::results::*;
use std::ffi::{CStr, c_char};
use std::ptr::null_mut;
use windows_core::{GUID, HRESULT, IUnknown, implement, interface};

pub const CLSID_XPERSISTENT_LOCAL_STORAGE: GUID =
    GUID::from_u128(0xf4faf4d4_2d04_4fce_b3e0_474a713a3e84);

#[repr(C)]
pub struct XPersistentLocalStorageSpaceInfo {
    availableFreeBytes: u64,
    totalFreeBytes: u64,
    usedBytes: u64,
    totalBytes: u64,
}

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
    pub(crate) tmp_path: String,
}

impl IXPersistentLocalStorage_Impl for XPersistentLocalStorage_Impl {
    unsafe fn x_persistent_local_storage_get_path_size(&self, path_size: *mut usize) {
        // SAFETY: `path_size` is an out-pointer per XPersistentLocalStorageGetPathSize's GDK
        // contract; the caller is required to pass a valid pointer.
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
            // SAFETY: `index < len <= path_size - 1`, and `path` is a caller-supplied buffer
            // of `path_size` bytes per the GDK contract.
            unsafe {
                *path.add(index) = byte as c_char;
            }
        }
        if path_size != 0 {
            // SAFETY: `path_size != 0` was checked above, and `len <= path_size - 1`, so
            // `path.add(len)` is in bounds.
            unsafe {
                *path.add(len) = 0;
            }
        }
        // SAFETY: `path_used` is an out-pointer per XPersistentLocalStorageGetPath's GDK
        // contract; the caller is required to pass a valid pointer.
        unsafe {
            *path_used = len + 1;
        }
    }

    /// Real numbers when `xodus-cli run` found a `<PersistentLocalStorage>` element in
    /// `MicrosoftGame.config` (`ipc::persistent_local_storage_space`); the old placeholder
    /// otherwise (not running under `xodus-cli run`, or the title didn't declare one) - a
    /// "can't tell" fallback, not a claim this title has no storage need.
    unsafe fn x_persistent_local_storage_get_space_info(
        &self,
        info: *mut XPersistentLocalStorageSpaceInfo,
    ) {
        let (total_bytes, growable_to_bytes) = crate::ipc::persistent_local_storage_space()
            .unwrap_or((1024 * 1024 * 1024, 2 * 1024 * 1024 * 1024));
        // SAFETY: `info` is an out-pointer per XPersistentLocalStorageGetSpaceInfo's GDK
        // contract; the caller is required to pass a valid pointer.
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
    /// immediately as an approval - the alternative (failing every call) would break titles
    /// that gate on this succeeding before writing to persistent local storage, for a prompt
    /// Xodus has no way to present anyway.
    unsafe fn x_persistent_local_storage_prompt_user_for_space_async(
        &self,
        _requested_bytes: u64,
        async_block: *mut XAsyncBlock,
    ) {
        // SAFETY: `async_block` is the caller-supplied `XAsyncBlock` for this async op,
        // matching `run_sync`'s own pointer contract.
        let _ = unsafe { xasync::run_sync(async_block, || Ok(())) };
    }

    unsafe fn x_persistent_local_storage_prompt_user_for_space_result(
        &self,
        async_block: *mut XAsyncBlock,
    ) {
        // SAFETY: `async_block` is the same `XAsyncBlock` the caller supplied to the matching
        // `_async` call, matching `get_result`'s pointer contract.
        let _ = unsafe { get_result::<()>(async_block, null_mut(), &mut ()) };
    }

    /// `packageIdentifier` is a `PackageFamilyName`, matching
    /// `XPackageGetCurrentProcessPackageIdentifier`, which is backed by Win32's own
    /// `GetCurrentPackageFamilyName`. Two cases are answerable:
    /// self-mount (the running title's own PFN, `ENV_PACKAGE_FAMILY_NAME`) maps to this
    /// title's own persistent-storage root, and any other PFN is resolved to a `StoreId` via
    /// `xodus-service` and checked against this title's own declared `RelatedProducts`
    /// (`MicrosoftGame.config`, published through `ENV_RELATED_PRODUCTS`). Mounting storage
    /// for a product that's neither has no real meaning to grant, so that case fails
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
        // SAFETY: `package_identifier` was checked non-null above; the GDK contract requires
        // it be a NUL-terminated C string.
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
        // SAFETY: `mount_handle` was checked non-null above.
        unsafe {
            *mount_handle = handle;
        }
        S_OK
    }
}
