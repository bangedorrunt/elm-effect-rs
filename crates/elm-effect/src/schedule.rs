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
    state: Mutex<RandomState>,
}

enum RandomState {
    Seed(u64),
    /// Values supplied by [`Random::from_draws`], consumed in order.
    Draws(std::collections::VecDeque<f64>),
}

/// The one failure a draw can produce (`NoSuchElementError`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RandomError {
    /// [`Random::choice`] was handed an empty slice.
    NoSuchElement,
}

impl RandomError {
    /// The pin's message text.
    pub fn message(&self) -> &'static str {
        match self {
            Self::NoSuchElement => "Cannot select a random element from an empty array",
        }
    }
}

impl std::fmt::Display for RandomError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for RandomError {}

impl Random {
    /// A generator with a fixed seed. Same seed, same sequence.
    pub fn with_seed(seed: u64) -> Self {
        Self {
            state: Mutex::new(RandomState::Seed(seed | 1)),
        }
    }

    /// A generator seeded from a string. The pin hashes its seeds; the hash
    /// only has to separate seeds, not reproduce the pin's sequence.
    pub fn with_string_seed(seed: &str) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        seed.hash(&mut hasher);
        Self::with_seed(hasher.finish())
    }

    /// A source pinned to `draws`, the port's stand-in for providing a fake
    /// `Random.Random` service. Exhausted draws read as `0.0`.
    pub fn from_draws(draws: impl IntoIterator<Item = f64>) -> Self {
        Self {
            state: Mutex::new(RandomState::Draws(draws.into_iter().collect())),
        }
    }

    /// Next pseudorandom unit in `[0, 1)`.
    pub fn next_unit(&self) -> f64 {
        let mut guard = self.state.lock().unwrap();
        match &mut *guard {
            RandomState::Draws(draws) => draws.pop_front().unwrap_or(0.0),
            RandomState::Seed(seed) => {
                // SplitMix64: good enough for jitter, no dependency.
                *seed = seed.wrapping_add(0x9E3779B97F4A7C15);
                let mut z = *seed;
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
                z ^= z >> 31;
                let bits = (z >> 11) | 0x3FF0000000000000u64;
                f64::from_bits(bits) - 1.0
            }
        }
    }

    /// A boolean above a strict `0.5` threshold (`nextBoolean`).
    pub fn next_boolean(&self) -> bool {
        self.next_unit() > 0.5
    }

    /// A safe integer across the full range (`nextInt`).
    pub fn next_int(&self) -> i64 {
        ((self.next_unit() * 2.0 - 1.0) * 9_007_199_254_740_991.0) as i64
    }

    /// A half-open `[min, max)` draw (`nextBetween`). A draw that rounds up
    /// to the bound steps down to its predecessor instead of returning `max`.
    pub fn next_between(&self, min: f64, max: f64) -> f64 {
        if min == max {
            return min;
        }
        let value = min + self.next_unit() * (max - min);
        if value >= max {
            predecessor(max)
        } else {
            value
        }
    }

    /// An integer in `[min, max]` (`nextIntBetween`).
    pub fn next_int_between(&self, min: i64, max: i64) -> i64 {
        if min >= max {
            return min;
        }
        let offset = (self.next_unit() * (max - min + 1) as f64) as i64;
        min + offset.clamp(0, max - min)
    }

    /// An integer in `[min, max)` (`nextIntBetween` with `halfOpen`).
    pub fn next_int_between_half_open(&self, min: i64, max: i64) -> i64 {
        if min >= max {
            return min;
        }
        let offset = (self.next_unit() * (max - min) as f64) as i64;
        min + offset.clamp(0, max - min - 1)
    }

    /// A shuffled copy (`shuffle`); the input keeps its order.
    pub fn shuffle<T: Clone>(&self, values: &[T]) -> Vec<T> {
        let mut out = values.to_vec();
        for index in (1..out.len()).rev() {
            let other = self.next_int_between_half_open(0, index as i64 + 1) as usize;
            out.swap(index, other);
        }
        out
    }

    /// One element (`choice`), or `NoSuchElement` when there is nothing to
    /// pick.
    pub fn choice<'a, T>(&self, values: &'a [T]) -> Result<&'a T, RandomError> {
        if values.is_empty() {
            return Err(RandomError::NoSuchElement);
        }
        let index = self.next_int_between_half_open(0, values.len() as i64) as usize;
        Ok(&values[index])
    }
}

/// The float just below `x`. Infinities are unchanged so the pin's unsupported
/// bounds (`nextBetween(10, Infinity)`) pass through.
fn predecessor(x: f64) -> f64 {
    if x.is_nan() || x.is_infinite() {
        return x;
    }
    if x == 0.0 {
        return -f64::from_bits(1);
    }
    let bits = x.to_bits();
    f64::from_bits(if x > 0.0 { bits - 1 } else { bits + 1 })
}

