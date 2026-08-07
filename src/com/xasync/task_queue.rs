//! A native `XTaskQueue`.
//!
//! A task queue is the GDK's callback scheduler: every asynchronous API takes an
//! `XAsyncBlock` naming a queue, does its work against that queue's *work port*, and
//! delivers the caller's completion callback on its *completion port*. Splitting the two
//! is the whole point of the design - a game typically runs work on a thread pool but
//! wants completions delivered on its own main thread, where touching game state is
//! safe. That is what a "composite" queue is for: it borrows one port from one queue and
//! the other port from another.
//!
//! Ports are what actually hold callbacks; a queue is a pair of them plus monitors.
//! Handles are reference-counted (see [`QueueHandle`]) because a composite queue keeps
//! its source ports alive after the queues they came from are closed.
//!
//! This is a from-scratch implementation of the documented behaviour rather than a port
//! of Microsoft's; the observable contract is the ordering and cancellation rules
//! encoded in the tests at the bottom of this file.

use crate::com::handle_table::HandleTable;
use crate::diag::{diag, now_ms};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

// True while a callback is being run inline by `Port::dispatch` on the pump thread.
// A callback that then blocks in `XAsyncGetStatus(wait: true)` would deadlock: the async
// it is waiting on needs its DoWork dispatched on a queue only this same thread pumps, and
// it cannot pump while blocked. See `XAsyncGetStatus` for how the flag breaks the wait.
thread_local! {
    static IN_DISPATCH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
pub fn in_dispatch() -> bool {
    IN_DISPATCH.with(|f| f.get())
}

/// `void CALLBACK (*)(void* context, bool canceled)`.
///
/// `canceled` is true when the queue was terminated before the callback ran. It is still
/// invoked, because the context is almost always an owned allocation that only the
/// callback knows how to free - skipping it would leak.
pub type TaskCallback = unsafe extern "system" fn(context: *mut c_void, canceled: bool);
pub type TerminatedCallback = unsafe extern "system" fn(context: *mut c_void);
pub type MonitorCallback =
    unsafe extern "system" fn(context: *mut c_void, queue: u64, port: u32) -> ();

/// `XTaskQueueDispatchMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchMode {
    /// Callbacks only run when the game calls `XTaskQueueDispatch`.
    Manual,
    /// Callbacks run on background threads, possibly several at once.
    ThreadPool,
    /// Callbacks run on a background thread, one at a time and in order.
    SerializedThreadPool,
    /// Callbacks run inline, on whichever thread submitted them.
    Immediate,
}

impl DispatchMode {
    pub fn from_raw(value: u64) -> Option<Self> {
        Some(match value {
            0 => Self::Manual,
            1 => Self::ThreadPool,
            2 => Self::SerializedThreadPool,
            3 => Self::Immediate,
            _ => return None,
        })
    }
}

/// `XTaskQueuePort`: which half of a queue an operation refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortKind {
    Work,
    Completion,
}

impl PortKind {
    pub fn from_raw(value: u64) -> Option<Self> {
        Some(match value {
            0 => Self::Work,
            1 => Self::Completion,
            _ => return None,
        })
    }

    fn as_raw(self) -> u32 {
        match self {
            Self::Work => 0,
            Self::Completion => 1,
        }
    }
}

struct Task {
    context: *mut c_void,
    callback: TaskCallback,
    /// A unique id tagging which `PortContext` (which queue) submitted this callback, so
    /// that terminating one queue's port context cancels exactly *its* pending callbacks
    /// and never another queue's that shares the same underlying `Port`.
    owner: u64,
    /// `None` for "as soon as possible"; delayed callbacks carry their deadline.
    due: Option<Instant>,
}

// The context is opaque to us - it belongs to whoever submitted the callback, and the
// GDK contract is that a callback may run on any thread the queue chooses.
unsafe impl Send for Task {}

impl Task {
    fn run(self, canceled: bool) {
        unsafe { (self.callback)(self.context, canceled) };
    }

    fn ready_at(&self, now: Instant) -> bool {
        self.due.is_none_or(|due| due <= now)
    }
}

/// Source of unique per-`PortContext` ids (for tagging tasks by owner).
static NEXT_CONTEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Live ports, for the periodic starvation report. Weak so a port that is genuinely gone
/// drops out of the report instead of being kept alive by the instrumentation.
static PORT_REGISTRY: Mutex<Vec<std::sync::Weak<Port>>> = Mutex::new(Vec::new());
static REPORTER: OnceLock<()> = OnceLock::new();

