//! `policy::hedge`: one duplicate for idempotent reads.
//!
//! Start the call; start one duplicate after a caller-supplied delay if the
//! first is still running; take the first success; cancel (interrupt + join)
//! the loser. The duplicate runs only when the first is slow: a fast first
//! returns before the delay elapses and the duplicate closure never runs.
//!
//! Idempotent reads only. Reads have no observable side effects, so running
//! the call twice is safe; never put this in the default stack, where an
//! arbitrary effect could run twice.
//!
//! Failure handling: a failure only wins when the other racer also fails
//! (the first cause is returned then). A lone failure keeps waiting for the
//! other racer, which may still succeed.

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::mpsc;

use crate::{ClockImpl, Ctx, Duration, Exit, ForkOpts, Runtime};

/// Which racer answered first.
enum Done<A, E> {
    First(Exit<A, E>),
    Dup(Exit<A, E>),
}

/// Race `first` against one delayed duplicate.
///
/// Blocks like [`crate::timeout`]: with a virtual clock, drive time from
/// another thread (wait for the delay sleep to park, then `adjust` past it).
pub fn hedge<A, E>(
    rt: &Runtime,
    clock: Arc<dyn ClockImpl>,
    delay: Duration,
    first: impl FnOnce(&Ctx) -> Exit<A, E> + Send + 'static,
    duplicate: impl FnOnce(&Ctx) -> Exit<A, E> + Send + 'static,
) -> Exit<A, E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    let (tx, rx) = mpsc::channel::<Done<A, E>>();
    let immediate = ForkOpts::immediate();

    let tx_first = tx.clone();
    let first_h = rt.fork_detach_with(
        move |ctx| {
            let exit = first(ctx);
            let _ = tx_first.send(Done::First(exit));
            Exit::<(), Infallible>::succeed(())
        },
        immediate,
    );

    // Give the first call the whole delay to finish alone. A fast first is
    // already in the channel when this returns, so the duplicate never runs.
    // (No interrupt source on the calling thread; virtual time is driven by
    // whoever holds the clock.)
    clock.sleep(delay, &|| false);
    if let Ok(Done::First(exit)) = rx.try_recv() {
        first_h.join();
        return exit;
    }

    let tx_dup = tx;
    let dup_h = rt.fork_detach_with(
        move |ctx| {
            let exit = duplicate(ctx);
            let _ = tx_dup.send(Done::Dup(exit));
            Exit::<(), Infallible>::succeed(())
        },
        immediate,
    );

    let (mut first_done, mut dup_done) = (false, false);
    let mut first_failure: Option<Exit<A, E>> = None;
    loop {
        match rx.recv().expect("a racer always answers") {
            Done::First(exit) => {
                first_done = true;
                match exit {
                    Exit::Success(_) => {
                        dup_h.interrupt();
                        dup_h.join();
                        first_h.join();
                        return exit;
                    }
                    failed => {
                        if dup_done {
                            // Both failed: the first cause wins.
                            first_h.join();
                            dup_h.join();
                            return failed;
                        }
                        first_failure = Some(failed);
                    }
                }
            }
            Done::Dup(exit) => {
                dup_done = true;
                match exit {
                    Exit::Success(_) => {
                        first_h.interrupt();
                        first_h.join();
                        dup_h.join();
                        return exit;
                    }
                    Exit::Failure(_) => {
                        if first_done {
                            // The first racer failed earlier (a success would
                            // have returned already): prefer the first cause.
                            first_h.join();
                            dup_h.join();
                            return first_failure
                                .take()
                                .expect("first failed before the duplicate");
                        }
                        // Otherwise the first is still running and may succeed.
                    }
                }
            }
        }
    }
}

// `policy::remote`: one value stacking filter, fallback, cache, budget
// timeout, retry, circuit breaker, rate limit, bulkhead, and attempt
// timeout around idempotent calls, innermost first.
//
// Attempt timeout starts once the call is admitted: neither limiter nor
// bulkhead queue waits are covered by it. Everything shared across calls
// (breaker state, bulkhead counts, cache) lives on the [`Remote`] handle;
// factories rebuild the one-shot effect per attempt.

use std::sync::{Condvar, Mutex};
use std::time::Instant;

