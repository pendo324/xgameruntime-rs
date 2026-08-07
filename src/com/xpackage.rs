use super::{E_NOTIMPL, XPackageMountHandle, XPackageMountHandleTable};
use crate::results::*;
use std::ffi::{c_char, c_void};
use windows_core::{GUID, HRESULT, IUnknown, implement, interface};
use windows_sys::core::BOOL;

use super::bool_stub;
use super::hresult_stub;
use super::void_stub;

pub const CLSID_XPACKAGE: GUID = GUID::from_u128(0xaf406016_e850_4aa8_a88d_2f3dcb9dac7e);
/// `wine/include/xpackage.idl`'s `IXPackageImpl`, `__PADDING__`/`__PADDING_2__`/.../`__PADDING_5__`
/// slots included in their exact positions since these are real (unnamed-in-practice) vtable
/// slots, not something to compact away. Only `XPackageGetMountPathSize`/`XPackageGetMountPath`/
/// `XPackageCloseMountHandle` have real bodies (see [`XPackageMountHandleTable`], populated by
/// `IXPersistentLocalStorage::mount_for_package`) - everything else here has no real backing
/// (package install/chunk-download management, which Xodus doesn't model) and reports
/// `E_NOTIMPL`/`FALSE` rather than a guess.
#[interface("3720de07-e8e4-44a3-ad32-b359e8adbe55")]
pub unsafe trait IXPackageImpl: IUnknown {
    pub unsafe fn XPackageGetCurrentProcessPackageIdentifier(
        &self,
        bufferSize: usize,
        buffer: *mut c_char,
    ) -> HRESULT;
    pub unsafe fn XPackageIsPackagedProcess(&self) -> BOOL;
    pub unsafe fn XPackageCreateInstallationMonitor(
        &self,
        packageIdentifier: *const c_char,
        selectorCount: u32,
        selectors: *mut c_void,
        minimumUpdateIntervalMs: u32,
        queue: *mut c_void,
        installationMonitor: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XPackageCloseInstallationMonitorHandle(&self, installationMonitor: u64) -> ();
    pub unsafe fn XPackageGetInstallationProgress(
        &self,
        installationMonitor: u64,
        progress: *mut c_void,
    ) -> ();
    pub unsafe fn XPackageUpdateInstallationMonitor(&self, installationMonitor: u64) -> BOOL;
    pub unsafe fn XPackageRegisterInstallationProgressChanged(
        &self,
        installationMonitor: u64,
        context: *mut c_void,
        callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XPackageUnregisterInstallationProgressChanged(
        &self,
        installationMonitor: u64,
        token: u64,
        wait: BOOL,
    ) -> BOOL;
    pub unsafe fn XPackageGetUserLocale(&self, localeSize: usize, locale: *mut c_char) -> HRESULT;
    pub unsafe fn XPackageFindChunkAvailability(
        &self,
        packageIdentifier: *const c_char,
        selectorCount: u32,
        selectors: *mut c_void,
        availability: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XPackageEnumerateChunkAvailability(
        &self,
        packageIdentifier: *const c_char,
        selectorType: u32,
        context: *mut c_void,
        callback: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XPackageChangeChunkInstallOrder(
        &self,
        packageIdentifier: *const c_char,
        selectorCount: u32,
        selectors: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XPackageInstallChunks(
        &self,
        packageIdentifier: *const c_char,
        selectorCount: u32,
        selectors: *mut c_void,
        minimumUpdateIntervalMs: u32,
        suppressUserConfirmation: BOOL,
        queue: *mut c_void,
        installationMonitor: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XPackageInstallChunksAsync(
        &self,
        packageIdentifier: *const c_char,
        selectorCount: u32,
        selectors: *mut c_void,
        minimumUpdateIntervalMs: u32,
        suppressUserConfirmation: BOOL,
        asyncBlock: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XPackageInstallChunksResult(
        &self,
        asyncBlock: *mut c_void,
        installationMonitor: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XPackageEstimateDownloadSize(
        &self,
        packageIdentifier: *const c_char,
        selectorCount: u32,
        selectors: *mut c_void,
        downloadSize: *mut u64,
        shouldPresentUserConfirmation: *mut BOOL,
    ) -> HRESULT;
    pub unsafe fn XPackageUninstallChunks(
        &self,
        packageIdentifier: *const c_char,
        selectorCount: u32,
        selectors: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn __PADDING__(&self) -> HRESULT;
    pub unsafe fn __PADDING_2__(&self) -> HRESULT;
    pub unsafe fn XPackageUnregisterPackageInstalled(&self, token: u64, wait: BOOL) -> BOOL;
    pub unsafe fn __PADDING_3__(&self) -> HRESULT;
    pub unsafe fn XPackageGetMountPathSize(
        &self,
        mount: XPackageMountHandle,
        pathSize: *mut usize,
    ) -> HRESULT;
    pub unsafe fn XPackageGetMountPath(
        &self,
        mount: XPackageMountHandle,
        pathSize: usize,
        path: *mut c_char,
    ) -> HRESULT;
    pub unsafe fn XPackageCloseMountHandle(&self, mount: XPackageMountHandle) -> ();
    pub unsafe fn __PADDING_4__(&self) -> HRESULT;
    pub unsafe fn XPackageEnumeratePackages(
        &self,
        kind: u32,
        scope: u32,
        context: *mut c_void,
        callback: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XPackageRegisterPackageInstalled(
        &self,
        queue: u64,
        context: *mut c_void,
        callback: *mut c_void,
        token: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XPackageGetWriteStats(&self, writeStats: *mut c_void) -> HRESULT;
    pub unsafe fn __PADDING_5__(&self) -> HRESULT;
    pub unsafe fn XPackageUninstallUWPInstance(&self, packageName: *const c_char) -> HRESULT;
    pub unsafe fn XPackageEnumerateFeatures(
        &self,
        packageIdentifier: *const c_char,
        context: *mut c_void,
        callback: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XPackageUninstallPackage(&self, packageIdentifier: *const c_char) -> BOOL;
}

/// Adds `XPackageMountWithUiAsync`/`Result` over [`IXPackageImpl`] - not implemented, since
/// Xodus has no UI surface to show (same rationale as
/// `IXPersistentLocalStorage::prompt_user_for_space_async`, but this one has no
/// always-succeed answer: mounting requires actually resolving a package, unlike a storage-space
/// prompt).
#[interface("f92d8712-2b27-4d8a-bf01-11a6f8e3eb42")]
pub unsafe trait IXPackageImpl2: IXPackageImpl {
    pub unsafe fn XPackageMountWithUiAsync(
        &self,
        packageIdentifier: *const c_char,
        async_: *mut c_void,
    ) -> HRESULT;
    pub unsafe fn XPackageMountWithUiResult(
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
        let Some(path) = XPackageMountHandleTable::get(mount) else {
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
        let Some(mount_path) = XPackageMountHandleTable::get(mount) else {
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
        XPackageMountHandleTable::close(mount);
    }
}

impl IXPackageImpl2_Impl for XPackageObject_Impl {
    hresult_stub! {
        unsafe fn XPackageMountWithUiAsync(&self, packageIdentifier: *const c_char, async_: *mut c_void) -> HRESULT;
        unsafe fn XPackageMountWithUiResult(&self, async_: *mut c_void, mount: *mut XPackageMountHandle) -> HRESULT;
    }
}

impl IXPackageImpl3_Impl for XPackageObject_Impl {}
