//! Retry policies as values, plus the [`retry`]/[`repeat`] drivers.
//!
//! A [`Schedule`] decides per attempt whether to continue and how long to
//! wait. Drivers re-run a factory (effects are one-shot, so the recipe is a
//! closure, like layers): [`retry`] after typed failure, [`repeat`] after
//! success. Defects and interrupts are never retried.
//!
//! Delays block the interpreting thread (`std::thread::sleep`): fibers run
//! on the blocking pool, so this composes the same way any blocking section
//! does.

use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use crate::effect::{ErasedStep, Step};
use crate::service::{Services, empty_services};
use crate::time::Duration;
use crate::{Cause, Effect, Reason};

/// A seeded, shareable random source (the `Random` service).
pub struct Random {
    state: Mutex<u64>,
}

impl Random {
    /// A generator with a fixed seed. Same seed, same sequence.
    pub fn with_seed(seed: u64) -> Self {
        Self {
            state: Mutex::new(seed | 1),
        }
    }

    /// Next pseudorandom unit in `[0, 1)`.
    pub fn next_unit(&self) -> f64 {
        let mut guard = self.state.lock().unwrap();
        // SplitMix64: good enough for jitter, no dependency.
        *guard = guard.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = *guard;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^= z >> 31;
        drop(guard);
        let bits = (z >> 11) | 0x3FF0000000000000u64;
        f64::from_bits(bits) - 1.0
    }
}

impl Clone for Random {
    fn clone(&self) -> Self {
        Self {
            state: Mutex::new(*self.state.lock().unwrap()),
        }
    }
}

/// The `Random` service tag.
pub struct RandomTag;

impl crate::service::ServiceTag for RandomTag {
    type Service = Random;
    const ID: &'static str = "elm-effect/random";
}

/// Retry policy as a value: given the attempt index (0-based count of
/// retries so far) and the input, decide whether to continue and how long
/// to wait. `None` stops.
type StepFn<In, Out> = Arc<dyn Fn(u64, &In) -> Option<(Out, Duration)> + Send + Sync + 'static>;

/// Marker for a driver loop over `A`, failing with `E`, needing `R`.
type LoopMarker<A, E, R, O> = PhantomData<fn() -> (A, E, R, O)>;

#[derive(Clone)]
pub struct Schedule<In: Send + Sync + 'static, Out: Send + 'static> {
    step: StepFn<In, Out>,
}

impl<In: Send + Sync + 'static, Out: Send + 'static> Schedule<In, Out> {
    /// Decide attempt `attempt`: `None` stops, otherwise the output and delay.
    pub fn decide(&self, attempt: u64, input: &In) -> Option<(Out, Duration)> {
        (self.step)(attempt, input)
    }
}

impl<In: Send + Sync + 'static> Schedule<In, u64> {
    /// Up to `n` retries after the first attempt, with no wait. Compose with
    /// a backoff (via [`max`](Schedule::max)) to add delays.
    pub fn recurs(n: u64) -> Schedule<In, u64> {
        Schedule {
            step: Arc::new(move |attempt, _| {
                if attempt < n {
                    Some((attempt, Duration::ZERO))
                } else {
                    None
                }
            }),
        }
    }
}

impl<In: Send + Sync + 'static> Schedule<In, ()> {
    /// A fixed wait per attempt, continuing forever.
    pub fn spaced(delay: Duration) -> Schedule<In, ()> {
        Schedule {
            step: Arc::new(move |_, _| Some(((), delay))),
        }
    }

    /// Exponential backoff from `base`, doubling per attempt (saturating),
    /// continuing forever. Bound it with [`recurs`](Schedule::recurs) via
    /// [`max`](Schedule::max).
    pub fn exponential(base: Duration) -> Schedule<In, ()> {
        Schedule {
            step: Arc::new(move |attempt, _| {
                let shift = attempt.min(63);
                let millis = base.as_millis().saturating_mul(1u64 << shift);
                Some(((), Duration::from_millis(millis)))
            }),
        }
    }
}

impl<In: Send + Sync + 'static, A: Send + 'static> Schedule<In, A> {
    /// Cap every delay at `limit`.
    pub fn capped(self, limit: Duration) -> Self {
        let step = self.step;
        Schedule {
            step: Arc::new(move |attempt, input| {
                step(attempt, input).map(|(out, delay)| {
                    let capped = delay.as_millis().min(limit.as_millis());
                    (out, Duration::from_millis(capped))
                })
            }),
        }
    }

    /// Scale every delay by a factor in `[0.5, 1.5)` drawn from `rng`.
    pub fn jitter(self, rng: Random) -> Self {
        let step = self.step;
        Schedule {
            step: Arc::new(move |attempt, input| {
                step(attempt, input).map(|(out, delay)| {
                    let factor = 0.5 + rng.next_unit();
                    let millis = (delay.as_millis() as f64 * factor) as u64;
                    (out, Duration::from_millis(millis))
                })
            }),
        }
    }

    /// Continue only while `pred` holds (e.g. `error.retryable`).
    pub fn given(self, pred: impl Fn(&In) -> bool + Send + Sync + 'static) -> Self {
        let step = self.step;
        Schedule {
            step: Arc::new(move |attempt, input| {
                if pred(input) {
                    step(attempt, input)
                } else {
                    None
                }
            }),
        }
    }

