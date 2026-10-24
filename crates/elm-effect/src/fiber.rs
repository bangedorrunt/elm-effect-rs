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
use std::cell::RefCell;
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
    /// Resolve [`Mask::Inherit`] against the spawning fiber's mask. A spawn
    /// with no spawning fiber passes `false`, which makes `Inherit` mean
    /// `Off` (there is no parent state to inherit).
    pub(crate) fn resolve_mask(&self, parent_masked: bool) -> bool {
        match self.mask {
            Mask::Inherit => parent_masked,
            Mask::On => true,
            Mask::Off => false,
        }
    }

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

/// A generation-counted wake-up point: `notify` bumps the generation and
/// wakes every waiter, and a waiter re-checks its own predicate after waking.
/// One per fiber, so an interrupt or a child's completion reaches every thread
/// parked on that fiber without a poll.
pub(crate) struct Signal {
    generation: Mutex<u64>,
    condvar: Condvar,
}

impl Signal {
    fn new() -> Self {
        Self {
            generation: Mutex::new(0),
            condvar: Condvar::new(),
        }
    }

    pub(crate) fn notify(&self) {
        let mut generation = self.generation.lock().unwrap();
        *generation = generation.wrapping_add(1);
        self.condvar.notify_all();
    }

    /// Wait until `predicate` holds or the signal fires. The predicate is
    /// checked while the generation lock is held, so a notification that lands
    /// before the wait cannot be missed.
    pub(crate) fn wait(&self, predicate: &dyn Fn() -> bool) {
        let mut generation = self.generation.lock().unwrap();
        while !predicate() {
            generation = self.condvar.wait(generation).unwrap();
        }
    }

    /// Wait up to `timeout` for `predicate`; true when it holds.
    pub(crate) fn wait_timeout(
        &self,
        timeout: std::time::Duration,
        predicate: &dyn Fn() -> bool,
    ) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        let mut generation = self.generation.lock().unwrap();
        loop {
            if predicate() {
                return true;
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return predicate();
            }
            let (next, _) = self.condvar.wait_timeout(generation, remaining).unwrap();
            generation = next;
        }
    }
}

/// An interrupt waker registered with the current fiber. Dropping it
/// unregisters the waker, so a finished wait cannot be woken later.
pub(crate) struct InterruptHook {
    control: std::sync::Weak<FiberControl>,
    id: u64,
}

impl Drop for InterruptHook {
    fn drop(&mut self) {
        if let Some(control) = self.control.upgrade() {
            control
                .wakers
                .lock()
                .unwrap()
                .retain(|(id, _)| *id != self.id);
        }
    }
}

