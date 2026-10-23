//! The runtime: worker threads, the fiber registry, and shutdown.
//!
//! Everything here is blocking from the caller's side. The Tokio runtime
//! exists to host fiber tasks (4 workers by default); orchestration uses
//! plain [`std::sync`] primitives so tests stay `#[test]`, never nested
//! runtimes. Interruption is cooperative: `shutdown` waits for fibers that
//! observe the flag and does not abort threads.

use std::collections::HashMap;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
};

use crate::executor::{Executor, TokioExecutor};
use crate::fiber::{
    Body, Ctx, FiberControl, FiberHandle, ForkOpts, Registry, StartLatch, StartMode, children_of,
    next_id, preempted,
};
use crate::scope::Scope;
use crate::supervisor;
use crate::{Cause, Defect, Effect, Exit, FiberId};

/// Upper bound for a clean [`Runtime::shutdown`]: live fibers that ignore
/// interrupts past this point are named and detached, never waited on
/// forever. Use [`Runtime::shutdown_with_deadline`] for a shorter bound.
pub const SHUTDOWN_DEADLINE: crate::Duration = crate::Duration::from_millis(30_000);

/// Cloneable spawn entry: what [`Ctx`] needs to fork without owning the runtime.
pub(crate) type Driver = Arc<Shared>;

pub(crate) struct Shared {
    exec: Box<dyn Executor>,
    /// Live fibers by id, for interrupt broadcast.
    live: Mutex<HashMap<u64, Arc<FiberControl>>>,
    children: Registry,
    root: Arc<Scope>,
    active: (Mutex<usize>, Condvar),
    pending: Mutex<Vec<Arc<StartLatch>>>,
    down: AtomicBool,
}

impl Shared {
    pub(crate) fn drain_pending(&self) {
        let latches = std::mem::take(&mut *self.pending.lock().unwrap());
        for latch in latches {
            latch.fire();
        }
    }

    /// The fibers registered as children of `id`, as of now.
    pub(crate) fn children_of(&self, id: FiberId) -> Vec<Arc<FiberControl>> {
        children_of(&self.children, id)
    }

    /// Request interruption for every descendant of `id`, depth first.
    ///
    /// Delivered when the interrupt is requested, not at the end of the
    /// interrupted body: a body blocked joining its own child - which is what
    /// `runtime::timeout` around a spawn does - would otherwise never reach
    /// the code that stops it, and the interrupt would wait forever.
    /// Non-blocking; each fiber ends on its own.
    pub(crate) fn interrupt_tree(&self, id: FiberId) {
        let mut pending = vec![id];
        while let Some(parent) = pending.pop() {
            for child in children_of(&self.children, parent) {
                // Fire the start latch first: a deferred child that never ran
                // still has to end, and only running lets it observe the flag.
                child.start.fire();
                child.request_interrupt();
                pending.push(child.id);
            }
        }
    }

    fn track(&self, control: &Arc<FiberControl>) {
        self.live
            .lock()
            .unwrap()
            .insert(control.id.0, Arc::clone(control));
        let mut active = self.active.0.lock().unwrap();
        *active += 1;
    }

    fn finish(&self, control: &Arc<FiberControl>) {
        self.live.lock().unwrap().remove(&control.id.0);
        self.children.lock().unwrap().remove(&control.id.0);
        let mut active = self.active.0.lock().unwrap();
        *active -= 1;
        self.active.1.notify_all();
    }