    /// Continue while every member continues; wait the longer delay.
    pub fn max<B: Send + 'static>(self, second: Schedule<In, B>) -> Schedule<In, (A, B)> {
        let first = self;
        Schedule {
            step: Arc::new(move |attempt, input| {
                match (first.decide(attempt, input), second.decide(attempt, input)) {
                    (Some((a, da)), Some((b, db))) => {
                        let longer = if da.as_millis() >= db.as_millis() {
                            da
                        } else {
                            db
                        };
                        Some(((a, b), longer))
                    }
                    _ => None,
                }
            }),
        }
    }

    /// Continue while any member continues; wait the shorter of the decided
    /// delays. A stopped side contributes `None` to the output pair.
    pub fn min<B: Send + 'static>(
        self,
        second: Schedule<In, B>,
    ) -> Schedule<In, (Option<A>, Option<B>)> {
        let first = self;
        Schedule {
            step: Arc::new(move |attempt, input| {
                let a = first.decide(attempt, input);
                let b = second.decide(attempt, input);
                match (&a, &b) {
                    (None, None) => None,
                    _ => {
                        let mut millis = u64::MAX;
                        if let Some((_, d)) = &a {
                            millis = millis.min(d.as_millis());
                        }
                        if let Some((_, d)) = &b {
                            millis = millis.min(d.as_millis());
                        }
                        let (oa, ob) = (a.map(|(o, _)| o), b.map(|(o, _)| o));
                        Some(((oa, ob), Duration::from_millis(millis)))
                    }
                }
            }),
        }
    }
}

/// Re-run the recipe after typed failure, until the policy stops. Defects
/// and interrupts are never retried: they pass through on first sight.
/// Effects are one-shot, so the recipe is a factory (like layers).
pub fn retry<A, E, R, O, F>(make: F, policy: Schedule<E, O>) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + Sync + 'static,
    R: crate::effect::Requirement,
    O: Send + 'static,
    F: Fn() -> Effect<A, E, R> + Send + Sync + 'static,
{
    Effect {
        step: Step::Suspend(Box::new(RetryLoop {
            make: Arc::new(make),
            policy,
            marker: PhantomData,
        })),
        services: empty_services(),
        marker: PhantomData,
    }
}

/// Re-run the recipe after success, until the policy stops, returning the
/// last value. Any failure stops immediately and is returned as-is.
pub fn repeat<A, E, R, O, F>(make: F, policy: Schedule<A, O>) -> Effect<A, E, R>
where
    A: Send + Sync + 'static,
    E: Send + 'static,
    R: crate::effect::Requirement,
    O: Send + 'static,
    F: Fn() -> Effect<A, E, R> + Send + Sync + 'static,
{
    Effect {
        step: Step::Suspend(Box::new(RepeatLoop {
            make: Arc::new(make),
            policy,
            marker: PhantomData,
        })),
        services: empty_services(),
        marker: PhantomData,
    }
}

struct RetryLoop<A, E: Send + Sync + 'static, R, O: Send + 'static, F> {
    make: Arc<F>,
    policy: Schedule<E, O>,
    marker: LoopMarker<A, E, R, O>,
}

impl<A, E, R, O, F> ErasedStep<A, E> for RetryLoop<A, E, R, O, F>
where
    A: Send + 'static,
    E: Send + Sync + 'static,
    R: crate::effect::Requirement,
    O: Send + 'static,
    F: Fn() -> Effect<A, E, R> + Send + Sync,
{
    fn run_erased(self: Box<Self>, scope: &Services) -> Result<A, Cause<E>> {
        let mut attempt = 0u64;
        loop {
            let effect = (self.make)();
            match effect.run_sync_with(scope) {
                Ok(a) => return Ok(a),
                Err(cause) => match only_fail(&cause) {
                    Some(input) => match self.policy.decide(attempt, input) {
                        None => return Err(cause),
                        Some((_, delay)) => {
                            std::thread::sleep(std::time::Duration::from_millis(delay.as_millis()));
                            attempt += 1;
                        }
                    },
                    None => return Err(cause),
                },
            }
        }
    }
}

struct RepeatLoop<A: Send + Sync + 'static, E, R, O: Send + 'static, F> {
    make: Arc<F>,
    policy: Schedule<A, O>,
    marker: LoopMarker<A, E, R, O>,
}

impl<A, E, R, O, F> ErasedStep<A, E> for RepeatLoop<A, E, R, O, F>
where
    A: Send + Sync + 'static,
    E: Send + 'static,
    R: crate::effect::Requirement,
    O: Send + 'static,
    F: Fn() -> Effect<A, E, R> + Send + Sync,
{
    fn run_erased(self: Box<Self>, scope: &Services) -> Result<A, Cause<E>> {
        let mut attempt = 0u64;
        loop {
            let effect = (self.make)();
            let a = effect.run_sync_with(scope)?;
            match self.policy.decide(attempt, &a) {
                None => return Ok(a),
                Some((_, delay)) => {
                    std::thread::sleep(std::time::Duration::from_millis(delay.as_millis()));
                    attempt += 1;
                }
            }
        }
    }
}

/// The typed error if the cause is exactly one `Fail`; `None` for defects,
/// interrupts, and mixed causes (never retried).
fn only_fail<E>(cause: &Cause<E>) -> Option<&E> {
    match cause.reasons.as_slice() {
        [Reason::Fail(e)] => Some(e),
        _ => None,
    }
}
