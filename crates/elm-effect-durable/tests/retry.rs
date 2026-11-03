//! Retry backoff through park: count the attempt, then wait (ADR-0005 §2).
//!
//! A failed attempt parks to its backoff instant; exhaustion dead-letters;
//! a crash between the count and the park loses one delay, never an attempt.

use std::sync::Arc;

use elm_effect::{ClockImpl, Duration, Effect, ManagedRuntime, TestClock};
use elm_effect_durable::retry::{RetryOutcome, retry_laws};
use elm_effect_durable::{
    DurableEngine, OurEngine, ParkOutcome, RunPhase, StartOptions, StartOutcome,
};
use elm_effect_elm::{
    Boot, Cmd, Durability, Failure, Interpret, InterpretError, Loop, Producer, Program, Update,
};
use elm_effect_journal::EffectState;
use elm_effect_journal::durable_log::MemLog;
use elm_effect_journal::retry::{BackoffPolicy, RetryPlan, plan_retry};
use elm_effect_journal::store::{InMemoryStore, JournalStore};
use serde::{Deserialize, Serialize};

fn policy() -> BackoffPolicy {
    BackoffPolicy::Exponential {
        base_ms: 100,
        factor: 2,
        cap_ms: 10_000,
    }
}

fn started(engine: &mut OurEngine, key: &str, max_attempts: u32) -> u64 {
    let opts = StartOptions {
        max_attempts,
        ..StartOptions::default()
    };
    match engine.start_with(key, 0, &opts).expect("start_with") {
        StartOutcome::Started { generation } => generation,
        StartOutcome::Already { .. } => panic!("fresh key must start: {key}"),
    }
}

#[test]
fn exponential_backoff_doubles_to_the_cap() {
    let exp = policy();
    assert_eq!(exp.delay_ms(1), 100);
    assert_eq!(exp.delay_ms(2), 200);
    assert_eq!(exp.delay_ms(3), 400);
    assert_eq!(exp.delay_ms(7), 6_400);
    assert_eq!(exp.delay_ms(8), 10_000);
    assert_eq!(exp.delay_ms(100), 10_000);
}

#[test]
fn fixed_backoff_waits_the_delay() {
    let fixed = BackoffPolicy::Fixed { delay_ms: 1_500 };
    assert_eq!(fixed.delay_ms(1), 1_500);
    assert_eq!(fixed.delay_ms(9), 1_500);
}

#[test]
fn degenerate_factors_and_overflows_stay_bounded() {
    // factor < 2 behaves as fixed base_ms, still under the cap.
    for factor in [0, 1] {
        let flat = BackoffPolicy::Exponential {
            base_ms: 250,
            factor,
            cap_ms: 10_000,
        };
        assert_eq!(flat.delay_ms(1), 250);
        assert_eq!(flat.delay_ms(50), 250);
    }
    // A base above the cap clamps; saturation never wraps.
    let huge = BackoffPolicy::Exponential {
        base_ms: u64::MAX,
        factor: 10,
        cap_ms: 60_000,
    };
    assert_eq!(huge.delay_ms(1), 60_000);
    assert_eq!(huge.delay_ms(30), 60_000);
    let fixed_huge = BackoffPolicy::Fixed { delay_ms: u64::MAX };
    assert_eq!(
        plan_retry(1, 8, &fixed_huge, u64::MAX),
        RetryPlan::Backoff {
            attempt: 1,
            delay_ms: u64::MAX,
            until_ms: u64::MAX,
        }
    );
}

#[test]
fn exhaustion_boundary_equals_the_journal_dead_letter_rule() {
    // Pin plan_retry to InMemoryStore::attempt for max_attempts 0..=5:
    // the planner exhausts exactly when the journal dead-letters.
    for max_attempts in 0..=5u32 {
        let store = InMemoryStore::new(1024);
        let key = format!("boundary/{max_attempts}");
        store.record_intent(&key, max_attempts).expect("intent");
        let claimed = store
            .claim(&key, "worker", 9_999, 0, 0)
            .expect("claim")
            .expect("row");
        store
            .settle(&key, claimed.generation, EffectState::InFlight)
            .expect("settle")
            .expect("row");
        for recorded in 1..=(max_attempts + 2) {
            let row = store
                .attempt(&key, claimed.generation)
                .expect("attempt")
                .expect("row");
            let dead = row.state == EffectState::Failed;
            let planned = plan_retry(recorded, max_attempts, &policy(), 0);
            assert_eq!(
                matches!(planned, RetryPlan::Exhausted { .. }),
                dead,
                "max {max_attempts} attempt {recorded}: planner and journal must agree"
            );
            if dead {
                assert_eq!(
                    row.disposition,
                    Some(elm_effect_journal::store::Disposition::TerminalFailed)
                );
                break;
            }
            assert!(matches!(planned, RetryPlan::Backoff { .. }));
        }
    }
}