impl Clone for Random {
    fn clone(&self) -> Self {
        let state = match &*self.state.lock().unwrap() {
            RandomState::Seed(seed) => RandomState::Seed(*seed),
            RandomState::Draws(draws) => RandomState::Draws(draws.clone()),
        };
        Self {
            state: Mutex::new(state),
        }
    }
}

/// The `Random` service tag.
pub struct RandomTag;

impl crate::service::ServiceTag for RandomTag {
    type Service = Random;
    const ID: &'static str = "elm-effect/random";
}

/// What one step is told before it decides: the pin's `InputMetadata`.
///
/// `attempt` is 1-based, counting the step being taken. `start` is the `now`
/// of the run's first step, so `elapsed` is time since the run began and
/// `elapsed_since_previous` is time since the last step was asked.
#[derive(Clone, Copy)]
pub struct Meta<'a, In> {
    /// The value the driver handed this step.
    pub input: &'a In,
    /// 1-based count of the step being taken.
    pub attempt: u64,
    /// Timestamp of the run's first step.
    pub start: f64,
    /// This step's timestamp.
    pub now: f64,
    /// `now - start`.
    pub elapsed: f64,
    /// `now` minus the previous step's `now`; zero on the first step.
    pub elapsed_since_previous: f64,
}

/// A completed step's metadata: the pin's `Metadata`, which adds the output
/// and delay the step chose to [`Meta`].
pub struct Metadata<'a, In, Out> {
    /// The value the driver handed this step.
    pub input: &'a In,
    /// What the step emitted.
    pub output: Out,
    /// How long the step asked to wait.
    pub duration: Duration,
    /// 1-based count of the step that just ran.
    pub attempt: u64,
    /// Timestamp of the run's first step.
    pub start: f64,
    /// This step's timestamp.
    pub now: f64,
    /// `now - start`.
    pub elapsed: f64,
    /// Time since the previous step; zero on the first step.
    pub elapsed_since_previous: f64,
}

/// What one step decided: keep going after a delay, or complete.
///
/// The pin's pull carries the value the step produced even when it completes,
/// so a stop is [`Done`](Decision::Done) with that value rather than an
/// absence. Combinators that refuse a step after it ran (`upTo`, `while`)
/// therefore report the refused output, which is what their rows assert.
pub enum Decision<Out> {
    /// Continue, waiting `duration` before the next step.
    Continue {
        /// What this step emitted.
        output: Out,
        /// How long to wait before the next step.
        duration: Duration,
    },
    /// Stop. `output` is what this step produced, when it produced one.
    Done {
        /// The step's own output, if it had one.
        output: Option<Out>,
    },
}

impl<Out> Decision<Out> {
    /// This step's output, whichever way it went.
    pub fn into_output(self) -> Option<Out> {
        match self {
            Self::Continue { output, .. } => Some(output),
            Self::Done { output } => output,
        }
    }

    /// The delay if this step continues; `None` when the schedule is done.
    pub fn duration(&self) -> Option<Duration> {
        match self {
            Self::Continue { duration, .. } => Some(*duration),
            Self::Done { .. } => None,
        }
    }

    /// Map the output of either shape, keeping the decision.
    fn map_output<B>(self, f: impl Fn(Out) -> B) -> Decision<B> {
        match self {
            Self::Continue { output, duration } => Decision::Continue {
                output: f(output),
                duration,
            },
            Self::Done { output } => Decision::Done {
                output: output.map(f),
            },
        }
    }
}

/// One schedule's own metadata counters, the pin's `metadataFn`.
///
/// Every schedule instance owns one, so `attempt` counts the steps *that
/// schedule* took. A wrapper (`upTo`, `tap`) therefore counts its own calls,
/// and a concatenated second schedule starts again at attempt 1, exactly as
/// the pin's per-`fromStepWithMetadata` closure does.
struct Counters {
    attempt: u64,
    start: Option<f64>,
    previous: Option<f64>,
}

impl Counters {
    fn new() -> Self {
        Self {
            attempt: 0,
            start: None,
            previous: None,
        }
    }

    fn advance<'i, In>(&mut self, now: f64, input: &'i In) -> Meta<'i, In> {
        let start = *self.start.get_or_insert(now);
        let elapsed = now - start;
        let elapsed_since_previous = self.previous.map_or(0.0, |previous| now - previous);
        self.previous = Some(now);
        self.attempt += 1;
        Meta {
            input,
            attempt: self.attempt,
            start,
            now,
            elapsed,
            elapsed_since_previous,
        }
    }
}