/// Shared fiber control block. Thread-safe; the interpreter never touches it.
pub(crate) struct FiberControl {
    pub id: FiberId,
    interrupt: AtomicBool,
    /// Wake point for anyone parked on this fiber: interrupt requests and
    /// child completions notify it.
    signal: Arc<Signal>,
    /// The parent's wake point, so a child's completion releases a parent
    /// waiting on it.
    parent_signal: Option<Arc<Signal>>,
    /// Condvar notifiers registered by interrupt-aware waits that wait on
    /// their own lock. Fired on interrupt, so those waits need no poll.
    wakers: Mutex<Vec<(u64, Arc<dyn Fn() + Send + Sync>)>>,
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
        parent: Option<Arc<FiberControl>>,
        bound: Vec<ScopeHandle>,
        masked: bool,
        start: Arc<StartLatch>,
    ) -> Self {
        let parent_id = parent.as_ref().map(|control| control.id);
        let parent_signal = parent.map(|control| Arc::clone(&control.signal));
        Self {
            id,
            interrupt: AtomicBool::new(false),
            signal: Arc::new(Signal::new()),
            parent_signal,
            wakers: Mutex::new(Vec::new()),
            depth: AtomicUsize::new(usize::from(masked)),
            start,
            scope: Scope::new(id.0),
            bound,
            parent: parent_id,
            done: (Mutex::new(false), Condvar::new()),
            exit: Mutex::new(None),
        }
    }

    pub(crate) fn request_interrupt(&self) {
        self.interrupt.store(true, Ordering::SeqCst);
        self.wake_interrupt_waiters();
    }

    /// Wake everyone parked on this fiber's interrupt. Called on a request,
    /// and again when the depth returns to zero: a wait that re-checked while
    /// masked would otherwise sleep through the delivery point.
    fn wake_interrupt_waiters(&self) {
        self.signal.notify();
        let wakers: Vec<Arc<dyn Fn() + Send + Sync>> = self
            .wakers
            .lock()
            .unwrap()
            .iter()
            .map(|(_, waker)| Arc::clone(waker))
            .collect();
        // Outside the lock: a waker may drop its hook, which locks `wakers`.
        for waker in wakers {
            waker();
        }
    }

    /// A pending interrupt becomes deliverable when the depth reaches zero.
    fn wake_if_deliverable(&self) {
        if self.is_interrupted() && !self.masked_now() {
            self.wake_interrupt_waiters();
        }
    }

    /// Wake every thread parked on this fiber.
    pub(crate) fn wait_change(&self, predicate: &dyn Fn() -> bool) {
        self.signal.wait(predicate);
    }

    /// Park until an interrupt is requested. No timeout, no poll: the waiting
    /// thread is woken by `request_interrupt` itself.
    pub(crate) fn wait_interrupted(&self) {
        self.signal.wait(&|| self.is_interrupted());
    }

    /// Register `waker` to run when this fiber is interrupted. The returned
    /// hook unregisters it on drop.
    pub(crate) fn subscribe_interrupt(
        self: &Arc<Self>,
        waker: Arc<dyn Fn() + Send + Sync>,
    ) -> InterruptHook {
        static WAKER_IDS: AtomicU64 = AtomicU64::new(1);
        let id = WAKER_IDS.fetch_add(1, Ordering::Relaxed);
        self.wakers.lock().unwrap().push((id, waker));
        InterruptHook {
            control: Arc::downgrade(self),
            id,
        }
    }

    pub fn is_interrupted(&self) -> bool {
        self.interrupt.load(Ordering::SeqCst)
    }

    pub(crate) fn masked_now(&self) -> bool {
        self.depth.load(Ordering::SeqCst) > 0
    }

    /// Raise the interruptibility depth (L7: releases run masked).
    pub(crate) fn mask(&self) {
        self.depth.fetch_add(1, Ordering::SeqCst);
    }

    /// Lower it again, floored at zero: `interruptible` inside a region nested
    /// deeper than one still means "deliver now", and an underflow would make
    /// a later `mask` read as unmasked.
    pub(crate) fn unmask(&self) {
        let _ = self
            .depth
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |depth| {
                Some(depth.saturating_sub(1))
            });
        self.wake_if_deliverable();
    }

    /// Raise the depth and hand back a guard that lowers it on drop, so a
    /// panic (or early return) inside a masked region can never leak depth.
    pub(crate) fn mask_guard(&self) -> MaskGuard<'_> {
        self.mask();
        MaskGuard { control: self }
    }

    /// `restore` in Effect 4: make the fiber interruptible for one region and
    /// hand back a guard that puts the previous depth back on drop.
    ///
    /// Swaps to zero rather than decrementing because Effect 4 models this as a
    /// flag, not a count: `interruptible` inside two nested `uninterruptible`
    /// regions is still interruptible (`alternating interruptibility` needs
    /// exactly that). Decrementing would leave depth 1 and stay masked.
    pub(crate) fn unmask_guard(&self) -> UnmaskGuard<'_> {
        let previous = self.depth.swap(0, Ordering::SeqCst);
        UnmaskGuard {
            control: self,
            previous,
        }
    }

    /// Register this fiber as the current one for this thread until the
    /// returned guard drops. The runtime holds it for the whole task, so
    /// [`crate::Scope::close`] finds the closing fiber whichever path called
    /// it.
    pub(crate) fn enter_current(self: &Arc<Self>) -> CurrentGuard {
        let prev = CURRENT_FIBER.with(|c| c.borrow_mut().replace(Arc::clone(self)));
        CurrentGuard { prev }
    }

    /// The id of the fiber running on this thread, if any. A scope close
    /// uses it to avoid waiting for the fiber that is closing it, which
    /// cannot finish until the close returns.
    pub(crate) fn current_id() -> Option<FiberId> {
        CURRENT_FIBER.with(|c| c.borrow().as_ref().map(|fiber| fiber.id))
    }

    /// Run `f` masked on the current thread's fiber. `None` when this thread
    /// runs no fiber: sync finalizers already run to completion there, so
    /// there is nothing to mask.
    pub(crate) fn with_current_masked<R>(f: impl FnOnce() -> R) -> Option<R> {
        let current = CURRENT_FIBER.with(|c| c.borrow().clone())?;
        let _guard = current.mask_guard();
        Some(f())
    }

    pub(crate) fn is_done(&self) -> bool {
        *self.done.0.lock().unwrap()
    }

    pub(crate) fn mark_done(&self, exit: Box<dyn Any + Send>) {
        *self.exit.lock().unwrap() = Some(exit);
        {
            let mut done = self.done.0.lock().unwrap();
            *done = true;
        }
        self.done.1.notify_all();
        // A parent or a combinator waits on the signal, not on this fiber's
        // own done latch. Notify after releasing `done`, so a waiter holding
        // the signal lock cannot deadlock against it.
        self.signal.notify();
        if let Some(parent) = &self.parent_signal {
            parent.notify();
        }
    }

    pub(crate) fn wait_done(&self) {
        // A fiber cannot wait for its own completion: the thread that would
        // mark it done is the one parked here. This is reachable through
        // `FiberHandle::interrupt` when a fiber holds its own handle, and the
        // guard keeps that mistake from hanging instead of failing loudly.
        if Self::current_id() == Some(self.id) {
            return;
        }
        let mut done = self.done.0.lock().unwrap();
        while !*done {
            done = self.done.1.wait(done).unwrap();
        }
    }

    /// Wait until done or the deadline passes; true when done. Used by a
    /// scope close so a member's releases finish before the scope releases
    /// the resources they use, without making `Drop` block forever.
    pub(crate) fn wait_done_until(&self, deadline: std::time::Instant) -> bool {
        let mut done = self.done.0.lock().unwrap();
        while !*done {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let (guard, _) = self.done.1.wait_timeout(done, remaining).unwrap();
            done = guard;
        }
        true
    }
}