#[test]
fn retry_after_parks_to_the_planned_instant() {
    let mut engine = OurEngine::new();
    let generation = started(&mut engine, "rt-park", 8);
    match engine
        .retry_after(
            "rt-park",
            generation,
            &BackoffPolicy::Fixed { delay_ms: 1_000 },
            60,
            0,
            None,
        )
        .expect("retry_after")
    {
        RetryOutcome::Parked {
            attempt,
            until_ms,
            park,
        } => {
            assert_eq!(attempt, 1);
            assert_eq!(until_ms, 1_000);
            assert!(matches!(park, ParkOutcome::Wait { .. }));
        }
        other => panic!("first failure must park: {other:?}"),
    }
    // One ms early nothing fires; at the deadline one Woke with no overdue.
    assert!(engine.drain_due(999).expect("drain").is_empty());
    let wakes = engine.drain_due(1_000).expect("drain");
    assert_eq!(wakes.len(), 1);
    let elm_effect_durable::EngineMsg::Woke {
        key, overdue_ms, ..
    } = &wakes[0];
    assert_eq!(key, "rt-park");
    assert_eq!(*overdue_ms, 0);
}

#[test]
fn backoff_grows_across_attempts() {
    let mut engine = OurEngine::new();
    let generation = started(&mut engine, "rt-grow", 8);
    let first = engine
        .retry_after("rt-grow", generation, &policy(), 60, 0, None)
        .expect("first retry");
    let RetryOutcome::Parked { until_ms: u1, .. } = first else {
        panic!("first failure must park: {first:?}");
    };
    assert_eq!(u1, 100);
    let second = engine
        .retry_after("rt-grow", generation, &policy(), 60, u1, None)
        .expect("second retry");
    let RetryOutcome::Parked {
        attempt,
        until_ms: u2,
        ..
    } = second
    else {
        panic!("second failure must park: {second:?}");
    };
    assert_eq!(attempt, 2);
    assert_eq!(u2, u1 + 200);
}

#[test]
fn over_ceiling_refuses_but_counts_the_attempt() {
    let mut engine = OurEngine::new();
    let generation = started(&mut engine, "rt-ceil", 8);
    match engine
        .retry_after("rt-ceil", generation, &policy(), 200_000, 0, Some(86_400))
        .expect("retry_after")
    {
        RetryOutcome::Refused {
            asked_secs,
            ceiling_secs,
        } => {
            assert_eq!(asked_secs, 200_000);
            assert_eq!(ceiling_secs, 86_400);
        }
        other => panic!("over-ceiling must refuse: {other:?}"),
    }
    let row = engine.journal().get("rt-ceil").expect("get").expect("row");
    assert_eq!(row.attempts, 1, "the refused attempt still counts");
    assert_eq!(
        engine.query("rt-ceil").expect("query").phase,
        RunPhase::Running
    );
}

#[test]
fn exhaustion_dead_letters_and_clears_the_park() {
    let mut engine = OurEngine::new();
    let generation = started(&mut engine, "rt-dead", 0);
    let _ = engine
        .park_until("rt-dead", 5_000, 60, 0, None)
        .expect("control park");
    assert_eq!(engine.parked().expect("parked").len(), 1);
    match engine
        .retry_after("rt-dead", generation, &policy(), 60, 0, None)
        .expect("retry_after")
    {
        RetryOutcome::DeadLettered { row } => {
            assert_eq!(row.state, EffectState::Failed);
        }
        other => panic!("exhaustion must dead-letter: {other:?}"),
    }
    assert_eq!(
        engine.query("rt-dead").expect("query").phase,
        RunPhase::Failed
    );
    assert!(
        engine.parked().expect("parked").is_empty(),
        "dead-letter clears the park"
    );
}

#[test]
fn stale_fence_reports_stale_and_projects() {
    let mut engine = OurEngine::new();
    let generation = started(&mut engine, "rt-stale", 8);
    let claimed = engine
        .claim("rt-stale", "worker", 10_000, 0, generation)
        .expect("claim")
        .expect("row");
    match engine
        .retry_after("rt-stale", generation, &policy(), 60, 0, None)
        .expect("retry_after")
    {
        RetryOutcome::Stale => {}
        other => panic!("zombie fence must be Stale: {other:?}"),
    }
    // Storage carries the live fence, not the zombie's.
    assert_eq!(
        engine.query("rt-stale").expect("query").generation,
        claimed.generation
    );
}

#[test]
fn absent_key_is_stale() {
    let mut engine = OurEngine::new();
    match engine
        .retry_after("rt-nope", 0, &policy(), 60, 0, None)
        .expect("retry_after")
    {
        RetryOutcome::Stale => {}
        other => panic!("absent key must be Stale: {other:?}"),
    }
}

