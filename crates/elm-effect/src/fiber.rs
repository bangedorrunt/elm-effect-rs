//! Fibers: spawned units of work with interruption and finalizers.
//!
//! A fiber body is `FnOnce(&Ctx) -> Exit<A, E>`, run on a worker thread under
//! the [`supervisor`](crate::supervisor). Sync-only [`Effect`]s run via
//! [`Ctx::spawn_effect`] (atomic with respect to interruption in this wave;
//! stepwise preemption lands with the async executor). Interruption is
//! cooperative: the supervisor checks the flag at every boundary it owns
//! (pre-start, post-body, pre-finalizers); loop bodies observe it through
//! [`Ctx::is_interrupted`]. Regions wrapped in [`Ctx::masked`] run to
//! completion: the flag fires when depth returns to zero.

use std::any::Any;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

use crate::scope::Scope;
use crate::{Cause, Effect, Exit, FiberId};

/// How a spawned fiber starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartMode {
    /// The child waits for a start latch: it begins when the spawning side
    /// next yields control (`join`, `interrupt`, block boundary, shutdown).
    /// No step runs before then. This is the default.
    Deferred,
    /// The child is eligible to run immediately.
    Immediate,
}

/// Initial interruptibility mask for a spawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mask {
    /// Interruptible from the start (the runtime default).
    Off,
    /// Start masked (depth 1); the first `masked` exit unmasks.
    On,
    /// Inherit the masked state of the spawning parent.
    Inherit,
}

/// Options for [`crate::Runtime`] fork operations.
#[derive(Clone, Copy, Debug)]
pub struct ForkOpts {
    pub start: StartMode,
    pub mask: Mask,
}

impl ForkOpts {
    pub fn deferred() -> Self {
        Self {
            start: StartMode::Deferred,
            mask: Mask::Off,
        }
    }

    pub fn immediate() -> Self {
        Self {
            start: StartMode::Immediate,
            mask: Mask::Off,
        }
    }
}

impl Default for ForkOpts {
    fn default() -> Self {
        Self::deferred()
    }
}

/// One-shot start gate for deferred fibers. Fired by join, interrupt, or
/// runtime shutdown: a deferred fiber that is never yielded to still ends.
#[derive(Default)]
pub(crate) struct StartLatch {
    state: (Mutex<bool>, Condvar),
}

impl StartLatch {
    pub(crate) fn fire(&self) {
        *self.state.0.lock().unwrap() = true;
        self.state.1.notify_all();
    }

    pub(crate) fn wait(&self) {
        let mut started = self.state.0.lock().unwrap();
        while !*started {
            started = self.state.1.wait(started).unwrap();
        }
    }
}

/// Shared fiber control block. Thread-safe; the interpreter never touches it.
pub(crate) struct FiberControl {
    pub id: FiberId,
    interrupt: AtomicBool,
    depth: AtomicUsize,
    /// Start gate: a completing parent fires its children's gates so a
    /// deferred child that never ran still ends (preempted) instead of
    /// hanging the parent's completion wait.
    pub(crate) start: Arc<StartLatch>,
    /// The fiber's own scope: finalizers registered here run at completion.
    pub scope: Scope,
    /// Extra scopes this fiber is bound to (`fork_in` / `fork_scoped`).
    bound: Vec<ScopeHandle>,
    pub parent: Option<FiberId>,
    done: (Mutex<bool>, Condvar),
    exit: Mutex<Option<Box<dyn Any + Send>>>,
}

/// A bound scope: shared ownership so the fiber keeps it alive.
type ScopeHandle = Arc<Scope>;

impl FiberControl {
    pub(crate) fn new(
        id: FiberId,
        parent: Option<FiberId>,
        bound: Vec<ScopeHandle>,
        masked: bool,
        start: Arc<StartLatch>,
    ) -> Self {
        Self {
            id,
            interrupt: AtomicBool::new(false),
            depth: AtomicUsize::new(usize::from(masked)),
            start,
            scope: Scope::new(id.0),
            bound,
            parent,
            done: (Mutex::new(false), Condvar::new()),
            exit: Mutex::new(None),
        }
    }

    pub(crate) fn request_interrupt(&self) {
        self.interrupt.store(true, Ordering::SeqCst);
    }

