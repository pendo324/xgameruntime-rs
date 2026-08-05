use super::E_NOTIMPL;
use crate::results::*;
use std::ffi::c_char;
use std::sync::OnceLock;
use windows_core::{GUID, HRESULT, IUnknown, implement, interface};
/// `IXGameImpl`'s own IID, reused as the coclass id (same pattern as `CLSID_XSYSTEM`) - the
/// game/title identity interface. XSAPI (via `XblInitialize`, statically linked into GDK
/// titles like Minecraft Bedrock) reads the title id here to scope its Xbox Live requests;
/// features that check "is this a genuine signed-in Microsoft account" (distinct from a
/// PlayFab-only session, which needs no title identity) appear to depend on it. Confirmed via
/// Wine trace logs as one of the classes this title queries and previously got `E_NOTIMPL` for.
pub const CLSID_XGAME: GUID = GUID::from_u128(0x973a344e_24bf_4d0f_8457_56c534892b29);

#[interface("973a344e-24bf-4d0f-8457-56c534892b29")]
pub unsafe trait IXGameImpl: IUnknown {
    pub unsafe fn XGameGetXboxTitleId(&self, value: *mut u32) -> HRESULT;
}

#[interface("50849859-0ad8-4f81-80e4-5bc78626f852")]
pub unsafe trait IXGameImpl2: IXGameImpl {
    pub unsafe fn XLaunchNewGame(
        &self,
        exe_path: *const c_char,
        args: *const c_char,
        default_user: u64,
    ) -> ();
}

#[interface("2549f142-6419-4a06-97b5-931aab7c2f34")]
pub unsafe trait IXGameImpl3: IXGameImpl2 {
    pub unsafe fn XLaunchRestartOnCrash(&self, args: *const c_char, reserved: u32) -> HRESULT;
}

#[implement(IXGameImpl, IXGameImpl2, IXGameImpl3)]
pub struct XGame;

/// Parses the real `<TitleId>` out of the launched title's `MicrosoftGame.Config`, walking up
/// from the game executable (the file lives next to the exe, occasionally a parent directory) -
/// not hardcoded, so the title id is the launched title's own rather than baked in.
fn read_game_title_id() -> Option<u32> {
    static TITLE_ID: OnceLock<Option<u32>> = OnceLock::new();
    *TITLE_ID.get_or_init(|| {
        let exe = std::env::current_exe().ok()?;
        let mut dir = exe.parent()?.to_path_buf();
        loop {
            for name in ["MicrosoftGame.Config", "MicrosoftGame.config"] {
                let candidate = dir.join(name);
                if let Ok(contents) = std::fs::read_to_string(&candidate)
                    && let Some(id) = parse_title_id_from_config(&contents)
                {
                    return Some(id);
                }
            }
            if !dir.pop() {
                return None;
            }
        }
    })
}

fn parse_title_id_from_config(contents: &str) -> Option<u32> {
    let start = contents.find("<TitleId")?;
    let open_end = contents[start..].find('>')? + start + 1;
    let close = contents[open_end..].find("</TitleId>")? + open_end;
    let text = contents[open_end..close].trim();
    if text.len() != 8 {
        return None;
    }
    u32::from_str_radix(text, 16).ok()
}

impl IXGameImpl_Impl for XGame_Impl {
    unsafe fn XGameGetXboxTitleId(&self, value: *mut u32) -> HRESULT {
        if value.is_null() {
            return E_POINTER;
        }
        match read_game_title_id() {
            Some(id) => {
                unsafe {
                    *value = id;
                }
                S_OK
            }
            None => {
                unsafe {
                    *value = 0;
                }
                E_NOTIMPL
            }
        }
    }
}

impl IXGameImpl2_Impl for XGame_Impl {
    /// Not something Xodus can actually do under Wine (no shell to hand off to, no second
    /// process registration), so this is a no-op - the method has no return value to signal
    /// failure with.
    unsafe fn XLaunchNewGame(
        &self,
        _exe_path: *const c_char,
        _args: *const c_char,
        _default_user: u64,
    ) {
    }
}

impl IXGameImpl3_Impl for XGame_Impl {
    /// Not implemented - no crash-restart facility exists to drive.
    unsafe fn XLaunchRestartOnCrash(&self, _args: *const c_char, _reserved: u32) -> HRESULT {
        E_NOTIMPL
    }
}
