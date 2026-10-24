//! `Metric.test.ts`'s track family: the exit hooks that update a registry.
//!
//! The pin reaches the registry through the fiber running the effect. A
//! description here holds no fiber, so every hook takes the [`Metrics`] handle
//! it writes to; that is the only difference the tests below work with. What
//! each row asserts is unchanged: which exits count, and what the hook maps
//! them to.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Instant;

use elm_effect::{
    Attributes, Clock, Cons, Counter, Defect, Duration, Effect, Exit, Gauge, ManagedRuntime,
    Metrics, Requirement, Runtime, TestClock, timer,
};

fn counter(id: &str) -> Counter {
    // Every row counts one occurrence, so the pin's `withConstantInput(1)` is
    // what makes a counter's numeric input the count itself.
    Counter::new(id).with_constant_input(1.0)
}

fn count(metrics: &Metrics, counter: &Counter) -> f64 {
    metrics.read(|registry| registry.value(counter).count)
}

/// `Effect.track(counter)` with one occurrence per exit: the shape every row
/// below starts from. The counter is cloned because the hook outlives the call.
fn tracked<A, E, R>(
    effect: Effect<A, E, R>,
    metrics: &Metrics,
    counter: &Counter,
) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    let counter = counter.clone();
    effect.track(metrics, move |_, registry| registry.update(&counter, 1.0))
}

fn run<A, E>(effect: Effect<A, E>) -> Exit<A, E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    Runtime::run(effect)
}

/// :863 — a success counts.
#[test]
fn track_updates_on_success() {
    let metrics = Metrics::new();
    let counter = counter("track-success");

    run(tracked(
        Effect::<i32, i32, ()>::succeed(1),
        &metrics,
        &counter,
    ));

    assert_eq!(count(&metrics, &counter), 1.0);
}

/// :874 — a typed failure counts.
#[test]
fn track_updates_on_failure() {
    let metrics = Metrics::new();
    let counter = counter("track-failure");

    run(tracked(Effect::<(), i32, ()>::fail(1), &metrics, &counter).exit());

    assert_eq!(count(&metrics, &counter), 1.0);
}

/// :886 — a defect counts.
#[test]
fn track_updates_on_defect() {
    let metrics = Metrics::new();
    let counter = counter("track-defect");

    run(tracked(
        Effect::<(), i32, ()>::die(Defect::new("boom")),
        &metrics,
        &counter,
    )
    .exit());

    assert_eq!(count(&metrics, &counter), 1.0);
}

/// :900 — the hook maps the exit and leaves the effect's result alone.
#[test]
fn track_maps_exits_while_preserving_the_result() {
    let metrics = Metrics::new();
    let counted = counter("track-preserves");
    let record = counted.clone();

    let exit = run(
        Effect::<i32, i32, ()>::succeed(1).track(&metrics, move |exit, registry| {
            let amount = match exit {
                Exit::Success(value) => *value as f64,
                Exit::Failure(_) => 0.0,
            };
            registry.update(&record, amount)
        }),
    );

    assert_eq!(exit, Exit::Success(1));
    assert_eq!(count(&metrics, &counted), 1.0);
}

/// :913 — the failure arm of the second `track` block counts once.
#[test]
fn track_counts_a_failed_exit_once() {
    let metrics = Metrics::new();
    let counter = counter("track-failure-once");

    run(tracked(Effect::<(), i32, ()>::fail(7), &metrics, &counter).exit());

    assert_eq!(count(&metrics, &counter), 1.0);
}

/// :927 — the defect arm of the second `track` block counts once.
#[test]
fn track_counts_a_defective_exit_once() {
    let metrics = Metrics::new();
    let counter = counter("track-defect-once");

    run(tracked(
        Effect::<(), i32, ()>::die(Defect::new("boom")),
        &metrics,
        &counter,
    )
    .exit());

    assert_eq!(count(&metrics, &counter), 1.0);
}

