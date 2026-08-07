//! `IXGameSaveImpl`/`2`/`3` (`wine/include/xgamesave.idl`) - local, per-user, per-title save
//! container storage. Scope is "local container store first; cloud sync deferred": every
//! method that only needs a local directory tree has a real implementation; nothing here
//! talks to a cloud save service, since Xodus has none to talk to.
//!
//! Storage layout: `<game_save_root>/<xuid>/<configurationId>/containers/<containerName>/`, one
//! file per blob plus a hidden [`META_FILE`] sidecar holding the container's display name.
//! `<game_save_root>` is `ipc::game_save_root()` (a real, persistent directory `xodus-cli run`
//! published - see `xodus::ipc::ENV_GAME_SAVE_ROOT`'s docs) when available, falling back to
//! `temp_dir()` otherwise (not persistent across reboots, but still functional) - same
//! fallback stance as `com.rs`'s `IXPersistentLocalStorage::tmp_path`.
//!
//! Layout: [`storage`] holds the on-disk save-store engine (directory layout, blob/container
//! enumeration, the handle tables); [`object`] holds the `XGameSaveObject` COM object and its
//! vtables; this file re-exports the ABI types and interfaces.

pub mod object;
pub mod storage;

pub(crate) use object::*;
pub(crate) use storage::*;

use std::ffi::{CStr, c_char, c_void};

use windows_core::{GUID, HRESULT, IUnknown, interface};

use crate::com::xasync::XAsyncBlock;

type Boolean = u8;
const FALSE: Boolean = 0;

pub const CLSID_XGAMESAVE: GUID = GUID::from_u128(0x704c3f58_e629_4cc2_b197_30511b996fe2);

pub type XGameSaveProviderHandle = u64;
pub type XGameSaveContainerHandle = u64;
pub type XGameSaveUpdateHandle = u64;

#[repr(C)]
pub struct XGameSaveBlobInfo {
    pub name: *const c_char,
    pub size: u32,
}

#[repr(C)]
pub struct XGameSaveBlob {
    pub info: XGameSaveBlobInfo,
    pub data: *mut u8,
}

#[repr(C)]
pub struct XGameSaveContainerInfo {
    pub name: *const c_char,
    pub displayName: *const c_char,
    pub blobCount: u32,
    pub totalSize: u64,
    pub lastModifiedTime: i64,
    pub needsSync: Boolean,
}

type XGameSaveBlobInfoCallback =
    unsafe extern "system" fn(*const XGameSaveBlobInfo, *mut c_void) -> Boolean;
type XGameSaveContainerInfoCallback =
    unsafe extern "system" fn(*const XGameSaveContainerInfo, *mut c_void) -> Boolean;

// ---------------------------------------------------------------------------------------
// On-disk layout helpers
// ---------------------------------------------------------------------------------------

/// Sidecar filename holding a container's display name - excluded from blob enumeration/counts
/// by exact-name match, not a naming convention imposed on real blob names.
pub(crate) const META_FILE: &str = ".xgamesave_display_name";

