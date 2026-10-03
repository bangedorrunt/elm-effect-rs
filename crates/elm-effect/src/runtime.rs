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

use crate::fiber::{
    Body, Ctx, FiberControl, FiberHandle, ForkOpts, Registry, StartLatch, StartMode, children_of,
    next_id, preempted,
};
use crate::scope::Scope;
use crate::supervisor;
use crate::{Effect, Exit, FiberId};

/// Cloneable spawn entry: what [`Ctx`] needs to fork without owning the runtime.
pub(crate) type Driver = Arc<Shared>;

pub(crate) struct Shared {
    tokio: tokio::runtime::Runtime,
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
        self.tokio.spawn_blocking(move || {
            task_latch.wait();
            let ctx = Ctx::new(Arc::clone(&task_control), Arc::clone(&this));
            let exit: Exit<A, E> = if task_control.is_interrupted() && !task_control.masked_now() {
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
            task_control.scope.close();
            task_control.mark_done(Box::new(exit));
            this.finish(&task_control);
        });

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

/// The fiber runtime. Owns 4 worker threads; `shutdown` (or `drop`)
/// interrupts every live fiber and waits for finalizers.
pub struct Runtime {
    shared: Arc<Shared>,
}

impl Runtime {
    pub fn new() -> Self {
        let tokio = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("elm-effect-fiber")
            .enable_all()
            .build()
            .expect("fiber runtime builds");
        Self {
            shared: Arc::new(Shared {
                tokio,
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
