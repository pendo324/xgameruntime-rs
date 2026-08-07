//! The `StorageFile` a completed save-pick hands back.
//!
//! A title gets at the picked file one of two ways: read `Path` and do its own file IO, or open
//! the file as a WinRT stream and write through that. Only the first is implemented here. The
//! second needs a full `IRandomAccessStream` - `IBuffer` marshalling and all - which is a much
//! larger surface than the picker itself, and no title is known to need it yet. The stream
//! entry points therefore log and refuse, so a title that does take that route says so in the
//! log instead of failing somewhere less obvious.
//!
//! Unlike the pickers, `windows-rs` does generate `_Impl` traits for these interfaces, so the
//! vtables come from `#[implement]` rather than by hand.

use std::path::{Path, PathBuf};

use windows::Storage::FileProperties::BasicProperties;
use windows::Storage::FileProperties::{
    StorageItemContentProperties, StorageItemThumbnail, ThumbnailMode, ThumbnailOptions,
};
use windows::Storage::Streams::{
    IInputStream, IInputStreamReference_Impl, IRandomAccessStreamReference_Impl,
    IRandomAccessStreamWithContentType,
};
use windows::Storage::{
    FileAccessMode, FileAttributes, IStorageFile, IStorageFile_Impl, IStorageFile2,
    IStorageFile2_Impl, IStorageFilePropertiesWithAvailability,
    IStorageFilePropertiesWithAvailability_Impl, IStorageFolder, IStorageItem, IStorageItem_Impl,
    IStorageItem2, IStorageItem2_Impl, IStorageItemProperties, IStorageItemProperties_Impl,
    IStorageItemProperties2, IStorageItemProperties2_Impl, IStorageItemPropertiesWithProvider,
    IStorageItemPropertiesWithProvider_Impl, NameCollisionOption, StorageDeleteOption, StorageFile,
    StorageFolder, StorageItemTypes, StorageOpenOptions, StorageProvider, StorageStreamTransaction,
};
use windows_core::{HSTRING, Ref, Result, implement};
use windows_future::{IAsyncAction, IAsyncOperation};

use crate::diag::stub;

/// Ticks between the Windows epoch (1601-01-01) and the Unix epoch, in 100ns units - the
/// conversion `DateTime` needs to carry a `SystemTime`.
const TICKS_TO_UNIX_EPOCH: i64 = 11_644_473_600 * 10_000_000;

/// A file on disk, presented to a title as WinRT's `StorageFile`.
///
/// The interface list is the real class's, not just the one interface a file is usually reached
/// through. A caller that holds a `StorageFile` may ask it for any of them, and a refusal is not
/// something callers check for - the ones seen here take the null a failed `QueryInterface`
/// leaves behind and call through it. Answering an interface and then declining the individual
/// method at least fails somewhere the caller is looking.
///
/// Every interface has to be named here, including the ones the others require. WinRT's "requires"
/// is not COM inheritance: implementing `IStorageFile` obliges this type to supply `IStorageItem`'s
/// methods, but it does not give the object an `IStorageItem` vtable to hand out. Leaving one off
/// compiles, and the class only comes apart when a caller asks for it.
#[implement(
    IStorageFile,
    IStorageFile2,
    IStorageItem,
    IStorageItem2,
    IStorageItemProperties,
    IStorageItemProperties2,
    IStorageItemPropertiesWithProvider,
    IStorageFilePropertiesWithAvailability
)]
pub(super) struct PickedFile {
    path: PathBuf,
}

impl PickedFile {
    /// Wraps `path` and hands back the `StorageFile` a caller expects.
    ///
    /// `StorageFile` is a runtime class, and this is not an instance of it - but at the ABI
    /// there is no difference between the two: both are one pointer, and every caller reaches
    /// the file through `IStorageFile`/`IStorageItem`, which this does implement. The cast is
    /// what lets the operation's result carry the declared type.
    pub(super) fn create(path: PathBuf) -> StorageFile {
        let file: IStorageFile = PickedFile { path }.into();
        // SAFETY: `StorageFile` and `IStorageFile` are both `repr(transparent)` wrappers around
        // a single interface pointer, and the pointer being wrapped implements `IStorageFile`.
        unsafe { std::mem::transmute(file) }
    }

    /// The trailing component of the path, which is what WinRT calls the file's name.
    fn file_name(&self) -> HSTRING {
        HSTRING::from(
            self.path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
        )
    }
}

impl IStorageItem_Impl for PickedFile_Impl {
    fn Name(&self) -> Result<HSTRING> {
        Ok(self.file_name())
    }

    fn Path(&self) -> Result<HSTRING> {
        Ok(HSTRING::from(self.path.to_string_lossy().into_owned()))
    }

