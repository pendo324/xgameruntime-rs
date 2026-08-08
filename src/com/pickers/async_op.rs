//! The `IAsyncOperation<T>` a pick hands back.
//!
//! `windows-future` can build one of these from a closure, but not this one: a cancelled pick
//! completes *successfully* with a null result, and the generated `GetResults` takes a
//! `Result<T>` whose `Ok` cannot hold a null - `IUnknown` wraps a `NonNull`. Refusing instead
//! would turn a user pressing Escape into an exception in the title. So the vtable functions are
//! written out here, where `GetResults` can write the null the contract calls for.
//!
//! Their *layouts* still come from `windows-future` wherever they can. `IAsyncInfo`'s vtable is
//! used as generated. `IAsyncOperation`'s cannot be - it carries a private field, so nothing
//! outside that crate can build one - so it is restated below and held to the generated one by
//! [`PickOperation::LAYOUT`], which fails to compile if the slots ever stop lining up.
//!
//! What a completed pick produces differs between the picker families this runtime serves - one
//! hands back a `StorageFile`, the other a `PickFileResult` - but nothing else about the
//! operation does. The result type is therefore a parameter, described by [`PickOutcome`], and
//! everything below is written once against it.

use std::ffi::c_void;
use std::marker::PhantomData;
use std::mem::{ManuallyDrop, offset_of};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use windows_core::{GUID, HRESULT, IInspectable_Vtbl, IUnknown_Vtbl, Interface, RuntimeType};
use windows_future::{
    AsyncOperationCompletedHandler, AsyncStatus, IAsyncInfo_Vtbl, IAsyncOperation,
    IAsyncOperation_Vtbl,
};

use super::deferred::run_later;
use super::{E_POINTER, IID_IAGILE_OBJECT, S_OK, com_release, spy_get_iids, spy_get_trust_level};
use crate::diag::stub;

/// `IAsyncInfo`, whose IID is fixed by the WinRT ABI.
const IID_IASYNC_INFO: GUID = GUID::from_u128(0x00000036_0000_0000_c000_000000000046);
/// `E_ILLEGAL_METHOD_CALL`, which is what asking for results before they exist earns.
const E_ILLEGAL_METHOD_CALL: HRESULT = HRESULT(0x8000000Eu32 as i32);

/// What one picker family's pick produces, and how its operation identifies itself.
///
/// The IIDs and the class name are not interchangeable between families even though the operation
/// behaves identically: a parameterised WinRT interface has a distinct IID per type argument, and
/// a title that asks for the wrong one gets `E_NOINTERFACE` from an object that would otherwise
/// have served it. Naming the type argument here is what makes those follow rather than be
/// restated - the IID is read off `IAsyncOperation<Self::Value>`.
pub(super) trait PickOutcome: 'static {
    /// How this family's operation names itself in diagnostics.
    const LABEL: &'static str;
    /// The runtime class name of `IAsyncOperation<Self::Value>`.
    const RUNTIME_CLASS_NAME: &'static str;
    /// The type the operation is parameterised by - the WinRT runtime class a completed pick
    /// hands back, which at the ABI is one interface pointer.
    type Value: RuntimeType;
    /// Builds the object `GetResults` hands back for a pick that landed somewhere.
    fn create_result(path: PathBuf) -> Self::Value;
}

/// The work a pick does once the title is back in its message loop, and what it produced.
pub(super) type PickJob = Box<dyn FnOnce() -> Result<Option<PathBuf>, HRESULT>>;