use crate::schedule::{Random, Schedule};
use crate::{Cause, Effect, Reason};

/// A remote failure, typed so handlers can match on it. `Inner` is the
/// call's own typed error; the rest are stack decisions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteError<E> {
    /// The call failed with its own typed error.
    Inner(E),
    /// The breaker is open: no attempt ran.
    BreakerOpen,
    /// An attempt ran past its timeout (interrupted and released).
    AttemptTimeout,
    /// The whole operation ran past its budget.
    BudgetTimeout,
    /// The bulkhead queue is full: rejected without running.
    BulkheadFull,
    /// The filter rejected the call: it never ran.
    Filtered,
}

/// Bulkhead bounds: `permits` running, `queue` waiting, `wait` at most.
#[derive(Clone, Debug)]
pub struct Bulkhead {
    /// Concurrently running calls.
    pub permits: usize,
    /// Calls waiting for a permit.
    pub queue: usize,
    /// How long a waiter waits (wall clock) before rejection.
    pub wait: crate::Duration,
}

/// Breaker bounds: opens after `max_failures` consecutive typed failures,
/// half-opens one probe after `reset`.
#[derive(Clone, Debug)]
pub struct BreakerConfig {
    /// Consecutive typed failures that open the breaker.
    pub max_failures: u32,
    /// How long the open breaker rejects before one probe.
    pub reset: crate::Duration,
}

/// Admission control in front of a remote call. Blocks until admitted.
pub trait RateLimiter: Send + Sync {
    /// Block until the call is admitted.
    fn acquire(&self);
}

/// The remote stack as one value. Every knob overridable; nothing infinite:
/// 3 retries after the first attempt; 250 ms backoff; 10 s ceiling; full
/// jitter; 30 s attempt timeout; 2 min budget; bulkhead 8 running / 32 queued
/// / 5 s wait; no limiter, closed breaker, no cache, no fallback, no filter.
pub struct RemotePolicy<A, E> {
    /// Retries after the first attempt.
    pub retries: u32,
    /// Backoff base, doubled per attempt.
    pub backoff: crate::Duration,
    /// Backoff ceiling.
    pub ceiling: crate::Duration,
    /// Jitter seed.
    pub seed: u64,
    /// Per-attempt timeout, starting at admission.
    pub attempt_timeout: crate::Duration,
    /// Whole-operation budget.
    pub budget: crate::Duration,
    /// Bulkhead bounds.
    pub bulkhead: Bulkhead,
    /// Admission control. None admits immediately.
    pub limiter: Option<std::sync::Arc<dyn RateLimiter>>,
    /// Breaker bounds. None stays closed.
    pub breaker: Option<BreakerConfig>,
    /// Memoize successes per handle.
    pub cache: bool,
    /// Value substituted for a final typed failure.
    pub fallback: Option<std::sync::Arc<dyn Fn() -> A + Send + Sync>>,
    /// Pre-check: false rejects without running.
    pub filter: Option<std::sync::Arc<dyn Fn() -> bool + Send + Sync>>,
    /// Carries `E` for the typed fallible fields.
    pub marker: std::marker::PhantomData<fn() -> E>,
}

impl<A, E> Default for RemotePolicy<A, E> {
    fn default() -> Self {
        Self {
            retries: 3,
            backoff: crate::Duration::from_millis(250),
            ceiling: crate::Duration::from_secs(10),
            seed: 42,
            attempt_timeout: crate::Duration::from_secs(30),
            budget: crate::Duration::from_secs(120),
            bulkhead: Bulkhead {
                permits: 8,
                queue: 32,
                wait: crate::Duration::from_secs(5),
            },
            limiter: None,
            breaker: None,
            cache: false,
            fallback: None,
            filter: None,
            marker: std::marker::PhantomData,
        }
    }
}

impl<A: Clone, E> Clone for RemotePolicy<A, E> {
    fn clone(&self) -> Self {
        Self {
            retries: self.retries,
            backoff: self.backoff,
            ceiling: self.ceiling,
            seed: self.seed,
            attempt_timeout: self.attempt_timeout,
            budget: self.budget,
            bulkhead: self.bulkhead.clone(),
            limiter: self.limiter.clone(),
            breaker: self.breaker.clone(),
            cache: self.cache,
            fallback: self.fallback.clone(),
            filter: self.filter.clone(),
            marker: std::marker::PhantomData,
        }
    }
}

