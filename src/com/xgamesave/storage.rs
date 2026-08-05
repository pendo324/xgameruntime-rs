//! The XGameSave on-disk engine: the directory layout under `<game_save_root>/<xuid>/<config>`,
//! container/blob enumeration, the quota estimate, and the provider/container/update handle
//! tables that the COM object in [`super::object`] drives. No `#[implement]` here.

use super::*;
use crate::E_FAIL;
use crate::results::*;

use std::env::temp_dir;
use std::ffi::c_char;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub(crate) fn provider_root(user_id: u64, configuration_id: &str) -> PathBuf {
    let base = crate::ipc::game_save_root()
        .map(PathBuf::from)
        .unwrap_or_else(|| temp_dir().join("xodus_gamesaves"));
    base.join(user_id.to_string()).join(configuration_id)
}

pub(crate) fn containers_dir(provider_root: &Path) -> PathBuf {
    provider_root.join("containers")
}

pub(crate) fn container_dir(provider_root: &Path, name: &str) -> PathBuf {
    containers_dir(provider_root).join(name)
}

pub(crate) fn read_display_name(container_root: &Path) -> String {
    std::fs::read_to_string(container_root.join(META_FILE)).unwrap_or_default()
}

pub(crate) fn write_display_name(container_root: &Path, display_name: &str) {
    let _ = std::fs::write(container_root.join(META_FILE), display_name);
}

/// Blob files under a container directory - everything except [`META_FILE`].
pub(crate) fn blob_files(container_root: &Path) -> Vec<std::fs::DirEntry> {
    std::fs::read_dir(container_root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_name() != META_FILE && entry.path().is_file())
        .collect()
}

pub(crate) fn container_total_size(container_root: &Path) -> u64 {
    blob_files(container_root)
        .iter()
        .filter_map(|entry| entry.metadata().ok())
        .map(|metadata| metadata.len())
        .sum()
}

pub(crate) fn container_last_modified(container_root: &Path) -> i64 {
    blob_files(container_root)
        .iter()
        .filter_map(|entry| entry.metadata().ok()?.modified().ok())
        .max()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// Reads a nul-terminated C string, if `ptr` is non-null.
pub(crate) unsafe fn read_cstr(ptr: *const c_char) -> Option<String> {
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

pub(crate) struct ProviderHandleTable;

impl ProviderHandleTable {
    pub(crate) fn create(root: PathBuf) -> u64 {
        Box::into_raw(Box::new(root)) as u64
    }

    /// # Safety
    /// `handle` must be zero or a handle from [`Self::create`] that has not been closed.
    pub(crate) unsafe fn get<'a>(handle: u64) -> Option<&'a PathBuf> {
        if handle == 0 {
            return None;
        }
        Some(unsafe { &*(handle as *const PathBuf) })
    }

    /// # Safety
    /// `handle` must be an open handle from [`Self::create`]; it is invalid afterwards.
    pub(crate) unsafe fn close(handle: u64) {
        if handle == 0 {
            return;
        }
        drop(unsafe { Box::from_raw(handle as *mut PathBuf) });
    }
}

pub(crate) struct ContainerState {
    pub(crate) root: PathBuf,
}

pub(crate) struct ContainerHandleTable;

impl ContainerHandleTable {
    pub(crate) fn create(state: ContainerState) -> u64 {
        Box::into_raw(Box::new(state)) as u64
    }

    /// # Safety
    /// `handle` must be zero or a handle from [`Self::create`] that has not been closed.
    pub(crate) unsafe fn get<'a>(handle: u64) -> Option<&'a ContainerState> {
        if handle == 0 {
            return None;
        }
        Some(unsafe { &*(handle as *const ContainerState) })
    }

    /// # Safety
    /// `handle` must be an open handle from [`Self::create`]; it is invalid afterwards.
    pub(crate) unsafe fn close(handle: u64) {
        if handle == 0 {
            return;
        }
        drop(unsafe { Box::from_raw(handle as *mut ContainerState) });
    }
}

/// A pending, uncommitted batch of blob writes/deletes - real writes only land on disk when
/// `XGameSaveSubmitUpdate(Async)` runs, matching the real API's transactional shape.
pub(crate) struct UpdateState {
    pub(crate) container_root: PathBuf,
    pub(crate) display_name: String,
    pub(crate) writes: Vec<(String, Vec<u8>)>,
    pub(crate) deletes: Vec<String>,
}

pub(crate) struct UpdateHandleTable;

impl UpdateHandleTable {
    pub(crate) fn create(state: UpdateState) -> u64 {
        Box::into_raw(Box::new(state)) as u64
    }

    /// # Safety
    /// `handle` must be zero or a handle from [`Self::create`] that has not been closed.
    pub(crate) unsafe fn get<'a>(handle: u64) -> Option<&'a mut UpdateState> {
        if handle == 0 {
            return None;
        }
        Some(unsafe { &mut *(handle as *mut UpdateState) })
    }

    /// # Safety
    /// `handle` must be an open handle from [`Self::create`]; it is invalid afterwards.
    pub(crate) unsafe fn close(handle: u64) {
        if handle == 0 {
            return;
        }
        drop(unsafe { Box::from_raw(handle as *mut UpdateState) });
    }
}

// ---------------------------------------------------------------------------------------
// Shared logic behind the sync/Async/Result trios
// ---------------------------------------------------------------------------------------

pub(crate) fn initialize_provider(
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
/// 1 GiB budget minus what's actually on disk - the same placeholder stance as
/// `IXPersistentLocalStorage::GetSpaceInfo`'s fallback numbers, not a real per-title limit.
pub(crate) const PLACEHOLDER_QUOTA_BYTES: i64 = 1024 * 1024 * 1024;

pub(crate) fn remaining_quota(provider_root: &Path) -> i64 {
    let used: u64 = std::fs::read_dir(containers_dir(provider_root))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| container_total_size(&entry.path()))
        .sum();
    PLACEHOLDER_QUOTA_BYTES.saturating_sub(used as i64).max(0)
}

pub(crate) fn delete_container(
    provider_root: &Path,
    container_name: Option<String>,
) -> Result<(), HRESULT> {
    let container_name = container_name.ok_or(E_INVALIDARG)?;
    let dir = container_dir(provider_root, &container_name);
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(E_FAIL),
    }
}

/// Builds a real [`XGameSaveContainerInfo`] for `name` if its directory exists.
pub(crate) fn container_info(
    provider_root: &Path,
    name: &str,
) -> Option<(String, String, u32, u64, i64)> {
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
pub(crate) fn enumerate_containers(
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

pub(crate) fn enumerate_blobs(
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