/// One step. `Done` stops the schedule.
type StepFn<In, Out> = Arc<dyn Fn(&Meta<'_, In>) -> Decision<Out> + Send + Sync + 'static>;

/// Marker for a driver loop over `A`, failing with `E`, needing `R`.
type LoopMarker<A, E, R, O> = PhantomData<fn() -> (A, E, R, O)>;

#[derive(Clone)]
pub struct Schedule<In: Send + Sync + 'static, Out: Send + 'static> {
    counters: Arc<Mutex<Counters>>,
    step: StepFn<In, Out>,
}

impl<In: Send + Sync + 'static, Out: Send + 'static> Schedule<In, Out> {
    /// A schedule whose step sees its own attempt and timing counters.
    pub fn from_step(f: impl Fn(&Meta<'_, In>) -> Decision<Out> + Send + Sync + 'static) -> Self {
        Self {
            counters: Arc::new(Mutex::new(Counters::new())),
            step: Arc::new(f),
        }
    }

    /// Take one step at timestamp `now` (milliseconds), advancing this
    /// schedule's own counters.
    pub fn step(&self, now: f64, input: &In) -> Decision<Out> {
        let meta = self.counters.lock().unwrap().advance(now, input);
        (self.step)(&meta)
    }

    /// Decide attempt `attempt` with no clock, reporting `None` when the
    /// schedule completes.
    ///
    /// The caller owns the count: `attempt` is 0-based, so the step runs as
    /// attempt `attempt + 1` with zero timestamps. Policies whose delay
    /// depends on time (`fixed`'s window alignment, `during`, `up_to` with a
    /// duration) report what they would decide at the start of a run. Use
    /// [`to_step`](Self::to_step) to drive a schedule over a clock.
    pub fn decide(&self, attempt: u64, input: &In) -> Option<(Out, Duration)> {
        {
            let mut counters = self.counters.lock().unwrap();
            counters.attempt = attempt;
            // Zeroed timing is the pin's `toStep(0, …)` starting point, and
            // keeps `decide` idempotent for a caller that repeats an index.
            counters.start = Some(0.0);
            counters.previous = Some(0.0);
        }
        match self.step(0.0, input) {
            Decision::Continue { output, duration } => Some((output, duration)),
            Decision::Done { .. } => None,
        }
    }

    /// `Schedule.toStep`: this schedule's step function over a caller-supplied
    /// timestamp in milliseconds.
    pub fn to_step(&self) -> Stepper<'_, In, Out> {
        Stepper { schedule: self }
    }
}

/// A schedule's step function bound to a timestamp source (the pin's
/// `toStep`/`toStepWithMetadata`). The schedule's own counters still advance,
/// so a test can drive it over a virtual clock without sleeping.
pub struct Stepper<'a, In: Send + Sync + 'static, Out: Send + 'static> {
    schedule: &'a Schedule<In, Out>,
}

impl<'a, In: Send + Sync + 'static, Out: Send + 'static> Stepper<'a, In, Out> {
    /// Take one step.
    pub fn step(&mut self, now: f64, input: &In) -> Decision<Out> {
        self.schedule.step(now, input)
    }

    /// Take one step and report its metadata, the pin's
    /// `toStepWithMetadata`. `None` means the schedule completed.
    ///
    /// The pin also sleeps for the delay after a step; a caller that wants
    /// that sleeps for `metadata.duration` and advances `now` by it.
    pub fn step_with_metadata<'i>(
        &mut self,
        now: f64,
        input: &'i In,
    ) -> Option<Metadata<'i, In, Out>> {
        let meta = self.schedule.counters.lock().unwrap().advance(now, input);
        match (self.schedule.step)(&meta) {
            Decision::Continue { output, duration } => Some(metadata_of(&meta, output, duration)),
            Decision::Done { .. } => None,
        }
    }
}

impl<In: Send + Sync + 'static> Schedule<In, u64> {
    /// Up to `n` retries after the first attempt, with no wait. Compose with
    /// a backoff (via [`max`](Schedule::max)) to add delays.
    pub fn recurs(n: u64) -> Schedule<In, u64> {
        Schedule::from_step(move |meta| {
            if meta.attempt <= n {
                Decision::Continue {
                    output: meta.attempt - 1,
                    duration: Duration::ZERO,
                }
            } else {
                Decision::Done { output: None }
            }
        })
    }
}

impl<In: Send + Sync + 'static> Schedule<In, u64> {
    /// A fixed wait per attempt, continuing forever, outputting the zero-based
    /// attempt index (`spaced`).
    pub fn spaced(delay: Duration) -> Schedule<In, u64> {
        Schedule::from_step(move |meta| Decision::Continue {
            output: meta.attempt - 1,
            duration: delay,
        })
    }
}

impl<In: Send + Sync + 'static> Schedule<In, ()> {
    /// Continue exactly once, then stop (`once`).
    pub fn once() -> Schedule<In, ()> {
        Schedule::from_step(|meta| {
            if meta.attempt == 1 {
                Decision::Continue {
                    output: (),
                    duration: Duration::ZERO,
                }
            } else {
                Decision::Done { output: None }
            }
        })
    }
}

