//! Lease heartbeat: renew the lease without moving the fence (ADR-0005 §1).
//!
//! A long worker extends its claim and learns it was cancelled; zombies,
//! thieves, and lapsed leases renew nothing.

use std::sync::Arc;

use elm_effect::{ClockImpl, Duration, Effect, ManagedRuntime, TestClock};
use elm_effect_durable::lease::{HeartbeatOutcome, heartbeat_laws};
use elm_effect_durable::{
    DurableEngine, EngineError, OurEngine, RunPhase, StartOptions, StartOutcome,
};
use elm_effect_elm::{
    Boot, Cmd, Durability, Failure, Interpret, InterpretError, Loop, Producer, Program, Update,
};
use elm_effect_journal::EffectState;
use elm_effect_journal::durable_log::MemLog;
use elm_effect_journal::laws::journal_lease_laws;
use elm_effect_journal::plan::{HeartbeatRejection, check_heartbeat};
use elm_effect_journal::store::{InMemoryStore, JournalStore};
use serde::{Deserialize, Serialize};

fn leased(engine: &mut OurEngine, key: &str) -> u64 {
    let opts = StartOptions {
        max_attempts: 8,
        lease_ms: 10_000,
        owner: "worker".into(),
    };
    match engine.start_with(key, 0, &opts).expect("start_with") {
        StartOutcome::Started { generation } => generation,
        StartOutcome::Already { .. } => panic!("fresh key must start: {key}"),
    }
}

#[test]
fn renew_moves_the_deadline_and_keeps_the_fence() {
    let mut engine = OurEngine::new();
    let generation = leased(&mut engine, "hb-renew");
    let before = engine
        .journal()
        .get("hb-renew")
        .expect("get")
        .expect("journal row");
    assert_eq!(before.lease_deadline, Some(10_000));

    match engine
        .heartbeat("hb-renew", generation, "worker", 10_000, 5_000)
        .expect("heartbeat")
    {
        HeartbeatOutcome::Renewed {
            lease_deadline,
            row,
        } => {
            assert_eq!(lease_deadline, 15_000, "deadline moves to now + lease");
            assert_eq!(row.generation, generation, "the fence does not move");
            assert_eq!(row.owner.as_deref(), Some("worker"), "the owner stays");
            assert_eq!(row.lease_deadline, Some(15_000));
            assert!(row.offset > before.offset, "a renew commits an offset");
        }
        other => panic!("live holder must renew: {other:?}"),
    }
    assert_eq!(
        engine.query("hb-renew").expect("query").phase,
        RunPhase::Running
    );
}

#[test]
fn renewed_lease_defers_expiry_then_lapses() {
    let mut engine = OurEngine::new();
    let generation = leased(&mut engine, "hb-expire");
    match engine
        .heartbeat("hb-expire", generation, "worker", 10_000, 500)
        .expect("heartbeat")
    {
        HeartbeatOutcome::Renewed { .. } => {}
        other => panic!("live holder must renew: {other:?}"),
    }
    // Past the original deadline but before the renewed one: a no-op.
    assert!(
        engine
            .expire_lease("hb-expire", 1_000)
            .expect("expire")
            .is_none(),
        "renewed lease survives its old deadline"
    );
    assert_eq!(
        engine.query("hb-expire").expect("query").phase,
        RunPhase::Running
    );
    // At the renewed deadline the lease lapses to Unknown.
    let updated = engine
        .expire_lease("hb-expire", 10_500)
        .expect("expire")
        .expect("lapsed lease must transition");
    assert_eq!(updated.state, EffectState::Unknown);
    assert_eq!(
        engine.query("hb-expire").expect("query").phase,
        RunPhase::Unknown
    );
}

#[test]
fn zombie_generation_is_rejected() {
    let mut engine = OurEngine::new();
    let generation = leased(&mut engine, "hb-zombie");
    match engine
        .heartbeat("hb-zombie", 0, "worker", 10_000, 0)
        .expect("heartbeat")
    {
        HeartbeatOutcome::Rejected(HeartbeatRejection::StaleGeneration { expected, actual }) => {
            assert_eq!(expected, 0);
            assert_eq!(actual, generation);
        }
        other => panic!("zombie must be StaleGeneration: {other:?}"),
    }
}