/// Lowers the interruptibility depth when dropped. The unmask lives here
/// instead of after `f()` so a panic inside a masked region still restores
/// depth on unwind.
pub(crate) struct MaskGuard<'a> {
    control: &'a FiberControl,
}

impl Drop for MaskGuard<'_> {
    fn drop(&mut self) {
        self.control.unmask();
    }
}

/// Puts the interruptibility depth back when dropped: the other half of
/// [`FiberControl::unmask_guard`].
pub(crate) struct UnmaskGuard<'a> {
    control: &'a FiberControl,
    previous: usize,
}

impl Drop for UnmaskGuard<'_> {
    fn drop(&mut self) {
        self.control.depth.store(self.previous, Ordering::SeqCst);
        self.control.wake_if_deliverable();
    }
}

thread_local! {
    /// The fiber running on this thread, set for the whole task body by the
    /// runtime spawn closure and cleared when the task ends. Lets
    /// [`crate::Scope::close`] mask the closing fiber itself, so direct
    /// close, `Drop`, layer-failure, and dispose paths all run finalizers
    /// masked (L7) instead of relying on one caller to remember.
    static CURRENT_FIBER: RefCell<Option<Arc<FiberControl>>> = const { RefCell::new(None) };
}

/// Restores the previous thread-local fiber on drop, so a panic anywhere in
/// the task still clears the registration.
pub(crate) struct CurrentGuard {
    prev: Option<Arc<FiberControl>>,
}

impl Drop for CurrentGuard {
    fn drop(&mut self) {
        CURRENT_FIBER.with(|c| *c.borrow_mut() = self.prev.take());
    }
}

