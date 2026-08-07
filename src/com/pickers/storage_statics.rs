//! `Windows.Storage.StorageFile`'s static factory.
//!
//! This is not a picker, but it is the other half of one. A title that picks a file to read gets
//! back a path, and the way it turns that path into something it can work with is
//! `StorageFile.GetFileFromPathAsync` - so a pick that succeeds still ends in a failed activation
//! unless this class answers too. What it hands back is the same [`PickedFile`] a save pick
//! produces, which is why it lives beside the pickers rather than somewhere of its own.
//!
//! Wine registers this class to a module that does not implement it, so reaching this needs the
//! class's `DllPath` pointed here - the same registry route the pickers take.
//!
//! Only `GetFileFromPathAsync` is served. The rest of the interface builds files out of app URIs
//! and streamed-file callbacks, neither of which a title can reach without more of WinRT than
//! this runtime has.

use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::path::PathBuf;

use windows::Storage::{IStorageFileStatics, IStorageFileStatics_Vtbl};
use windows_core::{GUID, HRESULT, HSTRING, IInspectable_Vtbl, IUnknown_Vtbl, Interface};

use super::async_op::PickOperation;
use super::storage_file::PickedFile;
use super::{
    E_NOINTERFACE, E_NOTIMPL, E_POINTER, IID_IAGILE_OBJECT, S_OK, spy_get_iids, spy_get_trust_level,
};
use crate::diag::stub;

/// The class this module serves, spelled the way a title asks for it.
pub(super) const STORAGE_FILE: &str = "Windows.Storage.StorageFile";

/// `HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)`, which is what WinRT reports for a path that names
/// nothing.
const HRESULT_FILE_NOT_FOUND: HRESULT = HRESULT(0x80070002u32 as i32);

/// The factory is stateless, so one static instance serves every lookup and its reference count
/// never has to mean anything.
#[repr(C)]
struct StorageFileStatics {
    vtable: &'static IStorageFileStatics_Vtbl,
}

// SAFETY: the factory holds no state, so sharing the one static instance across threads is safe.
unsafe impl Sync for StorageFileStatics {}

static STATICS: StorageFileStatics = StorageFileStatics {
    vtable: &STATICS_VTABLE,
};

/// Returns the factory for this class, which the caller does not have to release.
pub(super) fn activation_factory() -> *mut c_void {
    (&STATICS) as *const _ as *mut c_void
}

unsafe extern "system" fn statics_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    interface: *mut *mut c_void,
) -> HRESULT {
    if iid.is_null() || interface.is_null() {
        return E_POINTER;
    }
    // SAFETY: both pointers were just null-checked and COM guarantees they stay valid here.
    unsafe {
        let requested = *iid;
        let known = requested == windows_core::IUnknown::IID
            || requested == windows_core::IInspectable::IID
            || requested == IStorageFileStatics::IID
            || requested == IID_IAGILE_OBJECT;
        if known {
            *interface = this;
            S_OK
        } else {
            // `IActivationFactory` lands here: `StorageFile` has no constructor, so there is
            // nothing for `ActivateInstance` to make, and refusing says exactly that.
            stub!("StorageFileStatics::QueryInterface({requested:?}) -> E_NOINTERFACE");
            *interface = std::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

/// The factory outlives every caller, so its reference count is a constant.
unsafe extern "system" fn statics_add_ref(_this: *mut c_void) -> u32 {
    2
}

unsafe extern "system" fn statics_release(_this: *mut c_void) -> u32 {
    1
}

unsafe extern "system" fn statics_runtime_class_name(
    _this: *mut c_void,
    value: *mut *mut c_void,
) -> HRESULT {
    if value.is_null() {
        return E_POINTER;
    }
    // SAFETY: `value` was just null-checked; the string is handed to the caller to free.
    unsafe {
        *value = std::mem::transmute::<HSTRING, *mut c_void>(HSTRING::from(STORAGE_FILE));
    }
    S_OK
}

/// # Safety
/// `path` must be a valid `HSTRING` or null, and `operation` a writable out-parameter.
unsafe extern "system" fn GetFileFromPathAsync(
    _this: *mut c_void,
    path: *mut c_void,
    operation: *mut *mut c_void,
) -> HRESULT {
    if operation.is_null() {
        return E_POINTER;
    }
    // SAFETY: `operation` was just null-checked. The path belongs to the caller, so it is read
    // without taking ownership - dropping it here would free a string the caller still uses.
    unsafe {
        *operation = std::ptr::null_mut();
        if path.is_null() {
            return E_POINTER;
        }
        let borrowed = ManuallyDrop::new(std::mem::transmute::<*mut c_void, HSTRING>(path));
        let path = PathBuf::from(borrowed.to_string_lossy());

        // A file that is not there is the one failure a caller of this is written to expect, and
        // it is worth telling apart from anything else that could go wrong.
        let outcome = if path.is_file() {
            stub!("StorageFile::GetFileFromPathAsync({path:?})");
            Ok(Some(path))
        } else {
            stub!("StorageFile::GetFileFromPathAsync({path:?}) -> not found");
            Err(HRESULT_FILE_NOT_FOUND)
        };
        *operation = PickOperation::<PickedFile>::completed(outcome);
    }
    S_OK
}

/// The routes that build a file out of something other than a path.
///
/// # Safety
/// `operation` must be a writable out-parameter.
unsafe extern "system" fn refuse_2(
    _this: *mut c_void,
    _one: *mut c_void,
    operation: *mut *mut c_void,
) -> HRESULT {
    stub!("StorageFileStatics: unimplemented route");
    // SAFETY: COM guarantees `operation` is writable for the duration of the call.
    unsafe {
        if !operation.is_null() {
            *operation = std::ptr::null_mut();
        }
    }
    E_NOTIMPL
}

/// # Safety
/// `operation` must be a writable out-parameter.
unsafe extern "system" fn refuse_4(
    _this: *mut c_void,
    _one: *mut c_void,
    _two: *mut c_void,
    _three: *mut c_void,
    operation: *mut *mut c_void,
) -> HRESULT {
    stub!("StorageFileStatics: unimplemented route");
    // SAFETY: COM guarantees `operation` is writable for the duration of the call.
    unsafe {
        if !operation.is_null() {
            *operation = std::ptr::null_mut();
        }
    }
    E_NOTIMPL
}

static STATICS_VTABLE: IStorageFileStatics_Vtbl = IStorageFileStatics_Vtbl {
    base__: IInspectable_Vtbl {
        base: IUnknown_Vtbl {
            QueryInterface: statics_query_interface,
            AddRef: statics_add_ref,
            Release: statics_release,
        },
        GetIids: spy_get_iids,
        GetRuntimeClassName: statics_runtime_class_name,
        GetTrustLevel: spy_get_trust_level,
    },
    GetFileFromPathAsync,
    GetFileFromApplicationUriAsync: refuse_2,
    CreateStreamedFileAsync: refuse_4,
    ReplaceWithStreamedFileAsync: refuse_4,
    CreateStreamedFileFromUriAsync: refuse_4,
    ReplaceWithStreamedFileFromUriAsync: refuse_4,
};