    fn Attributes(&self) -> Result<FileAttributes> {
        // The picked file may not exist yet - a save picker names a file, it does not create
        // one - so a missing file is reported as an ordinary file rather than an error.
        let attributes = match std::fs::metadata(&self.path) {
            Ok(metadata) if metadata.is_dir() => FileAttributes::Directory,
            _ => FileAttributes::Normal,
        };
        Ok(attributes)
    }

    fn DateCreated(&self) -> Result<windows_time::DateTime> {
        let created = std::fs::metadata(&self.path)
            .and_then(|metadata| metadata.created())
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|since_epoch| since_epoch.as_nanos() as i64 / 100 + TICKS_TO_UNIX_EPOCH)
            .unwrap_or_default();
        Ok(windows_time::DateTime {
            universal_time: created,
        })
    }

    fn IsOfType(&self, kind: StorageItemTypes) -> Result<bool> {
        Ok(kind == StorageItemTypes::File)
    }

    fn GetBasicPropertiesAsync(&self) -> Result<IAsyncOperation<BasicProperties>> {
        refuse("GetBasicPropertiesAsync")
    }

    fn RenameAsyncOverloadDefaultOptions(&self, _name: &HSTRING) -> Result<IAsyncAction> {
        refuse("RenameAsync")
    }

    fn RenameAsync(&self, _name: &HSTRING, _option: NameCollisionOption) -> Result<IAsyncAction> {
        refuse("RenameAsync")
    }

    fn DeleteAsyncOverloadDefaultOptions(&self) -> Result<IAsyncAction> {
        refuse("DeleteAsync")
    }

    fn DeleteAsync(&self, _option: StorageDeleteOption) -> Result<IAsyncAction> {
        refuse("DeleteAsync")
    }
}

impl IStorageFile_Impl for PickedFile_Impl {
    fn FileType(&self) -> Result<HSTRING> {
        // WinRT reports the extension with its dot, and the empty string when there is none.
        let extension = self
            .path
            .extension()
            .map(|extension| format!(".{}", extension.to_string_lossy()))
            .unwrap_or_default();
        Ok(HSTRING::from(extension))
    }

    fn ContentType(&self) -> Result<HSTRING> {
        Ok(HSTRING::from(content_type(&self.path)))
    }

    fn OpenAsync(
        &self,
        _mode: FileAccessMode,
    ) -> Result<IAsyncOperation<windows::Storage::Streams::IRandomAccessStream>> {
        refuse("OpenAsync")
    }

    fn OpenTransactedWriteAsync(&self) -> Result<IAsyncOperation<StorageStreamTransaction>> {
        refuse("OpenTransactedWriteAsync")
    }

    fn CopyOverloadDefaultNameAndOptions(
        &self,
        _folder: Ref<IStorageFolder>,
    ) -> Result<IAsyncOperation<StorageFile>> {
        refuse("CopyAsync")
    }

    fn CopyOverloadDefaultOptions(
        &self,
        _folder: Ref<IStorageFolder>,
        _name: &HSTRING,
    ) -> Result<IAsyncOperation<StorageFile>> {
        refuse("CopyAsync")
    }

    fn CopyOverload(
        &self,
        _folder: Ref<IStorageFolder>,
        _name: &HSTRING,
        _option: NameCollisionOption,
    ) -> Result<IAsyncOperation<StorageFile>> {
        refuse("CopyAsync")
    }

    fn CopyAndReplaceAsync(&self, _target: Ref<IStorageFile>) -> Result<IAsyncAction> {
        refuse("CopyAndReplaceAsync")
    }

    fn MoveOverloadDefaultNameAndOptions(
        &self,
        _folder: Ref<IStorageFolder>,
    ) -> Result<IAsyncAction> {
        refuse("MoveAsync")
    }

    fn MoveOverloadDefaultOptions(
        &self,
        _folder: Ref<IStorageFolder>,
        _name: &HSTRING,
    ) -> Result<IAsyncAction> {
        refuse("MoveAsync")
    }

    fn MoveOverload(
        &self,
        _folder: Ref<IStorageFolder>,
        _name: &HSTRING,
        _option: NameCollisionOption,
    ) -> Result<IAsyncAction> {
        refuse("MoveAsync")
    }

    fn MoveAndReplaceAsync(&self, _target: Ref<IStorageFile>) -> Result<IAsyncAction> {
        refuse("MoveAndReplaceAsync")
    }
}

impl IInputStreamReference_Impl for PickedFile_Impl {
    fn OpenSequentialReadAsync(&self) -> Result<IAsyncOperation<IInputStream>> {
        refuse("OpenSequentialReadAsync")
    }
}

impl IRandomAccessStreamReference_Impl for PickedFile_Impl {
    fn OpenReadAsync(&self) -> Result<IAsyncOperation<IRandomAccessStreamWithContentType>> {
        refuse("OpenReadAsync")
    }
}