#[test]
fn law_suite_passes_and_types_roundtrip() {
    let mut engine = OurEngine::new();
    for (name, outcome) in retry_laws(&mut engine) {
        assert!(outcome.is_ok(), "retry law failed: {name}");
    }
    let fixed = BackoffPolicy::Fixed { delay_ms: 7 };
    let json = serde_json::to_string(&fixed).expect("serialize");
    assert_eq!(
        serde_json::from_str::<BackoffPolicy>(&json).expect("deserialize"),
        fixed
    );
    let plan = plan_retry(2, 8, &policy(), 1_000);
    let json = serde_json::to_string(&plan).expect("serialize");
    assert_eq!(
        serde_json::from_str::<RetryPlan>(&json).expect("deserialize"),
        plan
    );
}

// --- Live Loop witness: fail twice, back off twice, succeed. ---

#[derive(Clone, Debug, PartialEq)]
struct RetryModel {
    wakes: u64,
    done: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum RetryMsg {
    Woke { until_ms: u64 },
    Done,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum RetryCmd {
    Work { key: String },
}

impl Cmd for RetryCmd {
    fn durability(&self) -> Durability {
        match self {
            Self::Work { key } => Durability::Durable { key: key.clone() },
        }
    }
}

struct Retries;

impl Program for Retries {
    type Model = RetryModel;
    type Msg = RetryMsg;
    type Cmd = RetryCmd;
    type Err = InterpretError;

    fn init(_boot: &Boot) -> Update<RetryModel, RetryCmd> {
        Update::quiet(RetryModel {
            wakes: 0,
            done: false,
        })
    }

    fn update(model: &RetryModel, msg: &RetryMsg) -> Update<RetryModel, RetryCmd> {
        match msg {
            RetryMsg::Woke { .. } => Update::quiet(RetryModel {
                wakes: model.wakes + 1,
                done: model.done,
            }),
            RetryMsg::Done => Update::quiet(RetryModel {
                wakes: model.wakes,
                done: true,
            }),
        }
    }

    fn failure(_failure: &Failure<InterpretError>) -> RetryMsg {
        RetryMsg::Done
    }
}

struct RetryRun;

impl Interpret<Retries> for RetryRun {
    fn interpret(_cmd: &RetryCmd) -> Effect<RetryMsg, InterpretError> {
        Effect::sync(|| RetryMsg::Done)
    }
}

#[test]
fn fail_twice_back_off_twice_then_succeed_on_test_clock() {
    let clock = TestClock::new();
    clock.set_time(0);
    let mut engine = OurEngine::new();
    let generation = started(&mut engine, "rt-loop", 8);
    let mut app = Loop::<Retries, RetryRun>::boot(
        &Boot::default(),
        ManagedRuntime::new(),
        Arc::new(MemLog::new()),
        Producer::new("rt"),
    )
    .expect("boot");

    // First failure backs off 100 ms on the virtual clock.
    match engine
        .retry_after(
            "rt-loop",
            generation,
            &policy(),
            60,
            clock.now_millis(),
            None,
        )
        .expect("first retry")
    {
        RetryOutcome::Parked { until_ms, .. } => {
            assert_eq!(until_ms, 100);
            clock.adjust(Duration::from_millis(100));
            let wakes = engine.drain_due(clock.now_millis()).expect("drain");
            assert_eq!(wakes.len(), 1);
            app.send(RetryMsg::Woke { until_ms }).expect("send");
        }
        other => panic!("first failure must park: {other:?}"),
    }
    assert_eq!(app.model().wakes, 1);

    // Second failure backs off twice as long.
    match engine
        .retry_after(
            "rt-loop",
            generation,
            &policy(),
            60,
            clock.now_millis(),
            None,
        )
        .expect("second retry")
    {
        RetryOutcome::Parked { until_ms, .. } => {
            assert_eq!(until_ms, 300);
            clock.adjust(Duration::from_millis(200));
            let wakes = engine.drain_due(clock.now_millis()).expect("drain");
            assert_eq!(wakes.len(), 1);
            app.send(RetryMsg::Woke { until_ms }).expect("send");
        }
        other => panic!("second failure must park: {other:?}"),
    }
    assert_eq!(app.model().wakes, 2);

    // The third attempt succeeds; the fold observes completion.
    engine
        .settle("rt-loop", generation, EffectState::Settled)
        .expect("settle")
        .expect("live fence settles");
    app.send(RetryMsg::Done).expect("send");
    assert!(app.model().done);
    assert_eq!(
        engine.query("rt-loop").expect("query").phase,
        RunPhase::Settled
    );
}