impl<In: Send + Sync + 'static> Schedule<In, u64> {
    /// Wait until the next window boundary, forever (`windowed`), outputting
    /// the zero-based attempt index.
    ///
    /// Unlike [`fixed`](Schedule::fixed), a step that starts late still waits
    /// the remainder of its window rather than skipping ahead: the delay is
    /// always `window - (elapsed % window)`, which is why a slow action still
    /// produces a partial wait.
    pub fn windowed(interval: Duration) -> Schedule<In, u64> {
        let window = interval.as_millis() as f64;
        Schedule::from_step(move |meta| {
            let delay = if window == 0.0 {
                0.0
            } else {
                window - (meta.elapsed % window)
            };
            Decision::Continue {
                output: meta.attempt - 1,
                duration: Duration::from_millis(delay as u64),
            }
        })
    }

    /// A wait aligned to window boundaries (`fixed`), outputting the
    /// zero-based attempt index as at the pin.
    ///
    /// The first step asks for one full window, and later steps wait only
    /// until the next boundary after the run started. A step that begins once
    /// its window has already passed waits nothing, so a slow action never
    /// pushes the schedule further behind.
    pub fn fixed(window: Duration) -> Schedule<In, u64> {
        let window = window.as_millis() as f64;
        // (run start, the time the current window ends): the same two numbers
        // the pin holds in its step closure.
        let state = Arc::new(Mutex::new((0.0f64, 0.0f64)));
        Schedule::from_step(move |meta| {
            if window == 0.0 {
                return Decision::Continue {
                    output: meta.attempt - 1,
                    duration: Duration::ZERO,
                };
            }
            let mut state = state.lock().unwrap();
            let (start, last_run) = &mut *state;
            if meta.attempt == 1 {
                *start = meta.now;
                *last_run = meta.now + window;
                return Decision::Continue {
                    output: 0,
                    duration: Duration::from_millis(window as u64),
                };
            }
            let running_behind = meta.now > *last_run + window;
            let boundary = window - ((meta.now - *start) % window);
            let delay = if running_behind {
                0.0
            } else if boundary == 0.0 {
                window
            } else {
                boundary
            };
            *last_run = if running_behind {
                meta.now
            } else {
                meta.now + delay
            };
            Decision::Continue {
                output: meta.attempt - 1,
                duration: Duration::from_millis(delay as u64),
            }
        })
    }
}

impl<In: Send + Sync + 'static> Schedule<In, Duration> {
    /// Exponential backoff from `base`, doubling per attempt (saturating),
    /// continuing forever and outputting the chosen delay. Bound it with
    /// [`recurs`](Schedule::recurs) via [`max`](Schedule::max).
    pub fn exponential(base: Duration) -> Schedule<In, Duration> {
        Schedule::from_step(move |meta| {
            let shift = (meta.attempt - 1).min(63);
            let millis = base.as_millis().saturating_mul(1u64 << shift);
            let delay = Duration::from_millis(millis);
            Decision::Continue {
                output: delay,
                duration: delay,
            }
        })
    }

    /// Recur while the run has not outlived `limit`, outputting the elapsed
    /// time (`during`). The step that crosses the limit completes with that
    /// elapsed time as its done value.
    pub fn during(limit: Duration) -> Schedule<In, Duration> {
        let limit = limit.as_millis() as f64;
        Schedule::from_step(move |meta| {
            let elapsed = Duration::from_millis(meta.elapsed as u64);
            if meta.elapsed > limit {
                Decision::Done {
                    output: Some(elapsed),
                }
            } else {
                Decision::Continue {
                    output: elapsed,
                    duration: Duration::ZERO,
                }
            }
        })
    }

    /// Recur exactly once, after `delay` (`duration`); output and delay are
    /// both the configured duration.
    pub fn duration(delay: Duration) -> Schedule<In, Duration> {
        Schedule::from_step(move |meta| {
            if meta.attempt == 1 {
                Decision::Continue {
                    output: delay,
                    duration: delay,
                }
            } else {
                Decision::Done { output: None }
            }
        })
    }

}

impl<In: Clone + Send + Sync + 'static> Schedule<In, In> {
    /// Echo the input as the output with no wait (`identity`).
    pub fn identity() -> Schedule<In, In> {
        Schedule::from_step(|meta: &Meta<'_, In>| Decision::Continue {
            output: meta.input.clone(),
            duration: Duration::ZERO,
        })
    }
}

/// Bounds for [`Schedule::up_to`] (the pin's `upTo` options record).
///
/// An unset field imposes no bound, and both unset leaves the schedule
/// unchanged.
#[derive(Clone, Copy, Default)]
pub struct UpTo {
    /// Stop after this many continuations.
    pub times: Option<u64>,
    /// Stop once the run has lasted longer than this.
    pub duration: Option<Duration>,
}

