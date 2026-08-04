//! `IXGameSaveImpl`/`2`/`3` (`wine/include/xgamesave.idl`) - local, per-user, per-title save
//! container storage. Scope is "local container store first; cloud sync deferred" (PLAN.md):
//! every method that only needs a local directory tree has a real implementation; nothing here
//! talks to a cloud save service, since Xodus has none to talk to.
//!
//! Storage layout: `<game_save_root>/<xuid>/<configurationId>/containers/<containerName>/`, one
//! file per blob plus a hidden [`META_FILE`] sidecar holding the container's display name.
//! `<game_save_root>` is `ipc::game_save_root()` (a real, persistent directory `xodus-cli run`
//! published - see `xodus::ipc::ENV_GAME_SAVE_ROOT`'s docs) when available, falling back to
//! `temp_dir()` otherwise (not persistent across reboots, but still functional) - same
//! honest-fallback stance as `com.rs`'s `IXPersistentLocalStorage::tmp_path`.

use std::env::temp_dir;
use std::ffi::{CStr, c_char, c_void};
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::sync::OnceLock;
use std::time::UNIX_EPOCH;

use windows_core::{GUID, HRESULT, IUnknown, implement, interface};

use crate::results::*;
use crate::xasync::{XAsyncBlock, get_result};
use crate::{E_FAIL, xasync};

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
const META_FILE: &str = ".xgamesave_display_name";

fn provider_root(user_id: u64, configuration_id: &str) -> PathBuf {
    let base = crate::ipc::game_save_root()
        .map(PathBuf::from)
        .unwrap_or_else(|| temp_dir().join("xodus_gamesaves"));
    base.join(user_id.to_string()).join(configuration_id)
}

fn containers_dir(provider_root: &Path) -> PathBuf {
    provider_root.join("containers")
}

fn container_dir(provider_root: &Path, name: &str) -> PathBuf {
    containers_dir(provider_root).join(name)
}

fn read_display_name(container_root: &Path) -> String {
    std::fs::read_to_string(container_root.join(META_FILE)).unwrap_or_default()
}

fn write_display_name(container_root: &Path, display_name: &str) {
    let _ = std::fs::write(container_root.join(META_FILE), display_name);
}

/// Blob files under a container directory - everything except [`META_FILE`].
fn blob_files(container_root: &Path) -> Vec<std::fs::DirEntry> {
    std::fs::read_dir(container_root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_name() != META_FILE && entry.path().is_file())
        .collect()
}

fn container_total_size(container_root: &Path) -> u64 {
    blob_files(container_root)
        .iter()
        .filter_map(|entry| entry.metadata().ok())
        .map(|metadata| metadata.len())
        .sum()
}