/// `IAsyncOperation<T>`'s vtable, restated because the generated one cannot be built here.
#[repr(C)]
struct AsyncOperationVtbl {
    base__: IInspectable_Vtbl,
    SetCompleted: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    Completed: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    GetResults: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[derive(Default)]
struct OperationState {
    status: AsyncStatus,
    error: HRESULT,
    /// Where the pick landed, or `None` for a cancelled pick - which is a successful completion
    /// with no file, not a failure.
    picked: Option<PathBuf>,
    /// The completion handler, held as a bare address because it is set on the title's thread
    /// and invoked on the one running the dialog. Owned: released when it is replaced or when
    /// the operation goes away.
    handler: usize,
}

/// The operation object, carrying an `IAsyncOperation` vtable and an `IAsyncInfo` vtable so both
/// interfaces are the same object with one reference count.
#[repr(C)]
pub(super) struct PickOperation<T: PickOutcome> {
    operation_vtable: &'static AsyncOperationVtbl,
    info_vtable: &'static IAsyncInfo_Vtbl,
    refs: AtomicU32,
    state: Mutex<OperationState>,
    outcome: PhantomData<fn() -> T>,
}

impl<T: PickOutcome> PickOperation<T> {
    /// Holds [`AsyncOperationVtbl`] to the layout `windows-future` generates for this family's
    /// operation: the same size, and the same three slots in the same order after `IInspectable`.
    /// Evaluated where the vtable is built, so a mismatch is a compile error rather than a call
    /// landing in the wrong function.
    const LAYOUT: () = {
        assert!(size_of::<AsyncOperationVtbl>() == size_of::<IAsyncOperation_Vtbl<T::Value>>());
        assert!(
            offset_of!(AsyncOperationVtbl, SetCompleted)
                == offset_of!(IAsyncOperation_Vtbl<T::Value>, SetCompleted)
        );
        assert!(
            offset_of!(AsyncOperationVtbl, Completed)
                == offset_of!(IAsyncOperation_Vtbl<T::Value>, Completed)
        );
        assert!(
            offset_of!(AsyncOperationVtbl, GetResults)
                == offset_of!(IAsyncOperation_Vtbl<T::Value>, GetResults)
        );
    };

    /// The vtables this family's operations carry. Written as constants rather than statics
    /// because they are per-result-type, and a `static` cannot be generic.
    const OPERATION_VTABLE: &'static AsyncOperationVtbl = &AsyncOperationVtbl {
        base__: IInspectable_Vtbl {
            base: IUnknown_Vtbl {
                QueryInterface: operation_query_interface::<T>,
                AddRef: operation_add_ref::<T>,
                Release: operation_release::<T>,
            },
            GetIids: spy_get_iids,
            GetRuntimeClassName: operation_runtime_class_name::<T>,
            GetTrustLevel: spy_get_trust_level,
        },
        SetCompleted: set_completed::<T>,
        Completed: get_completed::<T>,
        GetResults: get_results::<T>,
    };