impl<In: Send + Sync + 'static, A: Send + 'static> Schedule<In, A> {
    /// Stop after `times` continuations, or once `duration` has elapsed, or
    /// under both bounds when both are set (`upTo`).
    ///
    /// The bound is checked after the inner step decides, so the refused step
    /// completes with its own output, as the pin's `while` does.
    pub fn up_to(self, options: UpTo) -> Self {
        if options.times.is_none() && options.duration.is_none() {
            return self;
        }
        Schedule::from_step(move |meta| match self.step(meta.now, meta.input) {
            Decision::Continue { output, duration } => {
                let exhausted = options.times.is_some_and(|times| meta.attempt > times)
                    || options
                        .duration
                        .is_some_and(|limit| meta.elapsed > limit.as_millis() as f64);
                if exhausted {
                    Decision::Done {
                        output: Some(output),
                    }
                } else {
                    Decision::Continue { output, duration }
                }
            }
            Decision::Done { output } => Decision::Done { output },
        })
    }

    /// [`up_to`](Self::up_to) with only a times bound: `times(2)` lets two
    /// continuations through.
    pub fn up_to_times(self, times: u64) -> Self {
        self.up_to(UpTo {
            times: Some(times),
            duration: None,
        })
    }

    /// Cap every delay at `limit`.
    pub fn capped(self, limit: Duration) -> Self {
        Schedule::from_step(move |meta| match self.step(meta.now, meta.input) {
            Decision::Continue { output, duration } => {
                let capped = duration.as_millis().min(limit.as_millis());
                Decision::Continue {
                    output,
                    duration: Duration::from_millis(capped),
                }
            }
            Decision::Done { output } => Decision::Done { output },
        })
    }

