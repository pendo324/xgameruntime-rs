//! Shows the dialogs the pickers are a front end for.
//!
//! A picker's properties map almost one for one onto the shell's `IFileSaveDialog` and
//! `IFileOpenDialog`, which Wine implements: the file-type choices become `SetFileTypes`, the
//! suggested name becomes `SetFileName`, and the window the title handed over becomes the owner
//! `Show` is modal to. The dialogs also hand back a filesystem path directly, so nothing has to
//! translate between host and prefix paths.

use std::path::PathBuf;

use windows::combaseapi::{CoCreateInstance, CoInitializeEx, CoUninitialize};
use windows::objbase::COINIT_APARTMENTTHREADED;
use windows::shobjidl_core::{
    FileOpenDialog, FileSaveDialog, IFileOpenDialog, IFileSaveDialog, IShellItem, SIGDN_FILESYSPATH,
};
use windows::shtypes::COMDLG_FILTERSPEC;
use windows::windef::HWND;
use windows::wtypesbase::CLSCTX_INPROC_SERVER;
use windows_core::{HRESULT, HSTRING, PCWSTR};

use crate::diag::stub;

/// `HRESULT_FROM_WIN32(ERROR_CANCELLED)`, which is how the shell reports that the user dismissed
/// the dialog rather than that anything went wrong.
const HRESULT_ERROR_CANCELLED: HRESULT = HRESULT(0x800704C7u32 as i32);

/// What a title asked for, flattened out of the picker's live COM state so it can cross to the
/// thread that shows the dialog.
pub(super) struct SaveRequest {
    pub(super) suggested_name: String,
    pub(super) default_extension: String,
    pub(super) commit_button_text: String,
    /// Display name and its extensions, in the order the title inserted them.
    pub(super) file_types: Vec<(String, Vec<String>)>,
    /// The owner window, as a bare address so the request can move between threads.
    pub(super) owner: isize,
}

/// What a title asked for when it wants to read a file rather than write one.
///
/// The App SDK's open picker filters on bare extensions rather than on named groups, so the
/// types here are a flat list and the dialog names the group itself.
pub(super) struct OpenRequest {
    pub(super) commit_button_text: String,
    /// Extensions, each with its leading dot, in the order the title added them.
    pub(super) file_types: Vec<String>,
    /// The owner window, as a bare address so the request can move between threads.
    pub(super) owner: isize,
}

/// Shows the save dialog and returns the chosen path, or `None` if the user cancelled.
pub(super) fn show_save_dialog(request: SaveRequest) -> Result<Option<PathBuf>, HRESULT> {
    in_apartment(|| run_save_dialog(&request))
}

/// Shows the open dialog and returns the chosen path, or `None` if the user cancelled.
pub(super) fn show_open_dialog(request: OpenRequest) -> Result<Option<PathBuf>, HRESULT> {
    in_apartment(|| run_open_dialog(&request))
}

/// Runs `show` with an apartment the shell dialogs can live in.
///
/// This runs on the thread that started the pick, and blocks it until the user is done. That is
/// not how a WinRT picker behaves, but it is the only shape that works here: a completion handler
/// invoked from any other thread tries to marshal itself back to the apartment it was created in,
/// and the context-switching call that requires is not implemented in this runtime - the attempt
/// fails and takes the title with it. Completing on the calling thread means the handler is
/// already where it wants to be. The dialog is modal to the window the title handed over anyway,
/// so blocking that thread is what a caller would see either way.
fn in_apartment(
    show: impl FnOnce() -> Result<Option<PathBuf>, HRESULT>,
) -> Result<Option<PathBuf>, HRESULT> {
    // The calling thread already has an apartment, so this usually reports that rather than
    // creating one. The dialogs want a single-threaded apartment; if the caller is in a
    // multi-threaded one this reports `RPC_E_CHANGED_MODE`, and the dialog is created in the
    // apartment the thread already has instead of a new one.
    // SAFETY: balanced by the `CoUninitialize` below on the same thread whenever it succeeded.
    let apartment = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED as u32) };
    let result = show();
    if apartment.is_ok() {
        // SAFETY: balances the `CoInitializeEx` above, on the same thread.
        unsafe { CoUninitialize() };
    }
    result
}