#[test]
fn foreign_owner_is_rejected() {
    let mut engine = OurEngine::new();
    let generation = leased(&mut engine, "hb-thief");
    match engine
        .heartbeat("hb-thief", generation, "thief", 10_000, 0)
        .expect("heartbeat")
    {
        HeartbeatOutcome::Rejected(HeartbeatRejection::NotOwner { owner }) => {
            assert_eq!(owner.as_deref(), Some("worker"));
        }
        other => panic!("foreign owner must be NotOwner: {other:?}"),
    }
}

#[test]
fn lapsed_lease_is_rejected() {
    let mut engine = OurEngine::new();
    let generation = leased(&mut engine, "hb-lapsed");
    // The deadline (10_000) is at now: already expired.
    match engine
        .heartbeat("hb-lapsed", generation, "worker", 10_000, 10_000)
        .expect("heartbeat")
    {
        HeartbeatOutcome::Rejected(HeartbeatRejection::LeaseExpired { lease_deadline }) => {
            assert_eq!(lease_deadline, Some(10_000));
        }
        other => panic!("lapsed lease must be LeaseExpired: {other:?}"),
    }
}

#[test]
fn cancel_piggybacks_cancelled() {
    let mut engine = OurEngine::new();
    let generation = leased(&mut engine, "hb-cancel");
    engine.cancel("hb-cancel", 0).expect("cancel");
    match engine
        .heartbeat("hb-cancel", generation, "worker", 10_000, 0)
        .expect("heartbeat")
    {
        HeartbeatOutcome::Cancelled => {}
        other => panic!("cancelled row must piggyback Cancelled: {other:?}"),
    }
}

#[test]
fn terminal_rows_are_rejected() {
    for (key, to) in [
        ("hb-settled", EffectState::Settled),
        ("hb-failed", EffectState::Failed),
    ] {
        let mut engine = OurEngine::new();
        let generation = leased(&mut engine, key);
        engine
            .settle(key, generation, to)
            .expect("settle")
            .expect("live fence settles");
        match engine
            .heartbeat(key, generation, "worker", 10_000, 0)
            .expect("heartbeat")
        {
            HeartbeatOutcome::Rejected(HeartbeatRejection::Terminal { state }) => {
                assert_eq!(state, to);
            }
            other => panic!("terminal row must be Terminal: {other:?}"),
        }
    }
}

#[test]
fn non_running_rows_are_rejected() {
    let mut engine = OurEngine::new();
    // Prepared: a bare intent holds no lease.
    engine
        .journal()
        .record_intent("hb-prepared", 8)
        .expect("record intent");
    match engine
        .heartbeat("hb-prepared", 0, "worker", 10_000, 0)
        .expect("heartbeat")
    {
        HeartbeatOutcome::Rejected(HeartbeatRejection::NotRunning { state }) => {
            assert_eq!(state, EffectState::Prepared);
        }
        other => panic!("prepared row must be NotRunning: {other:?}"),
    }
    // Unknown: the outcome is uncertain, not leased.
    let generation = leased(&mut engine, "hb-unknown");
    engine.mark_unknown("hb-unknown").expect("mark unknown");
    match engine
        .heartbeat("hb-unknown", generation, "worker", 10_000, 0)
        .expect("heartbeat")
    {
        HeartbeatOutcome::Rejected(HeartbeatRejection::NotRunning { state }) => {
            assert_eq!(state, EffectState::Unknown);
        }
        other => panic!("unknown row must be NotRunning: {other:?}"),
    }
    // ReconcilePending: park the journal there (the outcome edge refuses,
    // the first reconcile edge already committed).
    let generation = leased(&mut engine, "hb-reconcile");
    engine.mark_unknown("hb-reconcile").expect("mark unknown");
    let _ = engine.reconcile("hb-reconcile", generation, EffectState::ReconcilePending);
    assert_eq!(
        engine
            .journal()
            .get("hb-reconcile")
            .expect("get")
            .expect("row")
            .state,
        EffectState::ReconcilePending
    );
    match engine
        .heartbeat("hb-reconcile", generation, "worker", 10_000, 0)
        .expect("heartbeat")
    {
        HeartbeatOutcome::Rejected(HeartbeatRejection::NotRunning { state }) => {
            assert_eq!(state, EffectState::ReconcilePending);
        }
        other => panic!("reconciling row must be NotRunning: {other:?}"),
    }
}