#[interface("704c3f58-e629-4cc2-b197-30511b996fe2")]
pub unsafe trait IXGameSaveImpl: IUnknown {
    unsafe fn XGameSaveInitializeProvider(
        &self,
        requestingUser: u64,
        configurationId: *const c_char,
        syncOnDemand: Boolean,
        provider: *mut XGameSaveProviderHandle,
    ) -> HRESULT;
    unsafe fn XGameSaveInitializeProviderAsync(
        &self,
        requestingUser: u64,
        configurationId: *const c_char,
        syncOnDemand: Boolean,
        async_: *mut XAsyncBlock,
    ) -> HRESULT;
    unsafe fn XGameSaveInitializeProviderResult(
        &self,
        async_: *mut XAsyncBlock,
        provider: *mut XGameSaveProviderHandle,
    ) -> HRESULT;
    unsafe fn XGameSaveCloseProvider(&self, provider: XGameSaveProviderHandle) -> ();
    unsafe fn XGameSaveGetRemainingQuota(
        &self,
        provider: XGameSaveProviderHandle,
        remainingQuota: *mut i64,
    ) -> HRESULT;
    unsafe fn XGameSaveGetRemainingQuotaAsync(
        &self,
        provider: XGameSaveProviderHandle,
        async_: *mut XAsyncBlock,
    ) -> HRESULT;
    unsafe fn XGameSaveGetRemainingQuotaResult(
        &self,
        async_: *mut XAsyncBlock,
        remainingQuota: *mut i64,
    ) -> HRESULT;
    unsafe fn XGameSaveDeleteContainer(
        &self,
        provider: XGameSaveProviderHandle,
        containerName: *const c_char,
    ) -> HRESULT;
    unsafe fn XGameSaveDeleteContainerAsync(
        &self,
        provider: XGameSaveProviderHandle,
        containerName: *const c_char,
        async_: *mut XAsyncBlock,
    ) -> HRESULT;
    unsafe fn XGameSaveDeleteContainerResult(&self, async_: *mut XAsyncBlock) -> HRESULT;
    unsafe fn XGameSaveGetContainerInfo(
        &self,
        provider: XGameSaveProviderHandle,
        containerName: *const c_char,
        context: *mut c_void,
        callback: Option<XGameSaveContainerInfoCallback>,
    ) -> HRESULT;
    unsafe fn XGameSaveEnumerateContainerInfo(
        &self,
        provider: XGameSaveProviderHandle,
        context: *mut c_void,
        callback: Option<XGameSaveContainerInfoCallback>,
    ) -> HRESULT;
    unsafe fn XGameSaveEnumerateContainerInfoByName(
        &self,
        provider: XGameSaveProviderHandle,
        containerNamePrefix: *const c_char,
        context: *mut c_void,
        callback: Option<XGameSaveContainerInfoCallback>,
    ) -> HRESULT;
    unsafe fn XGameSaveCreateContainer(
        &self,
        provider: XGameSaveProviderHandle,
        containerName: *const c_char,
        containerContext: *mut XGameSaveContainerHandle,
    ) -> HRESULT;
    unsafe fn XGameSaveCloseContainer(&self, context: XGameSaveContainerHandle) -> ();
    unsafe fn XGameSaveEnumerateBlobInfo(
        &self,
        container: XGameSaveContainerHandle,
        context: *mut c_void,
        callback: Option<XGameSaveBlobInfoCallback>,
    ) -> HRESULT;
    unsafe fn XGameSaveEnumerateBlobInfoByName(
        &self,
        container: XGameSaveContainerHandle,
        blobNamePrefix: *const c_char,
        context: *mut c_void,
        callback: Option<XGameSaveBlobInfoCallback>,
    ) -> HRESULT;
    unsafe fn XGameSaveReadBlobData(
        &self,
        container: XGameSaveContainerHandle,
        blobNames: *const *const c_char,
        countOfBlobs: *mut u32,
        blobsSize: usize,
        blobData: *mut XGameSaveBlob,
    ) -> HRESULT;
    unsafe fn XGameSaveReadBlobDataAsync(
        &self,
        container: XGameSaveContainerHandle,
        blobNames: *const *const c_char,
        countOfBlobs: u32,
        async_: *mut XAsyncBlock,
    ) -> HRESULT;
    unsafe fn XGameSaveReadBlobDataResult(
        &self,
        async_: *mut XAsyncBlock,
        blobsSize: usize,
        blobData: *mut XGameSaveBlob,
        countOfBlobs: *mut u32,
    ) -> HRESULT;
    unsafe fn XGameSaveCreateUpdate(
        &self,
        container: XGameSaveContainerHandle,
        containerDisplayName: *const c_char,
        updateContext: *mut XGameSaveUpdateHandle,
    ) -> HRESULT;
    unsafe fn XGameSaveCloseUpdate(&self, context: XGameSaveUpdateHandle) -> ();
    unsafe fn XGameSaveSubmitBlobWrite(
        &self,
        updateContext: XGameSaveUpdateHandle,
        blobName: *const c_char,
        data: *mut u8,
        byteCount: usize,
    ) -> HRESULT;
    unsafe fn XGameSaveSubmitBlobDelete(
        &self,
        updateContext: XGameSaveUpdateHandle,
        blobName: *const c_char,
    ) -> HRESULT;
    unsafe fn XGameSaveSubmitUpdate(&self, updateContext: XGameSaveUpdateHandle) -> HRESULT;
    unsafe fn XGameSaveSubmitUpdateAsync(
        &self,
        updateContext: XGameSaveUpdateHandle,
        async_: *mut XAsyncBlock,
    ) -> HRESULT;
    unsafe fn XGameSaveSubmitUpdateResult(&self, async_: *mut XAsyncBlock) -> HRESULT;
}