fn container_last_modified(container_root: &Path) -> i64 {
    blob_files(container_root)
        .iter()
        .filter_map(|entry| entry.metadata().ok()?.modified().ok())
        .max()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// Reads a nul-terminated C string, if `ptr` is non-null.
unsafe fn read_cstr(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .ok()
        .map(String::from)
}

// ---------------------------------------------------------------------------------------
// Handle tables - same leaked-`Box` scheme as `com.rs`'s `XPackageMountHandleTable`.
// ---------------------------------------------------------------------------------------

struct ProviderHandleTable;

impl ProviderHandleTable {
    fn create(root: PathBuf) -> u64 {
        Box::into_raw(Box::new(root)) as u64
    }

    /// # Safety
    /// `handle` must be zero or a handle from [`Self::create`] that has not been closed.
    unsafe fn get<'a>(handle: u64) -> Option<&'a PathBuf> {
        if handle == 0 {
            return None;
        }
        Some(unsafe { &*(handle as *const PathBuf) })
    }

    /// # Safety
    /// `handle` must be an open handle from [`Self::create`]; it is invalid afterwards.
    unsafe fn close(handle: u64) {
        if handle == 0 {
            return;
        }
        drop(unsafe { Box::from_raw(handle as *mut PathBuf) });
    }
}

struct ContainerState {
    root: PathBuf,
}

struct ContainerHandleTable;

impl ContainerHandleTable {
    fn create(state: ContainerState) -> u64 {
        Box::into_raw(Box::new(state)) as u64
    }

    /// # Safety
    /// `handle` must be zero or a handle from [`Self::create`] that has not been closed.
    unsafe fn get<'a>(handle: u64) -> Option<&'a ContainerState> {
        if handle == 0 {
            return None;
        }
        Some(unsafe { &*(handle as *const ContainerState) })
    }

    /// # Safety
    /// `handle` must be an open handle from [`Self::create`]; it is invalid afterwards.
    unsafe fn close(handle: u64) {
        if handle == 0 {
            return;
        }
        drop(unsafe { Box::from_raw(handle as *mut ContainerState) });
    }
}

/// A pending, uncommitted batch of blob writes/deletes - real writes only land on disk when
/// `XGameSaveSubmitUpdate(Async)` runs, matching the real API's transactional shape.
struct UpdateState {
    container_root: PathBuf,
    display_name: String,
    writes: Vec<(String, Vec<u8>)>,
    deletes: Vec<String>,
}

struct UpdateHandleTable;

impl UpdateHandleTable {
    fn create(state: UpdateState) -> u64 {
        Box::into_raw(Box::new(state)) as u64
    }

    /// # Safety
    /// `handle` must be zero or a handle from [`Self::create`] that has not been closed.
    unsafe fn get<'a>(handle: u64) -> Option<&'a mut UpdateState> {
        if handle == 0 {
            return None;
        }
        Some(unsafe { &mut *(handle as *mut UpdateState) })
    }

    /// # Safety
    /// `handle` must be an open handle from [`Self::create`]; it is invalid afterwards.
    unsafe fn close(handle: u64) {
        if handle == 0 {
            return;
        }
        drop(unsafe { Box::from_raw(handle as *mut UpdateState) });
    }
}

// ---------------------------------------------------------------------------------------
// Shared logic behind the sync/Async/Result trios
// ---------------------------------------------------------------------------------------

fn initialize_provider(
    user_id: Option<u64>,
    configuration_id: Option<String>,
) -> Result<XGameSaveProviderHandle, HRESULT> {
    let configuration_id = configuration_id.ok_or(E_INVALIDARG)?;
    let user_id = user_id.ok_or(E_INVALIDARG)?;
    let root = provider_root(user_id, &configuration_id);
    std::fs::create_dir_all(containers_dir(&root)).map_err(|_| E_FAIL)?;
    Ok(ProviderHandleTable::create(root))
}

/// No real quota concept is derivable (`MicrosoftGame.config` has no `XGameSave` quota
/// schema the way it does for `PersistentLocalStorage`'s `SizeMB`), so this reports a fixed
/// 1 GiB budget minus what's actually on disk - the same honest-placeholder stance as
/// `IXPersistentLocalStorage::GetSpaceInfo`'s fallback numbers, not a real per-title limit.
const PLACEHOLDER_QUOTA_BYTES: i64 = 1024 * 1024 * 1024;

fn remaining_quota(provider_root: &Path) -> i64 {
    let used: u64 = std::fs::read_dir(containers_dir(provider_root))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| container_total_size(&entry.path()))
        .sum();
    PLACEHOLDER_QUOTA_BYTES.saturating_sub(used as i64).max(0)
}

fn delete_container(provider_root: &Path, container_name: Option<String>) -> Result<(), HRESULT> {
    let container_name = container_name.ok_or(E_INVALIDARG)?;
    let dir = container_dir(provider_root, &container_name);
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(E_FAIL),
    }
}

/// Builds a real [`XGameSaveContainerInfo`] for `name` if its directory exists.
fn container_info(provider_root: &Path, name: &str) -> Option<(String, String, u32, u64, i64)> {
    let dir = container_dir(provider_root, name);
    if !dir.is_dir() {
        return None;
    }
    let display_name = read_display_name(&dir);
    let blobs = blob_files(&dir);
    let blob_count = blobs.len() as u32;
    let total_size = container_total_size(&dir);
    let last_modified = container_last_modified(&dir);
    Some((
        name.to_string(),
        display_name,
        blob_count,
        total_size,
        last_modified,
    ))
}