#[test]
fn absent_key_is_not_found() {
    let mut engine = OurEngine::new();
    match engine
        .heartbeat("hb-nope", 0, "nobody", 10_000, 0)
        .expect("heartbeat")
    {
        HeartbeatOutcome::Rejected(HeartbeatRejection::NotFound) => {}
        other => panic!("absent key must be NotFound: {other:?}"),
    }
}

#[test]
fn zero_lease_is_a_rejected_write() {
    let mut engine = OurEngine::new();
    let generation = leased(&mut engine, "hb-zero");
    match engine.heartbeat("hb-zero", generation, "worker", 0, 0) {
        Err(EngineError::Journal(_)) => {}
        other => panic!("zero lease must be a journal Rejected: {other:?}"),
    }
    // The rejected write moved nothing.
    let row = engine.journal().get("hb-zero").expect("get").expect("row");
    assert_eq!(row.lease_deadline, Some(10_000));
}

#[test]
fn check_heartbeat_names_each_refusal_in_order() {
    // Terminal first, even with every other fence tripped.
    assert_eq!(
        check_heartbeat(EffectState::Settled, None, None, "w", 0, 0, 1),
        Err(HeartbeatRejection::Terminal {
            state: EffectState::Settled
        })
    );
    // Generation before state, owner, and lease.
    assert_eq!(
        check_heartbeat(EffectState::Prepared, Some("x"), None, "w", 0, 0, 1),
        Err(HeartbeatRejection::StaleGeneration {
            expected: 0,
            actual: 1
        })
    );
    // State before owner and lease.
    assert_eq!(
        check_heartbeat(EffectState::Prepared, Some("w"), Some(9_999), "w", 0, 1, 1),
        Err(HeartbeatRejection::NotRunning {
            state: EffectState::Prepared
        })
    );
    // Owner before lease.
    assert_eq!(
        check_heartbeat(
            EffectState::InFlight,
            Some("holder"),
            Some(9_999),
            "w",
            0,
            1,
            1
        ),
        Err(HeartbeatRejection::NotOwner {
            owner: Some("holder".into())
        })
    );
    // A missing deadline is already expired.
    assert_eq!(
        check_heartbeat(EffectState::Accepted, Some("w"), None, "w", 0, 1, 1),
        Err(HeartbeatRejection::LeaseExpired {
            lease_deadline: None
        })
    );
    // Lapsed deadline, then the live case.
    assert!(check_heartbeat(EffectState::InFlight, Some("w"), Some(5), "w", 5, 1, 1).is_err());
    assert_eq!(
        check_heartbeat(EffectState::InFlight, Some("w"), Some(6), "w", 5, 1, 1),
        Ok(())
    );
    assert_eq!(
        check_heartbeat(EffectState::Accepted, Some("w"), Some(6), "w", 5, 1, 1),
        Ok(())
    );
}

#[test]
fn law_suites_cover_the_new_verbs() {
    let store = InMemoryStore::new(1024);
    for (name, outcome) in journal_lease_laws(&store) {
        assert!(outcome.is_ok(), "journal lease law failed: {name}");
    }
    let mut engine = OurEngine::new();
    for (name, outcome) in heartbeat_laws(&mut engine) {
        assert!(outcome.is_ok(), "heartbeat law failed: {name}");
    }
}

#[test]
fn outcome_and_rejection_serde_roundtrip() {
    let row = InMemoryStore::new(1024)
        .record_intent("serde", 3)
        .expect("record intent");
    for outcome in [
        HeartbeatOutcome::Renewed {
            lease_deadline: 1_000,
            row: row.clone(),
        },
        HeartbeatOutcome::Cancelled,
        HeartbeatOutcome::Rejected(HeartbeatRejection::NotFound),
        HeartbeatOutcome::Rejected(HeartbeatRejection::LeaseExpired {
            lease_deadline: Some(7),
        }),
    ] {
        let json = serde_json::to_string(&outcome).expect("serialize");
        assert_eq!(
            serde_json::from_str::<HeartbeatOutcome>(&json).expect("deserialize"),
            outcome
        );
    }
    // Rejection display names the refusal.
    assert!(
        HeartbeatRejection::NotFound
            .to_string()
            .contains("nothing to renew")
    );
}