    /// The one spawn primitive. Everything (fork_child/scoped/in/detach)
    /// routes through here.
    pub(crate) fn spawn<A, E, F>(
        self: &Arc<Self>,
        body: F,
        parent: Option<FiberId>,
        bound: Vec<Arc<Scope>>,
        start: StartMode,
        masked: bool,
    ) -> FiberHandle<A, E>
    where
        A: Send + 'static,
        E: Send + 'static,
        F: FnOnce(&Ctx) -> Exit<A, E> + Send + 'static,
    {
        let latch = Arc::new(StartLatch::default());
        let control = Arc::new(FiberControl::new(
            next_id(),
            parent,
            bound.clone(),
            masked,
            Arc::clone(&latch),
        ));
        // Bind lifetime before the task can run: a scope that closes now
        // still interrupts this fiber.
        for scope in &bound {
            scope.add_member(&control);
        }
        if let Some(pid) = parent {
            self.children
                .lock()
                .unwrap()
                .entry(pid.0)
                .or_default()
                .push(Arc::clone(&control));
        }
        self.track(&control);
        if start == StartMode::Immediate {
            latch.fire();
        } else {
            self.pending.lock().unwrap().push(Arc::clone(&latch));
        }

        let this = Arc::clone(self);
        let task_control = Arc::clone(&control);
        let task_latch = Arc::clone(&latch);
        let body: Body<A, E> = Box::new(body);
        self.exec.spawn_blocking(Box::new(move || {
            task_latch.wait();
            // Own this thread for the task: Scope::close masks through it.
            let _current = task_control.enter_current();
            let ctx = Ctx::new(Arc::clone(&task_control), Arc::clone(&this));
            let mut exit: Exit<A, E> =
                if task_control.is_interrupted() && !task_control.masked_now() {
                    preempted()
                } else {
                    supervisor::run(&ctx, body)
                };
            // Children die with the parent: fire deferred starts so they can
            // observe the interrupt, then wait for their finalizers (child
            // completion runs them before marking done).
            for child in children_of(&this.children, task_control.id) {
                child.start.fire();
                child.request_interrupt();
            }
            for child in children_of(&this.children, task_control.id) {
                child.wait_done();
            }
            // L7: Scope::close masks the current fiber itself, so releases
            // run to completion, then cancellation takes effect. Spills
            // append as Die without erasing (L6).
            let spills = task_control.scope.close();
            match &mut exit {
                Exit::Success(_) if !spills.is_empty() => {
                    exit = Exit::Failure(Cause::from_defects(spills));
                }
                Exit::Failure(cause) => {
                    cause.append_defects(spills);
                }
                _ => {}
            }
            if matches!(exit, Exit::Success(_)) && task_control.is_interrupted() {
                exit = preempted();
            }
            task_control.mark_done(Box::new(exit));
            this.finish(&task_control);
        }));

        FiberHandle::new(control, latch, Arc::clone(self))
    }

    pub(crate) fn shutdown(&self) {
        let _ = self.shutdown_with_deadline(SHUTDOWN_DEADLINE);
    }

    /// Interrupt every live fiber and wait for finalizers up to `timeout`.
    /// Fibers still alive past the deadline are named and detached — their
    /// threads run on, but the caller is free — instead of hanging forever.
    /// A body that never observes interruption is unkillable by construction.
    pub(crate) fn shutdown_with_deadline(&self, timeout: crate::Duration) -> Vec<FiberId> {
        if self.down.swap(true, Ordering::SeqCst) {
            return Vec::new();
        }
        self.drain_pending();
        self.root.close();
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(timeout.as_millis());
        // Interrupt stragglers that forked during shutdown: keep sweeping
        // until no fiber is active, or the deadline passes.
        loop {
            // Deferred fibers forked mid-shutdown must still start to end.
            self.drain_pending();
            let live: Vec<Arc<FiberControl>> =
                self.live.lock().unwrap().values().cloned().collect();
            for control in &live {
                control.request_interrupt();
            }
            let mut active = self.active.0.lock().unwrap();
            if *active == 0 {
                return Vec::new();
            }
            if std::time::Instant::now() >= deadline {
                let stragglers: Vec<FiberId> = live.iter().map(|c| c.id).collect();
                return stragglers;
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let (guard, _) = self.active.1.wait_timeout(active, remaining).unwrap();
            active = guard;
        }
    }
}

/// The fiber runtime. Owns its executor (default 4 worker threads);
/// `shutdown` (or `drop`) interrupts every live fiber and waits for finalizers.
pub struct Runtime {
    shared: Arc<Shared>,
}

impl Runtime {
    pub fn new() -> Self {
        Self::with_executor(TokioExecutor::new())
    }