/// `Effect.trackErrors(counter)` with the same one-occurrence counter.
fn tracked_errors<A, E, R>(
    effect: Effect<A, E, R>,
    metrics: &Metrics,
    counter: &Counter,
) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    let counter = counter.clone();
    effect.track_errors(metrics, move |_, registry| registry.update(&counter, 1.0))
}

/// `Effect.trackDefects(counter)` with the same one-occurrence counter.
fn tracked_defects<A, E, R>(
    effect: Effect<A, E, R>,
    metrics: &Metrics,
    counter: &Counter,
) -> Effect<A, E, R>
where
    A: Send + 'static,
    E: Send + 'static,
    R: Requirement,
{
    let counter = counter.clone();
    effect.track_defects(metrics, move |_, registry| registry.update(&counter, 1.0))
}

/// :943 — a success is not a typed failure.
#[test]
fn track_errors_ignores_success() {
    let metrics = Metrics::new();
    let counter = counter("errors-success");

    run(tracked_errors(
        Effect::<i32, i32, ()>::succeed(1),
        &metrics,
        &counter,
    ));

    assert_eq!(count(&metrics, &counter), 0.0);
}

/// :954 — a typed failure counts.
#[test]
fn track_errors_updates_on_failure() {
    let metrics = Metrics::new();
    let counter = counter("errors-failure");

    run(tracked_errors(Effect::<(), i32, ()>::fail(1), &metrics, &counter).exit());

    assert_eq!(count(&metrics, &counter), 1.0);
}

/// :966 — a defect is not a typed failure.
#[test]
fn track_errors_ignores_defects() {
    let metrics = Metrics::new();
    let counter = counter("errors-defect");

    run(tracked_errors(
        Effect::<(), i32, ()>::die(Defect::new("boom")),
        &metrics,
        &counter,
    )
    .exit());

    assert_eq!(count(&metrics, &counter), 0.0);
}

/// :980 — a success is not a defect.
#[test]
fn track_defects_ignores_success() {
    let metrics = Metrics::new();
    let counter = counter("defects-success");

    run(tracked_defects(
        Effect::<i32, i32, ()>::succeed(1),
        &metrics,
        &counter,
    ));

    assert_eq!(count(&metrics, &counter), 0.0);
}

/// :991 — a typed failure is not a defect.
#[test]
fn track_defects_ignores_failures() {
    let metrics = Metrics::new();
    let counter = counter("defects-failure");

    run(tracked_defects(Effect::<(), i32, ()>::fail(1), &metrics, &counter).exit());

    assert_eq!(count(&metrics, &counter), 0.0);
}

/// :1003 — a defect counts.
#[test]
fn track_defects_updates_on_defect() {
    let metrics = Metrics::new();
    let counter = counter("defects-defect");

    run(tracked_defects(
        Effect::<(), i32, ()>::die(Defect::new("boom")),
        &metrics,
        &counter,
    )
    .exit());

    assert_eq!(count(&metrics, &counter), 1.0);
}

/// Wait until `expected` workers are parked on the virtual clock, so a test
/// knows the forked effect reached its sleep before driving time.
fn wait_for_sleepers(clock: &TestClock, expected: usize) {
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    while clock.pending_sleepers() != expected && Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(clock.pending_sleepers(), expected, "the sleep never parked");
}

/// :1017 — an hour of virtual time is the duration the timer records.
#[test]
fn track_duration_records_the_hours_the_clock_advanced() {
    let metrics = Metrics::new();
    let timer = timer("duration-hours");
    let clock = Arc::new(TestClock::new());
    let runtime = ManagedRuntime::new();

    let record = timer.clone();
    let sleep = Effect::<(), i32, Cons<Clock, ()>>::sleep(Duration::from_minutes(60))
        .provide::<Clock, ()>(clock.clone())
        .track_duration(&metrics, move |elapsed, registry| {
            registry.observe_histogram(&record, elapsed.to_millis())
        });
    let fiber = runtime.run_fork(sleep);

    wait_for_sleepers(&clock, 1);
    clock.adjust(Duration::from_minutes(60));
    let exit = fiber.join();
    assert_eq!(exit, Exit::Success(()), "the tracked sleep must finish");

    let state = metrics.read(|registry| registry.histogram_value(&timer));
    assert_eq!(state.count, 1);
    assert_eq!(state.min, 3_600_000.0);
    assert_eq!(state.max, 3_600_000.0);
    assert_eq!(state.sum, 3_600_000.0);
}