    /// Transform the output, keeping the delay (`map`).
    pub fn map<B: Send + 'static>(
        self,
        f: impl Fn(A) -> B + Send + Sync + 'static,
    ) -> Schedule<In, B> {
        Schedule::from_step(move |meta| self.step(meta.now, meta.input).map_output(|out| f(out)))
    }

    /// Observe each continuing step's full metadata (`tap`), leaving the
    /// decision untouched.
    pub fn tap<F>(self, f: F) -> Self
    where
        F: Fn(&Metadata<'_, In, A>) + Send + Sync + 'static,
    {
        Schedule::from_step(move |meta| match self.step(meta.now, meta.input) {
            Decision::Continue { output, duration } => {
                let full = metadata_of(meta, output, duration);
                f(&full);
                Decision::Continue {
                    output: full.output,
                    duration: full.duration,
                }
            }
            Decision::Done { output } => Decision::Done { output },
        })
    }

    /// Replace each delay with what `f` decides from the full metadata
    /// (`modifyDelay`).
    pub fn modify_delay<F>(self, f: F) -> Self
    where
        F: Fn(&Metadata<'_, In, A>) -> Duration + Send + Sync + 'static,
    {
        Schedule::from_step(move |meta| match self.step(meta.now, meta.input) {
            Decision::Continue { output, duration } => {
                let full = metadata_of(meta, output, duration);
                let replaced = f(&full);
                Decision::Continue {
                    output: full.output,
                    duration: replaced,
                }
            }
            Decision::Done { output } => Decision::Done { output },
        })
    }

    /// Add what `f` decides from the full metadata to each delay
    /// (`addDelay`).
    pub fn add_delay<F>(self, f: F) -> Self
    where
        F: Fn(&Metadata<'_, In, A>) -> Duration + Send + Sync + 'static,
    {
        Schedule::from_step(move |meta| match self.step(meta.now, meta.input) {
            Decision::Continue { output, duration } => {
                let full = metadata_of(meta, output, duration);
                let extra = f(&full);
                let summed = duration.as_millis().saturating_add(extra.as_millis());
                Decision::Continue {
                    output: full.output,
                    duration: Duration::from_millis(summed),
                }
            }
            Decision::Done { output } => Decision::Done { output },
        })
    }

    /// Stop when `pred` refuses a continuing step's metadata (`while`): the
    /// refused step completes with its own output.
    pub fn while_metadata<F>(self, pred: F) -> Self
    where
        F: Fn(&Metadata<'_, In, A>) -> bool + Send + Sync + 'static,
    {
        Schedule::from_step(move |meta| match self.step(meta.now, meta.input) {
            Decision::Continue { output, duration } => {
                let full = metadata_of(meta, output, duration);
                if pred(&full) {
                    Decision::Continue {
                        output: full.output,
                        duration: full.duration,
                    }
                } else {
                    Decision::Done {
                        output: Some(full.output),
                    }
                }
            }
            Decision::Done { output } => Decision::Done { output },
        })
    }

    /// Drop the policy's output, keeping only its continue/delay decision.
    /// Drivers that stop on a predicate never read the output, and `recurs`
    /// outputs `u64` while `spaced` outputs `()`.
    pub fn ignore_output(self) -> Schedule<In, ()> {
        Schedule::from_step(move |meta| match self.step(meta.now, meta.input) {
            Decision::Continue { duration, .. } => Decision::Continue {
                output: (),
                duration,
            },
            Decision::Done { .. } => Decision::Done { output: None },
        })
    }

    /// Run `self` to completion, then `other`, tagging each output with the
    /// side that produced it (`concatResult`): `Err` for `self` and `Ok` for
    /// `other`, as the pin's `Result` does.
    pub fn concat_result<B: Send + 'static>(
        self,
        other: Schedule<In, B>,
    ) -> Schedule<In, std::result::Result<B, A>> {
        // 0 while `self` is producing, 1 once it has completed: the pin keeps
        // the same `currentSide` flag in its step closure.
        let side = Arc::new(Mutex::new(0u8));
        Schedule::from_step(move |meta| {
            let mut side = side.lock().unwrap();
            if *side == 0 {
                match self.step(meta.now, meta.input) {
                    Decision::Continue { output, duration } => {
                        return Decision::Continue {
                            output: Err(output),
                            duration,
                        };
                    }
                    // The left side is done: the right side supplies this
                    // same step's decision. Its own counters start at attempt
                    // 1, as a freshly extracted step does at the pin.
                    Decision::Done { .. } => *side = 1,
                }
            }
            other
                .step(meta.now, meta.input)
                .map_output(|output| Ok(output))
        })
    }

    /// Run `self` to completion, then `other`, merging their outputs
    /// (`concat`). Both sides must emit the same type, which is what merging
    /// a union of one type means.
    pub fn concat(self, other: Schedule<In, A>) -> Schedule<In, A> {
        self.concat_result(other).map(|either| match either {
            Ok(output) | Err(output) => output,
        })
    }

    /// Scale every delay by a factor in `[0.8, 1.2)` drawn from `rng`, the
    /// pin's `jittered` band (`Schedule.test.ts:476`).
    pub fn jitter(self, rng: Random) -> Self {
        Schedule::from_step(move |meta| match self.step(meta.now, meta.input) {
            Decision::Continue { output, duration } => {
                let factor = 0.8 + 0.4 * rng.next_unit();
                let millis = (duration.as_millis() as f64 * factor) as u64;
                Decision::Continue {
                    output,
                    duration: Duration::from_millis(millis),
                }
            }
            Decision::Done { output } => Decision::Done { output },
        })
    }

    /// Continue only while `pred` holds (e.g. `error.retryable`).
    pub fn given(self, pred: impl Fn(&In) -> bool + Send + Sync + 'static) -> Self {
        Schedule::from_step(move |meta| {
            if pred(meta.input) {
                self.step(meta.now, meta.input)
            } else {
                Decision::Done { output: None }
            }
        })
    }

    /// Continue while every member continues; wait the longer delay.
    pub fn max<B: Send + 'static>(self, second: Schedule<In, B>) -> Schedule<In, (A, B)> {
        Schedule::from_step(move |meta| {
            match (
                self.step(meta.now, meta.input),
                second.step(meta.now, meta.input),
            ) {
                (
                    Decision::Continue {
                        output: a,
                        duration: da,
                    },
                    Decision::Continue {
                        output: b,
                        duration: db,
                    },
                ) => {
                    let longer = if da.as_millis() >= db.as_millis() {
                        da
                    } else {
                        db
                    };
                    Decision::Continue {
                        output: (a, b),
                        duration: longer,
                    }
                }
                _ => Decision::Done { output: None },
            }
        })
    }

    /// Continue while any member continues; wait the shorter of the decided
    /// delays. A stopped side contributes `None` to the output pair.
    pub fn min<B: Send + 'static>(
        self,
        second: Schedule<In, B>,
    ) -> Schedule<In, (Option<A>, Option<B>)> {
        Schedule::from_step(move |meta| {
            let a = self.step(meta.now, meta.input);
            let b = second.step(meta.now, meta.input);
            let (da, db) = (a.duration(), b.duration());
            match (da, db) {
                (None, None) => Decision::Done { output: None },
                _ => {
                    let mut millis = u64::MAX;
                    if let Some(d) = da {
                        millis = millis.min(d.as_millis());
                    }
                    if let Some(d) = db {
                        millis = millis.min(d.as_millis());
                    }
                    Decision::Continue {
                        output: (a.into_output(), b.into_output()),
                        duration: Duration::from_millis(millis),
                    }
                }
            }
        })
    }
}

