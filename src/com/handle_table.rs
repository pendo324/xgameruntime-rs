//! A checked table from opaque `u64` handles to reference-counted objects.
//!
//! Handles used to be raw pointers cast to `u64` (`Box::into_raw(Box::new(T))`), which
//! trusts the caller never to hand back a stale, foreign, or otherwise-garbage value - a
//! real risk here, since the handle crosses the FFI boundary into game code we do not
//! control. A bad value there does not fail cleanly; it dereferences whatever bytes
//! happen to live at that address as a `T`, which is memory corruption, not an error -
//! and even a well-behaved caller racing a `get` against a `close` on the same handle
//! from two threads hits a use-after-free, since the old scheme had no synchronization
//! at all. Handing out small sequential IDs instead means a bad handle is just a failed
//! hash-map lookup, and the table's own mutex makes concurrent access safe.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

pub(crate) struct HandleTable<T> {
    next: AtomicU64,
    entries: OnceLock<Mutex<HashMap<u64, T>>>,
}

impl<T: Clone> HandleTable<T> {
    pub(crate) const fn new() -> Self {
        Self {
            // 0 is reserved for "no handle" throughout this API, so IDs start at 1.
            next: AtomicU64::new(1),
            entries: OnceLock::new(),
        }
    }

    fn entries(&self) -> &Mutex<HashMap<u64, T>> {
        self.entries.get_or_init(|| Mutex::new(HashMap::new()))
    }

    pub(crate) fn create(&self, value: T) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.entries()
            .lock()
            .expect("handle table poisoned")
            .insert(id, value);
        id
    }

    pub(crate) fn get(&self, handle: u64) -> Option<T> {
        if handle == 0 {
            return None;
        }
        self.entries()
            .lock()
            .expect("handle table poisoned")
            .get(&handle)
            .cloned()
    }

    pub(crate) fn close(&self, handle: u64) {
        if handle == 0 {
            return;
        }
        self.entries()
            .lock()
            .expect("handle table poisoned")
            .remove(&handle);
    }
}