/// Adds `XGameSaveFilesGetFolderWithUiAsync`/`Result`/`XGameSaveFilesGetRemainingQuota` over
/// [`IXGameSaveImpl`] - the "files" compat surface for titles using classic file-based saves
/// instead of the blob/container API. `GetFolderWithUi` has no UI to show, but unlike
/// `IXPackageImpl2::XPackageMountWithUiAsync` it does have a non-UI answer: the same
/// per-user/per-configuration directory `XGameSaveInitializeProvider` would resolve to, so it
/// completes immediately with that path rather than failing every call.
#[interface("704c3f58-e629-4cc2-b197-30511b996ee2")]
pub unsafe trait IXGameSaveImpl2: IXGameSaveImpl {
    unsafe fn XGameSaveFilesGetFolderWithUiAsync(
        &self,
        requestingUser: u64,
        configurationId: *const c_char,
        async_: *mut XAsyncBlock,
    ) -> HRESULT;
    unsafe fn XGameSaveFilesGetFolderWithUiResult(
        &self,
        async_: *mut XAsyncBlock,
        folderSize: usize,
        folderResult: *mut c_char,
    ) -> HRESULT;
    unsafe fn XGameSaveFilesGetRemainingQuota(
        &self,
        userContext: u64,
        configurationId: *const c_char,
        remainingQuota: *mut i64,
    ) -> HRESULT;
}

/// No new methods over [`IXGameSaveImpl2`] - confirmed both by the IDL and
/// `xgameruntime-docs/COM/XGameSaveImpl/IXGameSaveImpl3.md` ("Layout changes unknown, all
/// methods listed under IXGameSaveImpl2"). Default interface of the `XGameSaveImpl` coclass.
#[interface("1bfff3af-f14a-40a3-8e35-9ada906593f9")]
pub unsafe trait IXGameSaveImpl3: IXGameSaveImpl2 {}

#[cfg(test)]
// Test code exercises this crate's own already-documented internal APIs against
// synthetic, controlled inputs, not untrusted FFI callers - a per-site SAFETY comment
// here would just restate the production contract already documented at each fn.
#[allow(clippy::undocumented_unsafe_blocks)]
mod tests {
    use std::path::PathBuf;
    use std::ptr::null_mut;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::com::xasync::{self, XAsyncBlock};
    use crate::results::*;
    use crate::{InitializeApiImplEx2, UninitializeApiImpl};

    /// A fresh `(user_id, configuration_id)` pair per call, so parallel tests each get their
    /// own directory under [`provider_root`] instead of racing on a shared one.
    fn unique_scope(tag: &str) -> (u64, String) {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        let user_id = COUNTER.fetch_add(1, Ordering::Relaxed);
        (user_id, format!("{tag}_{user_id}"))
    }

    fn open_provider(tag: &str) -> (XGameSaveProviderHandle, PathBuf) {
        let (user_id, configuration_id) = unique_scope(tag);
        let handle =
            initialize_provider(Some(user_id), Some(configuration_id)).expect("initialize");
        let root = ProviderHandleTable::get(handle).unwrap();
        (handle, root)
    }

