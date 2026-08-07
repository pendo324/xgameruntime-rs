//! `Microsoft.Windows.Storage.Pickers`, the Windows App SDK's pickers.
//!
//! These are a different class family from `Windows.Storage.Pickers`, not a newer version of it:
//! a different namespace, different interfaces, and a different result type - a `PickFileResult`
//! carrying a path, rather than a `StorageFile`. A title built against the App SDK asks for these
//! by name, and answering it with the classic classes is not possible, so both families are
//! served here.
//!
//! A picker in this family takes its owner window at construction, through a factory rather than
//! through `IInitializeWithWindow`: `RoActivateInstance` alone cannot make one. So the factory
//! answers `IFileOpenPickerFactory::CreateInstance(WindowId)` as well as `IActivationFactory`,
//! and the window travels with the picker from the moment it exists.
//!
//! Unlike the classic pickers, these interfaces come with generated `_Impl` traits - see
//! [`bindings`] - so the vtables come from `#[implement]` rather than by hand.
//!
//! Only the open picker is implemented. The save and folder pickers in this namespace are
//! separate classes, and nothing that runs here has asked for them.

use std::ffi::c_void;

use windows::activation::{IActivationFactory, IActivationFactory_Impl};
use windows_core::{IInspectable, Result, implement};

use self::bindings::{
    FileOpenPicker, IFileOpenPickerFactory, IFileOpenPickerFactory_Impl, WindowId,
};
use self::open_picker::OpenPicker;
use crate::diag::stub;

#[allow(
    dead_code,
    non_camel_case_types,
    non_upper_case_globals,
    clippy::undocumented_unsafe_blocks,
    clippy::missing_transmute_annotations
)]
mod bindings;
mod open_picker;
mod pick_result;

/// The class this module serves, spelled the way a title asks for it.
pub(super) const FILE_OPEN_PICKER: &str = "Microsoft.Windows.Storage.Pickers.FileOpenPicker";

/// The factory behind that class name.
///
/// It holds nothing: a picker's only construction-time state is the window it is given, and that
/// is handed to the picker rather than kept here. A fresh one per activation is therefore as good
/// as a shared one, and lets `#[implement]` do the reference counting.
#[implement(IActivationFactory, IFileOpenPickerFactory)]
struct OpenPickerFactory;

impl IActivationFactory_Impl for OpenPickerFactory_Impl {
    /// Activation with no window at all. The App SDK's own picker refuses this, but a picker
    /// without an owner still works here - the dialog is simply not modal to anything - and
    /// refusing would be the less useful answer.
    fn ActivateInstance(&self) -> Result<IInspectable> {
        stub!("FileOpenPickerFactory::ActivateInstance (no window)");
        Ok(OpenPicker::create(WindowId { Value: 0 }).into())
    }
}

impl IFileOpenPickerFactory_Impl for OpenPickerFactory_Impl {
    fn CreateInstance(&self, windowId: &WindowId) -> Result<FileOpenPicker> {
        stub!(
            "FileOpenPickerFactory::CreateInstance(window={:#x})",
            windowId.Value
        );
        Ok(OpenPicker::create(*windowId))
    }
}

/// Returns the activation factory for `name`, or `None` if this family has nothing by that name.
///
/// The pointer carries one reference, which passes to the caller.
pub(super) fn activation_factory(name: &str) -> Option<*mut c_void> {
    if name != FILE_OPEN_PICKER {
        return None;
    }
    let factory: IActivationFactory = OpenPickerFactory.into();
    // SAFETY: `IActivationFactory` is a `repr(transparent)` interface pointer, and the reference
    // it holds passes to the caller rather than being dropped here.
    unsafe {
        let raw = std::mem::transmute_copy(&factory);
        std::mem::forget(factory);
        Some(raw)
    }
}