    /// A runtime driving fibers on `exec` instead of the shared Tokio pool:
    /// the substitution hook for deterministic tests, which pass their
    /// virtual executor here so fan-out, race, and timeout paths run
    /// without real threads or wall-clock time.
    pub fn with_executor(exec: impl Executor) -> Self {
        let exec: Box<dyn Executor> = Box::new(exec);
        Self {
            shared: Arc::new(Shared {
                exec,
                live: Mutex::new(HashMap::new()),
                children: Mutex::new(HashMap::new()),
                root: Arc::new(Scope::new(0)),
                active: (Mutex::new(0), Condvar::new()),
                pending: Mutex::new(Vec::new()),
                down: AtomicBool::new(false),
            }),
        }
    }

    /// Spawn a detached fiber on the root scope (no supervisor parameter).
    /// It ends at the latest at [`Runtime::shutdown`].
    pub fn fork_detach<A, E, F>(&self, body: F) -> FiberHandle<A, E>
    where
        A: Send + 'static,
        E: Send + 'static,
        F: FnOnce(&Ctx) -> Exit<A, E> + Send + 'static,
    {
        self.fork_detach_with(body, ForkOpts::default())
    }

    pub fn fork_detach_with<A, E, F>(&self, body: F, opts: ForkOpts) -> FiberHandle<A, E>
    where
        A: Send + 'static,
        E: Send + 'static,
        F: FnOnce(&Ctx) -> Exit<A, E> + Send + 'static,
    {
        let masked = opts.mask == crate::fiber::Mask::On;
        let handle = self.shared.spawn(
            body,
            None,
            vec![Arc::clone(&self.shared.root)],
            opts.start,
            masked,
        );
        self.shared.root.add_member(handle.control_ref());
        handle
    }

    /// Create a standalone scope for `fork_scoped` / `fork_in`. Closing it
    /// interrupts every fiber bound to it.
    pub fn create_scope(&self) -> Arc<Scope> {
        Arc::new(Scope::new(crate::fiber::next_id().0))
    }

    /// Spawn bound to a caller-supplied scope.
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
        let masked = opts.mask == crate::fiber::Mask::On;
        self.shared
            .spawn(body, None, vec![Arc::clone(scope)], opts.start, masked)
    }

    /// Spawn into an arbitrary scope.
    pub fn fork_in<A, E, F>(&self, scope: &Arc<Scope>, body: F, opts: ForkOpts) -> FiberHandle<A, E>
    where
        A: Send + 'static,
        E: Send + 'static,
        F: FnOnce(&Ctx) -> Exit<A, E> + Send + 'static,
    {
        self.fork_scoped(scope, body, opts)
    }

    /// Run a sync-only [`Effect`] as a detached fiber. The fiber publishes
    /// itself under `FiberTag` for the run, so `Effect::with_fiber` inside
    /// `effect` reports the fiber executing it.
    pub fn spawn_effect<A, E>(&self, effect: Effect<A, E>) -> FiberHandle<A, E>
    where
        A: Send + 'static,
        E: Send + 'static,
    {
        self.fork_detach(move |ctx| {
            let ambient = ctx.services_with_fiber();
            match effect.run_sync_with(&ambient) {
                Ok(a) => Exit::Success(a),
                Err(cause) => Exit::Failure(cause),
            }
        })
    }

    /// Interrupt every live fiber and wait until all finalizers finished.
    /// Bounded by [`SHUTDOWN_DEADLINE`]: fibers still alive past it are
    /// named in the returned list and detached instead of hanging the caller.
    /// Idempotent; also runs on `drop` (which discards the list).
    pub fn shutdown(&self) -> Vec<FiberId> {
        self.shared.shutdown_with_deadline(SHUTDOWN_DEADLINE)
    }

    /// Like [`shutdown`](Runtime::shutdown) with a caller-supplied bound.
    /// Returns the ids of fibers still alive past the deadline (empty means
    /// every finalizer finished). Name, don't hang: an unkillable body shows
    /// up here instead of blocking the caller forever.
    pub fn shutdown_with_deadline(&self, timeout: crate::Duration) -> Vec<FiberId> {
        self.shared.shutdown_with_deadline(timeout)
    }