/// Invokes `callback` once per container directory under `provider_root`, honoring the early-stop
/// (`FALSE` return) convention shared with every other enumeration callback in this crate.
/// Restricted to `prefix` when given (`EnumerateContainerInfoByName`).
fn enumerate_containers(
    provider_root: &Path,
    prefix: Option<&str>,
    context: *mut c_void,
    callback: XGameSaveContainerInfoCallback,
) {
    let Ok(entries) = std::fs::read_dir(containers_dir(provider_root)) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(String::from) else {
            continue;
        };
        if let Some(prefix) = prefix
            && !name.starts_with(prefix)
        {
            continue;
        }
        let Some((name, display_name, blob_count, total_size, last_modified)) =
            container_info(provider_root, &name)
        else {
            continue;
        };
        let name_c = std::ffi::CString::new(name).unwrap_or_default();
        let display_name_c = std::ffi::CString::new(display_name).unwrap_or_default();
        let info = XGameSaveContainerInfo {
            name: name_c.as_ptr(),
            displayName: display_name_c.as_ptr(),
            blobCount: blob_count,
            totalSize: total_size,
            lastModifiedTime: last_modified,
            needsSync: FALSE,
        };
        let keep_going = unsafe { callback(&info, context) };
        if keep_going == FALSE {
            break;
        }
    }
}

fn enumerate_blobs(
    container_root: &Path,
    prefix: Option<&str>,
    context: *mut c_void,
    callback: XGameSaveBlobInfoCallback,
) {
    for entry in blob_files(container_root) {
        let Some(name) = entry.file_name().to_str().map(String::from) else {
            continue;
        };
        if let Some(prefix) = prefix
            && !name.starts_with(prefix)
        {
            continue;
        }
        let size = entry.metadata().map(|m| m.len() as u32).unwrap_or(0);
        let name_c = std::ffi::CString::new(name).unwrap_or_default();
        let info = XGameSaveBlobInfo {
            name: name_c.as_ptr(),
            size,
        };
        let keep_going = unsafe { callback(&info, context) };
        if keep_going == FALSE {
            break;
        }
    }
}

// ---------------------------------------------------------------------------------------
// IXGameSaveImpl / 2 / 3
// ---------------------------------------------------------------------------------------

/// `wine/include/xgamesave.idl`'s `IXGameSaveImpl` - local container-store methods (provider,
/// container, blob, update lifecycle) are real, backed by [`provider_root`]'s directory tree.
/// See this module's docs for the scope rationale ("local first; cloud sync deferred").
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
/// `IXPackageImpl2::XPackageMountWithUiAsync` it does have an honest non-UI answer: the same
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

#[implement(IXGameSaveImpl, IXGameSaveImpl2, IXGameSaveImpl3)]
pub struct XGameSaveObject;

impl IXGameSaveImpl_Impl for XGameSaveObject_Impl {
    unsafe fn XGameSaveInitializeProvider(
        &self,
        requestingUser: u64,
        configurationId: *const c_char,
        syncOnDemand: Boolean,
        provider: *mut XGameSaveProviderHandle,
    ) -> HRESULT {
        let _ = syncOnDemand; // no cloud sync to schedule - local store only
        if provider.is_null() {
            return E_POINTER;
        }
        let configuration_id = unsafe { read_cstr(configurationId) };
        let user_id = unsafe { crate::xuser::user_id_for_handle(requestingUser) };
        match initialize_provider(user_id, configuration_id) {
            Ok(handle) => {
                unsafe { *provider = handle };
                S_OK
            }
            Err(hr) => hr,
        }
    }

    unsafe fn XGameSaveInitializeProviderAsync(
        &self,
        requestingUser: u64,
        configurationId: *const c_char,
        syncOnDemand: Boolean,
        async_: *mut XAsyncBlock,
    ) -> HRESULT {
        let _ = syncOnDemand;
        let configuration_id = unsafe { read_cstr(configurationId) };
        let user_id = unsafe { crate::xuser::user_id_for_handle(requestingUser) };
        unsafe {
            xasync::run_sync(async_, move || {
                initialize_provider(user_id, configuration_id.clone())
            })
        }
    }

    unsafe fn XGameSaveInitializeProviderResult(
        &self,
        async_: *mut XAsyncBlock,
        provider: *mut XGameSaveProviderHandle,
    ) -> HRESULT {
        if provider.is_null() {
            return E_POINTER;
        }
        match unsafe { get_result::<XGameSaveProviderHandle>(async_, null_mut(), provider) } {
            Ok(()) => S_OK,
            Err(hr) => hr,
        }
    }

