//! `Microsoft.Windows.Storage.Pickers.FileOpenPicker`.

use std::ffi::c_void;
use std::sync::Mutex;

use windows_collections::IVector;
use windows_core::{HSTRING, Result, implement};
use windows_future::IAsyncOperation;

use super::bindings::WindowId;
use super::bindings::{
    FileOpenPicker, IFileOpenPicker, IFileOpenPicker_Impl, PickFileResult, PickerLocationId,
    PickerViewMode,
};
use super::pick_result::PickedPath;
use crate::com::pickers::async_op::PickOperation;
use crate::com::pickers::dialog::{OpenRequest, show_open_dialog};
use crate::diag::stub;

/// The properties a title sets before it picks.
struct PickerState {
    view_mode: PickerViewMode,
    suggested_start_location: PickerLocationId,
    commit_button_text: HSTRING,
}

#[implement(IFileOpenPicker)]
pub(super) struct OpenPicker {
    /// The owner window this picker was constructed with, kept as a bare address so the pick
    /// request built from it can move to the thread that shows the dialog.
    owner: isize,
    state: Mutex<PickerState>,
    /// The extension list, created once and handed out by reference: a title adds its extensions
    /// to whatever `FileTypeFilter` returns, so returning a fresh vector would discard them.
    filter: IVector<HSTRING>,
}

impl OpenPicker {
    pub(super) fn create(window: WindowId) -> FileOpenPicker {
        let picker: IFileOpenPicker = OpenPicker {
            owner: window.Value as isize,
            state: Mutex::new(PickerState {
                view_mode: PickerViewMode::List,
                suggested_start_location: PickerLocationId::DocumentsLibrary,
                commit_button_text: HSTRING::new(),
            }),
            filter: IVector::from(Vec::new()),
        }
        .into();
        // SAFETY: `FileOpenPicker` and `IFileOpenPicker` are both `repr(transparent)` wrappers
        // around a single interface pointer, and the pointer being wrapped implements
        // `IFileOpenPicker`.
        unsafe { std::mem::transmute::<IFileOpenPicker, FileOpenPicker>(picker) }
    }
}

impl IFileOpenPicker_Impl for OpenPicker_Impl {
    fn ViewMode(&self) -> Result<PickerViewMode> {
        Ok(self.state.lock().expect("picker state poisoned").view_mode)
    }

    /// The shell dialog decides its own view, and it remembers what the user last chose there.
    /// Overriding that would be worse than honouring the request.
    fn SetViewMode(&self, value: PickerViewMode) -> Result<()> {
        self.state.lock().expect("picker state poisoned").view_mode = value;
        Ok(())
    }

    fn SuggestedStartLocation(&self) -> Result<PickerLocationId> {
        Ok(self
            .state
            .lock()
            .expect("picker state poisoned")
            .suggested_start_location)
    }

    fn SetSuggestedStartLocation(&self, value: PickerLocationId) -> Result<()> {
        self.state
            .lock()
            .expect("picker state poisoned")
            .suggested_start_location = value;
        Ok(())
    }

    fn CommitButtonText(&self) -> Result<HSTRING> {
        Ok(self
            .state
            .lock()
            .expect("picker state poisoned")
            .commit_button_text
            .clone())
    }

    fn SetCommitButtonText(&self, value: &HSTRING) -> Result<()> {
        self.state
            .lock()
            .expect("picker state poisoned")
            .commit_button_text = value.clone();
        Ok(())
    }

    fn FileTypeFilter(&self) -> Result<IVector<HSTRING>> {
        Ok(self.filter.clone())
    }

    fn PickSingleFileAsync(&self) -> Result<IAsyncOperation<PickFileResult>> {
        let request = OpenRequest {
            commit_button_text: self
                .state
                .lock()
                .expect("picker state poisoned")
                .commit_button_text
                .to_string_lossy(),
            file_types: read_file_types(&self.filter),
            owner: self.owner,
        };
        stub!(
            "FileOpenPicker::PickSingleFileAsync types={}",
            request.file_types.len()
        );

        let operation =
            PickOperation::<PickedPath>::start(Box::new(move || show_open_dialog(request)));
        // SAFETY: `IAsyncOperation` is a `repr(transparent)` interface pointer; `start` returns a
        // non-null one carrying the reference that passes to the caller here.
        Ok(unsafe {
            std::mem::transmute::<*mut c_void, IAsyncOperation<PickFileResult>>(operation)
        })
    }

    /// Picking several files at once, which no title seen here does. The dialog can do it, but
    /// the result is an `IVectorView<PickFileResult>` and the operation that carries one is a
    /// third result type; there is no reason to write it before something asks.
    fn PickMultipleFilesAsync(
        &self,
    ) -> Result<IAsyncOperation<windows_collections::IVectorView<PickFileResult>>> {
        stub!("FileOpenPicker::PickMultipleFilesAsync -> E_NOTIMPL");
        Err(crate::com::pickers::E_NOTIMPL.into())
    }
}

/// Reads the extensions out of the filter, skipping any entry that cannot be read rather than
/// failing the pick: a dialog with fewer filters is still a usable dialog.
fn read_file_types(filter: &IVector<HSTRING>) -> Vec<String> {
    let count = filter.Size().unwrap_or(0);
    (0..count)
        .filter_map(|index| filter.GetAt(index).ok())
        .map(|extension| extension.to_string_lossy())
        .collect()
}