/// Register `waker` on the fiber running this thread, if there is one.
///
/// Interrupt-aware waits use this so their own condvar is signalled by
/// `request_interrupt`; a thread outside a fiber has no source to subscribe
/// to and falls back to a bounded wait.
pub(crate) fn subscribe_current_interrupt(
    waker: impl Fn() + Send + Sync + 'static,
) -> Option<InterruptHook> {
    let control = CURRENT_FIBER.with(|c| c.borrow().clone())?;
    Some(control.subscribe_interrupt(Arc::new(waker)))
}
/// The fiber running an effect, as [`Effect::with_fiber`] hands it over:
/// identity plus the cooperative interrupt flag. Wider control (finalizers,
/// spawning) stays on [`Ctx`], which a fiber body owns.
///
/// Effect 4's `fiber.context` has no counterpart here: services are immutable
/// per run, never a mutable per-fiber map, so a read of the running fiber's
/// context is not carried.
#[derive(Clone)]
pub struct Fiber {
    control: Arc<FiberControl>,
    /// Why a handle can spawn: [`Effect::fork_child`](crate::Effect::fork_child)
    /// runs inside an effect, which has no `Ctx`, so the spawn entry has to
    /// ride on the ambient fiber.
    driver: crate::runtime::Driver,
}

impl Fiber {
    /// This fiber's id. Same value [`Ctx::id`] reports for the body.
    pub fn id(&self) -> FiberId {
        self.control.id
    }

    /// Spawn a child bound to this fiber from inside an effect. Same contract
    /// as [`Ctx::fork_child`]: the child dies with this fiber and this fiber's
    /// completion waits for its finalizers.
    pub(crate) fn fork_child<A, E, F>(&self, body: F, opts: ForkOpts) -> FiberHandle<A, E>
    where
        A: Send + 'static,
        E: Send + 'static,
        F: FnOnce(&Ctx) -> Exit<A, E> + Send + 'static,
    {
        let mask = opts.resolve_mask(self.control.masked_now());
        self.driver.spawn(
            body,
            Some(Arc::clone(&self.control)),
            Vec::new(),
            opts.start,
            mask,
        )
    }

    /// The one spawn entry the four `Effect.fork*` forms share: `parented`
    /// decides whether the child dies with this fiber, `scopes` are the extra
    /// scopes whose close interrupts it, and `opts` decides when it starts.
    pub(crate) fn fork_effect_bound<A, E, R>(
        &self,
        effect: Effect<A, E, R>,
        ambient: crate::service::Services,
        parented: bool,
        scopes: Vec<Arc<Scope>>,
        opts: ForkOpts,
    ) -> FiberHandle<A, E>
    where
        A: Send + 'static,
        E: Send + 'static,
        R: crate::effect::Requirement,
    {
        let mask = opts.resolve_mask(self.control.masked_now());
        self.driver.spawn(
            move |ctx| {
                let mut maps = (*ambient).clone();
                maps.replace_tag(<FiberTag as crate::ServiceTag>::ID, Arc::new(ctx.fiber()));
                let services: crate::service::Services = Arc::new(maps);
                match effect.run_sync_with(&services) {
                    Ok(a) => Exit::Success(a),
                    Err(cause) => Exit::Failure(cause),
                }
            },
            parented.then(|| Arc::clone(&self.control)),
            scopes,
            opts.start,
            mask,
        )
    }

    /// The children registered to this fiber, as of now.
    ///
    /// [`Effect::await_all_children`](crate::Effect::await_all_children) takes
    /// this snapshot before running its body: a child that was already there
    /// belongs to an enclosing scope, not to the wrapped effect.
    pub(crate) fn children_snapshot(&self) -> Vec<Arc<FiberControl>> {
        self.driver.children_of(self.control.id)
    }

    /// Fire every deferred start latch. Effect 4's `yieldNow` does this; the
    /// await is where a parent first gives its deferred children the CPU.
    pub(crate) fn drain_pending(&self) {
        self.driver.drain_pending();
    }

    /// Run `f` with this fiber's interruptibility depth raised by one, lowered
    /// by a drop guard so a panic inside `f` still restores it.
    pub(crate) fn with_masked<R>(&self, f: impl FnOnce() -> R) -> R {
        let _guard = self.control.mask_guard();
        f()
    }

    /// Run `f` with the depth lowered by one: `restore` in Effect 4. Raising
    /// happens on drop, so a panic inside `f` still leaves the depth as it
    /// was.
    pub(crate) fn with_unmasked<R>(&self, f: impl FnOnce() -> R) -> R {
        let _guard = self.control.unmask_guard();
        f()
    }

    /// True once interruption was requested. Reads the same flag
    /// [`Ctx::is_interrupted`] does.
    pub fn is_interrupted(&self) -> bool {
        self.control.is_interrupted()
    }

