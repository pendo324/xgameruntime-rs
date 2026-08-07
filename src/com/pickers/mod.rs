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
//! Only the save picker is implemented. The open picker is a separate interface with its own
//! result shape, and nothing that runs here has asked for it yet.

use std::ffi::c_void;

use windows_core::{GUID, HRESULT, HSTRING, IInspectable_Vtbl, IUnknown_Vtbl, Interface};

use crate::diag::stub;

mod async_op;
mod deferred;
mod dialog;
mod save_picker;
mod storage_file;

const S_OK: HRESULT = HRESULT(0);
const E_FAIL: HRESULT = HRESULT(0x80004005u32 as i32);
const E_NOTIMPL: HRESULT = HRESULT(0x80004001u32 as i32);
const E_POINTER: HRESULT = HRESULT(0x80004003u32 as i32);
const E_NOINTERFACE: HRESULT = HRESULT(0x80004002u32 as i32);
const CLASS_E_CLASSNOTAVAILABLE: HRESULT = HRESULT(0x80040111u32 as i32);

/// `IActivationFactory`, whose IID is fixed by the WinRT ABI.
const IID_IACTIVATION_FACTORY: GUID = GUID::from_u128(0x00000035_0000_0000_c000_000000000046);
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

/// `IActivationFactory`'s vtable: `IInspectable` plus a single `ActivateInstance`.
///
/// Declared here rather than imported because the generated binding for it carries no `_Impl`
/// trait either, and the layout is fixed by the WinRT ABI.
#[repr(C)]
struct ActivationFactoryVtbl {
    base__: IInspectable_Vtbl,
    ActivateInstance: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

/// The factory is stateless, so one static instance serves every activation and its reference
/// count never has to mean anything.
#[repr(C)]
struct ActivationFactory {
    vtable: &'static ActivationFactoryVtbl,
}

// SAFETY: the factory holds no state, so sharing the one static instance across threads is safe.
unsafe impl Sync for ActivationFactory {}

static FACTORY: ActivationFactory = ActivationFactory {
    vtable: &FACTORY_VTABLE,
};

unsafe extern "system" fn factory_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    interface: *mut *mut c_void,
) -> HRESULT {
    if iid.is_null() || interface.is_null() {
        return E_POINTER;
    }
    // SAFETY: both pointers were just null-checked and COM guarantees they stay valid here.
    unsafe {
        let requested = *iid;
        let known = requested == windows_core::IUnknown::IID
            || requested == windows_core::IInspectable::IID
            || requested == IID_IACTIVATION_FACTORY
            || requested == IID_IAGILE_OBJECT;
        if known {
            *interface = this;
            S_OK
        } else {
            stub!("FileSavePickerFactory::QueryInterface({requested:?}) -> E_NOINTERFACE");
            *interface = std::ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

/// The factory outlives every caller, so its reference count is a constant.
unsafe extern "system" fn factory_add_ref(_this: *mut c_void) -> u32 {
    2
}

unsafe extern "system" fn factory_release(_this: *mut c_void) -> u32 {
    1
}

unsafe extern "system" fn factory_runtime_class_name(
    _this: *mut c_void,
    value: *mut *mut c_void,
) -> HRESULT {
    if value.is_null() {
        return E_POINTER;
    }
    // SAFETY: `value` was just null-checked; the string is handed to the caller to free.
    unsafe {
        *value = std::mem::transmute::<HSTRING, *mut c_void>(HSTRING::from(
            "Windows.Storage.Pickers.FileSavePicker",
        ));
    }
    S_OK
}

/// # Safety
/// `instance` must be a writable out-parameter per the `IActivationFactory` contract.
unsafe extern "system" fn factory_activate_instance(
    _this: *mut c_void,
    instance: *mut *mut c_void,
) -> HRESULT {
    if instance.is_null() {
        return E_POINTER;
    }
    // SAFETY: `instance` was just null-checked and COM guarantees it is writable here.
    unsafe { *instance = save_picker::FileSavePickerObject::create() };
    S_OK
}

static FACTORY_VTABLE: ActivationFactoryVtbl = ActivationFactoryVtbl {
    base__: IInspectable_Vtbl {
        base: IUnknown_Vtbl {
            QueryInterface: factory_query_interface,
            AddRef: factory_add_ref,
            Release: factory_release,
        },
        GetIids: spy_get_iids,
        GetRuntimeClassName: factory_runtime_class_name,
        GetTrustLevel: spy_get_trust_level,
    },
    ActivateInstance: factory_activate_instance,
};

/// Serves the activation factory for the save picker, and logs every class asked for.
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
    if name != "Windows.Storage.Pickers.FileSavePicker" {
        stub!("DllGetActivationFactory({name:?}) -> CLASS_E_CLASSNOTAVAILABLE");
        return CLASS_E_CLASSNOTAVAILABLE;
    }

    // SAFETY: `factory` is non-null and writable, checked above. The factory is static, so the
    // pointer stays valid for the life of the process however the caller refcounts it.
    unsafe { *factory = (&FACTORY) as *const _ as *mut c_void };
    S_OK
}
