//! The `IAsyncOperation<T>` a pick hands back.
//!
//! `windows-future` can build one of these from a closure, but not this one. A cancelled pick
//! completes *successfully* with no file, and the only way to say that at the ABI is `S_OK` with
//! a null result - refusing instead would turn a user pressing Escape into an exception in the
//! title. A generated `GetResults` returns `Result<T>`, whose `Ok` cannot hold a null because
//! `IUnknown` wraps a `NonNull`.
//!
//! It can still say it, though: `Error::empty()` carries a sentinel whose `HRESULT` reads back as
//! zero, and the generated vtable returns that code without writing the caller's out-parameter.
//! That is the same value the calling side produces from a null result, so the two halves agree.
//! Which means the vtables here need not be written out at all - `#[implement]` supplies them,
//! along with the reference counting and the `QueryInterface` between the operation, its
//! `IAsyncInfo`, and the agility every caller asks about.
//!
//! One thing it supplies imperfectly. `GetRuntimeClassName` builds its answer from the result
//! type's own name, and a generated runtime class carries an empty one - the bindings emit that
//! name for interfaces and value types but not for classes - so the operation reports itself as
//! ``Windows.Foundation.IAsyncOperation`1<>``. That is what any implementation of a parameterised
//! operation reports, and nothing that reaches this runtime reads it.
//!
//! What a completed pick produces differs between the picker families this runtime serves - one
//! hands back a `StorageFile`, the other a `PickFileResult` - but nothing else about the
//! operation does. The result type is therefore a parameter, described by [`PickOutcome`], and
//! everything below is written once against it.

use std::path::PathBuf;
use std::sync::Mutex;

use windows_core::{ComObject, Error, HRESULT, IUnknownImpl, Ref, Result, RuntimeType, implement};
use windows_future::{
    AsyncOperationCompletedHandler, AsyncStatus, IAsyncInfo, IAsyncInfo_Impl, IAsyncOperation,
    IAsyncOperation_Impl,
};

use super::deferred::run_later;
use crate::diag::stub;

/// `E_ILLEGAL_METHOD_CALL`, which is what asking for results before they exist earns.
const E_ILLEGAL_METHOD_CALL: HRESULT = HRESULT(0x8000000Eu32 as i32);

/// What one picker family's pick produces.
///
/// The type named here is what makes one operation distinguishable from the other: a parameterised
/// WinRT interface has a distinct IID per type argument, and a title that asks for the wrong one
/// gets `E_NOINTERFACE` from an object that would otherwise have served it. Naming the type is
/// enough - the IID and the runtime class name both follow from it.
pub(super) trait PickOutcome: 'static {
    /// How this family's operation names itself in diagnostics.
    const LABEL: &'static str;
    /// The type the operation is parameterised by - the WinRT runtime class a completed pick
    /// hands back.
    type Value: RuntimeType;
    /// Builds the object `GetResults` hands back for a pick that landed somewhere.
    fn create_result(path: PathBuf) -> Self::Value;
}

/// The work a pick does once the title is back in its message loop, and what it produced.
pub(super) type PickJob = Box<dyn FnOnce() -> std::result::Result<Option<PathBuf>, HRESULT>>;

/// What the pick produced, once it has.
struct OperationState<T: PickOutcome> {
    status: AsyncStatus,
    error: HRESULT,
    /// Where the pick landed, or `None` for a cancelled pick - which is a successful completion
    /// with no file, not a failure.
    picked: Option<PathBuf>,
    /// The completion handler, set on the title's thread and invoked on the one running the
    /// dialog.
    handler: Option<AsyncOperationCompletedHandler<T::Value>>,
}

impl<T: PickOutcome> Default for OperationState<T> {
    fn default() -> Self {
        Self {
            status: AsyncStatus::Started,
            error: HRESULT(0),
            picked: None,
            handler: None,
        }
    }
}

/// The operation object, which is both the `IAsyncOperation` a caller holds and the `IAsyncInfo`
/// it may ask that for.
#[implement(IAsyncOperation<T::Value>, IAsyncInfo)]
pub(super) struct PickOperation<T: PickOutcome> {
    state: Mutex<OperationState<T>>,
}

impl<T: PickOutcome> PickOperation<T> {
    /// Returns an operation for a pick that has not happened yet, and arranges for it to happen.
    ///
    /// The dialog is deliberately not shown here. It runs later, on this same thread, once the
    /// title has returned to its message loop - which is both where the dialog has to be, since
    /// it is modal to the title's own window, and where the completion has to be, since a
    /// handler invoked from any other thread tries an apartment switch this runtime cannot
    /// perform. Showing it here instead would complete the operation before the title has even
    /// seen it, and a pick that is already finished by the time it is handed over is not a
    /// sequence a title has any reason to expect.
    pub(super) fn start(job: PickJob) -> IAsyncOperation<T::Value> {
        let object = ComObject::new(Self {
            state: Mutex::new(OperationState::default()),
        });
        let operation = object.to_interface();

        let deferred = object.clone();
        let queued = run_later(Box::new(move || Self::finish(&deferred, job())));
        if !queued {
            // The work never ran, so the operation completes as a failure rather than never
            // completing at all.
            Self::finish(&object, Err(super::E_FAIL));
        }

        operation
    }