fn run_save_dialog(request: &SaveRequest) -> Result<Option<PathBuf>, HRESULT> {
    // SAFETY: every call below is a COM method on an interface this function created and still
    // owns, on a thread whose apartment was initialized by the caller.
    unsafe {
        let dialog: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)
            .map_err(|err| {
                stub!("show_save_dialog: no FileSaveDialog: {:?}", err.code());
                err.code()
            })?;

        // The filter strings have to outlive the `SetFileTypes` call, so they are built into
        // owned `HSTRING`s that stay alive for the rest of this scope and pointed at from the
        // specs the shell reads.
        let filters: Vec<(HSTRING, HSTRING)> = request
            .file_types
            .iter()
            .map(|(name, extensions)| {
                let pattern = extensions
                    .iter()
                    .map(|extension| format!("*{extension}"))
                    .collect::<Vec<_>>()
                    .join(";");
                (HSTRING::from(name.as_str()), HSTRING::from(pattern))
            })
            .collect();
        let specs: Vec<COMDLG_FILTERSPEC> = filters
            .iter()
            .map(|(name, pattern)| COMDLG_FILTERSPEC {
                pszName: PCWSTR::from_raw(name.as_ptr()),
                pszSpec: PCWSTR::from_raw(pattern.as_ptr()),
            })
            .collect();
        if !specs.is_empty() {
            dialog
                .SetFileTypes(specs.len() as u32, specs.as_ptr())
                .ok()
                .map_err(|err| err.code())?;
        }

        if !request.suggested_name.is_empty() {
            let name = HSTRING::from(request.suggested_name.as_str());
            let _ = dialog.SetFileName(PCWSTR::from_raw(name.as_ptr()));
        }
        if !request.default_extension.is_empty() {
            // The shell wants the extension without its leading dot, unlike WinRT.
            let extension = HSTRING::from(request.default_extension.trim_start_matches('.'));
            let _ = dialog.SetDefaultExtension(PCWSTR::from_raw(extension.as_ptr()));
        }
        if !request.commit_button_text.is_empty() {
            let label = HSTRING::from(request.commit_button_text.as_str());
            let _ = dialog.SetOkButtonLabel(PCWSTR::from_raw(label.as_ptr()));
        }

        let owner = (request.owner != 0).then_some(HWND(request.owner as *mut _));
        let shown = dialog.Show(owner);
        if shown == HRESULT_ERROR_CANCELLED {
            stub!("show_save_dialog: cancelled");
            return Ok(None);
        }
        shown.ok().map_err(|err| err.code())?;

        let item = dialog.GetResult().map_err(|err| err.code())?;
        let picked = item_path(&item)?;
        stub!("show_save_dialog: picked {picked:?}");
        Ok(Some(picked))
    }
}

fn run_open_dialog(request: &OpenRequest) -> Result<Option<PathBuf>, HRESULT> {
    // SAFETY: every call below is a COM method on an interface this function created and still
    // owns, on a thread whose apartment was initialized by the caller.
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
            .map_err(|err| {
                stub!("show_open_dialog: no FileOpenDialog: {:?}", err.code());
                err.code()
            })?;

        // One group covering everything the title asked for, plus a way out of it. The App SDK
        // picker gives the extensions no display name, so the dialog supplies one - and a title
        // that filters on nothing at all gets an unfiltered dialog rather than an empty group.
        // The strings have to outlive `SetFileTypes`, so they stay owned for the rest of this
        // scope and the specs the shell reads point into them.
        let pattern = HSTRING::from(
            request
                .file_types
                .iter()
                .map(|extension| format!("*{extension}"))
                .collect::<Vec<_>>()
                .join(";"),
        );
        let supported = HSTRING::from("Supported files");
        let all = (HSTRING::from("All files"), HSTRING::from("*.*"));
        let mut specs = Vec::new();
        if !request.file_types.is_empty() {
            specs.push(COMDLG_FILTERSPEC {
                pszName: PCWSTR::from_raw(supported.as_ptr()),
                pszSpec: PCWSTR::from_raw(pattern.as_ptr()),
            });
        }
        specs.push(COMDLG_FILTERSPEC {
            pszName: PCWSTR::from_raw(all.0.as_ptr()),
            pszSpec: PCWSTR::from_raw(all.1.as_ptr()),
        });
        dialog
            .SetFileTypes(specs.len() as u32, specs.as_ptr())
            .ok()
            .map_err(|err| err.code())?;

        if !request.commit_button_text.is_empty() {
            let label = HSTRING::from(request.commit_button_text.as_str());
            let _ = dialog.SetOkButtonLabel(PCWSTR::from_raw(label.as_ptr()));
        }

        let owner = (request.owner != 0).then_some(HWND(request.owner as *mut _));
        let shown = dialog.Show(owner);
        if shown == HRESULT_ERROR_CANCELLED {
            stub!("show_open_dialog: cancelled");
            return Ok(None);
        }
        shown.ok().map_err(|err| err.code())?;

        let item = dialog.GetResult().map_err(|err| err.code())?;
        let picked = item_path(&item)?;
        stub!("show_open_dialog: picked {picked:?}");
        Ok(Some(picked))
    }
}

/// Reads the filesystem path out of what a dialog handed back.
fn item_path(item: &IShellItem) -> Result<PathBuf, HRESULT> {
    // SAFETY: `item` is a live shell item the caller owns, and the string it returns is freed
    // here, which is the contract `GetDisplayName` documents.
    unsafe {
        let path = item
            .GetDisplayName(SIGDN_FILESYSPATH)
            .map_err(|err| err.code())?;
        let picked = PathBuf::from(path.to_string().map_err(|_| super::E_FAIL)?);
        windows::combaseapi::CoTaskMemFree(path.0 as *mut _);
        Ok(picked)
    }
}