    /// Park until `predicate` holds or this fiber is woken by an interrupt or
    /// a child's completion.
    pub(crate) fn wait_change(&self, predicate: &dyn Fn() -> bool) {
        self.control.wait_change(predicate);
    }

    /// Park until `predicate` holds, the signal fires, or `timeout` passes.
    pub(crate) fn wait_change_timeout(
        &self,
        timeout: std::time::Duration,
        predicate: &dyn Fn() -> bool,
    ) -> bool {
        self.control.signal.wait_timeout(timeout, predicate)
    }

    /// Park until an interrupt is requested. A fiber body uses this for the
    /// `Effect.never` shape: the request itself wakes the thread, so there is
    /// no poll and no latency beyond the scheduler.
    pub fn wait_interrupted(&self) {
        self.control.wait_interrupted();
    }

    /// Request interruption of this fiber. Non-blocking by construction: a
    /// fiber cannot wait for its own finalizers. The supervisor observes the
    /// flag at its next boundary, so a body that returns success after this
    /// still ends interrupted.
    pub fn interrupt(&self) {
        self.control.request_interrupt();
    }

    /// True while a masked region holds the interruptibility depth above zero,
    /// where a requested interrupt must wait. The interpreter polls this so a
    /// release still runs to completion after its fiber was interrupted.
    pub(crate) fn is_masked(&self) -> bool {
        self.control.masked_now()
    }
}

/// The ambient "which fiber is running this effect" tag. Injected only at the
/// spawn seams that run an effect on a fiber's behalf (`Ctx::spawn_effect`,
/// `Runtime::spawn_effect`); `Effect::with_fiber` reads it and no run without
/// a fiber carries one, so it is never user-providable.
pub(crate) struct FiberTag;

impl crate::ServiceTag for FiberTag {
    type Service = Fiber;
    const ID: &'static str = "elm-effect/fiber";
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

    /// This fiber as an effect running inside it sees it.
    pub fn fiber(&self) -> Fiber {
        Fiber {
            control: Arc::clone(&self.control),
            driver: Arc::clone(&self.driver),
        }
    }