    pub fn is_interrupted(&self) -> bool {
        self.interrupt.load(Ordering::SeqCst)
    }

    pub(crate) fn masked_now(&self) -> bool {
        self.depth.load(Ordering::SeqCst) > 0
    }

    fn is_done(&self) -> bool {
        *self.done.0.lock().unwrap()
    }

    pub(crate) fn mark_done(&self, exit: Box<dyn Any + Send>) {
        *self.exit.lock().unwrap() = Some(exit);
        *self.done.0.lock().unwrap() = true;
        self.done.1.notify_all();
    }

    pub(crate) fn wait_done(&self) {
        let mut done = self.done.0.lock().unwrap();
        while !*done {
            done = self.done.1.wait(done).unwrap();
        }
    }
}

/// The context a fiber body runs with: interruption, finalizers, and
/// child-spawning bound to this fiber as parent.
pub struct Ctx {
    control: Arc<FiberControl>,
    driver: crate::runtime::Driver,
}

impl Ctx {
    pub(crate) fn new(control: Arc<FiberControl>, driver: crate::runtime::Driver) -> Self {
        Self { control, driver }
    }

    /// This fiber's id.
    pub fn id(&self) -> FiberId {
        self.control.id
    }

    /// The parent this fiber was forked from, if any.
    pub fn parent(&self) -> Option<FiberId> {
        self.control.parent
    }

    /// True once interruption was requested. Loop bodies poll this.
    pub fn is_interrupted(&self) -> bool {
        self.control.is_interrupted()
    }

    /// Register a finalizer on this fiber's scope. Runs LIFO at completion,
    /// exactly once, before `join`/`interrupt` return.
    pub fn add_finalizer(&self, f: impl FnOnce() + Send + 'static) {
        self.control.scope.add_finalizer(f);
    }

    /// Run `f` with the interruptibility depth raised by one. The interrupt
    /// flag still records; delivery waits until depth returns to zero.
    pub fn masked<T>(&self, f: impl FnOnce() -> T) -> T {
        self.control.depth.fetch_add(1, Ordering::SeqCst);
        let out = f();
        self.control.depth.fetch_sub(1, Ordering::SeqCst);
        out
    }

    /// Ids of the extra scopes this fiber is bound to (`fork_in` /
    /// `fork_scoped`). The binding keeps each scope alive while the fiber runs.
    pub fn bound_scopes(&self) -> Vec<u64> {
        self.control.bound.iter().map(|s| s.id()).collect()
    }

    /// Spawn a child bound to this fiber: it dies with the parent, and the
    /// parent's completion waits for the child's finalizers.
    pub fn fork_child<A, E, F>(&self, body: F, opts: ForkOpts) -> FiberHandle<A, E>
    where
        A: Send + 'static,
        E: Send + 'static,
        F: FnOnce(&Ctx) -> Exit<A, E> + Send + 'static,
    {
        let mask = match opts.mask {
            Mask::Inherit => self.control.depth.load(Ordering::SeqCst) > 0,
            Mask::On => true,
            Mask::Off => false,
        };
        self.driver
            .spawn(body, Some(self.control.id), Vec::new(), opts.start, mask)
    }

    /// Spawn bound to a caller-supplied scope: closing the scope interrupts
    /// the fiber and runs its finalizers.
    pub fn fork_scoped<A, E, F>(
        &self,
        scope: &Arc<Scope>,
        body: F,
        opts: ForkOpts,
    ) -> FiberHandle<A, E>
    where
        A: Send + 'static,
        E: Send + 'static,
        F: FnOnce(&Ctx) -> Exit<A, E> + Send + 'static,
    {
        self.driver.spawn(
            body,
            Some(self.control.id),
            vec![Arc::clone(scope)],
            opts.start,
            opts.mask == Mask::On,
        )
    }

    /// Spawn into an arbitrary scope (not necessarily the caller's).
    pub fn fork_in<A, E, F>(&self, scope: &Arc<Scope>, body: F, opts: ForkOpts) -> FiberHandle<A, E>
    where
        A: Send + 'static,
        E: Send + 'static,
        F: FnOnce(&Ctx) -> Exit<A, E> + Send + 'static,
    {
        self.fork_scoped(scope, body, opts)
    }