    unsafe fn XGameSaveCloseProvider(&self, provider: XGameSaveProviderHandle) {
        unsafe { ProviderHandleTable::close(provider) };
    }

    unsafe fn XGameSaveGetRemainingQuota(
        &self,
        provider: XGameSaveProviderHandle,
        remainingQuota: *mut i64,
    ) -> HRESULT {
        if remainingQuota.is_null() {
            return E_POINTER;
        }
        let Some(root) = (unsafe { ProviderHandleTable::get(provider) }) else {
            return E_INVALIDARG;
        };
        unsafe { *remainingQuota = remaining_quota(root) };
        S_OK
    }

    unsafe fn XGameSaveGetRemainingQuotaAsync(
        &self,
        provider: XGameSaveProviderHandle,
        async_: *mut XAsyncBlock,
    ) -> HRESULT {
        let root = unsafe { ProviderHandleTable::get(provider) }.cloned();
        unsafe {
            xasync::run_sync(async_, move || {
                root.as_deref().map(remaining_quota).ok_or(E_INVALIDARG)
            })
        }
    }

    unsafe fn XGameSaveGetRemainingQuotaResult(
        &self,
        async_: *mut XAsyncBlock,
        remainingQuota: *mut i64,
    ) -> HRESULT {
        if remainingQuota.is_null() {
            return E_POINTER;
        }
        match unsafe { get_result::<i64>(async_, null_mut(), remainingQuota) } {
            Ok(()) => S_OK,
            Err(hr) => hr,
        }
    }

    unsafe fn XGameSaveDeleteContainer(
        &self,
        provider: XGameSaveProviderHandle,
        containerName: *const c_char,
    ) -> HRESULT {
        let Some(root) = (unsafe { ProviderHandleTable::get(provider) }) else {
            return E_INVALIDARG;
        };
        let container_name = unsafe { read_cstr(containerName) };
        match delete_container(root, container_name) {
            Ok(()) => S_OK,
            Err(hr) => hr,
        }
    }

    unsafe fn XGameSaveDeleteContainerAsync(
        &self,
        provider: XGameSaveProviderHandle,
        containerName: *const c_char,
        async_: *mut XAsyncBlock,
    ) -> HRESULT {
        let root = unsafe { ProviderHandleTable::get(provider) }.cloned();
        let container_name = unsafe { read_cstr(containerName) };
        unsafe {
            xasync::run_sync(async_, move || {
                let root = root.clone().ok_or(E_INVALIDARG)?;
                delete_container(&root, container_name.clone())
            })
        }
    }

    unsafe fn XGameSaveDeleteContainerResult(&self, async_: *mut XAsyncBlock) -> HRESULT {
        match unsafe { get_result::<()>(async_, null_mut(), &mut ()) } {
            Ok(()) => S_OK,
            Err(hr) => hr,
        }
    }

    unsafe fn XGameSaveGetContainerInfo(
        &self,
        provider: XGameSaveProviderHandle,
        containerName: *const c_char,
        context: *mut c_void,
        callback: Option<XGameSaveContainerInfoCallback>,
    ) -> HRESULT {
        let Some(root) = (unsafe { ProviderHandleTable::get(provider) }) else {
            return E_INVALIDARG;
        };
        let Some(callback) = callback else {
            return E_POINTER;
        };
        let Some(name) = (unsafe { read_cstr(containerName) }) else {
            return E_INVALIDARG;
        };
        // A container that doesn't exist yet has nothing to report - the callback is simply
        // never invoked, same as an empty result from an enumeration, not an error.
        if let Some((name, display_name, blob_count, total_size, last_modified)) =
            container_info(root, &name)
        {
            let name_c = std::ffi::CString::new(name).unwrap_or_default();
            let display_name_c = std::ffi::CString::new(display_name).unwrap_or_default();
            let info = XGameSaveContainerInfo {
                name: name_c.as_ptr(),
                displayName: display_name_c.as_ptr(),
                blobCount: blob_count,
                totalSize: total_size,
                lastModifiedTime: last_modified,
                needsSync: FALSE,
            };
            unsafe { callback(&info, context) };
        }
        S_OK
    }