    /// Ambient services for an effect this fiber runs on its own behalf:
    /// carries the [`FiberTag`] entry `Effect::with_fiber` reads. Freshly
    /// built per run, so the tag can never conflict with a provision.
    pub(crate) fn services_with_fiber(&self) -> crate::service::Services {
        let mut maps = crate::service::ServiceMaps::default();
        maps.insert_tag(<FiberTag as crate::ServiceTag>::ID, Arc::new(self.fiber()));
        Arc::new(maps)
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

    /// Register a finalizer that sees this fiber's exit. Same LIFO, once-only
    /// ordering; the callback reads success, typed-failure count, defects,
    /// and interruption through [`crate::ScopeExit`].
    pub fn add_finalizer_with_exit(&self, f: impl FnOnce(&crate::ScopeExit) + Send + 'static) {
        self.control.scope.add_finalizer_with_exit(f);
    }

    /// Run `f` with the interruptibility depth raised by one. The interrupt
    /// flag still records; delivery waits until depth returns to zero. The
    /// depth is lowered by a drop guard, so a panic in `f` still restores it.
    pub fn masked<T>(&self, f: impl FnOnce() -> T) -> T {
        let _guard = self.control.mask_guard();
        f()
    }

    /// True while any masked region (or release) holds the depth above zero.
    pub fn is_masked(&self) -> bool {
        self.control.masked_now()
    }

    /// Ids of the extra scopes this fiber is bound to (`fork_in` /
    /// `fork_scoped`). The binding keeps each scope alive while the fiber runs.
    pub fn bound_scopes(&self) -> Vec<u64> {
        self.control.bound.iter().map(|s| s.id()).collect()
    }

    /// Bracket acquisition to this fiber's own scope: success, typed
    /// failure, and interruption all release (L6). See [`Scope`].
    pub fn acquire_release<A, E, FRel, ERel>(
        &self,
        acquire: Effect<A, E>,
        release: FRel,
    ) -> Result<A, Cause<E>>
    where
        A: Send + 'static + Clone,
        E: Send + 'static,
        FRel: FnOnce(A) -> Effect<(), ERel> + Send + 'static,
        ERel: Send + 'static + std::fmt::Debug,
    {
        self.control.scope.acquire_release(acquire, release)
    }

    /// Spawn a child bound to this fiber: it dies with the parent, and the
    /// parent's completion waits for the child's finalizers.
    pub fn fork_child<A, E, F>(&self, body: F, opts: ForkOpts) -> FiberHandle<A, E>
    where
        A: Send + 'static,
        E: Send + 'static,
        F: FnOnce(&Ctx) -> Exit<A, E> + Send + 'static,
    {
        let mask = opts.resolve_mask(self.control.masked_now());
        self.driver.spawn(
            body,
            Some(Arc::clone(&self.control)),
            Vec::new(),
            opts.start,
            mask,
        )
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
            Some(Arc::clone(&self.control)),
            vec![Arc::clone(scope)],
            opts.start,
            opts.resolve_mask(self.control.masked_now()),
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
        self.driver.spawn(
            body,
            None,
            Vec::new(),
            opts.start,
            opts.resolve_mask(self.control.masked_now()),
        )
    }

    /// Run a sync-only [`Effect`] as this fiber's child. Atomic with respect
    /// to interruption in this wave. The child publishes itself under
    /// [`FiberTag`] for the run, so `Effect::with_fiber` inside `effect`
    /// reports the fiber actually executing it.
    pub fn spawn_effect<A, E>(&self, effect: Effect<A, E>) -> FiberHandle<A, E>
    where
        A: Send + 'static,
        E: Send + 'static,
    {
        self.fork_child(
            move |ctx| {
                let ambient = ctx.services_with_fiber();
                match effect.run_sync_with(&ambient) {
                    Ok(a) => Exit::Success(a),
                    Err(cause) => Exit::Failure(cause),
                }
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

    pub fn id(&self) -> FiberId {
        self.control.id
    }

    pub fn is_done(&self) -> bool {
        self.control.is_done()
    }

    /// Interrupt and wait until the target's finalizers finished
    /// (back-pressure). Fires the start latch first so a deferred fiber that
    /// never ran still ends. This is the default interrupter. The whole
    /// descendant subtree is interrupted too, in the same request.
    pub fn interrupt(&self) {
        self.driver.drain_pending();
        self.start.fire();
        self.control.request_interrupt();
        self.driver.interrupt_tree(self.control.id);
        self.control.wait_done();
    }

    /// Request interruption without waiting. The fiber and its descendants
    /// may still be running finalizers when this returns.
    pub fn interrupt_unsafe(&self) {
        self.control.request_interrupt();
        self.driver.interrupt_tree(self.control.id);
    }

    /// Register a finalizer from outside the fiber.
    pub fn add_finalizer(&self, f: impl FnOnce() + Send + 'static) {
        self.control.scope.add_finalizer(f);
    }

    /// Register an exit-aware finalizer from outside the fiber.
    pub fn add_finalizer_with_exit(&self, f: impl FnOnce(&crate::ScopeExit) + Send + 'static) {
        self.control.scope.add_finalizer_with_exit(f);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Runtime;

    /// Regression: `finish` removed the fiber's own child list but not the
    /// fiber from its parent's list, so a long-lived parent retained every
    /// child it ever forked. The registry is crate-private, so this witness
    /// has to live beside it rather than in `tests/`.
    #[test]
    fn a_joined_child_leaves_its_parent_registry() {
        let rt = Runtime::new();
        let handle = rt.fork_detach_with(
            move |ctx: &Ctx| -> Exit<(), String> {
                for _ in 0..8 {
                    let child =
                        ctx.fork_child(|_| Exit::<(), String>::succeed(()), ForkOpts::immediate());
                    assert_eq!(child.join(), Exit::Success(()));
                }
                // `finish` runs just after the child marks itself done, so
                // allow a bounded beat rather than assume same-instant order.
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                while !ctx.fiber().children_snapshot().is_empty() {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "a joined child stayed in its parent's registry"
                    );
                    std::thread::park_timeout(crate::COOPERATIVE_QUANTUM);
                }
                Exit::succeed(())
            },
            ForkOpts::immediate(),
        );
        assert_eq!(handle.join(), Exit::Success(()));
    }
}