/// Every 2s, one line per live port: how much went in, how much came out, and how long it
/// has been since anyone pumped it. A port whose `queued` climbs while `ran` stays flat is
/// the thing stalling the game.
fn start_port_reporter() {
    if !crate::diag::enabled() {
        return;
    }
    REPORTER.get_or_init(|| {
        std::thread::spawn(|| {
            loop {
                std::thread::sleep(Duration::from_secs(2));
                let ports: Vec<Arc<Port>> = PORT_REGISTRY
                    .lock()
                    .expect("port registry poisoned")
                    .iter()
                    .filter_map(std::sync::Weak::upgrade)
                    .collect();
                let t = now_ms();
                for port in ports {
                    let state = port.state.lock().expect("port state poisoned");
                    // Only report ports that have ever seen traffic; a fresh process has
                    // dozens of idle ports and they drown the interesting ones.
                    if state.submitted == 0 {
                        continue;
                    }
                    let since = state
                        .last_dispatch_ms
                        .map(|ms| (t.saturating_sub(ms)).to_string())
                        .unwrap_or_else(|| "never".to_string());
                    diag!(
                        "port={:p} mode={:?} depth={} queued={} ran={} in_flight={} since_last_dispatch_ms={since}",
                        Arc::as_ptr(&port),
                        port.mode,
                        state.tasks.len(),
                        state.submitted,
                        state.dispatched,
                        state.in_flight,
                    );
                }
            }
        });
    });
}

#[derive(Default)]
struct PortState {
    tasks: VecDeque<Task>,
    /// Instrumentation: total callbacks ever accepted onto this port.
    submitted: u64,
    /// Instrumentation: total callbacks ever run off this port (any dispatch path).
    dispatched: u64,
    /// Instrumentation: [`now_ms`] of the last dispatch that ran at least one callback.
    last_dispatch_ms: Option<u128>,
    /// Set once the *last* `PortContext` referencing this port is terminated. No further
    /// callbacks are accepted and workers exit. Deliberately not set when only one of
    /// several sharing contexts terminates - the others keep the port alive (this is the
    /// whole point of the context model, see `terminate_context`).
    terminated: bool,
    /// How many live `PortContext`s currently reference this port. A composite queue
    /// shares a source port with its source queue, so this is normally > 1.
    active_contexts: usize,
    /// Callbacks currently executing. `terminate(wait: true)` waits for this to drain.
    in_flight: usize,
    workers_started: bool,
}

/// One half of a task queue: an ordered list of pending callbacks plus the threads (if
/// any) that run them.
pub struct Port {
    mode: DispatchMode,
    state: Mutex<PortState>,
    /// Signalled when a task is queued or the port is terminated.
    ready: Condvar,
    /// Signalled when a callback finishes, for `terminate(wait: true)`.
    drained: Condvar,
}

impl Port {
    fn new(mode: DispatchMode) -> Arc<Self> {
        let port = Arc::new(Self {
            mode,
            state: Mutex::new(PortState::default()),
            ready: Condvar::new(),
            drained: Condvar::new(),
        });
        start_port_reporter();
        PORT_REGISTRY
            .lock()
            .expect("port registry poisoned")
            .push(Arc::downgrade(&port));
        diag!("port_create port={:p} mode={mode:?}", Arc::as_ptr(&port));
        port
    }

    pub fn mode(&self) -> DispatchMode {
        self.mode
    }

    pub fn is_terminated(&self) -> bool {
        self.state.lock().expect("port state poisoned").terminated
    }

    /// Register a new [`PortContext`] on this port. Each queue that uses a port - a
    /// primary queue or a composite - counts as one context, so a port shared by several
    /// queues is only terminated when its *last* context terminates.
    pub fn add_context(&self) {
        let mut state = self.state.lock().expect("port state poisoned");
        state.active_contexts += 1;
    }

    /// Terminate the given context: cancel only the callbacks it submitted, and if it was
    /// the last live context, terminate the port itself (workers exit, future submits
    /// refuse). This is what lets a game terminate a composite queue without aborting
    /// unrelated asyncs on another queue that shares the same port.
    pub fn terminate_context(&self, owner: u64, wait: bool) {
        let mut pending: Vec<Task> = Vec::new();
        let mut drain_rest = false;
        {
            let mut state = self.state.lock().expect("port state poisoned");
            if state.active_contexts > 0 {
                state.active_contexts -= 1;
            }
            // Pull out the callbacks this context submitted. Unlike `VecDeque::retain`,
            // this hands back the removed items so their (owned) contexts get freed by
            // running them with `canceled: true`.
            let mut keep = std::mem::take(&mut state.tasks);
            while let Some(front) = keep.pop_front() {
                if front.owner == owner {
                    pending.push(front);
                } else {
                    state.tasks.push_back(front);
                }
            }
            // If this was the last live context, the whole port is now dead for everyone.
            if state.active_contexts == 0 && !state.terminated {
                state.terminated = true;
                drain_rest = true;
            }
        }
        self.ready.notify_all();
        for task in pending {
            task.run(true);
        }
        if drain_rest {
            // No live contexts remain; anything still queued must be from an already-gone
            // context and would otherwise never run. Drain and cancel it.
            let rest: Vec<Task> = {
                let mut state = self.state.lock().expect("port state poisoned");
                state.tasks.drain(..).collect()
            };
            for task in rest {
                task.run(true);
            }
        }
        if wait {
            let mut state = self.state.lock().expect("port state poisoned");
            while state.in_flight > 0 {
                state = self.drained.wait(state).expect("port state poisoned");
            }
        }
    }