enum BreakerState {
    Closed { consecutive: u32 },
    Open { since_ms: u64 },
    HalfOpen,
}

struct BulkInner {
    running: usize,
    waiting: usize,
}

/// A built remote stack: shared breaker, bulkhead, and cache state.
pub struct Remote<A, E> {
    policy: RemotePolicy<A, E>,
    breaker: Mutex<BreakerState>,
    bulk: Mutex<BulkInner>,
    bulk_cond: Condvar,
    cache: Mutex<Option<A>>,
}

impl<A: Clone + Send + 'static, E: Send + 'static> Remote<A, E> {
    /// Build the stack. Nothing runs until [`call`](Remote::call).
    pub fn new(policy: RemotePolicy<A, E>) -> Self {
        Self {
            policy,
            breaker: Mutex::new(BreakerState::Closed { consecutive: 0 }),
            bulk: Mutex::new(BulkInner {
                running: 0,
                waiting: 0,
            }),
            bulk_cond: Condvar::new(),
            cache: Mutex::new(None),
        }
    }

    /// Run one call through the whole stack, innermost first: filter, cache
    /// lookup, limiter, bulkhead, then attempts (budget, breaker, timeout,
    /// backoff) with fallback on a final typed failure.
    pub fn call<F>(
        &self,
        rt: &Runtime,
        clock: &std::sync::Arc<dyn crate::ClockImpl>,
        make: F,
    ) -> Exit<A, RemoteError<E>>
    where
        F: Fn() -> Effect<A, E, ()> + Send + Sync + 'static,
    {
        if let Some(filter) = &self.policy.filter
            && !filter()
        {
            return Exit::Failure(Cause::fail(RemoteError::Filtered));
        }
        if self.policy.cache
            && let Some(hit) = self.cache.lock().unwrap().clone()
        {
            return Exit::Success(hit);
        }
        if let Some(limiter) = &self.policy.limiter {
            limiter.acquire();
        }
        // Bulkhead admission is outside the attempt timeout by construction:
        // the timeout starts after this block returns.
        {
            let mut inner = self.bulk.lock().unwrap();
            let permits = self.policy.bulkhead.permits;
            let queue = self.policy.bulkhead.queue;
            let wait = std::time::Duration::from_millis(self.policy.bulkhead.wait.as_millis());
            if inner.running < permits {
                inner.running += 1;
            } else if inner.waiting < queue {
                inner.waiting += 1;
                let deadline = Instant::now() + wait;
                loop {
                    if inner.running < permits {
                        inner.waiting -= 1;
                        inner.running += 1;
                        break;
                    }
                    let now = Instant::now();
                    if now >= deadline {
                        inner.waiting -= 1;
                        return Exit::Failure(Cause::fail(RemoteError::BulkheadFull));
                    }
                    let (guard, _) = self
                        .bulk_cond
                        .wait_timeout(inner, deadline.saturating_duration_since(now))
                        .unwrap();
                    inner = guard;
                }
            } else {
                return Exit::Failure(Cause::fail(RemoteError::BulkheadFull));
            }
        }
        let outcome = self.run_attempts(rt, clock, make);
        {
            let mut inner = self.bulk.lock().unwrap();
            inner.running -= 1;
            self.bulk_cond.notify_one();
        }
        if self.policy.cache
            && let Exit::Success(value) = &outcome
        {
            *self.cache.lock().unwrap() = Some(value.clone());
        }
        outcome
    }