    fn new_async_block() -> XAsyncBlock {
        XAsyncBlock {
            queue: null_mut(),
            context: null_mut(),
            callback: None,
            internal: [0; size_of::<*mut c_void>() * 4],
        }
    }

    #[test]
    fn initialize_provider_creates_a_real_directory_that_close_leaves_on_disk() {
        let (handle, root) = open_provider("init");
        assert!(containers_dir(&root).is_dir());
        ProviderHandleTable::close(handle);
        // Closing the provider handle is bookkeeping only - the store itself must survive,
        // since that's the entire point of a *persistent* container store.
        assert!(containers_dir(&root).is_dir());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn initialize_provider_rejects_missing_user_or_configuration() {
        assert_eq!(
            initialize_provider(None, Some("cfg".to_string())),
            Err(E_INVALIDARG)
        );
        assert_eq!(initialize_provider(Some(1), None), Err(E_INVALIDARG));
    }

    #[test]
    fn a_zero_provider_handle_is_rejected_not_silently_accepted() {
        let mut quota = -1i64;
        let hr = unsafe { xgamesave_singleton().XGameSaveGetRemainingQuota(0, &mut quota) };
        assert_eq!(hr, E_INVALIDARG);
    }

    #[test]
    fn container_write_submit_read_round_trips_real_bytes() {
        let (provider, root) = open_provider("blobs");
        let api = xgamesave_singleton();
        let name = std::ffi::CString::new("save1").unwrap();

        let mut container = 0u64;
        let hr = unsafe { api.XGameSaveCreateContainer(provider, name.as_ptr(), &mut container) };
        assert_eq!(hr, S_OK);
        assert_ne!(container, 0);

        let display_name = std::ffi::CString::new("My Save").unwrap();
        let mut update = 0u64;
        let hr =
            unsafe { api.XGameSaveCreateUpdate(container, display_name.as_ptr(), &mut update) };
        assert_eq!(hr, S_OK);
        assert_ne!(update, 0);

        let blob_name = std::ffi::CString::new("progress.bin").unwrap();
        let mut data = b"hello save".to_vec();
        let hr = unsafe {
            api.XGameSaveSubmitBlobWrite(update, blob_name.as_ptr(), data.as_mut_ptr(), data.len())
        };
        assert_eq!(hr, S_OK);

        // Nothing hits disk until the update is actually submitted.
        assert!(!root.join("containers/save1/progress.bin").exists());
        let hr = unsafe { api.XGameSaveSubmitUpdate(update) };
        assert_eq!(hr, S_OK);
        assert!(root.join("containers/save1/progress.bin").exists());
        unsafe { api.XGameSaveCloseUpdate(update) };

        let mut info_count = 1u32;
        let mut blob = [XGameSaveBlob {
            info: XGameSaveBlobInfo {
                name: null_mut(),
                size: 0,
            },
            data: null_mut(),
        }];
        let names = [blob_name.as_ptr()];
        let hr = unsafe {
            api.XGameSaveReadBlobData(
                container,
                names.as_ptr(),
                &mut info_count,
                1,
                blob.as_mut_ptr(),
            )
        };
        assert_eq!(hr, S_OK);
        assert_eq!(info_count, 1);
        let read_bytes =
            unsafe { std::slice::from_raw_parts(blob[0].data, blob[0].info.size as usize) };
        assert_eq!(read_bytes, b"hello save");
        unsafe {
            drop(Vec::from_raw_parts(
                blob[0].data,
                blob[0].info.size as usize,
                blob[0].info.size as usize,
            ));
            drop(std::ffi::CString::from_raw(
                blob[0].info.name as *mut c_char,
            ));
        }

        let (name, display, count, size, _) = container_info(&root, "save1").unwrap();
        assert_eq!(name, "save1");
        assert_eq!(display, "My Save");
        assert_eq!(count, 1);
        assert_eq!(size, "hello save".len() as u64);

        unsafe { api.XGameSaveCloseContainer(container) };
        ok(unsafe { api.XGameSaveDeleteContainer(provider, name_cstr("save1").as_ptr()) });
        assert!(!root.join("containers/save1").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    fn name_cstr(name: &str) -> std::ffi::CString {
        std::ffi::CString::new(name).unwrap()
    }

    /// Every `XGameSave` entry point reports failure through its `HRESULT`, so a test that
    /// dropped one would keep passing while the call it was setting up did nothing.
    #[track_caller]
    fn ok(hr: HRESULT) {
        assert_eq!(hr, S_OK);
    }

    #[test]
    fn submit_blob_delete_removes_a_previously_written_blob() {
        let (provider, root) = open_provider("delete_blob");
        let api = xgamesave_singleton();
        let container_name = name_cstr("c1");
        let mut container = 0u64;
        ok(unsafe {
            api.XGameSaveCreateContainer(provider, container_name.as_ptr(), &mut container)
        });

        let blob_name = name_cstr("a.bin");
        let mut update = 0u64;
        ok(unsafe { api.XGameSaveCreateUpdate(container, null_mut(), &mut update) });
        let mut data = vec![1u8, 2, 3];
        ok(unsafe {
            api.XGameSaveSubmitBlobWrite(update, blob_name.as_ptr(), data.as_mut_ptr(), data.len())
        });
        ok(unsafe { api.XGameSaveSubmitUpdate(update) });
        unsafe { api.XGameSaveCloseUpdate(update) };
        assert!(root.join("containers/c1/a.bin").exists());

        let mut update2 = 0u64;
        ok(unsafe { api.XGameSaveCreateUpdate(container, null_mut(), &mut update2) });
        ok(unsafe { api.XGameSaveSubmitBlobDelete(update2, blob_name.as_ptr()) });
        ok(unsafe { api.XGameSaveSubmitUpdate(update2) });
        unsafe { api.XGameSaveCloseUpdate(update2) };
        assert!(!root.join("containers/c1/a.bin").exists());

        unsafe { api.XGameSaveCloseContainer(container) };
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn enumerate_blob_info_excludes_the_meta_file_and_honors_a_prefix() {
        let (provider, root) = open_provider("enum_blobs");
        let api = xgamesave_singleton();
        let container_name = name_cstr("c1");
        let mut container = 0u64;
        ok(unsafe {
            api.XGameSaveCreateContainer(provider, container_name.as_ptr(), &mut container)
        });

        let display_name = name_cstr("Display");
        let mut update = 0u64;
        ok(unsafe { api.XGameSaveCreateUpdate(container, display_name.as_ptr(), &mut update) });
        for blob in ["alpha.bin", "alpha2.bin", "beta.bin"] {
            let blob_name = name_cstr(blob);
            let mut data = vec![9u8];
            ok(unsafe {
                api.XGameSaveSubmitBlobWrite(
                    update,
                    blob_name.as_ptr(),
                    data.as_mut_ptr(),
                    data.len(),
                )
            });
        }
        ok(unsafe { api.XGameSaveSubmitUpdate(update) });
        unsafe { api.XGameSaveCloseUpdate(update) };

        static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
        SEEN.lock().unwrap().clear();
        unsafe extern "system" fn collect(
            info: *const XGameSaveBlobInfo,
            _context: *mut c_void,
        ) -> Boolean {
            let name = unsafe { CStr::from_ptr((*info).name) }
                .to_string_lossy()
                .into_owned();
            SEEN.lock().unwrap().push(name);
            1
        }

        let hr = unsafe { api.XGameSaveEnumerateBlobInfo(container, null_mut(), Some(collect)) };
        assert_eq!(hr, S_OK);
        let mut seen = SEEN.lock().unwrap().clone();
        seen.sort();
        // The display-name sidecar must never show up as a blob.
        assert_eq!(seen, vec!["alpha.bin", "alpha2.bin", "beta.bin"]);

        SEEN.lock().unwrap().clear();
        let prefix = name_cstr("alpha");
        let hr = unsafe {
            api.XGameSaveEnumerateBlobInfoByName(
                container,
                prefix.as_ptr(),
                null_mut(),
                Some(collect),
            )
        };
        assert_eq!(hr, S_OK);
        let mut seen = SEEN.lock().unwrap().clone();
        seen.sort();
        assert_eq!(seen, vec!["alpha.bin", "alpha2.bin"]);

        unsafe { api.XGameSaveCloseContainer(container) };
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn remaining_quota_shrinks_as_blobs_are_written() {
        let (provider, root) = open_provider("quota");
        let api = xgamesave_singleton();

        let mut before = -1i64;
        ok(unsafe { api.XGameSaveGetRemainingQuota(provider, &mut before) });
        assert!(before > 0);

        let container_name = name_cstr("c1");
        let mut container = 0u64;
        ok(unsafe {
            api.XGameSaveCreateContainer(provider, container_name.as_ptr(), &mut container)
        });
        let blob_name = name_cstr("big.bin");
        let mut update = 0u64;
        ok(unsafe { api.XGameSaveCreateUpdate(container, null_mut(), &mut update) });
        let mut data = vec![0u8; 4096];
        ok(unsafe {
            api.XGameSaveSubmitBlobWrite(update, blob_name.as_ptr(), data.as_mut_ptr(), data.len())
        });
        ok(unsafe { api.XGameSaveSubmitUpdate(update) });
        unsafe { api.XGameSaveCloseUpdate(update) };

        let mut after = -1i64;
        ok(unsafe { api.XGameSaveGetRemainingQuota(provider, &mut after) });
        assert_eq!(before - after, 4096);

        unsafe { api.XGameSaveCloseContainer(container) };
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn delete_container_is_idempotent_for_a_missing_container() {
        let (provider, root) = open_provider("delete_missing");
        let api = xgamesave_singleton();
        let missing = name_cstr("never-existed");
        let hr = unsafe { api.XGameSaveDeleteContainer(provider, missing.as_ptr()) };
        assert_eq!(hr, S_OK);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// End-to-end through the real `XAsync` machinery (not the internal helpers directly),
    /// matching `com.rs`'s `query_game_license_async_blocks_via_xasync` - proves the
    /// `Fn()`-closure/`get_result` plumbing this module relies on for every `*Async` method
    /// actually works, not just the synchronous methods it wraps.
    #[test]
    fn initialize_provider_async_result_round_trips_through_xasync() {
        let init_hr = InitializeApiImplEx2(2604, 100000, 10, null_mut());
        assert_eq!(init_hr, S_OK);

        let api = xgamesave_singleton();
        let (user_id, configuration_id) = unique_scope("async_init");
        let configuration_id_c = name_cstr(&configuration_id);
        let user_handle = crate::com::xuser::create_test_user_handle(user_id);

        let mut async_block = new_async_block();
        let hr = unsafe {
            api.XGameSaveInitializeProviderAsync(
                user_handle,
                configuration_id_c.as_ptr(),
                FALSE,
                &mut async_block,
            )
        };
        assert_eq!(hr, S_OK);
        assert_eq!(
            unsafe { xasync::get_status(&mut async_block, true) },
            Ok(())
        );

        let mut provider = 0u64;
        let hr = unsafe { api.XGameSaveInitializeProviderResult(&mut async_block, &mut provider) };
        assert_eq!(hr, S_OK);
        assert_ne!(provider, 0);

        let root = ProviderHandleTable::get(provider).unwrap();
        assert!(containers_dir(&root).is_dir());
        ProviderHandleTable::close(provider);
        // `XUserCloseHandle` is a private trait method outside `xuser`'s own module - this
        // test-only user handle is deliberately leaked rather than closed.
        let _ = user_handle;
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(UninitializeApiImpl(), S_OK);
    }
}
