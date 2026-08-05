use super::E_NOTIMPL;
use std::ffi::c_void;
use windows_core::{GUID, HRESULT, IUnknown, implement, interface};
/// `IXErrorImpl`'s own IID, reused as the coclass id (same pattern as `CLSID_XGAME`) - confirmed
/// via Wine trace logs as one of the classes this title queries and previously got `E_NOTIMPL`
/// for.
pub const CLSID_XERROR: GUID = GUID::from_u128(0x8ca467f7_22e8_4096_8456_bb8aa13f79d8);

#[interface("8ca467f7-22e8-4096-8456-bb8aa13f79d8")]
pub unsafe trait IXErrorImpl: IUnknown {
    /// `xgameruntime-docs` lists this vtable slot (the first method after `IUnknown`'s three) as
    /// `*unknown*` - no name, no signature, and nothing in `xerror.idl` pins it down. This stub
    /// exists only to hold the slot's position so `XErrorSetCallback`/`XErrorSetOptions` land at
    /// the right vtable offsets; if this title ever actually calls slot 4 directly, whatever this
    /// returns is not meaningful.
    pub unsafe fn XErrorImpl_UnknownMethod0(&self) -> HRESULT;
    pub unsafe fn XErrorSetCallback(&self, callback: *mut c_void, context: *mut c_void) -> ();
    pub unsafe fn XErrorSetOptions(&self, options: u32) -> ();
}

#[implement(IXErrorImpl)]
pub struct XError;

impl IXErrorImpl_Impl for XError_Impl {
    unsafe fn XErrorImpl_UnknownMethod0(&self) -> HRESULT {
        E_NOTIMPL
    }

    /// No error-reporting sink to forward to (see `XErrorReport`'s own `E_NOTIMPL` in `lib.rs`) -
    /// accept the registration without ever invoking it, rather than silently dropping the call
    /// as unimplemented.
    unsafe fn XErrorSetCallback(&self, _callback: *mut c_void, _context: *mut c_void) {}

    unsafe fn XErrorSetOptions(&self, _options: u32) {}
}