    /// The edge: run `effect` to an [`Exit`]. The only place outside tests
    /// that interprets a description; library code never calls this.
    pub fn run<A, E>(effect: Effect<A, E>) -> Exit<A, E>
    where
        A: Send + 'static,
        E: Send + 'static,
    {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| effect.run_sync())) {
            Ok(Ok(a)) => Exit::Success(a),
            Ok(Err(cause)) => Exit::Failure(cause),
            Err(payload) => Exit::Failure(Cause::die(defect_from_panic(payload))),
        }
    }

    /// Turn an outcome into a process exit code: success 0, typed failure 1,
    /// defect 2, interrupt 130. `run_main` exits with this; tests pin it here.
    pub fn exit_code<A, E>(exit: &Exit<A, E>) -> i32 {
        match exit {
            Exit::Success(_) => 0,
            Exit::Failure(cause) if cause.is_interrupted() => 130,
            Exit::Failure(cause) if !cause.defects().is_empty() => 2,
            Exit::Failure(_) => 1,
        }
    }

    /// Own a whole process invocation: run to an [`Exit`], then exit with the
    /// outcome code: success 0, typed failure 1, defect 2, interrupt 130.
    /// Async signal delivery rides on the executor; this is the single place
    /// that turns an outcome into a process exit.
    pub fn run_main<A, E>(effect: Effect<A, E>) -> !
    where
        A: Send + 'static,
        E: Send + 'static + std::fmt::Debug,
    {
        let exit = Self::run(effect);
        if let Exit::Failure(cause) = &exit {
            if !cause.defects().is_empty() {
                eprintln!("defect: {:?}", cause.defects());
            } else if !cause.is_interrupted() {
                eprintln!("failure: {:?}", cause.failure());
            }
        }
        std::process::exit(Self::exit_code(&exit));
    }
}

/// Render an edge panic as a defect. Same shape as the supervisor's
/// payload rendering; kept local so the edge owns its boundary.
// ponytail: mirrors supervisor::defect_from_payload; unify if a third renderer appears.
/// A panic payload as a [`Defect`]. Shared with `Effect::on_exit`, which
/// turns a panicking finalizer into a defect beside the body's reasons.
pub(crate) fn defect_from_panic(payload: Box<dyn std::any::Any + Send>) -> Defect {
    let message = payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "edge panicked with a non-string payload".to_string());
    Defect::new(message)
}

/// An owned runtime for non-effect (imperative shell) callers: holds the
/// executor alive and shuts it down on drop.
pub struct ManagedRuntime {
    runtime: Runtime,
    /// Services built from the layer, when this runtime was made from one.
    services: Option<crate::BuiltServices>,
    /// The scope the layer was built in, owned by this runtime.
    ///
    /// Ownership is the point: a scope's finalizers run when it drops, so a
    /// locally-created build scope released every resource a layer acquired
    /// the instant `make` returned, before the runtime ran its first effect.
    /// Holding it here runs them when the runtime drops, which is after
    /// `dispose` has interrupted every live fiber.
    ///
    /// A scoped layer constructor (`Layer::from_scoped_factory`) registers its
    /// release here, so a pool or connection a layer acquires is freed on
    /// `dispose`, not at the end of the build.
    build_scope: Option<Arc<crate::Scope>>,
    /// The memo this runtime's layer built into, when built from a layer.
    /// [`memo`](ManagedRuntime::memo) hands it out so a second runtime can
    /// reuse the first's services (the pin's shared `MemoMap`).
    memo: Option<crate::LayerMemo>,
}

impl ManagedRuntime {
    /// Build with default pool knobs and no provided services.
    pub fn new() -> Self {
        Self {
            runtime: Runtime::new(),
            services: None,
            build_scope: None,
            memo: None,
        }
    }