    unsafe fn XGameSaveEnumerateContainerInfo(
        &self,
        provider: XGameSaveProviderHandle,
        context: *mut c_void,
        callback: Option<XGameSaveContainerInfoCallback>,
    ) -> HRESULT {
        let Some(root) = (unsafe { ProviderHandleTable::get(provider) }) else {
            return E_INVALIDARG;
        };
        let Some(callback) = callback else {
            return E_POINTER;
        };
        enumerate_containers(root, None, context, callback);
        S_OK
    }

    unsafe fn XGameSaveEnumerateContainerInfoByName(
        &self,
        provider: XGameSaveProviderHandle,
        containerNamePrefix: *const c_char,
        context: *mut c_void,
        callback: Option<XGameSaveContainerInfoCallback>,
    ) -> HRESULT {
        let Some(root) = (unsafe { ProviderHandleTable::get(provider) }) else {
            return E_INVALIDARG;
        };
        let Some(callback) = callback else {
            return E_POINTER;
        };
        let prefix = unsafe { read_cstr(containerNamePrefix) };
        enumerate_containers(root, prefix.as_deref(), context, callback);
        S_OK
    }

    unsafe fn XGameSaveCreateContainer(
        &self,
        provider: XGameSaveProviderHandle,
        containerName: *const c_char,
        containerContext: *mut XGameSaveContainerHandle,
    ) -> HRESULT {
        if containerContext.is_null() {
            return E_POINTER;
        }
        let Some(root) = (unsafe { ProviderHandleTable::get(provider) }) else {
            return E_INVALIDARG;
        };
        let Some(name) = (unsafe { read_cstr(containerName) }) else {
            return E_INVALIDARG;
        };
        let dir = container_dir(root, &name);
        if std::fs::create_dir_all(&dir).is_err() {
            return E_FAIL;
        }
        let handle = ContainerHandleTable::create(ContainerState { root: dir });
        unsafe { *containerContext = handle };
        S_OK
    }

    unsafe fn XGameSaveCloseContainer(&self, context: XGameSaveContainerHandle) {
        unsafe { ContainerHandleTable::close(context) };
    }

    unsafe fn XGameSaveEnumerateBlobInfo(
        &self,
        container: XGameSaveContainerHandle,
        context: *mut c_void,
        callback: Option<XGameSaveBlobInfoCallback>,
    ) -> HRESULT {
        let Some(state) = (unsafe { ContainerHandleTable::get(container) }) else {
            return E_INVALIDARG;
        };
        let Some(callback) = callback else {
            return E_POINTER;
        };
        enumerate_blobs(&state.root, None, context, callback);
        S_OK
    }

    unsafe fn XGameSaveEnumerateBlobInfoByName(
        &self,
        container: XGameSaveContainerHandle,
        blobNamePrefix: *const c_char,
        context: *mut c_void,
        callback: Option<XGameSaveBlobInfoCallback>,
    ) -> HRESULT {
        let Some(state) = (unsafe { ContainerHandleTable::get(container) }) else {
            return E_INVALIDARG;
        };
        let Some(callback) = callback else {
            return E_POINTER;
        };
        let prefix = unsafe { read_cstr(blobNamePrefix) };
        enumerate_blobs(&state.root, prefix.as_deref(), context, callback);
        S_OK
    }

    /// Real read, but only ever for `blobNames != NULL`: reading "every blob" (the documented
    /// `blobNames == NULL` meaning) would require the caller to have pre-sized `blobData` from
    /// an enumeration pass first, which this trivial in-process implementation has no way to
    /// coordinate across two separate calls without a second stateful handle - titles that need
    /// that shape should enumerate then read named blobs, which this does support fully.
    unsafe fn XGameSaveReadBlobData(
        &self,
        container: XGameSaveContainerHandle,
        blobNames: *const *const c_char,
        countOfBlobs: *mut u32,
        blobsSize: usize,
        blobData: *mut XGameSaveBlob,
    ) -> HRESULT {
        if countOfBlobs.is_null() {
            return E_POINTER;
        }
        let Some(state) = (unsafe { ContainerHandleTable::get(container) }) else {
            return E_INVALIDARG;
        };
        let requested = unsafe { *countOfBlobs } as usize;
        if blobNames.is_null() || requested == 0 {
            return E_INVALIDARG;
        }
        if blobsSize < requested || blobData.is_null() {
            return E_NOT_SUFFICIENT_BUFFER;
        }

        for index in 0..requested {
            let name_ptr = unsafe { *blobNames.add(index) };
            let Some(name) = (unsafe { read_cstr(name_ptr) }) else {
                return E_INVALIDARG;
            };
            let Ok(mut data) = std::fs::read(state.root.join(&name)) else {
                return E_FAIL;
            };
            // `shrink_to_fit` first: `Vec::from_raw_parts` on the caller's side needs
            // capacity == len, but `fs::read`'s buffer often over-allocates.
            data.shrink_to_fit();
            let size = data.len() as u32;
            let leaked = data.as_mut_ptr();
            std::mem::forget(data);
            let name_c = std::ffi::CString::new(name).unwrap_or_default();
            unsafe {
                *blobData.add(index) = XGameSaveBlob {
                    info: XGameSaveBlobInfo {
                        name: name_c.into_raw(),
                        size,
                    },
                    data: leaked,
                };
            }
        }
        unsafe { *countOfBlobs = requested as u32 };
        S_OK
    }