    /// Queue a callback, or run it inline for an [`DispatchMode::Immediate`] port.
    ///
    /// `owner` is the submitting context's id; `owner_terminated` says whether that context
    /// has already been terminated (in which case the callback is refused, matching the
    /// E_ABORT-on-terminated-queue contract without affecting other contexts on the port).
    /// Returns false if the port has been terminated (all its contexts are gone), which the
    /// caller reports as `E_ABORT`.
    pub fn submit(
        self: &Arc<Self>,
        owner: u64,
        owner_terminated: bool,
        context: *mut c_void,
        callback: TaskCallback,
        delay: Duration,
    ) -> bool {
        let task = Task {
            context,
            callback,
            owner,
            due: (!delay.is_zero()).then(|| Instant::now() + delay),
        };

        {
            let mut state = self.state.lock().expect("port state poisoned");
            if state.terminated || owner_terminated {
                return false;
            }

            // Immediate ports run on the submitting thread, so there is nothing to
            // queue - except that a delayed callback still has to wait somewhere, and
            // blocking the submitter would defeat the purpose.
            if self.mode == DispatchMode::Immediate && task.due.is_none() {
                state.in_flight += 1;
                drop(state);
                task.run(false);
                self.finish_one();
                return true;
            }
            if self.mode == DispatchMode::Immediate {
                let port = self.clone();
                std::thread::spawn(move || {
                    if let Some(due) = task.due {
                        let now = Instant::now();
                        if due > now {
                            std::thread::sleep(due - now);
                        }
                    }
                    let canceled = port.is_terminated();
                    task.run(canceled);
                });
                return true;
            }

            state.submitted += 1;
            // Keep the queue ordered by deadline so the head is always the next task to
            // become ready, which is what the wait below times out against.
            let position = state
                .tasks
                .iter()
                .position(|queued| match (queued.due, task.due) {
                    (Some(queued_due), Some(new_due)) => queued_due > new_due,
                    (Some(_), None) => true,
                    _ => false,
                })
                .unwrap_or(state.tasks.len());
            state.tasks.insert(position, task);
            self.start_workers(&mut state);
        }

        self.ready.notify_all();
        true
    }

    /// Thread-pool ports create their threads on first use, so a queue nobody submits to
    /// costs nothing. Manual and immediate ports never have workers.
    fn start_workers(self: &Arc<Self>, state: &mut PortState) {
        if state.workers_started {
            return;
        }
        let count = match self.mode {
            DispatchMode::SerializedThreadPool => 1,
            DispatchMode::ThreadPool => std::thread::available_parallelism()
                .map(|n| n.get().min(4))
                .unwrap_or(2),
            DispatchMode::Manual | DispatchMode::Immediate => return,
        };
        state.workers_started = true;
        for _ in 0..count {
            let port = self.clone();
            std::thread::spawn(move || port.worker());
        }
    }

    fn worker(self: Arc<Self>) {
        while let Some(task) = self.wait_for_task(None) {
            diag!(
                "WORKER running task owner={} ctx={:p} port={:p}",
                task.owner,
                task.context,
                Arc::as_ptr(&self)
            );
            task.run(false);
            self.finish_one();
        }
    }