    /// Returns an operation that has already finished.
    ///
    /// Not everything that hands back one of these has to wait for a user: a lookup that is just
    /// a path made into a file has its answer immediately. Deferring it the way a pick is
    /// deferred would be worse than pointless - the deferral runs on a message loop, and nothing
    /// promises the thread doing a lookup has one.
    pub(super) fn completed(
        outcome: std::result::Result<Option<PathBuf>, HRESULT>,
    ) -> IAsyncOperation<T::Value> {
        let object = ComObject::new(Self {
            state: Mutex::new(OperationState::default()),
        });
        // Nothing else has seen the operation yet, so there is no handler to invoke.
        Self::finish(&object, outcome);
        object.to_interface()
    }

    /// Records the outcome and tells whoever is waiting.
    fn finish(object: &ComObject<Self>, outcome: std::result::Result<Option<PathBuf>, HRESULT>) {
        let handler = {
            let mut state = object.state.lock().expect("pick operation state poisoned");
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
            state.handler.clone()
        };

        // The handler runs outside the lock: it calls straight back into `GetResults`, which
        // takes the same lock.
        if let Some(handler) = handler {
            Self::invoke(object, &handler);
        }
    }

    /// Calls a completion handler with this operation and its status.
    fn invoke(object: &ComObject<Self>, handler: &AsyncOperationCompletedHandler<T::Value>) {
        let status = object
            .state
            .lock()
            .expect("pick operation state poisoned")
            .status;
        // Nothing useful can be done with a handler that fails: the title wrote it, and this is
        // the notification that its own pick finished.
        let _ = handler.Invoke(&object.to_interface::<IAsyncOperation<T::Value>>(), status);
    }
}

impl<T: PickOutcome> IAsyncOperation_Impl<T::Value> for PickOperation_Impl<T> {
    fn SetCompleted(&self, value: Ref<'_, AsyncOperationCompletedHandler<T::Value>>) -> Result<()> {
        let handler = value.cloned();
        let complete_now = {
            let mut state = self.state.lock().expect("pick operation state poisoned");
            state.handler = handler.clone();
            state.status != AsyncStatus::Started
        };
        stub!(
            "{}::SetCompleted (already finished: {complete_now})",
            T::LABEL
        );
        // A handler registered after the pick finished still has to run - otherwise the title
        // waits for a completion that already happened.
        if let (true, Some(handler)) = (complete_now, handler) {
            PickOperation::invoke(&self.to_object(), &handler);
        }
        Ok(())
    }

    fn Completed(&self) -> Result<AsyncOperationCompletedHandler<T::Value>> {
        // An unset handler is a null result rather than a failure, which is what `Error::empty`
        // reports - see this module's header.
        self.state
            .lock()
            .expect("pick operation state poisoned")
            .handler
            .clone()
            .ok_or_else(Error::empty)
    }

    fn GetResults(&self) -> Result<T::Value> {
        let state = self.state.lock().expect("pick operation state poisoned");
        match state.status {
            AsyncStatus::Completed => {
                let Some(path) = state.picked.clone() else {
                    // A cancelled pick: successful, with no file.
                    stub!("{}::GetResults -> cancelled", T::LABEL);
                    return Err(Error::empty());
                };
                stub!("{}::GetResults -> {path:?}", T::LABEL);
                Ok(T::create_result(path))
            }
            AsyncStatus::Error => Err(Error::from_hresult(state.error)),
            _ => Err(Error::from_hresult(E_ILLEGAL_METHOD_CALL)),
        }
    }
}

impl<T: PickOutcome> IAsyncInfo_Impl for PickOperation_Impl<T> {
    fn Id(&self) -> Result<u32> {
        Ok(1)
    }

    fn Status(&self) -> Result<AsyncStatus> {
        Ok(self
            .state
            .lock()
            .expect("pick operation state poisoned")
            .status)
    }

    fn ErrorCode(&self) -> Result<HRESULT> {
        Ok(self
            .state
            .lock()
            .expect("pick operation state poisoned")
            .error)
    }

    /// The shell dialog cannot be dismissed from outside once it is up, so this records the
    /// request and lets the pick finish on its own rather than reporting a cancellation that did
    /// not happen.
    fn Cancel(&self) -> Result<()> {
        stub!("{}::Cancel", T::LABEL);
        Ok(())
    }

    fn Close(&self) -> Result<()> {
        Ok(())
    }
}
