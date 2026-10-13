//! `Latch` port (`Latch.ts` at the pin): an open/closed gate with two
//! admission paths.
//!
//! `open` admits current and future waiters; `release` admits only the
//! waiters parked at that moment and leaves the latch closed. The pin
//! resumes a waiter through Effect's `Scheduler`, so admission here is
//! two-step: `open`/`release` move waiters into a pending queue and
//! [`Latch::run_pending`] is the scheduler tick that resumes them. Keeping
//! that split is what lets an interrupt drop a waiter from a resume that has
//! not run yet, so an interrupted waiter's body never executes.

use std::sync::Mutex;

/// Waiter identity, handed out in parking order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Waiter(u64);

/// What `await` did: complete at once, leave interrupted, or suspend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Await {
    /// The latch was open, so `await` completed immediately.
    Admitted,
    /// The caller was already interrupted, so `await` cleaned up and left.
    Interrupted,
    /// Suspended until [`Latch::run_pending`] or [`Latch::interrupt`].
    Waiting(Waiter),
}

#[derive(Debug)]
struct State {
    open: bool,
    next: u64,
    /// Suspended waiters, in parking order.
    parked: Vec<Waiter>,
    /// Waiters an `open`/`release` admitted whose resume has not run yet.
    pending: Vec<Waiter>,
}

/// An open/closed gate.
#[derive(Debug)]
pub struct Latch {
    state: Mutex<State>,
}

impl Latch {
    /// Closed latch.
    pub fn new_closed() -> Self {
        Self::make(false)
    }

    /// Open latch.
    pub fn new_open() -> Self {
        Self::make(true)
    }

    fn make(open: bool) -> Self {
        Self {
            state: Mutex::new(State {
                open,
                next: 0,
                parked: Vec::new(),
                pending: Vec::new(),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("latch")
    }

    /// `true` while open.
    pub fn is_open(&self) -> bool {
        self.lock().open
    }

    /// `open`: admit current waiters and keep admitting future ones.
    /// `true` when this flipped the state (`false` when already open).
    pub fn open(&self) -> bool {
        let mut state = self.lock();
        if state.open {
            return false;
        }
        state.open = true;
        let admitted = std::mem::take(&mut state.parked);
        state.pending.extend(admitted);
        true
    }

    /// `openUnsafe`, the synchronous variant. Admission is the same; the
    /// pin's effectful/unsafe pair differs only in API shape.
    pub fn open_unsafe(&self) -> bool {
        self.open()
    }

    /// `release`: admit exactly the waiters parked right now and leave the
    /// latch closed, so future waiters still suspend. `true` when the latch
    /// was closed (the pin's contract), `false` when it was already open.
    pub fn release(&self) -> bool {
        let mut state = self.lock();
        if state.open {
            return false;
        }
        let admitted = std::mem::take(&mut state.parked);
        state.pending.extend(admitted);
        true
    }

    /// `close`: future waiters suspend again. `true` when this flipped the
    /// state. Queued resumes are not withdrawn, which is what lets an
    /// earlier release survive a reentrant close.
    pub fn close(&self) -> bool {
        let mut state = self.lock();
        if !state.open {
            return false;
        }
        state.open = false;
        true
    }

    /// `closeUnsafe`, the synchronous variant.
    pub fn close_unsafe(&self) -> bool {
        self.close()
    }

    /// `await`: park unless the latch is open. `interrupted` is the fiber's
    /// interrupt flag, so an already-interrupted caller cleans up without
    /// parking.
    pub fn await_open(&self, interrupted: &dyn Fn() -> bool) -> Await {
        let mut state = self.lock();
        if state.open {
            return Await::Admitted;
        }
        if interrupted() {
            return Await::Interrupted;
        }
        let waiter = Waiter(state.next);
        state.next += 1;
        state.parked.push(waiter);
        Await::Waiting(waiter)
    }

    /// Interrupt a waiter: drop it from the parked and pending sets, so a
    /// resume that was already queued never runs its body. `true` when the
    /// waiter was found in either set.
    pub fn interrupt(&self, waiter: Waiter) -> bool {
        let mut state = self.lock();
        let parked = state.parked.len();
        state.parked.retain(|held| *held != waiter);
        let was_parked = state.parked.len() != parked;
        let pending = state.pending.len();
        state.pending.retain(|held| *held != waiter);
        let was_pending = state.pending.len() != pending;
        was_parked || was_pending
    }

    /// The scheduler tick: resume every pending waiter, in admission order.
    pub fn run_pending(&self) -> Vec<Waiter> {
        std::mem::take(&mut self.lock().pending)
    }

    /// Suspended waiter count (test aid).
    pub fn waiters(&self) -> usize {
        self.lock().parked.len()
    }

    /// Queued resume count (test aid: the pin's queued tasks).
    pub fn pending(&self) -> usize {
        self.lock().pending.len()
    }
}
