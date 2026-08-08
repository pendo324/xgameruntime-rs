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

use std::path::PathBuf;

use windows_core::{HRESULT, HSTRING, Ref, Result, implement};
use windows_future::IAsyncOperation;

use super::async_op::PickOperation;
use super::bindings::{
    IRandomAccessStreamReference, IStorageFile, IStorageFileStatics, IStorageFileStatics_Impl,
    StorageFile, StreamedFileDataRequestedHandler, Uri,
};
use super::storage_file::PickedFile;
use crate::diag::stub;

/// The class this module serves, spelled the way a title asks for it.
pub(super) const STORAGE_FILE: &str = "Windows.Storage.StorageFile";

/// `HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)`, which is what WinRT reports for a path that names
/// nothing.
const HRESULT_FILE_NOT_FOUND: HRESULT = HRESULT(0x80070002u32 as i32);

/// The statics object behind that class name.
///
/// It holds nothing - every route is a function of its arguments - so a fresh one per activation
/// is as good as a shared one, and lets `#[implement]` do the reference counting.
#[implement(IStorageFileStatics)]
struct StorageFileStatics;

impl IStorageFileStatics_Impl for StorageFileStatics_Impl {
    fn GetFileFromPathAsync(&self, path: &HSTRING) -> Result<IAsyncOperation<StorageFile>> {
        let path = PathBuf::from(path.to_string_lossy());

        // A file that is not there is the one failure a caller of this is written to expect, and
        // it is worth telling apart from anything else that could go wrong.
        let outcome = if path.is_file() {
            stub!("StorageFile::GetFileFromPathAsync({path:?})");
            Ok(Some(path))
        } else {
            stub!("StorageFile::GetFileFromPathAsync({path:?}) -> not found");
            Err(HRESULT_FILE_NOT_FOUND)
        };
        let operation = PickOperation::<PickedFile>::completed(outcome);

        // SAFETY: `IAsyncOperation` is a `repr(transparent)` interface pointer; `completed`
        // returns a non-null one carrying the reference that passes to the caller here.
        Ok(unsafe {
            std::mem::transmute::<*mut std::ffi::c_void, IAsyncOperation<StorageFile>>(operation)
        })
    }

    fn GetFileFromApplicationUriAsync(
        &self,
        _uri: Ref<Uri>,
    ) -> Result<IAsyncOperation<StorageFile>> {
        stub!("StorageFile::GetFileFromApplicationUriAsync (unimplemented)");
        Err(super::E_NOTIMPL.into())
    }

    fn CreateStreamedFileAsync(
        &self,
        _displayNameWithExtension: &HSTRING,
        _dataRequested: Ref<StreamedFileDataRequestedHandler>,
        _thumbnail: Ref<IRandomAccessStreamReference>,
    ) -> Result<IAsyncOperation<StorageFile>> {
        stub!("StorageFile::CreateStreamedFileAsync (unimplemented)");
        Err(super::E_NOTIMPL.into())
    }

    fn ReplaceWithStreamedFileAsync(
        &self,
        _fileToReplace: Ref<IStorageFile>,
        _dataRequested: Ref<StreamedFileDataRequestedHandler>,
        _thumbnail: Ref<IRandomAccessStreamReference>,
    ) -> Result<IAsyncOperation<StorageFile>> {
        stub!("StorageFile::ReplaceWithStreamedFileAsync (unimplemented)");
        Err(super::E_NOTIMPL.into())
    }

    fn CreateStreamedFileFromUriAsync(
        &self,
        _displayNameWithExtension: &HSTRING,
        _uri: Ref<Uri>,
        _thumbnail: Ref<IRandomAccessStreamReference>,
    ) -> Result<IAsyncOperation<StorageFile>> {
        stub!("StorageFile::CreateStreamedFileFromUriAsync (unimplemented)");
        Err(super::E_NOTIMPL.into())
    }

    fn ReplaceWithStreamedFileFromUriAsync(
        &self,
        _fileToReplace: Ref<IStorageFile>,
        _uri: Ref<Uri>,
        _thumbnail: Ref<IRandomAccessStreamReference>,
    ) -> Result<IAsyncOperation<StorageFile>> {
        stub!("StorageFile::ReplaceWithStreamedFileFromUriAsync (unimplemented)");
        Err(super::E_NOTIMPL.into())
    }
}

/// Returns the factory for this class, with a reference the caller owns.
///
/// `StorageFile` has no constructor, so this object answers `IStorageFileStatics` and not
/// `IActivationFactory`: there is nothing for `ActivateInstance` to make.
pub(super) fn activation_factory() -> *mut std::ffi::c_void {
    let statics: IStorageFileStatics = StorageFileStatics.into();
    // SAFETY: `IStorageFileStatics` is a `repr(transparent)` interface pointer, and the reference
    // it holds passes to the caller rather than being dropped here.
    unsafe {
        let raw = std::mem::transmute_copy(&statics);
        std::mem::forget(statics);
        raw
    }
}