    const INFO_VTABLE: &'static IAsyncInfo_Vtbl = &IAsyncInfo_Vtbl {
        base__: IInspectable_Vtbl {
            base: IUnknown_Vtbl {
                QueryInterface: info_query_interface::<T>,
                AddRef: info_add_ref::<T>,
                Release: info_release::<T>,
            },
            GetIids: spy_get_iids,
            GetRuntimeClassName: operation_runtime_class_name::<T>,
            GetTrustLevel: spy_get_trust_level,
        },
        Id: info_id,
        Status: info_status::<T>,
        ErrorCode: info_error_code::<T>,
        Cancel: info_cancel::<T>,
        Close: info_close,
    };

    /// Returns an operation for a pick that has not happened yet, and arranges for it to happen.
    ///
    /// The dialog is deliberately not shown here. It runs later, on this same thread, once the
    /// title has returned to its message loop - which is both where the dialog has to be, since
    /// it is modal to the title's own window, and where the completion has to be, since a
    /// handler invoked from any other thread tries an apartment switch this runtime cannot
    /// perform. Showing it here instead would complete the operation before the title has even
    /// seen it, and a pick that is already finished by the time it is handed over is not a
    /// sequence a title has any reason to expect.
    pub(super) fn start(job: PickJob) -> *mut c_void {
        let () = Self::LAYOUT;
        let operation = Box::into_raw(Box::new(PickOperation::<T> {
            operation_vtable: Self::OPERATION_VTABLE,
            info_vtable: Self::INFO_VTABLE,
            // One reference for the caller, one for the deferred work below.
            refs: AtomicU32::new(2),
            state: Mutex::new(OperationState {
                status: AsyncStatus::Started,
                ..Default::default()
            }),
            outcome: PhantomData,
        }));

        let address = operation as usize;
        let queued = run_later(Box::new(move || {
            let outcome = job();
            // SAFETY: the reference taken above is still held, so the operation is still alive.
            unsafe { (*(address as *mut PickOperation<T>)).finish(outcome) };
            // SAFETY: releases that same reference, and touches nothing afterwards.
            unsafe { com_release_operation::<T>(address as *mut c_void) };
        }));

        if !queued {
            // SAFETY: the work never ran, so its reference is given back here and the operation
            // completes as a failure rather than never completing at all.
            unsafe {
                (*operation).finish(Err(super::E_FAIL));
                com_release_operation::<T>(operation.cast());
            }
        }

        operation.cast()
    }

    /// Returns an operation that has already finished.
    ///
    /// Not everything that hands back one of these has to wait for a user: a lookup that is just
    /// a path made into a file has its answer immediately. Deferring it the way a pick is
    /// deferred would be worse than pointless - the deferral runs on a message loop, and nothing
    /// promises the thread doing a lookup has one.
    pub(super) fn completed(outcome: Result<Option<PathBuf>, HRESULT>) -> *mut c_void {
        let () = Self::LAYOUT;
        let operation = Box::into_raw(Box::new(PickOperation::<T> {
            operation_vtable: Self::OPERATION_VTABLE,
            info_vtable: Self::INFO_VTABLE,
            // One reference, for the caller.
            refs: AtomicU32::new(1),
            state: Mutex::new(OperationState {
                status: AsyncStatus::Started,
                ..Default::default()
            }),
            outcome: PhantomData,
        }));

        // SAFETY: nothing else has seen the operation yet, so there is no handler to invoke and
        // the reference above is still held.
        unsafe { (*operation).finish(outcome) };
        operation.cast()
    }

    /// Records the outcome and tells whoever is waiting.
    ///
    /// # Safety
    /// `self` must be a live operation the caller holds a reference to.
    unsafe fn finish(&self, outcome: Result<Option<PathBuf>, HRESULT>) {
        let handler = {
            let mut state = self.state.lock().expect("pick operation state poisoned");
            match outcome {
                Ok(picked) => {
                    state.status = AsyncStatus::Completed;
                    state.picked = picked;
                }
                Err(error) => {
                    state.status = AsyncStatus::Error;
                    state.error = error;
                }
            }
            state.handler
        };

        if handler != 0 {
            // The handler runs outside the lock: it calls straight back into `GetResults`, which
            // takes the same lock.
            // SAFETY: `handler` is a completion handler this object holds a reference to, and
            // `self` is the operation it was registered on.
            unsafe { invoke_handler::<T>(handler, self as *const _ as *mut c_void) };
        }
    }
}

/// Calls a completion handler with this operation and its status.
///
/// The call goes through the generated `AsyncOperationCompletedHandler` rather than a vtable
/// written out here: this one is only ever called, never implemented, so nothing about it needs
/// the null a `GetResults` does.
///
/// # Safety
/// `handler` must be a live `AsyncOperationCompletedHandler<T::Value>` and `operation` the
/// operation it was registered on.
unsafe fn invoke_handler<T: PickOutcome>(handler: usize, operation: *mut c_void) {
    // SAFETY: both are `repr(transparent)` interface pointers, live per this function's contract.
    // Neither reference is this call's to give back, so neither wrapper is allowed to drop.
    unsafe {
        let handler = ManuallyDrop::new(std::mem::transmute_copy::<
            *mut c_void,
            AsyncOperationCompletedHandler<T::Value>,
        >(&(handler as *mut c_void)));
        let operation_ref = ManuallyDrop::new(std::mem::transmute_copy::<
            *mut c_void,
            IAsyncOperation<T::Value>,
        >(&operation));
        let status = (*(operation as *const PickOperation<T>))
            .state
            .lock()
            .expect("pick operation state poisoned")
            .status;
        // Nothing useful can be done with a handler that fails: the title wrote it, and this
        // is the notification that its own pick finished.
        let _ = handler.Invoke(&*operation_ref, status);
    }
}

/// # Safety
/// `this` must be a live `PickOperation<T>`, seen through its `IAsyncOperation` vtable.
unsafe fn operation<'a, T: PickOutcome>(this: *mut c_void) -> &'a PickOperation<T> {
    // SAFETY: guaranteed by this function's contract.
    unsafe { &*this.cast::<PickOperation<T>>() }
}

