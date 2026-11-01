//! Conformance suite for durable engines (ADR-0005).
//!
//! Runs against [`OurEngine`]. A future vendor adapter must pass the same
//! cases. Carrier laws stay in ADR-0002 / elm-effect-journal; these cases pin
//! the scheduling port.

use elm_effect::{ClockImpl, Duration, TestClock};
use elm_effect_durable::{
    DurableEngine, EngineMsg, EngineStorage, MemStorage, OurEngine, ParkOutcome, RunPhase,
    StartOutcome,
};
use elm_effect_journal::EffectState;
use elm_effect_journal::store::JournalStore;

fn clock_ms(clock: &TestClock) -> u64 {
    clock.now_millis()
}

#[test]
fn replay_never_runs_an_effect_twice() {
    let mut eng = OurEngine::new();
    let first = eng.start("run-a", 0).expect("start");
    assert!(matches!(first, StartOutcome::Started { .. }));
    let second = eng.start("run-a", 0).expect("re-start");
    assert!(
        matches!(second, StartOutcome::Already { .. }),
        "second start must not dispatch again: {second:?}"
    );
    // Journal still has exactly one intent row for the key.
    let row = eng.journal().get("run-a").expect("journal row");
    assert_eq!(row.key, "run-a");
    assert_eq!(row.attempts, 0);
}

#[test]
fn unknown_has_no_default_exit() {
    let mut eng = OurEngine::new();
    let StartOutcome::Started { generation } = eng.start("u1", 0).unwrap() else {
        panic!("expected Started");
    };
    eng.mark_unknown("u1").expect("mark unknown");
    assert_eq!(eng.query("u1").unwrap().phase, RunPhase::Unknown);
    // Direct Settled from Unknown is refused by the golden table.
    let settled = eng.settle("u1", generation, EffectState::Settled);
    assert!(
        settled.is_none(),
        "Unknown must not settle without reconcile"
    );
    // Cancel remains an admitted exit.
    let cancelled = eng.settle("u1", generation, EffectState::Cancelled);
    assert!(cancelled.is_some());
}

#[test]
fn park_is_not_terminal() {
    let mut eng = OurEngine::new();
    eng.start("p1", 0).unwrap();
    let out = eng.park_until("p1", 1_000, 60, 0, None).expect("park");
    assert!(matches!(out, ParkOutcome::Wait { .. }));
    let st = eng.query("p1").unwrap();
    assert_eq!(st.phase, RunPhase::Parked);
    assert!(!st.phase.is_terminal());
    // Wake resumes — not fail/settle.
    eng.wake("p1", 1_000).unwrap();
    assert_eq!(eng.query("p1").unwrap().phase, RunPhase::Running);
}

#[test]
fn wake_on_time_overdue_and_over_ceiling() {
    let clock = TestClock::new();
    let mut eng = OurEngine::new();
    eng.start("t1", clock_ms(&clock)).unwrap();

    // Over ceiling: refuse, do not shorten.
    let refused = eng
        .park_until("t1", 10_000_000, 200_000, clock_ms(&clock), Some(86_400))
        .unwrap();
    assert!(matches!(
        refused,
        ParkOutcome::RefusedOverCeiling {
            asked_secs: 200_000,
            ceiling_secs: 86_400
        }
    ));
    assert_eq!(eng.query("t1").unwrap().phase, RunPhase::Running);

    // Park to T=1000.
    eng.park_until("t1", 1_000, 60, clock_ms(&clock), None)
        .unwrap();
    assert_eq!(eng.query("t1").unwrap().phase, RunPhase::Parked);

    // On time.
    clock.set_time(1_000);
    let msgs = eng.drain_due(clock_ms(&clock)).unwrap();
    assert_eq!(
        msgs,
        vec![EngineMsg::Woke {
            key: "t1".into(),
            overdue_ms: 0,
            park_generation: 1,
        }]
    );
    assert_eq!(eng.query("t1").unwrap().phase, RunPhase::Running);

    // Park again, then overdue.
    eng.park_until("t1", 2_000, 60, clock_ms(&clock), None)
        .unwrap();
    clock.adjust(Duration::from_millis(500)); // now 1500 — not due yet
    assert!(eng.drain_due(clock_ms(&clock)).unwrap().is_empty());
    clock.set_time(2_500);
    let msgs = eng.drain_due(clock_ms(&clock)).unwrap();
    assert_eq!(msgs[0].overdue_ms_if_woke(), Some(500));
}