/// :1034 — a wall clock set backwards still measures the time that passed,
/// because the reading is monotonic.
#[test]
fn track_duration_uses_monotonic_time_when_wall_time_moves_backward() {
    let metrics = Metrics::new();
    let timer = timer("duration-monotonic");
    let clock = Arc::new(TestClock::new());
    clock.set_time(1_000);

    // The body requires the clock it drives: `use_service` is how an effect
    // declares that requirement, and `provide` is then allowed to answer it.
    let body = elm_effect::use_service::<Clock>().flat_map({
        let clock = Arc::clone(&clock);
        move |_| {
            let clock = Arc::clone(&clock);
            Effect::<(), Infallible, Clock>::sync(move || {
                clock.adjust(Duration::from_millis(100));
                clock.set_time(0);
            })
        }
    });
    let record = timer.clone();
    let timed = body.track_duration(&metrics, move |elapsed, registry| {
        registry.observe_histogram(&record, elapsed.to_millis())
    });
    let service: Arc<dyn elm_effect::ClockImpl> = clock.clone();
    let exit = run(timed.provide::<Clock, ()>(service));

    assert!(matches!(exit, Exit::Success(())), "the body must complete");
    let state = metrics.read(|registry| registry.histogram_value(&timer));
    assert_eq!(state.count, 1);
    assert_eq!(state.min, 100.0);
    assert_eq!(state.max, 100.0);
    assert_eq!(state.sum, 100.0);
}

/// :1053 — the hook maps the duration before it reaches the metric: an hour of
/// virtual time is 60 for a gauge read in minutes.
#[test]
fn track_duration_maps_the_duration_into_a_gauge() {
    let metrics = Metrics::new();
    let gauge = Gauge::new("duration-minutes");
    let clock = Arc::new(TestClock::new());
    let runtime = ManagedRuntime::new();

    let record = gauge.clone();
    let sleep = Effect::<(), i32, Cons<Clock, ()>>::sleep(Duration::from_minutes(60))
        .provide::<Clock, ()>(clock.clone())
        .track_duration(&metrics, move |elapsed, registry| {
            registry.set(&record, elapsed.to_minutes())
        });
    let fiber = runtime.run_fork(sleep);

    wait_for_sleepers(&clock, 1);
    clock.adjust(Duration::from_minutes(60));
    assert_eq!(fiber.join(), Exit::Success(()));

    assert_eq!(
        metrics.read(|registry| registry.gauge_value(&gauge).value),
        60.0
    );
}

/// :292 — two equal counter descriptions share a series, so tracking both
/// counts twice, while a differently attributed counter is its own series.
#[test]
fn tracking_equal_descriptions_shares_one_series() {
    let metrics = Metrics::new();
    let attributes = |pairs: [(&str, &str); 2]| Attributes::from_pairs(pairs);

    let counter1 = counter("referential").with_attributes(attributes([("x", "a"), ("y", "b")]));
    let counter2 = counter("referential").with_attributes(attributes([("x", "a"), ("y", "b")]));
    let counter3 = counter("referential").with_attributes(attributes([("z", "c"), ("y", "b")]));

    for counter in [&counter1, &counter2, &counter3] {
        run(tracked(
            Effect::<(), i32, ()>::succeed(()),
            &metrics,
            counter,
        ));
    }

    assert_eq!(count(&metrics, &counter1), 2.0);
    assert_eq!(count(&metrics, &counter2), 2.0);
    assert_eq!(count(&metrics, &counter3), 1.0);
}
