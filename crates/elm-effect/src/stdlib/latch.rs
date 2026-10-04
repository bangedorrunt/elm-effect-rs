//! `Latch` port (`Latch.ts` at the pin): an open/closed gate.
//!
//! [`await_open`](Latch::await_open) parks until [`open`](Latch::open);
//! interruption of a waiter is cooperative (waiters poll an interrupt flag).
//! `open`/`close`/`release` report whether they flipped the state.

use std::sync::{Condvar, Mutex};

#[derive(Debug)]
struct State {
    open: bool,
    waiters: usize,
}

/// An open/closed gate. Threads park in [`await_open`](Latch::await_open).
#[derive(Debug)]
pub struct Latch {
    state: Mutex<State>,
    changed: Condvar,
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
            state: Mutex::new(State { open, waiters: 0 }),
            changed: Condvar::new(),
        }
    }

    /// `true` while open.
    pub fn is_open(&self) -> bool {
        self.state.lock().expect("latch").open
    }

    /// Open; `true` when this flipped the state.
    pub fn open(&self) -> bool {
        let mut guard = self.state.lock().expect("latch");
        if guard.open {
            return false;
        }
        guard.open = true;
        self.changed.notify_all();
        true
    }

    /// Close; `true` when this flipped the state.
    pub fn close(&self) -> bool {
        let mut guard = self.state.lock().expect("latch");
        if !guard.open {
            return false;
        }
        guard.open = false;
        true
    }

    /// Open alias mirroring `release` (latches release waiters by opening).
    pub fn release(&self) -> bool {
        self.open()
    }

    /// Park until open or `interrupted()` turns true. Returns `true` when
    /// the latch opened, `false` on interruption (the waiter is gone, so a
    /// later flush never sees it).
    pub fn await_open(&self, interrupted: &dyn Fn() -> bool) -> bool {
        let mut guard = self.state.lock().expect("latch");
        guard.waiters += 1;
        loop {
            if guard.open {
                guard.waiters -= 1;
                return true;
            }
            if interrupted() {
                guard.waiters -= 1;
                return false;
            }
            guard = self
                .changed
                .wait_timeout(guard, std::time::Duration::from_millis(1))
                .expect("latch")
                .0;
        }
    }

    /// Parked waiter count (test aid: asserts scheduling happened or not).
    pub fn waiters(&self) -> usize {
        self.state.lock().expect("latch").waiters
    }
}
