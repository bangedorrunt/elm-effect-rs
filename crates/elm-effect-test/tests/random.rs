//! Law L8 (world substitution): seeded RNG replays, scripted values feed
//! sleepers, and the whole run stays on virtual time through `Fixture`.

use elm_effect::{Clock, ClockImpl, Duration, Effect, Exit, RandomTag, Scope};
use elm_effect_test::{TestContext, TestLive, effect_test};
use std::convert::Infallible;
use std::time::Instant;

#[test]
fn seeded_sequences_replay_deterministically() {
    let a = elm_effect_test::TestRandom::new(42);
    let b = elm_effect_test::TestRandom::new(42);
    let draws_a: Vec<f64> = (0..100).map(|_| a.next_unit()).collect();
    let draws_b: Vec<f64> = (0..100).map(|_| b.next_unit()).collect();
    assert_eq!(draws_a, draws_b);
    // Parity with the live type: the fake replays what live code would draw.
    let live = elm_effect::Random::with_seed(42);
    let c = elm_effect_test::TestRandom::new(42);
    for _ in 0..100 {
        assert_eq!(c.next_unit(), live.next_unit());
    }
}

#[test]
fn scripted_values_consumed_first_then_seeded_stream_resumes() {
    let rng = elm_effect_test::TestRandom::new(7);
    rng.feed([0.1, 0.2]);
    assert_eq!(rng.next_unit(), 0.1);
    assert_eq!(rng.next_unit(), 0.2);
    // Script exhausted: back on the seeded stream from its start.
    let live = elm_effect::Random::with_seed(7);
    assert_eq!(rng.next_unit(), live.next_unit());
}

#[test]
fn random_layer_serves_the_seeded_stream_as_the_random_service() {
    let ctx = TestContext::new(99);
    let expected = ctx.random.to_random().next_unit();
    let layer = ctx.random_layer();
    let scope = Scope::new(200);
    let e: Effect<f64, Infallible, RandomTag> =
        elm_effect::use_service::<RandomTag>().map(|r| r.next_unit());
    let provided = layer.provide(e, &scope).expect("closed layer provides");
    let rt = elm_effect::Runtime::new();
    assert_eq!(
        rt.spawn_effect(provided).join(),
        Exit::<f64, Infallible>::succeed(expected)
    );
}

effect_test! { fn scripted_random_feeds_a_sleeper_on_virtual_time(rt, clock) {
    // A fiber draws one scripted value, sleeps past it, and reports what it
    // drew. Virtual time drives the sleep; the script drives the draw.
    let rng = elm_effect_test::TestRandom::new(7);
    rng.feed([0.25]);
    let worker_rng = rng.clone();
    let worker_clock = clock.clone();
    let sleeper = rt.fork_detach_with(
        move |ctx| -> Exit<f64, String> {
            let drawn = worker_rng.next_unit();
            match worker_clock.sleep(Duration::from_minutes(5), &|| ctx.is_interrupted()) {
                elm_effect::SleepOutcome::Elapsed => Exit::<f64, String>::succeed(drawn),
                elm_effect::SleepOutcome::Interrupted => {
                    Exit::Failure(elm_effect::Cause::interrupt(None))
                }
            }
        },
        elm_effect::ForkOpts::immediate(),
    );
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    while clock.pending_sleepers() < 1 {
        assert!(Instant::now() < deadline, "sleeper never parked");
        std::thread::park_timeout(std::time::Duration::from_millis(1));
    }
    clock.adjust(Duration::from_minutes(5));
    assert_eq!(sleeper.join(), Exit::<f64, String>::succeed(0.25));
}}

#[test]
fn test_context_bundles_clock_and_random_layers() {
    let ctx = TestContext::new(11);
    let scope = Scope::new(200);
    let rt = elm_effect::Runtime::new();
    let clock_e: Effect<u64, Infallible, Clock> =
        elm_effect::use_service::<Clock>().map(|c| c.now_millis());
    let provided_clock = ctx
        .clock_layer()
        .provide(clock_e, &scope)
        .expect("closed layer provides");
    assert_eq!(
        rt.spawn_effect(provided_clock).join(),
        Exit::<u64, Infallible>::succeed(0)
    );
    let expected = ctx.random.to_random().next_unit();
    let rng_e: Effect<f64, Infallible, RandomTag> =
        elm_effect::use_service::<RandomTag>().map(|r| r.next_unit());
    let provided_rng = ctx
        .random_layer()
        .provide(rng_e, &scope)
        .expect("closed layer provides");
    assert_eq!(
        rt.spawn_effect(provided_rng).join(),
        Exit::<f64, Infallible>::succeed(expected)
    );
}

#[test]
fn live_services_stay_behind_test_live() {
    // The only wall-clock layer in the test crate is spelled `TestLive`.
    let layer = TestLive::system_clock_layer();
    let scope = Scope::new(200);
    let e: Effect<u64, Infallible, Clock> =
        elm_effect::use_service::<Clock>().map(|c| c.now_millis());
    let provided = layer.provide(e, &scope).expect("closed layer provides");
    let rt = elm_effect::Runtime::new();
    let Exit::Success(wall) = rt.spawn_effect(provided).join() else {
        panic!("live clock provides")
    };
    assert!(wall > 0, "wall clock reads real time");
    // The default world does not: TestContext starts at zero.
    let ctx = TestContext::new(0);
    assert_eq!(ctx.clock.now_millis(), 0);
}