/// Records an unimplemented route off the picked file and refuses it.
///
/// Which of these a title reaches - if any - is the thing worth knowing: it is the difference
/// between "reads the path" and "needs a whole stream stack".
fn refuse<T>(method: &str) -> Result<T> {
    stub!("PickedFile::{method}");
    Err(windows_core::Error::from_hresult(super::E_NOTIMPL))
}

/// A MIME type for the extensions a save picker realistically produces, falling back to the
/// generic binary type. WinRT resolves this from the registry; there is no such registration in
/// a Wine prefix, and a wrong-but-plausible type is better than an empty one.
fn content_type(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "txt" | "log" => "text/plain",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "zip" | "mcworld" | "mcpack" | "mctemplate" => "application/zip",
        _ => "application/octet-stream",
    }
}

impl IStorageFile2_Impl for PickedFile_Impl {
    fn OpenWithOptionsAsync(
        &self,
        _mode: FileAccessMode,
        _options: StorageOpenOptions,
    ) -> Result<IAsyncOperation<windows::Storage::Streams::IRandomAccessStream>> {
        refuse("OpenWithOptionsAsync")
    }

    fn OpenTransactedWriteWithOptionsAsync(
        &self,
        _options: StorageOpenOptions,
    ) -> Result<IAsyncOperation<StorageStreamTransaction>> {
        refuse("OpenTransactedWriteWithOptionsAsync")
    }
}

impl IStorageItem2_Impl for PickedFile_Impl {
    fn GetParentAsync(&self) -> Result<IAsyncOperation<StorageFolder>> {
        refuse("GetParentAsync")
    }

    fn IsEqual(&self, item: Ref<IStorageItem>) -> Result<bool> {
        // Two storage items are the same item when they name the same path, which is a question
        // this can answer without any of the machinery the rest of the class would need.
        let other = item.ok().and_then(|item| item.Path()).ok();
        Ok(other.is_some_and(|other| Path::new(&other.to_string_lossy()) == self.path))
    }
}

impl IStorageItemProperties_Impl for PickedFile_Impl {
    fn DisplayName(&self) -> Result<HSTRING> {
        // WinRT's display name is the file name with its extension taken off.
        Ok(HSTRING::from(
            self.path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ))
    }

    fn DisplayType(&self) -> Result<HSTRING> {
        Ok(HSTRING::from("File"))
    }

    fn FolderRelativeId(&self) -> Result<HSTRING> {
        // A real one identifies the item within the folder the app was granted access to. There
        // is no such grant here - the file came from a dialog, not a library - so the path is
        // the only identifier that means anything.
        Ok(HSTRING::from(self.path.to_string_lossy().into_owned()))
    }

    fn Properties(&self) -> Result<StorageItemContentProperties> {
        refuse("Properties")
    }

    fn GetThumbnailAsyncOverloadDefaultSizeDefaultOptions(
        &self,
        _mode: ThumbnailMode,
    ) -> Result<IAsyncOperation<StorageItemThumbnail>> {
        refuse("GetThumbnailAsync")
    }

    fn GetThumbnailAsyncOverloadDefaultOptions(
        &self,
        _mode: ThumbnailMode,
        _size: u32,
    ) -> Result<IAsyncOperation<StorageItemThumbnail>> {
        refuse("GetThumbnailAsync")
    }

    fn GetThumbnailAsync(
        &self,
        _mode: ThumbnailMode,
        _size: u32,
        _options: ThumbnailOptions,
    ) -> Result<IAsyncOperation<StorageItemThumbnail>> {
        refuse("GetThumbnailAsync")
    }
}

impl IStorageItemProperties2_Impl for PickedFile_Impl {
    fn GetScaledImageAsThumbnailAsyncOverloadDefaultSizeDefaultOptions(
        &self,
        _mode: ThumbnailMode,
    ) -> Result<IAsyncOperation<StorageItemThumbnail>> {
        refuse("GetScaledImageAsThumbnailAsync")
    }

    fn GetScaledImageAsThumbnailAsyncOverloadDefaultOptions(
        &self,
        _mode: ThumbnailMode,
        _size: u32,
    ) -> Result<IAsyncOperation<StorageItemThumbnail>> {
        refuse("GetScaledImageAsThumbnailAsync")
    }

    fn GetScaledImageAsThumbnailAsync(
        &self,
        _mode: ThumbnailMode,
        _size: u32,
        _options: ThumbnailOptions,
    ) -> Result<IAsyncOperation<StorageItemThumbnail>> {
        refuse("GetScaledImageAsThumbnailAsync")
    }
}

impl IStorageItemPropertiesWithProvider_Impl for PickedFile_Impl {
    fn Provider(&self) -> Result<StorageProvider> {
        refuse("Provider")
    }
}

impl IStorageFilePropertiesWithAvailability_Impl for PickedFile_Impl {
    fn IsAvailable(&self) -> Result<bool> {
        // The file is on local disk by construction: a dialog cannot name anything else here.
        Ok(true)
    }
}