    unsafe fn XGameSaveReadBlobDataAsync(
        &self,
        container: XGameSaveContainerHandle,
        blobNames: *const *const c_char,
        countOfBlobs: u32,
        async_: *mut XAsyncBlock,
    ) -> HRESULT {
        // The real payload copy happens synchronously in `Result` (it needs the caller's own
        // `blobsSize`/`blobData` buffer, not known yet here) - this just validates eagerly and
        // stashes the request, same shape as `XStoreQueryGameLicenseAsync` stashing nothing but
        // succeeding unconditionally when there's no real async work to do.
        let names: Vec<Option<String>> = (0..countOfBlobs)
            .map(|index| unsafe { read_cstr(*blobNames.add(index as usize)) })
            .collect();
        unsafe {
            xasync::run_sync(async_, move || {
                if names.iter().any(Option::is_none) {
                    return Err(E_INVALIDARG);
                }
                Ok((container, names.clone()))
            })
        }
    }

    unsafe fn XGameSaveReadBlobDataResult(
        &self,
        async_: *mut XAsyncBlock,
        blobsSize: usize,
        blobData: *mut XGameSaveBlob,
        countOfBlobs: *mut u32,
    ) -> HRESULT {
        if countOfBlobs.is_null() {
            return E_POINTER;
        }
        let mut payload: (XGameSaveContainerHandle, Vec<Option<String>>) = (0, Vec::new());
        if let Err(hr) = unsafe { get_result(async_, null_mut(), &mut payload) } {
            return hr;
        }
        let (container, names) = payload;
        let Some(state) = (unsafe { ContainerHandleTable::get(container) }) else {
            return E_INVALIDARG;
        };
        if blobsSize < names.len() || blobData.is_null() {
            return E_NOT_SUFFICIENT_BUFFER;
        }
        for (index, name) in names.into_iter().enumerate() {
            let Some(name) = name else {
                return E_INVALIDARG;
            };
            let Ok(mut data) = std::fs::read(state.root.join(&name)) else {
                return E_FAIL;
            };
            // `shrink_to_fit` first: `Vec::from_raw_parts` on the caller's side needs
            // capacity == len, but `fs::read`'s buffer often over-allocates.
            data.shrink_to_fit();
            let size = data.len() as u32;
            let leaked = data.as_mut_ptr();
            std::mem::forget(data);
            let name_c = std::ffi::CString::new(name).unwrap_or_default();
            unsafe {
                *blobData.add(index) = XGameSaveBlob {
                    info: XGameSaveBlobInfo {
                        name: name_c.into_raw(),
                        size,
                    },
                    data: leaked,
                };
            }
            unsafe { *countOfBlobs = (index + 1) as u32 };
        }
        S_OK
    }

    unsafe fn XGameSaveCreateUpdate(
        &self,
        container: XGameSaveContainerHandle,
        containerDisplayName: *const c_char,
        updateContext: *mut XGameSaveUpdateHandle,
    ) -> HRESULT {
        if updateContext.is_null() {
            return E_POINTER;
        }
        let Some(state) = (unsafe { ContainerHandleTable::get(container) }) else {
            return E_INVALIDARG;
        };
        let display_name = unsafe { read_cstr(containerDisplayName) }.unwrap_or_default();
        let handle = UpdateHandleTable::create(UpdateState {
            container_root: state.root.clone(),
            display_name,
            writes: Vec::new(),
            deletes: Vec::new(),
        });
        unsafe { *updateContext = handle };
        S_OK
    }

