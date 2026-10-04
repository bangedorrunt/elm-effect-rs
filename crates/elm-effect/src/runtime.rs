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

/// Cloneable spawn entry: what [`Ctx`] needs to fork without owning the runtime.
pub(crate) type Driver = Arc<Shared>;

pub(crate) struct Shared {
    exec: TokioExecutor,
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
            // L7: releases run masked to completion, then cancellation
            // takes effect. Spills append as Die without erasing (L6).
            task_control.mask();
            let spills = task_control.scope.close();
            task_control.unmask();
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
        if self.down.swap(true, Ordering::SeqCst) {
            return;
        }
        self.drain_pending();
        self.root.close();
        let live: Vec<Arc<FiberControl>> = self.live.lock().unwrap().values().cloned().collect();
        for control in live {
            control.request_interrupt();
        }
        // Interrupt stragglers that forked during shutdown: keep sweeping
        // until no fiber is active.
        loop {
            // Deferred fibers forked mid-shutdown must still start to end.
            self.drain_pending();
            let live: Vec<Arc<FiberControl>> =
                self.live.lock().unwrap().values().cloned().collect();
            for control in live {
                control.request_interrupt();
            }
            let mut active = self.active.0.lock().unwrap();
            if *active == 0 {
                break;
            }
            active = self.active.1.wait(active).unwrap();
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
        let exec = TokioExecutor::new();
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

    /// Run a sync-only [`Effect`] as a detached fiber.
    pub fn spawn_effect<A, E>(&self, effect: Effect<A, E>) -> FiberHandle<A, E>
    where
        A: Send + 'static,
        E: Send + 'static,
    {
        self.fork_detach(move |_| match effect.run_sync() {
            Ok(a) => Exit::Success(a),
            Err(cause) => Exit::Failure(cause),
        })
    }

    /// Interrupt every live fiber and wait until all finalizers finished.
    /// Idempotent; also runs on `drop`.
    pub fn shutdown(&self) {
        self.shared.shutdown();
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
            Err(payload) => Exit::Failure(Cause::die(edge_defect(payload))),
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
fn edge_defect(payload: Box<dyn std::any::Any + Send>) -> Defect {
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
}

impl ManagedRuntime {
    /// Build with default pool knobs.
    pub fn new() -> Self {
        Self {
            runtime: Runtime::new(),
        }
    }

    /// Borrow the managed runtime.
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// Interrupt live fibers and wait for finalizers. Idempotent.
    pub fn shutdown(&self) {
        self.runtime.shutdown();
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