/// Pair a step's timing with the output and delay it chose.
fn metadata_of<'a, In, Out>(
    meta: &Meta<'a, In>,
    output: Out,
    duration: Duration,
) -> Metadata<'a, In, Out> {
    Metadata {
        input: meta.input,
        output,
        duration,
        attempt: meta.attempt,
        start: meta.start,
        now: meta.now,
        elapsed: meta.elapsed,
        elapsed_since_previous: meta.elapsed_since_previous,
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

/// A loop predicate recorded as an effect, so the pin's `until`/`while`
/// callbacks can fail and have that failure end the loop.
pub type LoopPredicate<In, E> = Arc<dyn Fn(&In) -> Effect<bool, E, ()> + Send + Sync + 'static>;

/// `{ schedule, until, while }` for [`repeat_with`] and [`retry_with`].
///
/// Predicates run after each attempt, never before, so a loop always runs at
/// least once (the pin's "always evaluates at least once" rows). With no
/// schedule the loop repeats until a predicate stops it.
pub struct LoopOptions<In: Send + Sync + 'static, E: Send + 'static> {
    schedule: Option<Schedule<In, ()>>,
    until: Option<LoopPredicate<In, E>>,
    while_: Option<LoopPredicate<In, E>>,
    times: Option<u64>,
}

impl<In: Send + Sync + 'static, E: Send + 'static> Default for LoopOptions<In, E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<In: Send + Sync + 'static, E: Send + 'static> LoopOptions<In, E> {
    /// No schedule, no predicates: the loop only ends on failure. Set one of
    /// the builders, or this repeats forever.
    pub fn new() -> Self {
        Self {
            schedule: None,
            until: None,
            while_: None,
            times: None,
        }
    }

    /// Retry the base schedule's decision before consulting the predicates.
    pub fn schedule(mut self, schedule: Schedule<In, ()>) -> Self {
        self.schedule = Some(schedule);
        self
    }

    /// Stop once `predicate` holds for the last result.
    pub fn until<F>(mut self, predicate: F) -> Self
    where
        F: Fn(&In) -> bool + Send + Sync + 'static,
    {
        self.until = Some(Arc::new(move |input| Effect::succeed(predicate(input))));
        self
    }

    /// [`until`](Self::until) whose condition can fail; the failure ends the
    /// loop and is returned instead of the loop's own error.
    pub fn until_effect<F>(mut self, predicate: F) -> Self
    where
        F: Fn(&In) -> Effect<bool, E, ()> + Send + Sync + 'static,
    {
        self.until = Some(Arc::new(predicate));
        self
    }

    /// Keep going while `predicate` holds for the last result.
    pub fn while_<F>(mut self, predicate: F) -> Self
    where
        F: Fn(&In) -> bool + Send + Sync + 'static,
    {
        self.while_ = Some(Arc::new(move |input| Effect::succeed(predicate(input))));
        self
    }

    /// [`while_`](Self::while_) whose condition can fail, with the same
    /// failure rule as [`until_effect`](Self::until_effect).
    pub fn while_effect<F>(mut self, predicate: F) -> Self
    where
        F: Fn(&In) -> Effect<bool, E, ()> + Send + Sync + 'static,
    {
        self.while_ = Some(Arc::new(predicate));
        self
    }

    /// The pin treats a missing schedule as "repeat forever, no delay".
    fn base(&self) -> Schedule<In, ()> {
        match &self.schedule {
            // `#[derive(Clone)]` would demand `In: Clone`; the derived clone
            // shares the counters and step, like the pin's schedule object.
            Some(schedule) => Schedule {
                counters: Arc::clone(&schedule.counters),
                step: Arc::clone(&schedule.step),
            },
            None => Schedule::spaced(Duration::ZERO).ignore_output(),
        }
    }

    /// Stop after `times` repeats; `times(2)` therefore runs three times.
    pub fn times(mut self, times: u64) -> Self {
        self.times = Some(times);
        self
    }

    /// `times` wins over `until`, which wins over `while`; a failing
    /// predicate is the loop's failure.
    fn stop_after(&self, input: &In, attempt: u64, scope: &Services) -> Result<bool, Cause<E>> {
        if self.times.is_some_and(|times| attempt >= times) {
            return Ok(true);
        }
        if let Some(until) = &self.until
            && until(input).run_sync_with(scope)?
        {
            return Ok(true);
        }
        if let Some(while_) = &self.while_
            && !while_(input).run_sync_with(scope)?
        {
            return Ok(true);
        }
        Ok(false)
    }
}

/// Re-run the recipe after success until an `until`/`while` predicate or the
/// base schedule stops it, returning the last value. A failure passes through
/// untouched, as in [`repeat`].
pub fn repeat_with<A, E, F>(make: F, options: LoopOptions<A, E>) -> Effect<A, E>
where
    A: Send + Sync + 'static,
    E: Send + 'static,
    F: Fn() -> Effect<A, E, ()> + Send + Sync + 'static,
{
    Effect {
        step: Step::Suspend(Box::new(GuardedRepeat {
            make: Arc::new(make),
            options,
        })),
        services: empty_services(),
        marker: PhantomData,
    }
}

/// Re-run the recipe after typed failure until a predicate or the base
/// schedule stops it (`options` sees the error, as at the pin). Defects and
/// interrupts are never retried, and a failing predicate becomes the result.
pub fn retry_with<A, E, F>(make: F, options: LoopOptions<E, E>) -> Effect<A, E>
where
    A: Send + 'static,
    E: Send + Sync + 'static,
    F: Fn() -> Effect<A, E, ()> + Send + Sync + 'static,
{
    Effect {
        step: Step::Suspend(Box::new(GuardedRetry::<A, E, F> {
            make: Arc::new(make),
            options,
            marker: PhantomData,
        })),
        services: empty_services(),
        marker: PhantomData,
    }
}

struct GuardedRepeat<A: Send + Sync + 'static, E: Send + 'static, F> {
    make: Arc<F>,
    options: LoopOptions<A, E>,
}

impl<A, E, F> ErasedStep<A, E> for GuardedRepeat<A, E, F>
where
    A: Send + Sync + 'static,
    E: Send + 'static,
    F: Fn() -> Effect<A, E, ()> + Send + Sync,
{
    fn run_erased(self: Box<Self>, scope: &Services) -> Result<A, Cause<E>> {
        let schedule = self.options.base();
        let mut stepper = schedule.to_step();
        let origin = std::time::Instant::now();
        let mut attempt = 0u64;
        loop {
            let value = (self.make)().run_sync_with(scope)?;
            if self.options.stop_after(&value, attempt, scope)? {
                return Ok(value);
            }
            match stepper.step(elapsed_millis(origin), &value) {
                Decision::Done { .. } => return Ok(value),
                Decision::Continue { duration, .. } => {
                    std::thread::sleep(std::time::Duration::from_millis(duration.as_millis()));
                    attempt += 1;
                }
            }
        }
    }
}

struct GuardedRetry<A, E: Send + Sync + 'static, F> {
    make: Arc<F>,
    options: LoopOptions<E, E>,
    marker: PhantomData<fn() -> A>,
}

