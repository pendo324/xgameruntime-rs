//! `Windows.Storage.Pickers.FileSavePicker`.
//!
//! A packaged app's picker takes its owner window from the app view, but a Win32 host has none,
//! so it hands one over through `IInitializeWithWindow` instead - and every title seen here asks
//! for that interface before it touches a property. Naming both interfaces on the one object is
//! what gives them the shared identity and reference count COM requires.

use std::sync::Mutex;

use windows::shobjidl_core::{IInitializeWithWindow, IInitializeWithWindow_Impl};
use windows::windef::HWND;
use windows_collections::{IMap, IVector};
use windows_core::{HSTRING, Ref, Result, implement};
use windows_future::IAsyncOperation;

use super::async_op::PickOperation;
use super::bindings::{IFileSavePicker, IFileSavePicker_Impl, PickerLocationId, StorageFile};
use super::dialog::{SaveRequest, show_save_dialog};
use super::storage_file::PickedFile;
use crate::diag::stub;

/// The properties a title sets before it picks.
struct PickerState {
    settings_identifier: HSTRING,
    suggested_start_location: PickerLocationId,
    commit_button_text: HSTRING,
    default_file_extension: HSTRING,
    suggested_file_name: HSTRING,
    /// The owner window from `IInitializeWithWindow`, kept as a bare address so the pick request
    /// built from this state can move to the thread that shows the dialog.
    owner: isize,
}

#[implement(IFileSavePicker, IInitializeWithWindow)]
pub(super) struct SavePicker {
    state: Mutex<PickerState>,
    /// The file-type map, created once and handed out by reference: a title inserts its types
    /// into whatever `FileTypeChoices` returns, so returning a fresh map would discard them.
    choices: IMap<HSTRING, IVector<HSTRING>>,
}

impl SavePicker {
    /// Builds a picker and hands back the reference the caller owns.
    pub(super) fn create() -> IFileSavePicker {
        SavePicker {
            state: Mutex::new(PickerState {
                settings_identifier: HSTRING::new(),
                suggested_start_location: PickerLocationId::DocumentsLibrary,
                commit_button_text: HSTRING::new(),
                default_file_extension: HSTRING::new(),
                suggested_file_name: HSTRING::new(),
                owner: 0,
            }),
            choices: IMap::from(std::collections::BTreeMap::new()),
        }
        .into()
    }

    /// Flattens the picker's live COM state into something the dialog thread can own.
    ///
    /// The file types are read here, on the calling thread, rather than being handed over as a
    /// map: the map is the title's object as much as ours, and reading it from another thread
    /// would mean marshalling it there.
    fn to_request(&self) -> SaveRequest {
        let state = self.state.lock().expect("picker state poisoned");
        SaveRequest {
            suggested_name: state.suggested_file_name.to_string_lossy(),
            default_extension: state.default_file_extension.to_string_lossy(),
            commit_button_text: state.commit_button_text.to_string_lossy(),
            file_types: self.read_file_types(),
            owner: state.owner,
        }
    }

    /// Reads the file-type choices out of the map, skipping any entry that cannot be read rather
    /// than failing the pick: a dialog with fewer filters is still a usable dialog.
    fn read_file_types(&self) -> Vec<(String, Vec<String>)> {
        let mut types = Vec::new();
        let Ok(iterator) = self.choices.First() else {
            return types;
        };
        while iterator.HasCurrent().unwrap_or(false) {
            if let Ok(entry) = iterator.Current()
                && let (Ok(name), Ok(extensions)) = (entry.Key(), entry.Value())
            {
                let count = extensions.Size().unwrap_or(0);
                let extensions: Vec<String> = (0..count)
                    .filter_map(|index| extensions.GetAt(index).ok())
                    .map(|extension| extension.to_string_lossy())
                    .collect();
                types.push((name.to_string_lossy(), extensions));
            }
            if iterator.MoveNext().is_err() {
                break;
            }
        }
        types
    }
}

/// A property backed by a string, in the two halves WinRT splits it into.
macro_rules! string_property {
    ($get:ident, $set:ident, $field:ident) => {
        fn $get(&self) -> Result<HSTRING> {
            Ok(self
                .state
                .lock()
                .expect("picker state poisoned")
                .$field
                .clone())
        }

        fn $set(&self, value: &HSTRING) -> Result<()> {
            stub!("FileSavePicker::{}({value:?})", stringify!($set));
            self.state.lock().expect("picker state poisoned").$field = value.clone();
            Ok(())
        }
    };
}

impl IFileSavePicker_Impl for SavePicker_Impl {
    string_property!(
        SettingsIdentifier,
        SetSettingsIdentifier,
        settings_identifier
    );
    string_property!(CommitButtonText, SetCommitButtonText, commit_button_text);
    string_property!(
        DefaultFileExtension,
        SetDefaultFileExtension,
        default_file_extension
    );
    string_property!(SuggestedFileName, SetSuggestedFileName, suggested_file_name);

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

    fn FileTypeChoices(&self) -> Result<IMap<HSTRING, IVector<HSTRING>>> {
        // Cloned rather than moved, so the picker keeps holding the one the title writes into.
        Ok(self.choices.clone())
    }

    /// The save picker's `SuggestedSaveFile`, which nothing here tracks: it takes a `StorageFile`
    /// that would have to have come from a picker this runtime does not implement.
    ///
    /// WinRT would report an unset reference as a null result, which is expressible but not
    /// obviously worth expressing: nothing observed reads this property, and a failure says the
    /// same thing more loudly if something starts to.
    fn SuggestedSaveFile(&self) -> Result<StorageFile> {
        stub!("FileSavePicker::SuggestedSaveFile (unset)");
        Err(super::E_FAIL.into())
    }

    fn SetSuggestedSaveFile(&self, _value: Ref<StorageFile>) -> Result<()> {
        stub!("FileSavePicker::SetSuggestedSaveFile (ignored)");
        Ok(())
    }

    fn PickSaveFileAsync(&self) -> Result<IAsyncOperation<StorageFile>> {
        let request = self.to_request();
        stub!(
            "FileSavePicker::PickSaveFileAsync name={:?} types={}",
            request.suggested_name,
            request.file_types.len()
        );
        Ok(PickOperation::<PickedFile>::start(Box::new(move || {
            show_save_dialog(request)
        })))
    }
}

impl IInitializeWithWindow_Impl for SavePicker_Impl {
    fn Initialize(&self, hwnd: HWND) -> Result<()> {
        stub!("FileSavePicker::Initialize(hwnd={:?})", hwnd.0);
        self.state.lock().expect("picker state poisoned").owner = hwnd.0 as isize;
        Ok(())
    }
}