    /// Build from a layer: the layer's services are constructed eagerly
    /// (this build is synchronous, like the pin's `ManagedRuntime.make`),
    /// and the resulting services are readable through
    /// [`context`](ManagedRuntime::context) as the pin's `contextEffect`
    /// exposes them.
    ///
    /// Construction failure is returned, not swallowed. This is the pin's
    /// shape: `ManagedRuntime.make` there yields an `Effect`, so a layer
    /// whose constructor fails fails the build and the caller cannot ignore
    /// it. Returning `Result` here is what keeps that property — the earlier
    /// `build(..).ok()` turned a refused database connection into a runtime
    /// indistinguishable from `new()`, and a services interpreter then
    /// reported "no services" instead of the cause.
    ///
    /// The build scope is owned by this runtime, so a layer's finalizers run
    /// when the runtime drops rather than when `make` returns.
    pub fn make<O: crate::layer::RequirementIds, E: Clone + Send + 'static>(
        layer: &crate::Layer<O, E, ()>,
    ) -> Result<Self, Cause<E>> {
        // `build` is only available on a fully-satisfied layer (no remaining
        // requirements), which is exactly what a runtime entry point takes.
        let scope = Arc::new(crate::Scope::new(0));
        let services = layer.build(&scope)?;
        Ok(Self {
            runtime: Runtime::new(),
            services: Some(services),
            build_scope: Some(scope),
            memo: Some(layer.memo()),
        })
    }

    /// The services this runtime was built with, if any.
    pub fn context(&self) -> Option<&crate::BuiltServices> {
        self.services.as_ref()
    }

    /// The build memo this runtime's layer used, if it was built from one.
    /// Pass it to [`Layer::with_memo`] to share the build with another
    /// runtime: the pin's `{ memoMap: other.memoMap }`.
    pub fn memo(&self) -> Option<crate::LayerMemo> {
        self.memo.clone()
    }

    /// Interrupt live fibers and wait for their finalizers, then release the
    /// layer's resources by closing the build scope. That order is the point:
    /// a request fiber's cleanup runs before the layer's `close`, which is
    /// what the pin's dispose rows observe. Idempotent.
    pub fn dispose(&self) {
        self.runtime.shutdown();
        if let Some(scope) = &self.build_scope {
            let _ = scope.close();
        }
    }

    /// Fork `effect` as a fiber bound to this runtime, so [`dispose`] or
    /// `shutdown` interrupts it before the build scope closes. The fiber runs
    /// with the runtime's built services ambient, so `use_service` resolves
    /// without an explicit `provide_from`.
    pub fn run_fork<A, E>(&self, effect: Effect<A, E, ()>) -> FiberHandle<A, E>
    where
        A: Send + 'static,
        E: Send + 'static,
    {
        let env = match &self.services {
            Some(built) => built.env(),
            None => crate::service::empty_services(),
        };
        // Immediate start, like the pin's `runFork`: the fiber is running
        // before `run_fork` returns, so a caller can wait on a signal it
        // raises without draining a deferred latch.
        self.runtime.fork_detach_with(
            move |ctx| {
                // Merge so the fiber tag (which `Effect::never` and
                // `Effect::with_fiber` read) sits alongside the layer's
                // services.
                let ambient = crate::service::merged_services(&ctx.services_with_fiber(), &env);
                match effect.run_sync_with(&ambient) {
                    Ok(a) => Exit::Success(a),
                    Err(cause) => Exit::Failure(cause),
                }
            },
            ForkOpts::immediate(),
        )
    }

    /// Borrow the managed runtime.
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// The edge for a non-effect caller (a CLI handler, a test, a host
    /// loop): run `effect` to an [`Exit`]. Delegates to
    /// [`Runtime::run`]; it lives here so library crates never name it (L3).
    ///
    /// When this runtime was built from a layer, the layer's services are
    /// ambient, so a `use_service` in `effect` resolves without an explicit
    /// `provide_from`. That is the pin's `runtime.runPromise(tag)`. A runtime
    /// from [`new`](ManagedRuntime::new) has no services and runs exactly as
    /// before.
    pub fn run<A, E, R>(&self, effect: Effect<A, E, R>) -> Exit<A, E>
    where
        A: Send + 'static,
        E: Send + 'static,
        R: crate::Requirement,
    {
        let env = match &self.services {
            Some(built) => built.env(),
            None => crate::service::empty_services(),
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| effect.run_sync_with(&env)))
        {
            Ok(Ok(a)) => Exit::Success(a),
            Ok(Err(cause)) => Exit::Failure(cause),
            Err(payload) => Exit::Failure(Cause::die(defect_from_panic(payload))),
        }
    }

    /// Interrupt live fibers and wait for finalizers, then release the
    /// layer's resources. Idempotent.
    pub fn shutdown(&self) {
        self.dispose();
    }
}

impl Drop for ManagedRuntime {
    /// Dispose on drop, like the pin's `await using`. `dispose` is
    /// idempotent, so an explicit call before the drop changes nothing.
    fn drop(&mut self) {
        self.dispose();
    }
}