    unsafe fn XGameSaveCloseUpdate(&self, context: XGameSaveUpdateHandle) {
        // An update that was never submitted is simply discarded - no partial writes ever
        // touched disk, since `SubmitBlobWrite`/`Delete` only buffer in `UpdateState`.
        unsafe { UpdateHandleTable::close(context) };
    }

    unsafe fn XGameSaveSubmitBlobWrite(
        &self,
        updateContext: XGameSaveUpdateHandle,
        blobName: *const c_char,
        data: *mut u8,
        byteCount: usize,
    ) -> HRESULT {
        let Some(state) = (unsafe { UpdateHandleTable::get(updateContext) }) else {
            return E_INVALIDARG;
        };
        let Some(name) = (unsafe { read_cstr(blobName) }) else {
            return E_INVALIDARG;
        };
        if data.is_null() && byteCount != 0 {
            return E_POINTER;
        }
        let bytes = if byteCount == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(data, byteCount) }.to_vec()
        };
        state.deletes.retain(|deleted| deleted != &name);
        state.writes.retain(|(existing, _)| existing != &name);
        state.writes.push((name, bytes));
        S_OK
    }

    unsafe fn XGameSaveSubmitBlobDelete(
        &self,
        updateContext: XGameSaveUpdateHandle,
        blobName: *const c_char,
    ) -> HRESULT {
        let Some(state) = (unsafe { UpdateHandleTable::get(updateContext) }) else {
            return E_INVALIDARG;
        };
        let Some(name) = (unsafe { read_cstr(blobName) }) else {
            return E_INVALIDARG;
        };
        state.writes.retain(|(existing, _)| existing != &name);
        state.deletes.push(name);
        S_OK
    }

    unsafe fn XGameSaveSubmitUpdate(&self, updateContext: XGameSaveUpdateHandle) -> HRESULT {
        let Some(state) = (unsafe { UpdateHandleTable::get(updateContext) }) else {
            return E_INVALIDARG;
        };
        if std::fs::create_dir_all(&state.container_root).is_err() {
            return E_FAIL;
        }
        for (name, data) in &state.writes {
            if std::fs::write(state.container_root.join(name), data).is_err() {
                return E_FAIL;
            }
        }
        for name in &state.deletes {
            let _ = std::fs::remove_file(state.container_root.join(name));
        }
        if !state.display_name.is_empty() {
            write_display_name(&state.container_root, &state.display_name);
        }
        state.writes.clear();
        state.deletes.clear();
        S_OK
    }

    unsafe fn XGameSaveSubmitUpdateAsync(
        &self,
        updateContext: XGameSaveUpdateHandle,
        async_: *mut XAsyncBlock,
    ) -> HRESULT {
        let this = XGameSaveObject_Impl::XGameSaveSubmitUpdate;
        let result = unsafe { this(self, updateContext) };
        unsafe {
            xasync::run_sync(
                async_,
                move || if result == S_OK { Ok(()) } else { Err(result) },
            )
        }
    }

    unsafe fn XGameSaveSubmitUpdateResult(&self, async_: *mut XAsyncBlock) -> HRESULT {
        match unsafe { get_result::<()>(async_, null_mut(), &mut ()) } {
            Ok(()) => S_OK,
            Err(hr) => hr,
        }
    }
}

/// A fixed-size, `Copy` stand-in for a path string, sized like Win32's `MAX_PATH` - needed
/// because [`xasync::get_result`] copies a payload's raw bytes into the caller's buffer rather
/// than running its destructor; a `String` payload there would leave two owners of the same
/// heap allocation (one dropped when the async block is cleaned up, one handed to the caller)
/// and double-free. Every other payload type in this crate is already `Copy` for the same
/// reason - this just extends that to path results.
const PATH_PAYLOAD_CAPACITY: usize = 260;

#[derive(Clone, Copy)]
struct PathPayload {
    len: usize,
    bytes: [u8; PATH_PAYLOAD_CAPACITY],
}