    /// Take the next ready task, waiting up to `timeout` for one to appear or become
    /// due. Returns `None` on timeout or once the port is terminated and drained.
    fn wait_for_task(&self, timeout: Option<Duration>) -> Option<Task> {
        let deadline = timeout.map(|timeout| Instant::now() + timeout);
        let mut state = self.state.lock().expect("port state poisoned");
        loop {
            if state.terminated {
                return None;
            }

            let now = Instant::now();
            if state.tasks.front().is_some_and(|task| task.ready_at(now)) {
                state.in_flight += 1;
                return state.tasks.pop_front();
            }

            // Wake for whichever comes first: the caller's timeout, or the head task
            // becoming due.
            let next_due = state.tasks.front().and_then(|task| task.due);
            let wait_until = match (deadline, next_due) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };

            state = match wait_until {
                Some(until) => {
                    let now = Instant::now();
                    if until <= now {
                        if deadline.is_some_and(|deadline| deadline <= now) {
                            return None;
                        }
                        continue;
                    }
                    self.ready
                        .wait_timeout(state, until - now)
                        .expect("port state poisoned")
                        .0
                }
                None => self.ready.wait(state).expect("port state poisoned"),
            };
        }
    }

    fn finish_one(&self) {
        let mut state = self.state.lock().expect("port state poisoned");
        state.dispatched += 1;
        state.last_dispatch_ms = Some(now_ms());
        state.in_flight -= 1;
        drop(state);
        self.drained.notify_all();
    }

    /// Snapshot the tasks that are ready right now, popping them all under a single lock
    /// hold. Callers run the returned batch and then account for it in one `finish_n`.
    fn drain_ready(&self) -> Vec<Task> {
        let mut state = self.state.lock().expect("port state poisoned");
        let now = Instant::now();
        let mut out = Vec::new();
        while let Some(front) = state.tasks.front() {
            if !front.ready_at(now) {
                break;
            }
            let task = state.tasks.pop_front().expect("front just confirmed");
            state.in_flight += 1;
            out.push(task);
        }
        drop(state);
        out
    }

    fn finish_n(&self, n: usize) {
        if n == 0 {
            return;
        }
        let mut state = self.state.lock().expect("port state poisoned");
        state.dispatched += n as u64;
        state.last_dispatch_ms = Some(now_ms());
        state.in_flight -= n;
        drop(state);
        self.drained.notify_all();
    }

    /// Run callbacks pending on this port, waiting up to `timeout_ms` for the first one.
    /// The GDK contract for `XTaskQueueDispatch` is to process *all* callbacks currently
    /// queued on the port in one call. Running a single task per call strands later tasks
    /// whenever submissions outnumber dispatch calls - which makes the game re-issue asyncs
    /// it thinks never ran ("throttle / eventually loads"). So once the first callback is
    /// obtained we snapshot-drain every other callback already queued in one lock hold.
    /// Tasks submitted after the snapshot wait for the game's next dispatch call, so a busy
    /// producer can never pin the game thread inside this loop, and once the queued batch
    /// is done we return without blocking for new work (a hard freeze otherwise whenever
    /// submissions dry up and the rest of the process stalls).
    pub fn dispatch(&self, timeout_ms: u32) -> bool {
        if timeout_ms > 5000 {
            diag!(
                "Port::dispatch entering long wait, timeout_ms={timeout_ms} self={:p}",
                self
            );
        }
        let entered = now_ms();
        let (depth_on_entry, idle_ms) = {
            let state = self.state.lock().expect("port state poisoned");
            (
                state.tasks.len(),
                state
                    .last_dispatch_ms
                    .map(|ms| entered.saturating_sub(ms) as i128)
                    .unwrap_or(-1),
            )
        };
        let Some(first) = self.wait_for_task(Some(Duration::from_millis(timeout_ms as u64))) else {
            diag!(
                "dispatch EMPTY port={:p} mode={:?} timeout_ms={timeout_ms} since_last_ms={idle_ms}",
                std::ptr::from_ref(self),
                self.mode
            );
            return false;
        };
        let prev_in_dispatch = IN_DISPATCH.with(|f| f.replace(true));
        diag!(
            "Port::dispatch RUNNING task owner={} ctx={:p} port={:p}",
            first.owner,
            first.context,
            std::ptr::from_ref(self)
        );
        first.run(false);

        let batch = self.drain_ready();
        let total = 1 + batch.len();
        for task in batch {
            diag!(
                "Port::dispatch RUNNING task owner={} ctx={:p} port={:p}",
                task.owner,
                task.context,
                std::ptr::from_ref(self)
            );
            task.run(false);
        }
        IN_DISPATCH.with(|f| f.set(prev_in_dispatch));
        self.finish_n(total);
        let done = now_ms();
        let depth_after = self.state.lock().expect("port state poisoned").tasks.len();
        diag!(
            "dispatch RAN port={:p} mode={:?} ran={total} depth_on_entry={depth_on_entry} depth_after={depth_after} took_ms={} gap_since_last_ms={idle_ms}",
            std::ptr::from_ref(self),
            self.mode,
            done.saturating_sub(entered),
        );
        true
    }
}

struct Monitor {
    token: u64,
    context: *mut c_void,
    callback: MonitorCallback,
}

unsafe impl Send for Monitor {}

/// A queue's handle on an underlying [`Port`].
///
/// A primary queue owns the ports it created, but a *composite* queue borrows another
/// queue's ports via `XTaskQueueCreateComposite` — so several contexts can share one
/// `Port`. Termination is scoped to the context: cancelling a context cancels only the
/// callbacks *it* submitted (`Port::terminate_context`), and the shared port only dies
/// when its last context terminates - the `XTaskQueuePortContext` concept from
/// `xasyncprovider.idl`.
struct PortContext {
    port: Arc<Port>,
    /// Unique id tagging the callbacks this context submits, so `terminate_context` can
    /// find exactly the ones to cancel.
    owner: u64,
    terminated: AtomicBool,
}

impl PortContext {
    fn new(port: Arc<Port>) -> Self {
        let owner = NEXT_CONTEXT_ID.fetch_add(1, Ordering::Relaxed);
        port.add_context();
        Self {
            port,
            owner,
            terminated: AtomicBool::new(false),
        }
    }