/// What one failed attempt decided: stop (returning that cause) or retry
/// after `delay`. Separate from the cause so the borrow ends before the move.
enum RetryStep {
    Stop,
    Retry(Duration),
}

impl<A, E, F> ErasedStep<A, E> for GuardedRetry<A, E, F>
where
    A: Send + 'static,
    E: Send + Sync + 'static,
    F: Fn() -> Effect<A, E, ()> + Send + Sync,
{
    fn run_erased(self: Box<Self>, scope: &Services) -> Result<A, Cause<E>> {
        let schedule = self.options.base();
        let mut stepper = schedule.to_step();
        let origin = std::time::Instant::now();
        let mut attempt = 0u64;
        loop {
            let cause = match (self.make)().run_sync_with(scope) {
                Ok(value) => return Ok(value),
                Err(cause) => cause,
            };
            let step = match only_fail(&cause) {
                None => RetryStep::Stop,
                Some(error) => {
                    if self.options.stop_after(error, attempt, scope)? {
                        RetryStep::Stop
                    } else {
                        match stepper.step(elapsed_millis(origin), error) {
                            Decision::Done { .. } => RetryStep::Stop,
                            Decision::Continue { duration, .. } => RetryStep::Retry(duration),
                        }
                    }
                }
            };
            match step {
                RetryStep::Stop => return Err(cause),
                RetryStep::Retry(delay) => {
                    std::thread::sleep(std::time::Duration::from_millis(delay.as_millis()));
                    attempt += 1;
                }
            }
        }
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
        let mut stepper = self.policy.to_step();
        let origin = std::time::Instant::now();
        loop {
            let effect = (self.make)();
            match effect.run_sync_with(scope) {
                Ok(a) => return Ok(a),
                Err(cause) => match only_fail(&cause) {
                    Some(input) => match stepper.step(elapsed_millis(origin), input) {
                        Decision::Done { .. } => return Err(cause),
                        Decision::Continue { duration, .. } => {
                            std::thread::sleep(std::time::Duration::from_millis(
                                duration.as_millis(),
                            ));
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
        let mut stepper = self.policy.to_step();
        let origin = std::time::Instant::now();
        loop {
            let effect = (self.make)();
            let a = effect.run_sync_with(scope)?;
            match stepper.step(elapsed_millis(origin), &a) {
                Decision::Done { .. } => return Ok(a),
                Decision::Continue { duration, .. } => {
                    std::thread::sleep(std::time::Duration::from_millis(duration.as_millis()));
                }
            }
        }
    }
}

/// Milliseconds since `origin`, the `now` a driver hands its step.
fn elapsed_millis(origin: std::time::Instant) -> f64 {
    origin.elapsed().as_secs_f64() * 1000.0
}

/// The typed error if the cause is exactly one `Fail`; `None` for defects,
/// interrupts, and mixed causes (never retried).
fn only_fail<E>(cause: &Cause<E>) -> Option<&E> {
    match cause.reasons.as_slice() {
        [Reason::Fail(e)] => Some(e),
        _ => None,
    }
}