/// Recovers the object from a pointer to its second vtable.
///
/// # Safety
/// `this` must be a live `PickOperation<T>`, seen through its `IAsyncInfo` vtable.
unsafe fn operation_from_info<'a, T: PickOutcome>(this: *mut c_void) -> &'a PickOperation<T> {
    // SAFETY: `this` addresses the `info_vtable` field, so the object starts at that offset back.
    unsafe {
        &*(this
            .cast::<u8>()
            .sub(offset_of!(PickOperation<T>, info_vtable))
            .cast::<PickOperation<T>>())
    }
}

unsafe extern "system" fn operation_query_interface<T: PickOutcome>(
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
        let object = operation::<T>(this);
        if requested == IID_IASYNC_INFO {
            object.refs.fetch_add(1, Ordering::Relaxed);
            *interface = (&object.info_vtable) as *const _ as *mut c_void;
            return S_OK;
        }
        let known = requested == windows_core::IUnknown::IID
            || requested == windows_core::IInspectable::IID
            || requested == IAsyncOperation::<T::Value>::IID
            || requested == IID_IAGILE_OBJECT;
        if known {
            object.refs.fetch_add(1, Ordering::Relaxed);
            *interface = this;
            S_OK
        } else {
            stub!(
                "{}::QueryInterface({requested:?}) -> E_NOINTERFACE",
                T::LABEL
            );
            *interface = std::ptr::null_mut();
            super::E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn info_query_interface<T: PickOutcome>(
    this: *mut c_void,
    iid: *const GUID,
    interface: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: forwarded to the object's own QueryInterface, which shares this contract.
    unsafe {
        let object = operation_from_info::<T>(this);
        operation_query_interface::<T>(object as *const _ as *mut c_void, iid, interface)
    }
}

unsafe extern "system" fn operation_add_ref<T: PickOutcome>(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live operation per the vtable contract.
    unsafe { operation::<T>(this) }
        .refs
        .fetch_add(1, Ordering::Relaxed)
        + 1
}

unsafe extern "system" fn info_add_ref<T: PickOutcome>(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live operation seen through its second vtable.
    unsafe { operation_add_ref::<T>(operation_from_info::<T>(this) as *const _ as *mut c_void) }
}

/// # Safety
/// `this` must be a live `PickOperation<T>` and this release balanced against an earlier
/// reference.
unsafe fn com_release_operation<T: PickOutcome>(this: *mut c_void) -> u32 {
    // SAFETY: guaranteed by this function's contract.
    let remaining = unsafe { operation::<T>(this) }
        .refs
        .fetch_sub(1, Ordering::AcqRel)
        - 1;
    if remaining == 0 {
        // SAFETY: the count reached zero, so nothing else can be running against the object.
        let owned = unsafe { Box::from_raw(this.cast::<PickOperation<T>>()) };
        let handler = owned
            .state
            .lock()
            .expect("pick operation state poisoned")
            .handler;
        if handler != 0 {
            // SAFETY: the handler reference this object took in `SetCompleted` is given back.
            unsafe { com_release(handler as *mut c_void) };
        }
    }
    remaining
}

unsafe extern "system" fn operation_release<T: PickOutcome>(this: *mut c_void) -> u32 {
    // SAFETY: forwarded from the vtable slot, whose contract this shares.
    unsafe { com_release_operation::<T>(this) }
}

unsafe extern "system" fn info_release<T: PickOutcome>(this: *mut c_void) -> u32 {
    // SAFETY: forwarded from the second vtable, resolved back to the object first.
    unsafe { com_release_operation::<T>(operation_from_info::<T>(this) as *const _ as *mut c_void) }
}

unsafe extern "system" fn operation_runtime_class_name<T: PickOutcome>(
    _this: *mut c_void,
    value: *mut *mut c_void,
) -> HRESULT {
    if value.is_null() {
        return E_POINTER;
    }
    // SAFETY: `value` was just null-checked, and the string is handed to the caller to free.
    unsafe {
        *value = std::mem::transmute::<windows_core::HSTRING, *mut c_void>(
            windows_core::HSTRING::from(T::RUNTIME_CLASS_NAME),
        );
    }
    S_OK
}

unsafe extern "system" fn set_completed<T: PickOutcome>(
    this: *mut c_void,
    handler: *mut c_void,
) -> HRESULT {
    // SAFETY: `this` is a live operation per the vtable contract; `handler` is either null or an
    // interface pointer the caller owns, so a reference is taken before it is stored.
    unsafe {
        let object = operation::<T>(this);
        let (previous, complete_now) = {
            let mut state = object.state.lock().expect("pick operation state poisoned");
            let previous = state.handler;
            if !handler.is_null() {
                com_add_ref(handler);
            }
            state.handler = handler as usize;
            (previous, state.status != AsyncStatus::Started)
        };
        if previous != 0 {
            com_release(previous as *mut c_void);
        }
        stub!(
            "{}::SetCompleted (already finished: {complete_now})",
            T::LABEL
        );
        // A handler registered after the pick finished still has to run - otherwise the title
        // waits for a completion that already happened.
        if complete_now && !handler.is_null() {
            invoke_handler::<T>(handler as usize, this);
        }
    }
    S_OK
}

unsafe extern "system" fn get_completed<T: PickOutcome>(
    this: *mut c_void,
    out: *mut *mut c_void,
) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: `out` was just null-checked; `this` is live per the vtable contract.
    unsafe {
        let handler = operation::<T>(this)
            .state
            .lock()
            .expect("pick operation state poisoned")
            .handler;
        if handler != 0 {
            com_add_ref(handler as *mut c_void);
        }
        *out = handler as *mut c_void;
    }
    S_OK
}

unsafe extern "system" fn get_results<T: PickOutcome>(
    this: *mut c_void,
    out: *mut *mut c_void,
) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: `out` was just null-checked; `this` is live per the vtable contract.
    unsafe {
        *out = std::ptr::null_mut();
        let state = operation::<T>(this)
            .state
            .lock()
            .expect("pick operation state poisoned");
        match state.status {
            AsyncStatus::Completed => {
                let Some(path) = state.picked.clone() else {
                    // A cancelled pick: successful, with no file. This null is the whole reason
                    // these functions are written out here.
                    stub!("{}::GetResults -> cancelled", T::LABEL);
                    return S_OK;
                };
                stub!("{}::GetResults -> {path:?}", T::LABEL);
                let result = T::create_result(path);
                // A runtime class is one interface pointer at the ABI, and the reference it
                // carries passes to the caller rather than being dropped here.
                *out = std::mem::transmute_copy(&result);
                std::mem::forget(result);
                S_OK
            }
            AsyncStatus::Error => state.error,
            _ => E_ILLEGAL_METHOD_CALL,
        }
    }
}

