//! `Windows.Storage.Pickers.FileSavePicker`.
//!
//! The vtable is written out by hand because `windows-rs` generates no `_Impl` trait for the
//! picker interfaces - only the raw `*_Vtbl` structs - so there is no `#[implement]` shortcut.
//!
//! The object carries two vtables. A packaged app's picker takes its owner window from the app
//! view, but a Win32 host has none, so it hands one over through `IInitializeWithWindow`
//! instead - and every title seen here asks for that interface before it touches a property.
//! Both vtables live in the same allocation and share one reference count, which is what COM's
//! identity rule requires.

use std::ffi::c_void;
use std::mem::{ManuallyDrop, offset_of};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use windows::Storage::Pickers::{IFileSavePicker, IFileSavePicker_Vtbl, PickerLocationId};
use windows::shobjidl_core::{IInitializeWithWindow, IInitializeWithWindow_Vtbl};
use windows::windef::HWND;
use windows_collections::{IMap, IVector};
use windows_core::{GUID, HRESULT, HSTRING, IInspectable_Vtbl, IUnknown_Vtbl, Interface};

use super::async_op::SaveOperation;
use super::dialog::SaveRequest;
use super::{E_NOINTERFACE, E_POINTER, IID_IAGILE_OBJECT, S_OK, spy_get_iids, spy_get_trust_level};
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

#[repr(C)]
pub(super) struct FileSavePickerObject {
    picker_vtable: &'static IFileSavePicker_Vtbl,
    init_vtable: &'static IInitializeWithWindow_Vtbl,
    refs: AtomicU32,
    state: Mutex<PickerState>,
    /// The file-type map, created once and handed out by reference: a title inserts its types
    /// into whatever `FileTypeChoices` returns, so returning a fresh map would discard them.
    choices: IMap<HSTRING, IVector<HSTRING>>,
}

impl FileSavePickerObject {
    pub(super) fn create() -> *mut c_void {
        let object = Box::new(FileSavePickerObject {
            picker_vtable: &PICKER_VTABLE,
            init_vtable: &INITIALIZE_VTABLE,
            refs: AtomicU32::new(1),
            state: Mutex::new(PickerState {
                settings_identifier: HSTRING::new(),
                suggested_start_location: PickerLocationId::DocumentsLibrary,
                commit_button_text: HSTRING::new(),
                default_file_extension: HSTRING::new(),
                suggested_file_name: HSTRING::new(),
                owner: 0,
            }),
            choices: IMap::from(std::collections::BTreeMap::new()),
        });
        Box::into_raw(object).cast()
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

/// # Safety
/// `this` must be a live picker seen through its `IFileSavePicker` vtable.
unsafe fn picker<'a>(this: *mut c_void) -> &'a FileSavePickerObject {
    // SAFETY: guaranteed by this function's contract.
    unsafe { &*this.cast::<FileSavePickerObject>() }
}

/// # Safety
/// `this` must be a live picker seen through its `IInitializeWithWindow` vtable.
unsafe fn picker_from_init<'a>(this: *mut c_void) -> &'a FileSavePickerObject {
    // SAFETY: `this` addresses the `init_vtable` field, so the object starts at that offset back.
    unsafe {
        &*this
            .cast::<u8>()
            .sub(offset_of!(FileSavePickerObject, init_vtable))
            .cast::<FileSavePickerObject>()
    }
}

/// Reads an `HSTRING` argument without taking ownership of it: the caller still owns the string
/// it passed, and dropping it here would free a string it goes on to use.
///
/// # Safety
/// `value` must be a valid `HSTRING` or null.
unsafe fn borrowed_hstring(value: *mut c_void) -> HSTRING {
    if value.is_null() {
        return HSTRING::new();
    }
    // SAFETY: guaranteed by this function's contract.
    unsafe { (*ManuallyDrop::new(std::mem::transmute::<*mut c_void, HSTRING>(value))).clone() }
}

/// Hands an `HSTRING` to a caller that will free it.
///
/// # Safety
/// `out` must be a writable out-parameter.
unsafe fn write_hstring(out: *mut *mut c_void, value: HSTRING) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: `out` was just null-checked; ownership of the string passes to the caller.
    unsafe { *out = std::mem::transmute::<HSTRING, *mut c_void>(value) };
    S_OK
}

/// A property backed by a string, in the two halves WinRT splits it into.
macro_rules! string_property {
    ($get:ident, $set:ident, $field:ident) => {
        /// # Safety
        /// `this` must be a live picker and `out` a writable out-parameter.
        unsafe extern "system" fn $get(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
            // SAFETY: guaranteed by this function's contract.
            unsafe {
                let value = picker(this)
                    .state
                    .lock()
                    .expect("picker state poisoned")
                    .$field
                    .clone();
                write_hstring(out, value)
            }
        }

        /// # Safety
        /// `this` must be a live picker and `value` a valid `HSTRING` or null.
        unsafe extern "system" fn $set(this: *mut c_void, value: *mut c_void) -> HRESULT {
            // SAFETY: guaranteed by this function's contract.
            unsafe {
                let value = borrowed_hstring(value);
                stub!("FileSavePicker::{}({value:?})", stringify!($set));
                picker(this)
                    .state
                    .lock()
                    .expect("picker state poisoned")
                    .$field = value;
            }
            S_OK
        }
    };
}

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

/// # Safety
/// `this` must be a live picker and `out` a writable out-parameter.
unsafe extern "system" fn SuggestedStartLocation(
    this: *mut c_void,
    out: *mut PickerLocationId,
) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: `out` was just null-checked; `this` is live per the vtable contract.
    unsafe {
        *out = picker(this)
            .state
            .lock()
            .expect("picker state poisoned")
            .suggested_start_location;
    }
    S_OK
}