    fn port(&self) -> &Arc<Port> {
        &self.port
    }

    fn is_terminated(&self) -> bool {
        self.terminated.load(Ordering::Relaxed)
    }

    fn submit(&self, context: *mut c_void, callback: TaskCallback, delay: Duration) -> bool {
        self.port
            .submit(self.owner, self.is_terminated(), context, callback, delay)
    }

    fn terminate(&self, wait: bool) {
        self.terminated.store(true, Ordering::Relaxed);
        self.port.terminate_context(self.owner, wait);
    }
}

/// A work port and a completion port, plus any registered monitors.
///
/// Each queue owns a [`PortContext`] for its work side and one for its completion side.
/// The underlying `Port`s may be shared with other queues (composites); the contexts are
/// what scope submission and termination.
pub struct Queue {
    work: PortContext,
    completion: PortContext,
    monitors: Mutex<Vec<Monitor>>,
    next_token: AtomicU64,
    /// A handle registered once, on first use, and never closed.
    ///
    /// Monitor callbacks hand the game a queue handle it is expected to retain and later
    /// pass back to `XTaskQueueDispatch` - often from a different thread than the one that
    /// submitted the work. A `QueueHandle::borrow()` value doesn't survive that: it is
    /// deliberately unregistered and only valid for the duration of the call that produced
    /// it. This handle is a real `QueueHandle::create()` entry, so it keeps resolving no
    /// matter when or from where the game dispatches it.
    canonical_handle: OnceLock<u64>,
}

impl Queue {
    pub fn new(work_mode: DispatchMode, completion_mode: DispatchMode) -> Arc<Self> {
        Self::composite(Port::new(work_mode), Port::new(completion_mode))
    }

    /// Create a queue whose work and completion ports are held *by this queue's own
    /// contexts* wrapping the given (possibly shared) ports. Used for both fresh primary
    /// queues (`new`) and composites; a composite just passes ports it borrowed.
    pub fn composite(work: Arc<Port>, completion: Arc<Port>) -> Arc<Self> {
        Arc::new(Self {
            work: PortContext::new(work),
            completion: PortContext::new(completion),
            monitors: Mutex::new(Vec::new()),
            next_token: AtomicU64::new(1),
            canonical_handle: OnceLock::new(),
        })
    }

    /// This queue's context for the given side.
    fn context(&self, kind: PortKind) -> &PortContext {
        match kind {
            PortKind::Work => &self.work,
            PortKind::Completion => &self.completion,
        }
    }

    /// The underlying shared port for the given side (the object other queues' contexts
    /// can also reference, e.g. when building a composite from this queue's ports).
    pub fn port(&self, kind: PortKind) -> &Arc<Port> {
        self.context(kind).port()
    }

    /// Submit to one of this queue's ports, notifying monitors first.
    ///
    /// Monitors run before the callback is queued because their documented purpose is to
    /// let a host wake up a manual queue's pump thread - it has to learn about the work
    /// no later than the thread that would dispatch it.
    pub fn submit(
        self: &Arc<Self>,
        kind: PortKind,
        context: *mut c_void,
        callback: TaskCallback,
        delay: Duration,
    ) -> bool {
        self.notify_monitors(kind);
        self.context(kind).submit(context, callback, delay)
    }

    fn notify_monitors(self: &Arc<Self>, kind: PortKind) {
        let monitors = self.monitors.lock().expect("monitor list poisoned");
        if monitors.is_empty() {
            return;
        }
        // The handle handed to a monitor must keep resolving after this call returns: the
        // documented use of a monitor is to wake a pump thread that later calls
        // XTaskQueueDispatch(handle, port) itself, possibly from another thread. A
        // borrow()-style ephemeral handle doesn't survive that, so this is a real,
        // permanently-registered handle instead (never closed, matching a queue the game
        // is still actively monitoring).
        let handle = *self
            .canonical_handle
            .get_or_init(|| QueueHandle::create(self.clone()));
        for monitor in monitors.iter() {
            unsafe { (monitor.callback)(monitor.context, handle, kind.as_raw()) };
        }
    }

    pub fn register_monitor(&self, context: *mut c_void, callback: MonitorCallback) -> u64 {
        let token = self.next_token.fetch_add(1, Ordering::Relaxed);
        self.monitors
            .lock()
            .expect("monitor list poisoned")
            .push(Monitor {
                token,
                context,
                callback,
            });
        token
    }

    pub fn unregister_monitor(&self, token: u64) {
        self.monitors
            .lock()
            .expect("monitor list poisoned")
            .retain(|monitor| monitor.token != token);
    }

    pub fn terminate(&self, wait: bool) {
        self.work.terminate(wait);
        self.completion.terminate(wait);
    }
}