impl PathPayload {
    fn new(path: &str) -> Result<Self, HRESULT> {
        let bytes = path.as_bytes();
        // Reserve one byte for the nul terminator `Result` writes below.
        if bytes.len() >= PATH_PAYLOAD_CAPACITY {
            return Err(E_NOT_SUFFICIENT_BUFFER);
        }
        let mut buffer = [0u8; PATH_PAYLOAD_CAPACITY];
        buffer[..bytes.len()].copy_from_slice(bytes);
        Ok(Self {
            len: bytes.len(),
            bytes: buffer,
        })
    }
}

impl IXGameSaveImpl2_Impl for XGameSaveObject_Impl {
    /// See [`IXGameSaveImpl2`]'s docs: no UI, but an honest non-UI answer exists (the same
    /// directory `XGameSaveInitializeProvider` resolves to), so this completes immediately.
    unsafe fn XGameSaveFilesGetFolderWithUiAsync(
        &self,
        requestingUser: u64,
        configurationId: *const c_char,
        async_: *mut XAsyncBlock,
    ) -> HRESULT {
        let configuration_id = unsafe { read_cstr(configurationId) };
        let user_id = unsafe { crate::xuser::user_id_for_handle(requestingUser) };
        unsafe {
            xasync::run_sync(async_, move || {
                let configuration_id = configuration_id.clone().ok_or(E_INVALIDARG)?;
                let user_id = user_id.ok_or(E_INVALIDARG)?;
                let root = provider_root(user_id, &configuration_id);
                std::fs::create_dir_all(&root).map_err(|_| E_FAIL)?;
                PathPayload::new(&root.to_string_lossy())
            })
        }
    }

    unsafe fn XGameSaveFilesGetFolderWithUiResult(
        &self,
        async_: *mut XAsyncBlock,
        folderSize: usize,
        folderResult: *mut c_char,
    ) -> HRESULT {
        if folderResult.is_null() {
            return E_POINTER;
        }
        let mut payload = PathPayload {
            len: 0,
            bytes: [0u8; PATH_PAYLOAD_CAPACITY],
        };
        if let Err(hr) = unsafe { get_result(async_, null_mut(), &mut payload) } {
            return hr;
        }
        // +1 for the nul terminator, matching the real API's `[out,string]`-flavored contract.
        if folderSize < payload.len + 1 {
            return E_NOT_SUFFICIENT_BUFFER;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(
                payload.bytes.as_ptr(),
                folderResult.cast::<u8>(),
                payload.len,
            );
            *folderResult.add(payload.len) = 0;
        }
        S_OK
    }

    unsafe fn XGameSaveFilesGetRemainingQuota(
        &self,
        userContext: u64,
        configurationId: *const c_char,
        remainingQuota: *mut i64,
    ) -> HRESULT {
        if remainingQuota.is_null() {
            return E_POINTER;
        }
        let Some(configuration_id) = (unsafe { read_cstr(configurationId) }) else {
            return E_INVALIDARG;
        };
        let Some(user_id) = (unsafe { crate::xuser::user_id_for_handle(userContext) }) else {
            return E_INVALIDARG;
        };
        let root = provider_root(user_id, &configuration_id);
        unsafe { *remainingQuota = remaining_quota(&root) };
        S_OK
    }
}

impl IXGameSaveImpl3_Impl for XGameSaveObject_Impl {}

struct GlobalInterface<T>(T);

unsafe impl<T> Send for GlobalInterface<T> {}
unsafe impl<T> Sync for GlobalInterface<T> {}

static XGAMESAVE_SINGLETON: OnceLock<GlobalInterface<IXGameSaveImpl3>> = OnceLock::new();

pub fn xgamesave_singleton() -> &'static IXGameSaveImpl3 {
    &XGAMESAVE_SINGLETON
        .get_or_init(|| GlobalInterface(XGameSaveObject.into()))
        .0
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
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
        let root = unsafe { ProviderHandleTable::get(handle) }.unwrap().clone();
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
        unsafe { ProviderHandleTable::close(handle) };
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
        let user_handle = crate::xuser::create_test_user_handle(user_id);

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

        let root = unsafe { ProviderHandleTable::get(provider) }
            .unwrap()
            .clone();
        assert!(containers_dir(&root).is_dir());
        unsafe { ProviderHandleTable::close(provider) };
        // `XUserCloseHandle` is a private trait method outside `xuser`'s own module - this
        // test-only user handle is deliberately leaked rather than closed.
        let _ = user_handle;
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(UninitializeApiImpl(), S_OK);
    }
}