impl Default for ManagedRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.shared.shutdown();
    }
}

/// Race `body` against a [`Clock`](crate::Clock) sleep: whichever completes
/// first wins. The loser is interrupted and RELEASED (finalizers run) before
/// the winner's value is returned. A timed-out body yields `Success(None)`;
/// a finished body yields its outcome with `Some` on success.
///
/// Blocking: with a virtual clock, drive time from another thread (fork the
/// racers, wait for them to park, then `adjust`).
pub fn timeout<A, E>(
    rt: &Runtime,
    clock: std::sync::Arc<dyn crate::ClockImpl>,
    duration: crate::Duration,
    body: impl FnOnce(&crate::Ctx) -> Exit<A, E> + Send + 'static,
) -> Exit<Option<A>, E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    enum Race<A, E> {
        Body(Exit<A, E>),
        Tick,
    }
    let (tx, rx) = std::sync::mpsc::channel::<Race<A, E>>();
    let tx_body = tx.clone();
    // Immediate start: timeout() blocks on the outcome, so deferred latches
    // would never fire (nobody yields). Racers begin at once.
    let immediate = crate::ForkOpts::immediate();
    let body_h = rt.fork_detach_with(
        move |ctx| {
            let exit = body(ctx);
            let _ = tx_body.send(Race::Body(exit));
            Exit::<(), std::convert::Infallible>::succeed(())
        },
        immediate,
    );
    let tx_tick = tx;
    let sleep_h = rt.fork_detach_with(
        move |ctx| {
            clock.sleep(duration, &|| ctx.is_interrupted());
            let _ = tx_tick.send(Race::Tick);
            Exit::<(), std::convert::Infallible>::succeed(())
        },
        immediate,
    );
    match rx.recv().expect("a racer always answers") {
        Race::Body(exit) => {
            // Body won: stop the sleeper (released) before returning.
            sleep_h.interrupt();
            sleep_h.join();
            body_h.join();
            match exit {
                Exit::Success(a) => Exit::Success(Some(a)),
                Exit::Failure(cause) => Exit::Failure(cause),
            }
        }
        Race::Tick => {
            // Timeout won: stop the body (interrupted AND released) first.
            body_h.interrupt();
            body_h.join();
            sleep_h.join();
            Exit::Success(None)
        }
    }
}

/// `Effect.timeout` (the failing variant) at the pin: like [`timeout`], but a
/// deadline that arrives first is a typed failure instead of `None`.
///
/// The pin raises its own `TimeoutError`; here the caller names the error its
/// channel carries, the same convention `Effect::from_option` and
/// [`head`](crate::head) use. `CauseError::timeout_after(duration)` is the
/// pin's message.
pub fn timeout_fail<A, E>(
    rt: &Runtime,
    clock: std::sync::Arc<dyn crate::ClockImpl>,
    duration: crate::Duration,
    body: impl FnOnce(&crate::Ctx) -> Exit<A, E> + Send + 'static,
    on_timeout: impl FnOnce(crate::Duration) -> E + Send + 'static,
) -> Exit<A, E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    match timeout(rt, clock, duration, body) {
        Exit::Success(Some(value)) => Exit::Success(value),
        Exit::Success(None) => Exit::Failure(Cause::fail(on_timeout(duration))),
        Exit::Failure(cause) => Exit::Failure(cause),
    }
}

