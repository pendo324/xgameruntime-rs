//! The `PickFileResult` a completed App SDK pick hands back.
//!
//! This is the whole reason this family is cheaper to serve than the classic one: where a save
//! pick has to produce a `StorageFile` with its dozen interfaces, an App SDK pick produces an
//! object with a single read-only `Path`. A title that wants the file's contents opens the path
//! itself.

use std::path::PathBuf;

use windows_core::{HSTRING, Result, implement};

use super::bindings::{IPickFileResult, IPickFileResult_Impl, PickFileResult};
use crate::com::pickers::async_op::PickOutcome;

/// A picked path, presented to a title as the App SDK's `PickFileResult`.
#[implement(IPickFileResult)]
pub(super) struct PickedPath {
    path: PathBuf,
}

impl PickedPath {
    /// Wraps `path` and hands back the `PickFileResult` a caller expects.
    ///
    /// `PickFileResult` is a runtime class, and this is not an instance of it - but at the ABI
    /// there is no difference between the two: both are one pointer, and the only way to reach
    /// the result is through `IPickFileResult`, which this does implement.
    fn create(path: PathBuf) -> PickFileResult {
        let result: IPickFileResult = PickedPath { path }.into();
        // SAFETY: `PickFileResult` and `IPickFileResult` are both `repr(transparent)` wrappers
        // around a single interface pointer, and the pointer being wrapped implements
        // `IPickFileResult`.
        unsafe { std::mem::transmute::<IPickFileResult, PickFileResult>(result) }
    }
}

impl IPickFileResult_Impl for PickedPath_Impl {
    fn Path(&self) -> Result<HSTRING> {
        Ok(HSTRING::from(self.path.as_os_str()))
    }
}

/// What the open picker's operation completes with.
impl PickOutcome for PickedPath {
    const LABEL: &'static str = "PickFileResultOperation";
    const RUNTIME_CLASS_NAME: &'static str = concat!(
        "Windows.Foundation.IAsyncOperation`1<",
        "Microsoft.Windows.Storage.Pickers.PickFileResult>"
    );
    type Value = PickFileResult;

    fn create_result(path: PathBuf) -> Self::Value {
        PickedPath::create(path)
    }
}