trait WokeOverdue {
    fn overdue_ms_if_woke(&self) -> Option<u64>;
}
impl WokeOverdue for EngineMsg {
    fn overdue_ms_if_woke(&self) -> Option<u64> {
        match self {
            EngineMsg::Woke { overdue_ms, .. } => Some(*overdue_ms),
            _ => None,
        }
    }
}

#[test]
fn cancel_is_terminal() {
    let mut eng = OurEngine::new();
    eng.start("c1", 0).unwrap();
    eng.park_until("c1", 9_999, 60, 0, None).unwrap();
    eng.cancel("c1", 0).unwrap();
    assert_eq!(eng.query("c1").unwrap().phase, RunPhase::Cancelled);
    assert!(eng.query("c1").unwrap().phase.is_terminal());
    // Wake / park after cancel fail.
    assert!(eng.wake("c1", 9_999).is_err());
    assert!(eng.park_until("c1", 10_000, 60, 0, None).is_err());
    // Second cancel is idempotent.
    eng.cancel("c1", 0).unwrap();
}

#[test]
fn stale_generation_holder_cannot_settle() {
    let mut eng = OurEngine::new();
    let StartOutcome::Started { generation } = eng.start("g1", 0).unwrap() else {
        panic!("started");
    };
    // A zombie with generation 0 (pre-claim) cannot settle.
    assert!(eng.settle("g1", 0, EffectState::Settled).is_none());
    // Live generation can (InFlight → Settled).
    let row = eng
        .settle("g1", generation, EffectState::Settled)
        .expect("live generation settles");
    assert_eq!(row.state, EffectState::Settled);
    // Terminal row: further settle is refused.
    assert!(eng.settle("g1", generation, EffectState::Failed).is_none());
}

#[test]
fn parking_same_key_same_instant_is_one_row() {
    let mut eng = OurEngine::new();
    eng.start("s1", 0).unwrap();
    eng.park_until("s1", 5_000, 60, 0, None).unwrap();
    eng.park_until("s1", 5_000, 60, 0, None).unwrap();
    let parks = eng.parked();
    assert_eq!(parks.len(), 1, "one row for the same instant: {parks:?}");
    assert_eq!(parks[0].until_ms, 5_000);
}

#[test]
fn restart_recovery_wakes_pending_parks() {
    let mut eng = OurEngine::new();
    eng.start("r1", 0).unwrap();
    eng.park_until("r1", 100, 60, 0, None).unwrap();
    eng.start("r2", 0).unwrap();
    eng.park_until("r2", 200, 60, 0, None).unwrap();

    // Snapshot storage and "crash".
    let snapshot = {
        let mut snap = MemStorage::new();
        for key in ["r1", "r2"] {
            snap.put_run(eng.query(key).unwrap().into_run_row());
        }
        for p in eng.parked() {
            snap.upsert_park(p);
        }
        snap
    };
    drop(eng);

    let mut recovered = OurEngine::recover(snapshot);
    assert_eq!(recovered.parked().len(), 2);
    let msgs = recovered.drain_due(150).unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(
        msgs[0],
        EngineMsg::Woke {
            key: "r1".into(),
            overdue_ms: 50,
            park_generation: 1,
        }
    );
    assert_eq!(recovered.query("r1").unwrap().phase, RunPhase::Running);
    assert_eq!(recovered.query("r2").unwrap().phase, RunPhase::Parked);
}

/// Test helper: rebuild a [`elm_effect_durable::RunRow`] from status.
trait IntoRunRow {
    fn into_run_row(self) -> elm_effect_durable::RunRow;
}
impl IntoRunRow for elm_effect_durable::RunStatus {
    fn into_run_row(self) -> elm_effect_durable::RunRow {
        elm_effect_durable::RunRow {
            key: self.key,
            phase: self.phase,
            generation: self.generation,
            until_ms: self.until_ms,
            attempt: 1,
        }
    }
}