/// Race two bodies: the first SUCCESS wins. A failure does not beat a later
/// success; if every side fails, the causes are concatenated. The loser is
/// interrupted and released (finalizers run) before the winner's value is
/// returned.
pub fn race<A, E>(
    rt: &Runtime,
    left: impl FnOnce(&crate::Ctx) -> Exit<A, E> + Send + 'static,
    right: impl FnOnce(&crate::Ctx) -> Exit<A, E> + Send + 'static,
) -> Exit<A, E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    enum Leg<A, E> {
        Left(Exit<A, E>),
        Right(Exit<A, E>),
    }
    let (tx, rx) = std::sync::mpsc::channel::<Leg<A, E>>();
    // Immediate start: race() blocks on the outcome, so deferred latches
    // would never fire (nobody yields). Racers begin at once.
    let immediate = crate::ForkOpts::immediate();
    let tx_left = tx.clone();
    let left_h = rt.fork_detach_with(
        move |ctx| {
            let exit = left(ctx);
            let _ = tx_left.send(Leg::Left(exit));
            Exit::<(), std::convert::Infallible>::succeed(())
        },
        immediate,
    );
    let right_h = rt.fork_detach_with(
        move |ctx| {
            let exit = right(ctx);
            let _ = tx.send(Leg::Right(exit));
            Exit::<(), std::convert::Infallible>::succeed(())
        },
        immediate,
    );
    // Finish a leg: interrupt the loser and join both before returning, so
    // releases complete before the winner's value is returned.
    macro_rules! finish {
        ($winner:expr, $loser:expr) => {{
            $loser.interrupt();
            $loser.join();
            $winner.join();
        }};
    }
    match rx.recv().expect("a racer always answers") {
        Leg::Left(Exit::Success(a)) => {
            finish!(left_h, right_h);
            Exit::Success(a)
        }
        Leg::Right(Exit::Success(a)) => {
            finish!(right_h, left_h);
            Exit::Success(a)
        }
        Leg::Left(Exit::Failure(first)) => match rx.recv().expect("both racers answer") {
            Leg::Left(Exit::Failure(second)) => {
                right_h.interrupt();
                right_h.join();
                left_h.join();
                Exit::Failure(concat_causes(first, second))
            }
            Leg::Right(Exit::Success(a)) => {
                finish!(left_h, right_h);
                Exit::Success(a)
            }
            Leg::Right(Exit::Failure(second)) => {
                right_h.join();
                left_h.join();
                Exit::Failure(concat_causes(first, second))
            }
            Leg::Left(Exit::Success(a)) => {
                finish!(left_h, right_h);
                Exit::Success(a)
            }
        },
        Leg::Right(Exit::Failure(first)) => match rx.recv().expect("both racers answer") {
            Leg::Right(Exit::Failure(second)) => {
                left_h.interrupt();
                left_h.join();
                right_h.join();
                Exit::Failure(concat_causes(first, second))
            }
            Leg::Left(Exit::Success(a)) => {
                finish!(right_h, left_h);
                Exit::Success(a)
            }
            Leg::Left(Exit::Failure(second)) => {
                left_h.join();
                right_h.join();
                Exit::Failure(concat_causes(first, second))
            }
            Leg::Right(Exit::Success(a)) => {
                finish!(right_h, left_h);
                Exit::Success(a)
            }
        },
    }
}

/// Race two bodies: any completion (success or failure) decides. The loser
/// is interrupted and released before the winner is returned.
pub fn race_first<A, E>(
    rt: &Runtime,
    left: impl FnOnce(&crate::Ctx) -> Exit<A, E> + Send + 'static,
    right: impl FnOnce(&crate::Ctx) -> Exit<A, E> + Send + 'static,
) -> Exit<A, E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    enum Leg<A, E> {
        Left(Exit<A, E>),
        Right(Exit<A, E>),
    }
    let (tx, rx) = std::sync::mpsc::channel::<Leg<A, E>>();
    let immediate = crate::ForkOpts::immediate();
    let tx_left = tx.clone();
    let left_h = rt.fork_detach_with(
        move |ctx| {
            let exit = left(ctx);
            let _ = tx_left.send(Leg::Left(exit));
            Exit::<(), std::convert::Infallible>::succeed(())
        },
        immediate,
    );
    let right_h = rt.fork_detach_with(
        move |ctx| {
            let exit = right(ctx);
            let _ = tx.send(Leg::Right(exit));
            Exit::<(), std::convert::Infallible>::succeed(())
        },
        immediate,
    );
    let outcome = rx.recv().expect("a racer always answers");
    // Whoever finished first decides; stop and reap the other first.
    let (winner, loser) = match &outcome {
        Leg::Left(_) => (left_h, right_h),
        Leg::Right(_) => (right_h, left_h),
    };
    loser.interrupt();
    loser.join();
    winner.join();
    match outcome {
        Leg::Left(exit) | Leg::Right(exit) => exit,
    }
}

/// Concatenate two failure causes, preserving order.
fn concat_causes<E>(mut first: crate::Cause<E>, second: crate::Cause<E>) -> crate::Cause<E> {
    first.reasons.extend(second.reasons);
    first
}