/// Handles are `u64` on the wire. Each one owns a reference, so a duplicated handle is a
/// distinct value that keeps the queue alive on its own - a composite queue's source
/// queues are routinely closed while the composite is still in use.
pub struct QueueHandle;

static QUEUE_HANDLES: HandleTable<Arc<Queue>> = HandleTable::new();

impl QueueHandle {
    pub fn create(queue: Arc<Queue>) -> u64 {
        QUEUE_HANDLES.create(queue)
    }

    pub fn get(handle: u64) -> Option<Arc<Queue>> {
        QUEUE_HANDLES.get(handle)
    }

    pub fn close(handle: u64) {
        QUEUE_HANDLES.close(handle);
    }
}

/// The same arrangement for port handles, which `XTaskQueueGetPort` hands out and
/// `XTaskQueueCreateComposite` consumes.
pub struct PortHandle;

static PORT_HANDLES: HandleTable<Arc<Port>> = HandleTable::new();

impl PortHandle {
    pub fn create(port: Arc<Port>) -> u64 {
        PORT_HANDLES.create(port)
    }

    pub fn get(handle: u64) -> Option<Arc<Port>> {
        PORT_HANDLES.get(handle)
    }
}

static PROCESS_QUEUE: OnceLock<Mutex<Option<Arc<Queue>>>> = OnceLock::new();

fn process_queue_slot() -> &'static Mutex<Option<Arc<Queue>>> {
    PROCESS_QUEUE.get_or_init(|| Mutex::new(None))
}

static PROCESS_QUEUE_HANDLE: Mutex<Option<u64>> = Mutex::new(None);

/// A stable `u64` handle for the process queue, allocated once and cached.
///
/// `XAsyncBlock`s that name no queue get this handle written into their `queue`
/// field so libHttpClient-style providers that read `asyncBlock->queue` directly
/// (instead of going through `XTaskQueueGetCurrentProcessTaskQueue`) actually see a
/// non-NULL queue: the embedded XSAPI does exactly that, and a NULL queue
/// makes its `HttpCallPerformAsync` provider `Begin` return `E_INVALIDARG`. The
/// handle owns a table reference, so it stays valid across block reuse and queue
/// teardown for the lifetime of the process.
pub fn process_queue_handle() -> u64 {
    let mut cache = PROCESS_QUEUE_HANDLE
        .lock()
        .expect("process queue handle poisoned");
    if let Some(handle) = *cache {
        return handle;
    }
    let handle = QueueHandle::create(default_process_queue());
    *cache = Some(handle);
    handle
}

/// The queue used by an `XAsyncBlock` that names none.
///
/// Defaulting to a thread-pool queue matters: it is what makes a blocking
/// `XAsyncGetStatus(wait: true)` on a default block work at all. A manual default would
/// deadlock any caller that waits without also pumping.
pub fn default_process_queue() -> Arc<Queue> {
    let mut slot = process_queue_slot().lock().expect("process queue poisoned");
    slot.get_or_insert_with(|| Queue::new(DispatchMode::ThreadPool, DispatchMode::ThreadPool))
        .clone()
}

pub fn set_process_queue(queue: Option<Arc<Queue>>) {
    let clearing = queue.is_none();
    *process_queue_slot().lock().expect("process queue poisoned") = queue;
    if clearing {
        // Drop the cached handle so a later re-init allocates a fresh one instead of
        // pointing at the just-uninstalled (though still handle-table-alive) queue.
        *PROCESS_QUEUE_HANDLE
            .lock()
            .expect("process queue handle poisoned") = None;
    }
}

/// Whether `queue` is the current process task queue. Used to avoid re-routing a block that
/// already names the (pumped) process queue onto itself.
pub fn is_process_queue(queue: &Arc<Queue>) -> bool {
    Arc::ptr_eq(queue, &default_process_queue())
}