/// # Safety
/// `this` must be a live picker.
unsafe extern "system" fn SetSuggestedStartLocation(
    this: *mut c_void,
    value: PickerLocationId,
) -> HRESULT {
    // SAFETY: guaranteed by this function's contract.
    unsafe {
        picker(this)
            .state
            .lock()
            .expect("picker state poisoned")
            .suggested_start_location = value;
    }
    S_OK
}

/// # Safety
/// `this` must be a live picker and `out` a writable out-parameter.
unsafe extern "system" fn FileTypeChoices(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: `out` was just null-checked; `this` is live per the vtable contract. The map is
    // cloned rather than moved, so the picker keeps holding the one the title writes into.
    unsafe {
        let choices = picker(this).choices.clone();
        *out = std::mem::transmute_copy(&choices);
        std::mem::forget(choices);
    }
    S_OK
}

/// The save picker's `SuggestedSaveFile`, which nothing here tracks: it takes a `StorageFile`
/// that would have to have come from a picker this runtime does not implement.
///
/// # Safety
/// `out` must be a writable out-parameter.
unsafe extern "system" fn SuggestedSaveFile(_this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: `out` was just null-checked; an unset reference is spelled null.
    unsafe { *out = std::ptr::null_mut() };
    S_OK
}

/// # Safety
/// `this` must be a live picker.
unsafe extern "system" fn SetSuggestedSaveFile(_this: *mut c_void, _value: *mut c_void) -> HRESULT {
    stub!("FileSavePicker::SetSuggestedSaveFile (ignored)");
    S_OK
}

/// # Safety
/// `this` must be a live picker and `out` a writable out-parameter.
unsafe extern "system" fn PickSaveFileAsync(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: `out` was just null-checked; `this` is live per the vtable contract.
    unsafe {
        let request = picker(this).to_request();
        stub!(
            "FileSavePicker::PickSaveFileAsync name={:?} types={}",
            request.suggested_name,
            request.file_types.len()
        );
        *out = SaveOperation::start(request);
    }
    S_OK
}

