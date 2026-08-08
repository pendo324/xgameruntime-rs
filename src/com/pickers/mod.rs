//! `Windows.Storage.Pickers`, served through WinRT activation.
//!
//! Wine ships no implementation of these classes, so a title that opens a save dialog - an
//! export, a screenshot, a world backup - gets a failed activation, and the WinRT projection
//! turns that into an unhandled exception that ends the process. This module answers the
//! activation and puts the shell's own save dialog behind it.
//!
//! Wine's `combase` resolves a runtime class through
//! `HKLM\Software\Microsoft\WindowsRuntime\ActivatableClassId\<class>`, reads `DllPath`, loads
//! that module and calls its `DllGetActivationFactory`. That key has to point at this DLL for
//! any of this to be reached; without it the lookup fails with `REGDB_E_CLASSNOTREG` first.
//!
//! Two class families live here. `Windows.Storage.Pickers` is the classic one, and only its save
//! picker is implemented - its open picker has a separate interface with its own result shape,
//! and nothing that runs here has asked for it. The Windows App SDK's own pickers, which a title
//! built against that SDK asks for instead, are in [`appsdk`].
//!
//! The classic interfaces are bound in [`bindings`] rather than taken from the `windows` crate.
//! That crate binds them too, but only to call through: it emits an `_Impl` trait only for
//! interfaces a caller may implement, and a picker's interface belongs exclusively to its class.
//!
//! [`storage_statics`] rides along: a title that picks a file to read turns the path it got back
//! into a `StorageFile`, and the class that does that is registered to a Wine module which does
//! not implement it, so it fails the same way a picker does.

use std::ffi::c_void;

use windows::activation::{IActivationFactory, IActivationFactory_Impl};
use windows_core::{
    GUID, HRESULT, HSTRING, IInspectable, IUnknown_Vtbl, Interface, Result, implement,
};

use crate::diag::stub;

mod appsdk;
mod async_op;
#[allow(
    dead_code,
    non_camel_case_types,
    non_upper_case_globals,
    clippy::undocumented_unsafe_blocks,
    clippy::missing_transmute_annotations
)]
mod bindings;
mod deferred;
mod dialog;
mod save_picker;
mod storage_file;
mod storage_statics;

const S_OK: HRESULT = HRESULT(0);
const E_FAIL: HRESULT = HRESULT(0x80004005u32 as i32);
const E_NOTIMPL: HRESULT = HRESULT(0x80004001u32 as i32);
const E_POINTER: HRESULT = HRESULT(0x80004003u32 as i32);
const E_NOINTERFACE: HRESULT = HRESULT(0x80004002u32 as i32);
const CLASS_E_CLASSNOTAVAILABLE: HRESULT = HRESULT(0x80040111u32 as i32);

/// `IAgileObject`, asked for by anything that marshals an object across apartments. Everything
/// here is thread-safe by construction, so claiming it is honest.
const IID_IAGILE_OBJECT: GUID = GUID::from_u128(0x94ea2b94_e9cc_49e0_c0ff_ee64ca8f5b90);

/// `IInspectable::GetIids`, which nothing observed here calls for anything but completeness.
///
/// # Safety
/// `count` and `values` must be writable out-parameters or null.
unsafe extern "system" fn spy_get_iids(
    _this: *mut c_void,
    count: *mut u32,
    values: *mut *mut GUID,
) -> HRESULT {
    // SAFETY: COM guarantees both out-parameters are writable for the duration of the call.
    unsafe {
        if !count.is_null() {
            *count = 0;
        }
        if !values.is_null() {
            *values = std::ptr::null_mut();
        }
    }
    S_OK
}

/// # Safety
/// `value` must be a writable `TrustLevel` out-parameter or null.
unsafe extern "system" fn spy_get_trust_level(_this: *mut c_void, value: *mut i32) -> HRESULT {
    // SAFETY: COM guarantees `value` is writable for the duration of the call.
    unsafe {
        if !value.is_null() {
            *value = 0; // BaseTrust
        }
    }
    S_OK
}

/// Releases a COM interface pointer through its own vtable.
///
/// # Safety
/// `this` must be a live COM interface pointer this caller holds a reference to.
unsafe fn com_release(this: *mut c_void) -> u32 {
    // SAFETY: guaranteed by this function's contract - slot 2 of any COM vtable is `Release`.
    unsafe {
        let vtable = *(this as *const *const IUnknown_Vtbl);
        ((*vtable).Release)(this)
    }
}

/// The classic class this module serves, spelled the way a title asks for it.
const FILE_SAVE_PICKER: &str = "Windows.Storage.Pickers.FileSavePicker";

/// The factory behind that class name.
///
/// It holds nothing - a save picker's state all arrives after construction - so a fresh one per
/// activation is as good as a shared one, and lets `#[implement]` do the reference counting.
#[implement(IActivationFactory)]
struct SavePickerFactory;

impl IActivationFactory_Impl for SavePickerFactory_Impl {
    fn ActivateInstance(&self) -> Result<IInspectable> {
        // The picker answers `IInspectable` itself; this is a query for it, not a conversion,
        // because the minimal bindings carry no class type to convert through.
        save_picker::SavePicker::create().cast()
    }
}

/// Returns the save picker's factory, with a reference the caller owns.
fn save_picker_factory() -> *mut c_void {
    let factory: IActivationFactory = SavePickerFactory.into();
    // SAFETY: `IActivationFactory` is a `repr(transparent)` interface pointer, and the reference
    // it holds passes to the caller rather than being dropped here.
    unsafe {
        let raw = std::mem::transmute_copy(&factory);
        std::mem::forget(factory);
        raw
    }
}

/// Serves the activation factories for the pickers, and logs every class asked for.
///
/// Classes this runtime has nothing to say about are refused with `CLASS_E_CLASSNOTAVAILABLE`
/// rather than served something half-built: Wine's own implementations of the classes it does
/// register are better than a stub, and this export sees those only if the registry sends them
/// here by mistake.
pub(crate) fn get_activation_factory(class_id: &HSTRING, factory: *mut *mut c_void) -> HRESULT {
    if factory.is_null() {
        return E_POINTER;
    }
    // SAFETY: `factory` was just null-checked and COM guarantees it is writable here.
    unsafe { *factory = std::ptr::null_mut() };

    let name = class_id.to_string_lossy();
    if let Some(served) = appsdk::activation_factory(&name) {
        // SAFETY: `factory` is non-null and writable, checked above; the reference the App SDK
        // factory carries passes to the caller.
        unsafe { *factory = served };
        return S_OK;
    }
    if name == storage_statics::STORAGE_FILE {
        // SAFETY: `factory` is non-null and writable, checked above; the reference the statics
        // object carries passes to the caller.
        unsafe { *factory = storage_statics::activation_factory() };
        return S_OK;
    }
    if name != FILE_SAVE_PICKER {
        stub!("DllGetActivationFactory({name:?}) -> CLASS_E_CLASSNOTAVAILABLE");
        return CLASS_E_CLASSNOTAVAILABLE;
    }

    // SAFETY: `factory` is non-null and writable, checked above; the reference the factory
    // carries passes to the caller.
    unsafe { *factory = save_picker_factory() };
    S_OK
}