/// `Effect.raceAll` at the pin: run every body, and complete on the first
/// SUCCESS. A failure does not end the race -- it is a loser, interrupted
/// like any other, and the race continues to the first success. If every
/// body fails, the causes are concatenated. The winners' and losers'
/// releases all finish before the outcome is returned.
pub fn race_all<A: Send + 'static, E: Send + 'static>(
    rt: &Runtime,
    bodies: Vec<Body<A, E>>,
) -> Exit<A, E> {
    race_all_impl(rt, bodies, true)
}

/// `Effect.raceAllFirst` at the pin: run every body, and complete on the
/// first COMPLETION of any kind. A failure ends the race immediately (with
/// that failure), which is what distinguishes it from [`race_all`].
pub fn race_all_first<A: Send + 'static, E: Send + 'static>(
    rt: &Runtime,
    bodies: Vec<Body<A, E>>,
) -> Exit<A, E> {
    race_all_impl(rt, bodies, false)
}

/// Shared driver. `success_wins` selects `race_all` (true) over
/// `race_all_first` (false).
///
/// An empty list has no winner by construction. The pin fails with
/// NoSuchElementError; this port has no such type, so the empty case is a
/// named invariant rather than a silent default outcome.
fn race_all_impl<A: Send + 'static, E: Send + 'static>(
    rt: &Runtime,
    bodies: Vec<Body<A, E>>,
    success_wins: bool,
) -> Exit<A, E> {
    assert!(
        !bodies.is_empty(),
        "raceAll of an empty iterable has no winner; the pin fails with NoSuchElementError"
    );
    let (tx, rx) = std::sync::mpsc::channel::<Exit<A, E>>();
    let immediate = ForkOpts::immediate();
    let handles: Vec<FiberHandle<(), std::convert::Infallible>> = bodies
        .into_iter()
        .map(|body| {
            let tx = tx.clone();
            rt.fork_detach_with(
                move |ctx| {
                    let _ = tx.send(body(ctx));
                    Exit::<(), std::convert::Infallible>::succeed(())
                },
                immediate,
            )
        })
        .collect();
    // Drop the last sender: once every leg has answered, the channel closes
    // instead of blocking this thread on a nobody.
    drop(tx);

    let mut failures: Vec<crate::Reason<E>> = Vec::new();
    let winner = loop {
        match rx.recv() {
            Ok(Exit::Success(a)) => break Exit::Success(a),
            Ok(Exit::Failure(cause)) => {
                if !success_wins {
                    break Exit::Failure(cause);
                }
                failures.extend(cause.reasons);
            }
            // Every leg answered: the channel closed. Only reachable when
            // success_wins is true and every body failed.
            Err(_) => {
                break Exit::Failure(crate::Cause {
                    reasons: failures,
                    ..crate::Cause::empty()
                });
            }
        }
    };

    // Interrupt then join every leg, so all releases finish before returning.
    for handle in &handles {
        handle.interrupt();
    }
    for handle in handles {
        handle.join();
    }
    winner
}

/// `Effect.never` at the pin: an effect that never completes and can only end
/// by interruption. A fiber that must stay alive until its scope closes (the
/// `ManagedRuntime` dispose rows) is the caller.
///
/// Lives here rather than beside the other constructors so the change stays
/// inside one module's lease; an inherent impl may sit in any module of the
/// defining crate. Interruption is cooperative like everywhere else: this
/// parks in a short poll loop and ends interrupted once the fiber's flag is
/// set. Without the poll a body parked on a condvar would never observe the
/// flag, and `shutdown` (which waits for its fibers) would hang.
#[cfg(feature = "tokio")]
impl<A: Send + 'static, E: Send + 'static> Effect<A, E, ()> {
    pub fn never() -> Self {
        Effect::with_fiber(|fiber| {
            let fiber = fiber.clone();
            Effect::sync(move || {
                // ponytail: 1ms park; a condvar notified by request_interrupt
                // would remove the poll, but that touches FiberControl.
                while !fiber.is_interrupted() {
                    std::thread::park_timeout(std::time::Duration::from_millis(1));
                }
            })
            .flat_map(|()| Effect::<A, E, ()>::interrupt())
        })
    }
}