/// # Safety
/// `this` must be a live picker seen through its `IInitializeWithWindow` vtable.
unsafe extern "system" fn initialize_with_window(this: *mut c_void, hwnd: HWND) -> HRESULT {
    // SAFETY: guaranteed by this function's contract.
    unsafe {
        stub!("FileSavePicker::Initialize(hwnd={:?})", hwnd.0);
        picker_from_init(this)
            .state
            .lock()
            .expect("picker state poisoned")
            .owner = hwnd.0 as isize;
    }
    S_OK
}

unsafe extern "system" fn picker_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    interface: *mut *mut c_void,
) -> HRESULT {
    if iid.is_null() || interface.is_null() {
        return E_POINTER;
    }
    // SAFETY: both pointers were just null-checked, and `this` is live per the vtable contract.
    unsafe {
        let requested = *iid;
        let object = picker(this);
        if requested == IInitializeWithWindow::IID {
            object.refs.fetch_add(1, Ordering::Relaxed);
            *interface = (&object.init_vtable) as *const _ as *mut c_void;
            return S_OK;
        }
        let known = requested == windows_core::IUnknown::IID
            || requested == windows_core::IInspectable::IID
            || requested == IFileSavePicker::IID
            || requested == IID_IAGILE_OBJECT;
        if known {
            object.refs.fetch_add(1, Ordering::Relaxed);
            *interface = this;
            S_OK
        } else {
            stub!("FileSavePicker::QueryInterface({requested:?}) -> E_NOINTERFACE");
            *interface = std::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn init_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    interface: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: resolved back to the object, whose QueryInterface shares this contract.
    unsafe {
        let object = picker_from_init(this);
        picker_query_interface(object as *const _ as *mut c_void, iid, interface)
    }
}

unsafe extern "system" fn picker_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live picker per the vtable contract.
    unsafe { picker(this) }.refs.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn picker_release(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live picker per the vtable contract.
    let remaining = unsafe { picker(this) }.refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if remaining == 0 {
        // SAFETY: the count reached zero, so no method can still be running against it.
        drop(unsafe { Box::from_raw(this.cast::<FileSavePickerObject>()) });
    }
    remaining
}

unsafe extern "system" fn init_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: resolved back to the object, whose AddRef shares this contract.
    unsafe { picker_add_ref(picker_from_init(this) as *const _ as *mut c_void) }
}

unsafe extern "system" fn init_release(this: *mut c_void) -> u32 {
    // SAFETY: resolved back to the object, whose Release shares this contract.
    unsafe { picker_release(picker_from_init(this) as *const _ as *mut c_void) }
}

unsafe extern "system" fn picker_runtime_class_name(
    _this: *mut c_void,
    value: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: forwarded to the shared helper, which null-checks `value` itself.
    unsafe {
        write_hstring(
            value,
            HSTRING::from("Windows.Storage.Pickers.FileSavePicker"),
        )
    }
}

static PICKER_VTABLE: IFileSavePicker_Vtbl = IFileSavePicker_Vtbl {
    base__: IInspectable_Vtbl {
        base: IUnknown_Vtbl {
            QueryInterface: picker_query_interface,
            AddRef: picker_add_ref,
            Release: picker_release,
        },
        GetIids: spy_get_iids,
        GetRuntimeClassName: picker_runtime_class_name,
        GetTrustLevel: spy_get_trust_level,
    },
    SettingsIdentifier,
    SetSettingsIdentifier,
    SuggestedStartLocation,
    SetSuggestedStartLocation,
    CommitButtonText,
    SetCommitButtonText,
    FileTypeChoices,
    DefaultFileExtension,
    SetDefaultFileExtension,
    SuggestedSaveFile,
    SetSuggestedSaveFile,
    SuggestedFileName,
    SetSuggestedFileName,
    PickSaveFileAsync,
};

static INITIALIZE_VTABLE: IInitializeWithWindow_Vtbl = IInitializeWithWindow_Vtbl {
    base__: IUnknown_Vtbl {
        QueryInterface: init_query_interface,
        AddRef: init_add_ref,
        Release: init_release,
    },
    Initialize: initialize_with_window,
};