    /// Spawn a detached fiber from inside a running fiber (root scope).
    pub fn fork_detach<A, E, F>(&self, body: F, opts: ForkOpts) -> FiberHandle<A, E>
    where
        A: Send + 'static,
        E: Send + 'static,
        F: FnOnce(&Ctx) -> Exit<A, E> + Send + 'static,
    {
        self.driver
            .spawn(body, None, Vec::new(), opts.start, opts.mask == Mask::On)
    }

    /// Run a sync-only [`Effect`] as this fiber's child. Atomic with respect
    /// to interruption in this wave.
    pub fn spawn_effect<A, E>(&self, effect: Effect<A, E>) -> FiberHandle<A, E>
    where
        A: Send + 'static,
        E: Send + 'static,
    {
        self.fork_child(
            move |_| match effect.run_sync() {
                Ok(a) => Exit::Success(a),
                Err(cause) => Exit::Failure(cause),
            },
            ForkOpts::deferred(),
        )
    }
}

/// A fiber body, erased to a single object type at spawn. The spawn
/// function stays generic; only this box crosses the task boundary.
pub(crate) type Body<A, E> = Box<dyn FnOnce(&Ctx) -> Exit<A, E> + Send>;

/// A running (or finished) fiber. Joining yields its [`Exit`]..
pub struct FiberHandle<A: Send + 'static, E: Send + 'static> {
    control: Arc<FiberControl>,
    start: Arc<StartLatch>,
    driver: crate::runtime::Driver,
    marker: PhantomData<(A, E)>,
}

impl<A: Send + 'static, E: Send + 'static> FiberHandle<A, E> {
    pub(crate) fn new(
        control: Arc<FiberControl>,
        start: Arc<StartLatch>,
        driver: crate::runtime::Driver,
    ) -> Self {
        Self {
            control,
            start,
            driver,
            marker: PhantomData,
        }
    }

    pub(crate) fn control_ref(&self) -> &Arc<FiberControl> {
        &self.control
    }

    pub fn id(&self) -> FiberId {
        self.control.id
    }

    pub fn is_done(&self) -> bool {
        self.control.is_done()
    }

    /// Interrupt and wait until the target's finalizers finished
    /// (back-pressure). Fires the start latch first so a deferred fiber that
    /// never ran still ends. This is the default interrupter.
    pub fn interrupt(&self) {
        self.driver.drain_pending();
        self.start.fire();
        self.control.request_interrupt();
        self.control.wait_done();
    }

    /// Request interruption without waiting. The fiber may still be running
    /// finalizers when this returns.
    pub fn interrupt_unsafe(&self) {
        self.control.request_interrupt();
    }

    /// Register a finalizer from outside the fiber.
    pub fn add_finalizer(&self, f: impl FnOnce() + Send + 'static) {
        self.control.scope.add_finalizer(f);
    }

    /// Wait for completion and take the exit. Fires the start latch first.
    pub fn join(self) -> Exit<A, E> {
        self.driver.drain_pending();
        self.start.fire();
        self.control.wait_done();
        let mut guard = self.control.exit.lock().unwrap();
        let boxed = guard.take().expect("fiber done without an exit");
        *boxed
            .downcast::<Exit<A, E>>()
            .expect("fiber exit type mismatch")
    }
}

/// Next fiber id source. Exported for the runtime only.
static FIBER_IDS: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_id() -> FiberId {
    FiberId(FIBER_IDS.fetch_add(1, Ordering::Relaxed))
}

/// Children of one parent, for parent-completion waiting.
pub(crate) type Registry = Mutex<HashMap<u64, Vec<Arc<FiberControl>>>>;

pub(crate) fn children_of(registry: &Registry, parent: FiberId) -> Vec<Arc<FiberControl>> {
    registry
        .lock()
        .unwrap()
        .get(&parent.0)
        .cloned()
        .unwrap_or_default()
}

/// Pre-start interrupt exit: a fiber interrupted before its first step never
/// runs its body. Needs no `A`: only the interrupt reason is recorded.
pub(crate) fn preempted<A: Send + 'static, E: Send + 'static>() -> Exit<A, E> {
    Exit::Failure(Cause::interrupt(None))
}
