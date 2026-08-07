//! The `XGameSaveObject` COM object and its vtable implementations - a thin COM face over
//! the on-disk engine in [`super::storage`].

use super::*;
use crate::E_FAIL;
use crate::com::singleton;
use crate::com::xasync::{self, XAsyncBlock, get_result};
use crate::results::*;

use std::ffi::{c_char, c_void};
use std::ptr::null_mut;

use windows_core::{HRESULT, implement};

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
        let user_id = crate::com::xuser::user_id_for_handle(requestingUser);
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
        let user_id = crate::com::xuser::user_id_for_handle(requestingUser);
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
        ProviderHandleTable::close(provider);
    }

    unsafe fn XGameSaveGetRemainingQuota(
        &self,
        provider: XGameSaveProviderHandle,
        remainingQuota: *mut i64,
    ) -> HRESULT {
        if remainingQuota.is_null() {
            return E_POINTER;
        }
        let Some(root) = ProviderHandleTable::get(provider) else {
            return E_INVALIDARG;
        };
        unsafe { *remainingQuota = remaining_quota(&root) };
        S_OK
    }

    unsafe fn XGameSaveGetRemainingQuotaAsync(
        &self,
        provider: XGameSaveProviderHandle,
        async_: *mut XAsyncBlock,
    ) -> HRESULT {
        let root = ProviderHandleTable::get(provider);
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
        let Some(root) = ProviderHandleTable::get(provider) else {
            return E_INVALIDARG;
        };
        let container_name = unsafe { read_cstr(containerName) };
        match delete_container(&root, container_name) {
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
        let root = ProviderHandleTable::get(provider);
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
        let Some(root) = ProviderHandleTable::get(provider) else {
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
            container_info(&root, &name)
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
        let Some(root) = ProviderHandleTable::get(provider) else {
            return E_INVALIDARG;
        };
        let Some(callback) = callback else {
            return E_POINTER;
        };
        enumerate_containers(&root, None, context, callback);
        S_OK
    }

    unsafe fn XGameSaveEnumerateContainerInfoByName(
        &self,
        provider: XGameSaveProviderHandle,
        containerNamePrefix: *const c_char,
        context: *mut c_void,
        callback: Option<XGameSaveContainerInfoCallback>,
    ) -> HRESULT {
        let Some(root) = ProviderHandleTable::get(provider) else {
            return E_INVALIDARG;
        };
        let Some(callback) = callback else {
            return E_POINTER;
        };
        let prefix = unsafe { read_cstr(containerNamePrefix) };
        enumerate_containers(&root, prefix.as_deref(), context, callback);
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
        let Some(root) = ProviderHandleTable::get(provider) else {
            return E_INVALIDARG;
        };
        let Some(name) = (unsafe { read_cstr(containerName) }) else {
            return E_INVALIDARG;
        };
        let dir = container_dir(&root, &name);
        if std::fs::create_dir_all(&dir).is_err() {
            return E_FAIL;
        }
        let handle = ContainerHandleTable::create(ContainerState { root: dir });
        unsafe { *containerContext = handle };
        S_OK
    }

    unsafe fn XGameSaveCloseContainer(&self, context: XGameSaveContainerHandle) {
        ContainerHandleTable::close(context);
    }

    unsafe fn XGameSaveEnumerateBlobInfo(
        &self,
        container: XGameSaveContainerHandle,
        context: *mut c_void,
        callback: Option<XGameSaveBlobInfoCallback>,
    ) -> HRESULT {
        let Some(state) = ContainerHandleTable::get(container) else {
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
        let Some(state) = ContainerHandleTable::get(container) else {
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
        let Some(state) = ContainerHandleTable::get(container) else {
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
        let Some(state) = ContainerHandleTable::get(container) else {
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
        let Some(state) = ContainerHandleTable::get(container) else {
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
        UpdateHandleTable::close(context);
    }

    unsafe fn XGameSaveSubmitBlobWrite(
        &self,
        updateContext: XGameSaveUpdateHandle,
        blobName: *const c_char,
        data: *mut u8,
        byteCount: usize,
    ) -> HRESULT {
        let Some(state) = UpdateHandleTable::get(updateContext) else {
            return E_INVALIDARG;
        };
        let mut state = state.lock().expect("update state poisoned");
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
        let Some(state) = UpdateHandleTable::get(updateContext) else {
            return E_INVALIDARG;
        };
        let mut state = state.lock().expect("update state poisoned");
        let Some(name) = (unsafe { read_cstr(blobName) }) else {
            return E_INVALIDARG;
        };
        state.writes.retain(|(existing, _)| existing != &name);
        state.deletes.push(name);
        S_OK
    }

    unsafe fn XGameSaveSubmitUpdate(&self, updateContext: XGameSaveUpdateHandle) -> HRESULT {
        let Some(state) = UpdateHandleTable::get(updateContext) else {
            return E_INVALIDARG;
        };
        let mut state = state.lock().expect("update state poisoned");
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
    /// See [`IXGameSaveImpl2`]'s docs: no UI, but a non-UI answer exists (the same
    /// directory `XGameSaveInitializeProvider` resolves to), so this completes immediately.
    unsafe fn XGameSaveFilesGetFolderWithUiAsync(
        &self,
        requestingUser: u64,
        configurationId: *const c_char,
        async_: *mut XAsyncBlock,
    ) -> HRESULT {
        let configuration_id = unsafe { read_cstr(configurationId) };
        let user_id = crate::com::xuser::user_id_for_handle(requestingUser);
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
        // SAFETY: `folderSize >= payload.len + 1` was checked above.
        unsafe {
            crate::ffi_util::write_out_bytes(
                &payload.bytes[..payload.len],
                folderResult.cast::<u8>(),
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
        let Some(user_id) = crate::com::xuser::user_id_for_handle(userContext) else {
            return E_INVALIDARG;
        };
        let root = provider_root(user_id, &configuration_id);
        unsafe { *remainingQuota = remaining_quota(&root) };
        S_OK
    }
}

impl IXGameSaveImpl3_Impl for XGameSaveObject_Impl {}

singleton! {
    pub fn xgamesave_singleton() -> IXGameSaveImpl3 = XGameSaveObject;
}
