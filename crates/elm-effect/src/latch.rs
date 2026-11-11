//! An open/closed gate for fibers and threads.
//!
//! [`Latch::open`] admits every current and future waiter. [`Latch::release`]
//! admits only the waiters parked when it runs and leaves the latch closed.
//! [`Latch::await_open`] blocks until admission, and observes the current
//! fiber's interrupt request while it waits.

use std::sync::{Arc, Condvar, Mutex};

/// The caller was interrupted before the latch admitted it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LatchInterrupted;

struct Inner {
    state: Mutex<State>,
    changed: Condvar,
}

struct State {
    open: bool,
    next_ticket: u64,
    released_through: u64,
    waiters: usize,
}

/// An open/closed gate. Safe to share across fibers and threads.
pub struct Latch {
    inner: Arc<Inner>,
}

impl Latch {
    fn make(open: bool) -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    open,
                    next_ticket: 0,
                    released_through: 0,
                    waiters: 0,
                }),
                changed: Condvar::new(),
            }),
        }
    }

    /// A closed latch: waiters park until `open` or `release`.
    pub fn new_closed() -> Self {
        Self::make(false)
    }

    /// An open latch: waiters pass immediately.
    pub fn new_open() -> Self {
        Self::make(true)
    }

    /// True while open.
    pub fn is_open(&self) -> bool {
        self.inner.state.lock().unwrap().open
    }

    /// Admit current waiters and all future waiters. True when this call
    /// flipped the state.
    pub fn open(&self) -> bool {
        let mut state = self.inner.state.lock().unwrap();
        if state.open {
            return false;
        }
        state.open = true;
        self.inner.changed.notify_all();
        true
    }

    /// Admit only the waiters parked now, then leave the latch closed. True
    /// when the latch was closed; false when it was already open.
    pub fn release(&self) -> bool {
        let mut state = self.inner.state.lock().unwrap();
        if state.open {
            return false;
        }
        // Every ticket issued before this point belongs to a waiter that was
        // parked when release ran. Tickets issued later stay closed.
        state.released_through = state.next_ticket;
        self.inner.changed.notify_all();
        true
    }

    /// Close an open latch so later waiters park again. True when this call
    /// flipped the state.
    pub fn close(&self) -> bool {
        let mut state = self.inner.state.lock().unwrap();
        if !state.open {
            return false;
        }
        state.open = false;
        true
    }

    /// Block until the latch admits this caller.
    ///
    /// `interrupted` is checked while parked. A caller that is already
    /// interrupted does not park, and a caller interrupted later leaves
    /// without admission.
    pub fn await_open(&self, interrupted: &dyn Fn() -> bool) -> Result<(), LatchInterrupted> {
        let hook = crate::fiber::subscribe_current_interrupt({
            let inner = Arc::clone(&self.inner);
            move || {
                let _guard = inner.state.lock().unwrap();
                inner.changed.notify_all();
            }
        });
        let mut state = self.inner.state.lock().unwrap();
        if state.open {
            return Ok(());
        }
        if interrupted() {
            return Err(LatchInterrupted);
        }
        let ticket = state.next_ticket;
        state.next_ticket += 1;
        state.waiters += 1;
        loop {
            if state.open || ticket < state.released_through {
                state.waiters -= 1;
                return Ok(());
            }
            if interrupted() {
                state.waiters -= 1;
                return Err(LatchInterrupted);
            }
            state = if hook.is_some() {
                self.inner.changed.wait(state).unwrap()
            } else {
                self.inner
                    .changed
                    .wait_timeout(state, crate::COOPERATIVE_QUANTUM)
                    .unwrap()
                    .0
            };
        }
    }

    /// Waiters currently parked. Test aid.
    pub fn waiters(&self) -> usize {
        self.inner.state.lock().unwrap().waiters
    }
}