// --- Live Loop witness: long work heartbeats on TestClock time. ---

#[derive(Clone, Debug, PartialEq)]
struct BeatModel {
    beats: u64,
    stopped: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum BeatMsg {
    Beat { deadline: i64 },
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum BeatCmd {
    Work { key: String },
}

impl Cmd for BeatCmd {
    fn durability(&self) -> Durability {
        match self {
            Self::Work { key } => Durability::Durable { key: key.clone() },
        }
    }
}

struct Beats;

impl Program for Beats {
    type Model = BeatModel;
    type Msg = BeatMsg;
    type Cmd = BeatCmd;
    type Err = InterpretError;

    fn init(_boot: &Boot) -> Update<BeatModel, BeatCmd> {
        Update::quiet(BeatModel {
            beats: 0,
            stopped: false,
        })
    }

    fn update(model: &BeatModel, msg: &BeatMsg) -> Update<BeatModel, BeatCmd> {
        match msg {
            BeatMsg::Beat { .. } => Update::quiet(BeatModel {
                beats: model.beats + 1,
                stopped: model.stopped,
            }),
            BeatMsg::Stopped => Update::quiet(BeatModel {
                beats: model.beats,
                stopped: true,
            }),
        }
    }

    fn failure(_failure: &Failure<InterpretError>) -> BeatMsg {
        BeatMsg::Stopped
    }
}

struct BeatRun;

impl Interpret<Beats> for BeatRun {
    fn interpret(_cmd: &BeatCmd) -> Effect<BeatMsg, InterpretError> {
        Effect::sync(|| BeatMsg::Stopped)
    }
}

#[test]
fn long_work_heartbeats_on_test_clock_and_stops_on_cancel() {
    let clock = TestClock::new();
    clock.set_time(0);
    let mut engine = OurEngine::new();
    let opts = StartOptions {
        max_attempts: 8,
        lease_ms: 1_000,
        owner: "long-worker".into(),
    };
    let StartOutcome::Started { generation } =
        engine.start_with("hb-loop", 0, &opts).expect("start")
    else {
        panic!("fresh key must start");
    };
    let mut app = Loop::<Beats, BeatRun>::boot(
        &Boot::default(),
        ManagedRuntime::new(),
        Arc::new(MemLog::new()),
        Producer::new("hb"),
    )
    .expect("boot");

    // Three lease windows pass on the virtual clock; each renews.
    for expected_beat in 1..=3u64 {
        clock.adjust(Duration::from_millis(500));
        let now = i64::try_from(clock.now_millis()).expect("clock fits");
        match engine
            .heartbeat("hb-loop", generation, "long-worker", 1_000, now)
            .expect("heartbeat")
        {
            HeartbeatOutcome::Renewed { lease_deadline, .. } => {
                assert_eq!(lease_deadline, now + 1_000);
                app.send(BeatMsg::Beat {
                    deadline: lease_deadline,
                })
                .expect("send");
            }
            other => panic!("long work must keep renewing: {other:?}"),
        }
        assert_eq!(app.model().beats, expected_beat);
    }
    // Cancel mid-work: the next heartbeat learns it must stop.
    engine.cancel("hb-loop", 0).expect("cancel");
    let now = i64::try_from(clock.now_millis()).expect("clock fits");
    match engine
        .heartbeat("hb-loop", generation, "long-worker", 1_000, now)
        .expect("heartbeat")
    {
        HeartbeatOutcome::Cancelled => app.send(BeatMsg::Stopped).expect("send"),
        other => panic!("cancelled work must stop: {other:?}"),
    }
    assert!(app.model().stopped);
    assert_eq!(app.model().beats, 3);
    assert_eq!(
        engine.query("hb-loop").expect("query").phase,
        RunPhase::Cancelled
    );
}