    fn run_attempts<F>(
        &self,
        rt: &Runtime,
        clock: &std::sync::Arc<dyn crate::ClockImpl>,
        make: F,
    ) -> Exit<A, RemoteError<E>>
    where
        F: Fn() -> Effect<A, E, ()> + Send + Sync + 'static,
    {
        let start_ms = clock.now_millis();
        let backoff: Schedule<(), ()> = Schedule::exponential(self.policy.backoff)
            .capped(self.policy.ceiling)
            .jitter(Random::with_seed(self.policy.seed));
        let mut attempt: u32 = 0;
        let make = std::sync::Arc::new(make);
        loop {
            if clock.now_millis().saturating_sub(start_ms) > self.policy.budget.as_millis() {
                return Exit::Failure(Cause::fail(RemoteError::BudgetTimeout));
            }
            if !self.breaker_admit(clock.now_millis()) {
                return Exit::Failure(Cause::fail(RemoteError::BreakerOpen));
            }
            let attempt_make = std::sync::Arc::clone(&make);
            let outcome = crate::timeout(
                rt,
                std::sync::Arc::clone(clock),
                self.policy.attempt_timeout,
                move |_| match attempt_make().run_sync() {
                    Ok(a) => Exit::Success(a),
                    Err(cause) => Exit::Failure(cause),
                },
            );
            match outcome {
                Exit::Success(Some(a)) => {
                    self.breaker_success();
                    return Exit::Success(a);
                }
                Exit::Success(None) => {
                    // Timed out: interrupted and released by `timeout`. Not a
                    // typed failure, so the breaker does not count it.
                    if attempt >= self.policy.retries {
                        return Exit::Failure(Cause::fail(RemoteError::AttemptTimeout));
                    }
                }
                Exit::Failure(cause) => match only_inner_fail(&cause) {
                    Some(_) => {
                        self.breaker_typed_failure(clock.now_millis());
                        if attempt >= self.policy.retries {
                            return self.with_fallback(cause);
                        }
                    }
                    None => return Exit::Failure(map_cause(cause)),
                },
            }
            if let Some((_, delay)) = backoff.decide(attempt as u64, &()) {
                // The stack's own clock: virtual in tests (a TestClock
                // advance wakes this sleep), wall time in production.
                // Interruption is observed by the attempt timeout around
                // this call, so the sleep itself never needs the flag.
                let _ = clock.sleep(delay, &|| false);
            }
            attempt += 1;
        }
    }

    fn with_fallback(&self, cause: Cause<E>) -> Exit<A, RemoteError<E>> {
        if let Some(fallback) = &self.policy.fallback {
            return Exit::Success(fallback());
        }
        Exit::Failure(map_cause(cause))
    }

    fn breaker_admit(&self, now_ms: u64) -> bool {
        let cfg = match &self.policy.breaker {
            None => return true,
            Some(cfg) => cfg,
        };
        let mut state = self.breaker.lock().unwrap();
        match &mut *state {
            BreakerState::Closed { .. } => true,
            BreakerState::Open { since_ms } => {
                if now_ms.saturating_sub(*since_ms) >= cfg.reset.as_millis() {
                    *state = BreakerState::HalfOpen;
                    true
                } else {
                    false
                }
            }
            BreakerState::HalfOpen => false,
        }
    }

    fn breaker_success(&self) {
        if self.policy.breaker.is_some() {
            *self.breaker.lock().unwrap() = BreakerState::Closed { consecutive: 0 };
        }
    }

    fn breaker_typed_failure(&self, now_ms: u64) {
        let cfg = match &self.policy.breaker {
            None => return,
            Some(cfg) => cfg,
        };
        let mut state = self.breaker.lock().unwrap();
        match &mut *state {
            BreakerState::Closed { consecutive } => {
                *consecutive += 1;
                if *consecutive >= cfg.max_failures {
                    *state = BreakerState::Open { since_ms: now_ms };
                }
            }
            BreakerState::HalfOpen => {
                *state = BreakerState::Open { since_ms: now_ms };
            }
            BreakerState::Open { .. } => {}
        }
    }
}

/// The typed error if the cause is exactly one `Fail`.
fn only_inner_fail<E>(cause: &Cause<E>) -> Option<&E> {
    match cause.reasons.as_slice() {
        [Reason::Fail(e)] => Some(e),
        _ => None,
    }
}

/// Rewrap a call failure: typed errors become `Inner`, defects and
/// interrupts pass through untouched.
fn map_cause<E>(cause: Cause<E>) -> Cause<RemoteError<E>> {
    let mut out = Cause::empty();
    for reason in cause.reasons {
        out.reasons.push(match reason {
            Reason::Fail(e) => Reason::Fail(RemoteError::Inner(e)),
            Reason::Die(d) => Reason::Die(d),
            Reason::Interrupt { fiber_id } => Reason::Interrupt { fiber_id },
        });
    }
    out
}