unsafe extern "system" fn info_id(_this: *mut c_void, value: *mut u32) -> HRESULT {
    // SAFETY: COM guarantees `value` is writable for the duration of the call.
    unsafe {
        if !value.is_null() {
            *value = 1;
        }
    }
    S_OK
}

unsafe extern "system" fn info_status<T: PickOutcome>(
    this: *mut c_void,
    value: *mut AsyncStatus,
) -> HRESULT {
    if value.is_null() {
        return E_POINTER;
    }
    // SAFETY: `value` was just null-checked; `this` is live per the second vtable's contract.
    unsafe {
        *value = operation_from_info::<T>(this)
            .state
            .lock()
            .expect("pick operation state poisoned")
            .status;
    }
    S_OK
}

unsafe extern "system" fn info_error_code<T: PickOutcome>(
    this: *mut c_void,
    value: *mut HRESULT,
) -> HRESULT {
    if value.is_null() {
        return E_POINTER;
    }
    // SAFETY: `value` was just null-checked; `this` is live per the second vtable's contract.
    unsafe {
        *value = operation_from_info::<T>(this)
            .state
            .lock()
            .expect("pick operation state poisoned")
            .error;
    }
    S_OK
}

/// The shell dialog cannot be dismissed from outside once it is up, so this records the request
/// and lets the pick finish on its own rather than reporting a cancellation that did not happen.
unsafe extern "system" fn info_cancel<T: PickOutcome>(_this: *mut c_void) -> HRESULT {
    stub!("{}::Cancel", T::LABEL);
    S_OK
}

unsafe extern "system" fn info_close(_this: *mut c_void) -> HRESULT {
    S_OK
}

/// # Safety
/// `this` must be a live COM interface pointer.
unsafe fn com_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: guaranteed by this function's contract - slot 1 of any COM vtable is `AddRef`.
    unsafe {
        let vtable = *(this as *const *const IUnknown_Vtbl);
        ((*vtable).AddRef)(this)
    }
}