/// Whether a process queue has been set or lazily created yet - `XTaskQueueGetCurrentProcessTaskQueue`
/// reports this rather than creating one.
pub fn has_process_queue() -> bool {
    process_queue_slot()
        .lock()
        .expect("process queue poisoned")
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    struct Counter {
        ran: AtomicUsize,
        canceled: AtomicUsize,
    }

    unsafe extern "system" fn count(context: *mut c_void, canceled: bool) {
        let counter = unsafe { &*(context as *const Counter) };
        if canceled {
            counter.canceled.fetch_add(1, Ordering::SeqCst);
        } else {
            counter.ran.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn counter() -> Box<Counter> {
        Box::new(Counter {
            ran: AtomicUsize::new(0),
            canceled: AtomicUsize::new(0),
        })
    }

    #[test]
    fn a_manual_port_runs_nothing_until_dispatched() {
        let counter = counter();
        let context = (&*counter as *const Counter as *mut c_void).cast();
        let (port, ctx) = owned_port(DispatchMode::Manual);

        assert!(ctx.submit(context, count, Duration::ZERO));
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(
            counter.ran.load(Ordering::SeqCst),
            0,
            "a manual port must not run callbacks on its own"
        );

        assert!(port.dispatch(0));
        assert_eq!(counter.ran.load(Ordering::SeqCst), 1);
        assert!(!port.dispatch(0), "nothing left to dispatch");
    }

    /// A bare port plus the context that owns it, the way a Queue is constructed.
    fn owned_port(mode: DispatchMode) -> (Arc<Port>, Arc<PortContext>) {
        let port = Port::new(mode);
        let ctx = Arc::new(PortContext::new(port.clone()));
        (port, ctx)
    }

    #[test]
    fn a_thread_pool_port_runs_callbacks_without_dispatch() {
        let counter = counter();
        let context = (&*counter as *const Counter as *mut c_void).cast();
        let (port, ctx) = owned_port(DispatchMode::ThreadPool);

        for _ in 0..8 {
            assert!(ctx.submit(context, count, Duration::ZERO));
        }

        let deadline = Instant::now() + Duration::from_secs(5);
        while counter.ran.load(Ordering::SeqCst) < 8 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(counter.ran.load(Ordering::SeqCst), 8);
        ctx.terminate(true);
        assert!(port.is_terminated());
    }

    #[test]
    fn an_immediate_port_runs_on_the_submitting_thread() {
        let counter = counter();
        let context = (&*counter as *const Counter as *mut c_void).cast();
        let (_port, ctx) = owned_port(DispatchMode::Immediate);

        assert!(ctx.submit(context, count, Duration::ZERO));
        assert_eq!(
            counter.ran.load(Ordering::SeqCst),
            1,
            "an immediate callback has already run by the time submit returns"
        );
    }

    #[test]
    fn a_delayed_callback_is_not_ready_early() {
        let counter = counter();
        let context = (&*counter as *const Counter as *mut c_void).cast();
        let (port, ctx) = owned_port(DispatchMode::Manual);

        assert!(ctx.submit(context, count, Duration::from_millis(150)));
        assert!(!port.dispatch(10), "the delay has not elapsed");
        assert_eq!(counter.ran.load(Ordering::SeqCst), 0);

        assert!(port.dispatch(5_000), "dispatch should wait out the delay");
        assert_eq!(counter.ran.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn delayed_callbacks_are_dispatched_in_deadline_order() {
        static ORDER: Mutex<Vec<usize>> = Mutex::new(Vec::new());

        unsafe extern "system" fn record(context: *mut c_void, _canceled: bool) {
            ORDER.lock().unwrap().push(context as usize);
        }

        let (_port, ctx) = owned_port(DispatchMode::Manual);
        // Submitted latest-first, so insertion order alone would give the wrong answer.
        let port = ctx.port().clone();
        assert!(ctx.submit(3 as *mut c_void, record, Duration::from_millis(90)));
        assert!(ctx.submit(2 as *mut c_void, record, Duration::from_millis(60)));
        assert!(ctx.submit(1 as *mut c_void, record, Duration::from_millis(30)));

        // A single XTaskQueueDispatch drains every callback already queued (in deadline
        // order), not just one - that is the GDK contract. Delayed tasks that become ready
        // later are picked up by the next dispatch call rather than blocking this one.
        for _ in 0..3 {
            assert!(port.dispatch(5_000));
        }
        assert_eq!(*ORDER.lock().unwrap(), vec![1, 2, 3]);
        assert!(!port.dispatch(5_000), "port is now drained");
    }

    #[test]
    fn dispatch_drains_all_immediately_queued_callbacks_in_one_call() {
        let counter = counter();
        let context = (&*counter as *const Counter as *mut c_void).cast();
        let (port, ctx) = owned_port(DispatchMode::Manual);

        assert!(ctx.submit(context, count, Duration::ZERO));
        assert!(ctx.submit(context, count, Duration::ZERO));
        assert!(ctx.submit(context, count, Duration::ZERO));

        assert!(
            port.dispatch(10),
            "drains all three ready callbacks in one call"
        );
        assert_eq!(counter.ran.load(Ordering::SeqCst), 3);
        assert!(!port.dispatch(10), "nothing left after the drain");
    }

    #[test]
    fn terminating_cancels_queued_callbacks_rather_than_dropping_them() {
        let counter = counter();
        let context = (&*counter as *const Counter as *mut c_void).cast();
        let (_port, ctx) = owned_port(DispatchMode::Manual);

        assert!(ctx.submit(context, count, Duration::ZERO));
        assert!(ctx.submit(context, count, Duration::ZERO));
        ctx.terminate(true);

        assert_eq!(counter.ran.load(Ordering::SeqCst), 0);
        assert_eq!(
            counter.canceled.load(Ordering::SeqCst),
            2,
            "contexts are owned allocations; the callback has to run to free them"
        );
    }

    #[test]
    fn terminating_a_composite_does_not_kill_the_source_queues_shared_port() {
        // The regression this whole context model exists for: a composite borrows its
        // source queue's ports, so terminating the composite must ONLY cancel the
        // composite's in-flight callbacks - never work still queued by the source queue
        // on the same underlying port.
        let composite_box = counter();
        let context = (&*composite_box as *const Counter as *mut c_void).cast();

        let source_box = counter();
        let source_context = (&*source_box as *const Counter as *mut c_void).cast();

        let source = Queue::new(DispatchMode::Manual, DispatchMode::Manual);
        let composite = Queue::composite(
            source.port(PortKind::Work).clone(),
            source.port(PortKind::Completion).clone(),
        );

        // Source queue queues work on the shared port...
        assert!(source.submit(PortKind::Work, source_context, count, Duration::ZERO));
        // ...the composite queues work on the same shared port under its own context...
        assert!(composite.submit(PortKind::Work, context, count, Duration::ZERO));
        // ...and the composite is terminated.
        composite.terminate(true);

        // Only the composite's callback is cancelled; the source queue's is untouched
        // and still runs on the shared port.
        assert_eq!(composite_box.canceled.load(Ordering::SeqCst), 1);
        assert_eq!(source_box.canceled.load(Ordering::SeqCst), 0);
        assert!(
            source.port(PortKind::Work).dispatch(0),
            "the source queue's work on the shared port must still dispatch"
        );
        assert_eq!(
            source_box.ran.load(Ordering::SeqCst),
            1,
            "terminating the composite must not abort the source queue's own work"
        );
        // The shared port itself is still alive (the source context survives).
        assert!(!source.port(PortKind::Work).is_terminated());
    }

    #[test]
    fn a_composite_queue_borrows_ports_from_two_queues() {
        let counter = counter();
        let context = (&*counter as *const Counter as *mut c_void).cast();

        let work = Queue::new(DispatchMode::Manual, DispatchMode::Manual);
        let completion = Queue::new(DispatchMode::Manual, DispatchMode::Manual);
        let composite = Queue::composite(
            work.port(PortKind::Work).clone(),
            completion.port(PortKind::Completion).clone(),
        );

        assert!(composite.submit(PortKind::Completion, context, count, Duration::ZERO));

        // The callback went to the *completion* queue's completion port, not to
        // anything belonging to `work`.
        assert!(!work.port(PortKind::Completion).dispatch(0));
        assert!(completion.port(PortKind::Completion).dispatch(0));
        assert_eq!(counter.ran.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_composite_outlives_the_queues_it_borrowed_from() {
        let counter = counter();
        let context = (&*counter as *const Counter as *mut c_void).cast();

        let composite = {
            let source = Queue::new(DispatchMode::Manual, DispatchMode::Manual);
            Queue::composite(
                source.port(PortKind::Work).clone(),
                source.port(PortKind::Completion).clone(),
            )
        };

        assert!(composite.submit(PortKind::Work, context, count, Duration::ZERO));
        assert!(composite.port(PortKind::Work).dispatch(0));
        assert_eq!(counter.ran.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn duplicated_handles_are_distinct_and_independently_owned() {
        let queue = Queue::new(DispatchMode::Manual, DispatchMode::Manual);
        let first = QueueHandle::create(queue.clone());
        let second = QueueHandle::create(QueueHandle::get(first).unwrap());
        assert_ne!(first, second, "a duplicate is its own handle value");

        QueueHandle::close(first);
        // Closing one handle must leave the other usable.
        let still_open = QueueHandle::get(second).expect("second handle still open");
        assert!(Arc::ptr_eq(&still_open, &queue));
        drop(still_open);
        QueueHandle::close(second);
    }

    #[test]
    fn monitors_fire_on_submit_and_stop_after_unregister() {
        static SEEN: Mutex<Vec<u32>> = Mutex::new(Vec::new());

        unsafe extern "system" fn monitor(_context: *mut c_void, queue: u64, port: u32) {
            assert_ne!(queue, 0, "monitors receive a usable queue handle");
            SEEN.lock().unwrap().push(port);
        }

        unsafe extern "system" fn nop(_context: *mut c_void, _canceled: bool) {}

        let queue = Queue::new(DispatchMode::Manual, DispatchMode::Manual);
        let token = queue.register_monitor(std::ptr::null_mut(), monitor);

        queue.submit(PortKind::Work, std::ptr::null_mut(), nop, Duration::ZERO);
        queue.submit(
            PortKind::Completion,
            std::ptr::null_mut(),
            nop,
            Duration::ZERO,
        );
        assert_eq!(*SEEN.lock().unwrap(), vec![0, 1]);

        queue.unregister_monitor(token);
        queue.submit(PortKind::Work, std::ptr::null_mut(), nop, Duration::ZERO);
        assert_eq!(
            *SEEN.lock().unwrap(),
            vec![0, 1],
            "an unregistered monitor must not be called"
        );
    }
}
