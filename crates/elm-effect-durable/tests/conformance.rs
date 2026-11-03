//! Conformance suite for durable engines (ADR-0005).
//!
//! Runs against [`OurEngine`]. A future engine must pass the same cases; there
//! is no vendor adapter layer. Carrier laws stay in ADR-0002 /
//! elm-effect-journal; these cases pin the scheduling port.

use elm_effect::{ClockImpl, Duration, TestClock};
use elm_effect_durable::{
    ClaimOutcome, ClaimRejection, DurableEngine, EngineError, EngineMsg, EngineStorage, MemStorage,
    OurEngine, ParkOutcome, ParkedRow, ParseRunPhaseError, RunPhase, RunRow, RunStatus,
    SettleOutcome, SettleRejection, StartOptions, StartOutcome, WakeOutcome, child_key,
    engine_laws, step_key, storage_laws,
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
    let row = eng
        .journal()
        .get("run-a")
        .expect("get")
        .expect("journal row");
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
    let settled = eng
        .settle("u1", generation, EffectState::Settled)
        .expect("settle");
    assert!(
        settled.is_none(),
        "Unknown must not settle without reconcile"
    );
    // Cancel remains an admitted exit.
    let cancelled = eng
        .settle("u1", generation, EffectState::Cancelled)
        .expect("settle");
    assert!(cancelled.is_some());
}

#[test]
fn park_is_not_terminal() {
    let mut eng = OurEngine::new();
    let _ = eng.start("p1", 0).unwrap();
    let out = eng.park_until("p1", 1_000, 60, 0, None).expect("park");
    assert!(matches!(out, ParkOutcome::Wait { .. }));
    let st = eng.query("p1").unwrap();
    assert_eq!(st.phase, RunPhase::Parked);
    assert!(!st.phase.is_terminal());
    // Wake resumes — not fail/settle.
    let _ = eng.wake("p1", 1_000).unwrap();
    assert_eq!(eng.query("p1").unwrap().phase, RunPhase::Running);
}

#[test]
fn wake_on_time_overdue_and_over_ceiling() {
    let clock = TestClock::new();
    let mut eng = OurEngine::new();
    let _ = eng.start("t1", clock_ms(&clock)).unwrap();

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
    let _ = eng
        .park_until("t1", 1_000, 60, clock_ms(&clock), None)
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
    let _ = eng
        .park_until("t1", 2_000, 60, clock_ms(&clock), None)
        .unwrap();
    clock.adjust(Duration::from_millis(500)); // now 1500 — not due yet
    assert!(eng.drain_due(clock_ms(&clock)).unwrap().is_empty());
    clock.set_time(2_500);
    let msgs = eng.drain_due(clock_ms(&clock)).unwrap();
    let EngineMsg::Woke { overdue_ms, .. } = &msgs[0];
    assert_eq!(*overdue_ms, 500);
}

#[test]
fn cancel_is_terminal() {
    let mut eng = OurEngine::new();
    let _ = eng.start("c1", 0).unwrap();
    let _ = eng.park_until("c1", 9_999, 60, 0, None).unwrap();
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
    assert!(
        eng.settle("g1", 0, EffectState::Settled)
            .expect("settle")
            .is_none()
    );
    // Live generation can (InFlight → Settled).
    let row = eng
        .settle("g1", generation, EffectState::Settled)
        .expect("settle")
        .expect("live generation settles");
    assert_eq!(row.state, EffectState::Settled);
    // Terminal row: further settle is refused.
    assert!(
        eng.settle("g1", generation, EffectState::Failed)
            .expect("settle")
            .is_none()
    );
}

#[test]
fn parking_same_key_same_instant_is_one_row() {
    let mut eng = OurEngine::new();
    let _ = eng.start("s1", 0).unwrap();
    let _ = eng.park_until("s1", 5_000, 60, 0, None).unwrap();
    let _ = eng.park_until("s1", 5_000, 60, 0, None).unwrap();
    let parks = eng.parked().expect("parked");
    assert_eq!(parks.len(), 1, "one row for the same instant: {parks:?}");
    assert_eq!(parks[0].until_ms, 5_000);
}

#[test]
fn reparking_a_different_instant_keeps_one_park() {
    let mut eng = OurEngine::new();
    let _ = eng.start("p3", 0).unwrap();
    let _ = eng.park_until("p3", 1_000, 60, 0, None).unwrap();
    let _ = eng.park_until("p3", 2_000, 60, 0, None).unwrap();

    let parks = eng.parked().expect("parked");
    assert_eq!(parks.len(), 1, "one active park per run: {parks:?}");
    assert_eq!(parks[0].until_ms, 2_000);
    assert_eq!(parks[0].park_generation, 2);
}

#[test]
fn restart_recovery_wakes_pending_parks() {
    let mut eng = OurEngine::new();
    let _ = eng.start("r1", 0).unwrap();
    let _ = eng.park_until("r1", 100, 60, 0, None).unwrap();
    let _ = eng.start("r2", 0).unwrap();
    let _ = eng.park_until("r2", 200, 60, 0, None).unwrap();

    // Snapshot storage and "crash".
    let snapshot = {
        let mut snap = MemStorage::new();
        for key in ["r1", "r2"] {
            snap.put_run(eng.query(key).unwrap().into_run_row())
                .expect("put run");
        }
        for p in eng.parked().expect("parked") {
            snap.upsert_park(p).expect("park");
        }
        snap
    };
    drop(eng);

    let mut recovered = OurEngine::recover(snapshot);
    assert_eq!(recovered.parked().expect("parked").len(), 2);
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

#[test]
fn settle_projects_terminal_phase_and_clears_park() {
    let mut eng = OurEngine::new();
    let StartOutcome::Started { generation } = eng.start("s2", 0).unwrap() else {
        panic!("started");
    };
    let _ = eng.park_until("s2", 1_000, 60, 0, None).unwrap();
    assert_eq!(eng.parked().expect("parked").len(), 1);

    let row = eng
        .settle("s2", generation, EffectState::Settled)
        .expect("settle")
        .expect("row");
    assert_eq!(row.state, EffectState::Settled);
    let status = eng.query("s2").unwrap();
    assert_eq!(status.phase, RunPhase::Settled);
    assert_eq!(status.until_ms, None);
    assert!(
        eng.parked().expect("parked").is_empty(),
        "a terminal settle clears the park"
    );
}

#[test]
fn claim_projects_generation_and_clears_park() {
    let mut eng = OurEngine::new();
    let StartOutcome::Started { generation } = eng.start("c2", 0).unwrap() else {
        panic!("started");
    };
    let _ = eng.park_until("c2", 1_000, 60, 0, None).unwrap();

    let claimed = eng
        .claim("c2", "worker", 10_000, 0, generation)
        .expect("claim")
        .expect("row");
    assert!(claimed.generation > generation);
    let status = eng.query("c2").unwrap();
    assert_eq!(status.phase, RunPhase::Running);
    assert_eq!(status.generation, claimed.generation);
    assert!(
        eng.parked().expect("parked").is_empty(),
        "a claim is a dispatch, not a wait"
    );
}

#[test]
fn unknown_clears_park_and_cannot_resume() {
    let mut eng = OurEngine::new();
    let _ = eng.start("u2", 0).unwrap();
    let _ = eng.park_until("u2", 1_000, 60, 0, None).unwrap();
    eng.mark_unknown("u2").unwrap();
    assert!(
        eng.parked().expect("parked").is_empty(),
        "unknown is not a park"
    );

    assert!(matches!(
        eng.park_until("u2", 2_000, 60, 0, None),
        Err(EngineError::Uncertain { .. })
    ));
    assert!(matches!(
        eng.wake("u2", 0),
        Err(EngineError::Uncertain { .. })
    ));
    // Cancel remains an admitted exit.
    eng.cancel("u2", 0).unwrap();
    assert_eq!(eng.query("u2").unwrap().phase, RunPhase::Cancelled);
}

#[test]
fn drain_due_clears_orphaned_and_terminal_parks() {
    let mut storage = MemStorage::new();
    storage
        .put_run(RunRow {
            key: "gone".into(),
            phase: RunPhase::Settled,
            generation: 1,
            until_ms: Some(100),
            attempt: 1,
        })
        .expect("put run");
    storage
        .upsert_park(ParkedRow {
            key: "gone".into(),
            until_ms: 100,
            park_generation: 1,
        })
        .expect("park");

    let mut eng = OurEngine::recover(storage);
    assert!(eng.drain_due(200).unwrap().is_empty());
    assert!(
        eng.parked().expect("parked").is_empty(),
        "a stale park must not block the drain"
    );
    assert_eq!(eng.query("gone").unwrap().phase, RunPhase::Settled);
}

#[test]
fn prepared_intent_is_completed_by_start() {
    let mut eng = OurEngine::new();
    let prepared = eng.journal().record_intent("p2", 8).expect("record intent");
    assert_eq!(prepared.state, EffectState::Prepared);

    let outcome = eng.start("p2", 0).expect("start completes the intent");
    assert!(matches!(outcome, StartOutcome::Started { .. }));
    assert_eq!(
        eng.journal()
            .get("p2")
            .expect("get")
            .expect("journal row")
            .state,
        EffectState::InFlight
    );
    assert!(matches!(
        eng.start("p2", 0).unwrap(),
        StartOutcome::Already { .. }
    ));
}

#[test]
fn host_journal_is_injected() {
    use elm_effect_journal::store::InMemoryStore;

    let journal = InMemoryStore::new(1024);
    journal.record_intent("host", 8).expect("record intent");
    let mut eng = OurEngine::with_journal(MemStorage::new(), journal);

    assert!(matches!(
        eng.start("host", 0).unwrap(),
        StartOutcome::Started { .. }
    ));
    assert!(matches!(
        eng.start("host", 0).unwrap(),
        StartOutcome::Already { .. }
    ));
}

#[test]
fn journal_only_run_reattaches_with_a_run_row() {
    use elm_effect_journal::store::InMemoryStore;

    let journal = InMemoryStore::new(1024);
    let prepared = journal.record_intent("j1", 8).expect("record intent");
    let claimed = journal
        .claim("j1", "host", 0, 0, prepared.generation)
        .expect("claim")
        .expect("row");
    journal
        .settle("j1", claimed.generation, EffectState::InFlight)
        .expect("settle")
        .expect("row");

    let mut eng = OurEngine::with_journal(MemStorage::new(), journal);
    assert!(matches!(
        eng.start("j1", 0).unwrap(),
        StartOutcome::Already { .. }
    ));
    assert_eq!(eng.query("j1").unwrap().phase, RunPhase::Running);
    assert!(matches!(
        eng.park_until("j1", 1_000, 60, 0, None).unwrap(),
        ParkOutcome::Wait { .. }
    ));
}

#[test]
fn live_host_lease_reattaches_without_stealing() {
    let mut eng = OurEngine::new();
    let prepared = eng
        .journal()
        .record_intent("leased", 8)
        .expect("record intent");
    let claimed = eng
        .journal()
        .claim("leased", "host", 10_000, 0, prepared.generation)
        .expect("host claim")
        .expect("row");

    let outcome = eng.start("leased", 0).expect("re-attach");
    assert!(matches!(outcome, StartOutcome::Already { .. }));
    assert_eq!(
        eng.journal()
            .get("leased")
            .expect("get")
            .expect("journal")
            .generation,
        claimed.generation
    );

    // Once the lease expires, the engine may complete the prepared intent.
    let outcome = eng.start("leased", 20_000).expect("expired lease");
    assert!(matches!(outcome, StartOutcome::Started { .. }));
}

#[test]
fn journal_terminal_wins_over_stale_storage() {
    use elm_effect_journal::store::InMemoryStore;

    let journal = InMemoryStore::new(1024);
    let prepared = journal.record_intent("stale", 8).expect("record intent");
    let claimed = journal
        .claim("stale", "host", 0, 0, prepared.generation)
        .expect("claim")
        .expect("row");
    journal
        .settle("stale", claimed.generation, EffectState::InFlight)
        .expect("settle")
        .expect("row");
    journal
        .settle("stale", claimed.generation, EffectState::Settled)
        .expect("settle")
        .expect("row");

    let mut storage = MemStorage::new();
    storage
        .put_run(RunRow {
            key: "stale".into(),
            phase: RunPhase::Running,
            generation: claimed.generation,
            until_ms: None,
            attempt: 0,
        })
        .expect("put run");
    let mut eng = OurEngine::with_journal(storage, journal);

    assert!(matches!(
        eng.park_until("stale", 1_000, 60, 0, None),
        Err(EngineError::Terminal { .. })
    ));
    assert!(matches!(
        eng.wake("stale", 0),
        Err(EngineError::Terminal { .. })
    ));
    assert_eq!(eng.query("stale").unwrap().phase, RunPhase::Settled);
}

#[test]
fn newer_journal_fence_wins_over_stale_park() {
    use elm_effect_journal::store::InMemoryStore;

    let journal = InMemoryStore::new(1024);
    let prepared = journal.record_intent("fenced", 8).expect("record intent");
    let first = journal
        .claim("fenced", "host", 0, 0, prepared.generation)
        .expect("first claim")
        .expect("row");
    let second = journal
        .claim("fenced", "host", 0, 0, first.generation)
        .expect("second claim")
        .expect("row");
    journal
        .settle("fenced", second.generation, EffectState::InFlight)
        .expect("settle")
        .expect("row");

    let mut storage = MemStorage::new();
    storage
        .put_run(RunRow {
            key: "fenced".into(),
            phase: RunPhase::Parked,
            generation: first.generation,
            until_ms: Some(1_000),
            attempt: 0,
        })
        .expect("put run");

    let eng = OurEngine::with_journal(storage, journal);
    let status = eng.query("fenced").expect("query");
    assert_eq!(status.phase, RunPhase::Running);
    assert_eq!(status.generation, second.generation);
    assert_eq!(status.until_ms, None);
}

#[test]
fn restart_without_journal_redispatches_a_running_run() {
    let mut eng = OurEngine::new();
    let _ = eng.start("r3", 0).unwrap();
    let snapshot = {
        let mut snap = MemStorage::new();
        snap.put_run(eng.query("r3").unwrap().into_run_row())
            .expect("put run");
        snap
    };
    drop(eng);

    let mut recovered = OurEngine::recover(snapshot);
    let outcome = recovered.start("r3", 0).expect("re-dispatch");
    assert!(
        matches!(outcome, StartOutcome::Started { .. }),
        "a storage row without a journal intent is not a dispatch record"
    );
    assert!(matches!(
        recovered.start("r3", 0).unwrap(),
        StartOutcome::Already { .. }
    ));
}

#[test]
fn recovery_reseeds_park_generation() {
    let mut storage = MemStorage::new();
    storage
        .put_run(RunRow {
            key: "gen".into(),
            phase: RunPhase::Parked,
            generation: 1,
            until_ms: Some(100),
            attempt: 1,
        })
        .expect("put run");
    storage
        .upsert_park(ParkedRow {
            key: "gen".into(),
            until_ms: 100,
            park_generation: 7,
        })
        .expect("park");

    let mut eng = OurEngine::recover(storage);
    let _ = eng.park_until("gen", 100, 60, 0, None).unwrap();
    assert_eq!(eng.parked().expect("parked")[0].park_generation, 7);

    let _ = eng.park_until("gen", 200, 60, 0, None).unwrap();
    assert_eq!(eng.parked().expect("parked")[0].park_generation, 8);
    assert_eq!(eng.parked().expect("parked")[0].until_ms, 200);
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

#[test]
fn run_phase_codecs_roundtrip() {
    use std::str::FromStr as _;

    for phase in [
        RunPhase::Running,
        RunPhase::Parked,
        RunPhase::Cancelled,
        RunPhase::Settled,
        RunPhase::Failed,
        RunPhase::Unknown,
    ] {
        // as_str, Display, and FromStr agree.
        assert_eq!(
            RunPhase::from_str(phase.as_str()).expect("wire name parses"),
            phase
        );
        assert_eq!(phase.to_string(), phase.as_str());
        // serde keeps the lowercase wire name.
        let json = serde_json::to_string(&phase).expect("serialize");
        assert_eq!(json, format!("\"{}\"", phase.as_str()));
        assert_eq!(
            serde_json::from_str::<RunPhase>(&json).expect("deserialize"),
            phase
        );
    }
    assert!(matches!(
        RunPhase::from_str("bogus"),
        Err(ParseRunPhaseError(_))
    ));
}

#[test]
fn start_with_records_custom_options() {
    let mut eng = OurEngine::new();
    let opts = StartOptions {
        max_attempts: 3,
        lease_ms: 5_000,
        owner: "custom".into(),
    };
    let StartOutcome::Started { generation } =
        eng.start_with("opts", 0, &opts).expect("start_with")
    else {
        panic!("start_with must Start");
    };
    let journal = eng.journal().get("opts").expect("get").expect("row");
    assert_eq!(journal.max_attempts, 3);
    assert_eq!(journal.owner.as_deref(), Some("custom"));
    // A live host lease re-attaches instead of stealing; an expired one
    // completes the prepared intent.
    let prepared = eng.journal().record_intent("leased", 8).expect("intent");
    let _ = eng
        .journal()
        .claim("leased", "host", 10_000, 0, prepared.generation)
        .expect("host claim")
        .expect("row");
    assert!(
        matches!(
            eng.start_with("leased", 1_000, &opts).expect("re-attach"),
            StartOutcome::Already { .. }
        ),
        "live lease must re-attach"
    );
    // Once the lease lapses the intent completes.
    assert!(
        matches!(
            eng.start_with("leased", 60_000, &opts)
                .expect("expired lease"),
            StartOutcome::Started { .. }
        ),
        "expired lease completes the intent"
    );
    let _ = generation;
}

#[test]
fn attempt_dead_letter_projects_terminal_and_keeps_parks_otherwise() {
    // Dead-letter: max_attempts 0 turns the first attempt terminal.
    let mut eng = OurEngine::new();
    let opts = StartOptions {
        max_attempts: 0,
        ..StartOptions::default()
    };
    let StartOutcome::Started { generation } =
        eng.start_with("dead", 0, &opts).expect("start_with")
    else {
        panic!("start_with must Start");
    };
    let _ = eng.park_until("dead", 1_000, 60, 0, None).expect("park");
    let row = eng
        .attempt("dead", generation)
        .expect("attempt")
        .expect("row");
    assert_eq!(row.state, EffectState::Failed);
    let status = eng.query("dead").expect("query");
    assert_eq!(status.phase, RunPhase::Failed);
    assert!(
        eng.parked().expect("parked").is_empty(),
        "dead-letter clears the park"
    );

    // Ordinary retry: the fence moves, the live park survives.
    let mut eng = OurEngine::new();
    let StartOutcome::Started { generation } = eng.start("retry", 0).expect("start") else {
        panic!("start must Start");
    };
    let _ = eng.park_until("retry", 1_000, 60, 0, None).expect("park");
    let row = eng
        .attempt("retry", generation)
        .expect("attempt")
        .expect("row");
    assert_eq!(row.attempts, 1);
    assert_eq!(row.state, EffectState::InFlight);
    assert_eq!(eng.query("retry").expect("query").phase, RunPhase::Parked);
    assert_eq!(
        eng.parked().expect("parked").len(),
        1,
        "retry keeps the park"
    );
}

#[test]
fn reconcile_steps_unknown_to_outcome() {
    for (key, outcome, phase) in [
        ("rec-settled", EffectState::Settled, RunPhase::Settled),
        ("rec-failed", EffectState::Failed, RunPhase::Failed),
    ] {
        let mut eng = OurEngine::new();
        let StartOutcome::Started { generation } = eng.start(key, 0).expect("start") else {
            panic!("start must Start");
        };
        eng.mark_unknown(key).expect("mark unknown");
        let row = eng
            .reconcile(key, generation, outcome)
            .expect("reconcile")
            .expect("row");
        assert_eq!(row.state, outcome);
        assert_eq!(eng.query(key).expect("query").phase, phase);
    }
    // Only uncertain rows reconcile: a live run is not a reconcile input.
    let mut eng = OurEngine::new();
    let StartOutcome::Started { generation } = eng.start("rec-live", 0).expect("start") else {
        panic!("start must Start");
    };
    assert!(
        eng.reconcile("rec-live", generation, EffectState::Settled)
            .expect("reconcile")
            .is_none()
    );
}

#[test]
fn concurrent_settle_projects_the_live_fence() {
    let mut eng = OurEngine::new();
    let StartOutcome::Started { generation } = eng.start("conc", 0).expect("start") else {
        panic!("start must Start");
    };
    let claimed = eng
        .claim("conc", "worker", 10_000, 0, generation)
        .expect("claim")
        .expect("row");
    // A zombie holding the pre-claim fence settles stale.
    assert!(
        eng.settle("conc", generation, EffectState::Settled)
            .expect("settle")
            .is_none()
    );
    // Storage still carries the live fence, not the zombie's.
    let status = eng.query("conc").expect("query");
    assert_eq!(status.generation, claimed.generation);
    assert_eq!(status.phase, RunPhase::Running);
}

#[test]
fn law_suites_pass_on_the_shipped_store_and_engine() {
    let mut store = MemStorage::new();
    for (name, outcome) in storage_laws(&mut store) {
        assert!(
            outcome.is_ok(),
            "storage law failed: {name}: {}",
            outcome.unwrap_err()
        );
    }
    let mut engine = OurEngine::new();
    for (name, outcome) in engine_laws(&mut engine) {
        assert!(
            outcome.is_ok(),
            "engine law failed: {name}: {}",
            outcome.unwrap_err()
        );
    }
}

#[test]
fn expire_lease_recovers_through_reconcile() {
    let mut eng = OurEngine::new();

    // A live lease refuses without mutating anything.
    let live = StartOptions {
        max_attempts: 8,
        lease_ms: 10_000,
        owner: "live".into(),
    };
    let StartOutcome::Started { .. } = eng.start_with("exp-live", 0, &live).unwrap() else {
        panic!("live start must Start");
    };
    assert!(
        eng.expire_lease("exp-live", 0).unwrap().is_none(),
        "live lease must refuse"
    );
    assert_eq!(eng.query("exp-live").unwrap().phase, RunPhase::Running);

    // Absent keys and settled rows are not expire inputs.
    assert!(eng.expire_lease("exp-nope", 0).unwrap().is_none());

    // An expired lease moves InFlight to Unknown, clearing the park.
    let StartOutcome::Started { .. } = eng.start("exp-dead", 0).unwrap() else {
        panic!("start must Start");
    };
    let _ = eng.park_until("exp-dead", 1_000, 60, 0, None).unwrap();
    let updated = eng
        .expire_lease("exp-dead", 0)
        .unwrap()
        .expect("expired lease must transition");
    assert_eq!(updated.state, EffectState::Unknown);
    assert_eq!(eng.query("exp-dead").unwrap().phase, RunPhase::Unknown);
    assert!(eng.parked().unwrap().is_empty(), "expire clears the park");

    // Reconcile recovers cleanly to Settled.
    let row = eng
        .reconcile("exp-dead", updated.generation, EffectState::Settled)
        .unwrap()
        .expect("reconcile must return a row");
    assert_eq!(row.state, EffectState::Settled);
    assert_eq!(eng.query("exp-dead").unwrap().phase, RunPhase::Settled);
}

#[test]
fn root_key_helpers_match_cron() {
    assert_eq!(child_key("parent", "step", 2), "parent:step:2");
    assert_eq!(step_key("run", "step"), "run:step");
}

#[test]
fn engine_types_serde_roundtrip() {
    let opts = StartOptions {
        max_attempts: 3,
        lease_ms: 5_000,
        owner: "serde".into(),
    };
    let json = serde_json::to_string(&opts).unwrap();
    assert_eq!(serde_json::from_str::<StartOptions>(&json).unwrap(), opts);

    let started = StartOutcome::Started { generation: 7 };
    let json = serde_json::to_string(&started).unwrap();
    assert_eq!(
        serde_json::from_str::<StartOutcome>(&json).unwrap(),
        started
    );

    let status = RunStatus {
        key: "k".into(),
        phase: RunPhase::Parked,
        generation: 2,
        until_ms: Some(1_000),
    };
    let json = serde_json::to_string(&status).unwrap();
    assert_eq!(serde_json::from_str::<RunStatus>(&json).unwrap(), status);

    let already = StartOutcome::Already { status };
    let json = serde_json::to_string(&already).unwrap();
    assert_eq!(
        serde_json::from_str::<StartOutcome>(&json).unwrap(),
        already
    );

    let wait = ParkOutcome::Wait {
        until_ms: 1_000,
        wait_ms: 500,
    };
    let json = serde_json::to_string(&wait).unwrap();
    assert_eq!(serde_json::from_str::<ParkOutcome>(&json).unwrap(), wait);

    let woke = EngineMsg::Woke {
        key: "k".into(),
        overdue_ms: 3,
        park_generation: 1,
    };
    let json = serde_json::to_string(&woke).unwrap();
    assert_eq!(serde_json::from_str::<EngineMsg>(&json).unwrap(), woke);

    let run = RunRow {
        key: "k".into(),
        phase: RunPhase::Running,
        generation: 1,
        until_ms: None,
        attempt: 0,
    };
    let json = serde_json::to_string(&run).unwrap();
    assert_eq!(serde_json::from_str::<RunRow>(&json).unwrap(), run);

    let park = ParkedRow {
        key: "k".into(),
        until_ms: 1_000,
        park_generation: 1,
    };
    let json = serde_json::to_string(&park).unwrap();
    assert_eq!(serde_json::from_str::<ParkedRow>(&json).unwrap(), park);
}

#[test]
fn wake_checked_reports_woken_vs_not_parked() {
    let mut eng = OurEngine::new();
    let StartOutcome::Started { .. } = eng.start("wc1", 0).unwrap() else {
        panic!("expected Started");
    };
    // Live but never parked: names the Running phase.
    assert!(matches!(
        eng.wake_checked("wc1", 0).unwrap(),
        WakeOutcome::NotParked {
            phase: RunPhase::Running
        }
    ));
    // Parked then woken late: names the gap and the live generation.
    let ParkOutcome::Wait { until_ms, .. } = eng.park_until("wc1", 1_000, 60, 0, None).unwrap()
    else {
        panic!("expected Wait");
    };
    assert_eq!(until_ms, 1_000);
    match eng.wake_checked("wc1", 1_250).unwrap() {
        WakeOutcome::Woken {
            overdue_ms,
            park_generation,
        } => {
            assert_eq!(overdue_ms, 250);
            assert!(park_generation >= 1, "park generation is live");
        }
        WakeOutcome::NotParked { .. } => panic!("parked wake must be Woken"),
    }
    assert_eq!(eng.query("wc1").unwrap().phase, RunPhase::Running);
}

#[test]
fn claim_checked_names_every_refusal() {
    let mut eng = OurEngine::new();
    // Absent key.
    assert!(matches!(
        eng.claim_checked("cc-absent", "w", 10_000, 0, 0).unwrap(),
        ClaimOutcome::Rejected(ClaimRejection::NotFound)
    ));
    // Live claim succeeds; a zombie fence rejects stale.
    let StartOutcome::Started { generation } = eng.start("cc1", 0).unwrap() else {
        panic!("expected Started");
    };
    let ClaimOutcome::Claimed(row) = eng
        .claim_checked("cc1", "worker", 10_000, 0, generation)
        .unwrap()
    else {
        panic!("live claim must succeed");
    };
    assert!(row.generation > generation);
    match eng
        .claim_checked("cc1", "zombie", 10_000, 0, generation)
        .unwrap()
    {
        ClaimOutcome::Rejected(ClaimRejection::StaleGeneration { expected, actual }) => {
            assert_eq!(expected, generation);
            assert_eq!(actual, row.generation);
        }
        other => panic!("zombie claim must be StaleGeneration: {other:?}"),
    }
    // Terminal rows never move.
    eng.settle("cc1", row.generation, EffectState::Settled)
        .unwrap()
        .expect("live settle");
    assert!(matches!(
        eng.claim_checked("cc1", "late", 10_000, 0, row.generation)
            .unwrap(),
        ClaimOutcome::Rejected(ClaimRejection::Terminal { .. })
    ));
    // A live lease under another owner blocks the steal.
    let opts = StartOptions {
        max_attempts: 8,
        lease_ms: 10_000,
        owner: "holder".into(),
    };
    let StartOutcome::Started { generation: leased } =
        eng.start_with("cc-lease", 0, &opts).unwrap()
    else {
        panic!("expected Started");
    };
    match eng
        .claim_checked("cc-lease", "thief", 20_000, 0, leased)
        .unwrap()
    {
        ClaimOutcome::Rejected(ClaimRejection::LeaseLive { owner, .. }) => {
            assert_eq!(owner, "holder");
        }
        other => panic!("leased claim must be LeaseLive: {other:?}"),
    }
}

#[test]
fn settle_checked_names_every_refusal() {
    let mut eng = OurEngine::new();
    // Absent key.
    assert!(matches!(
        eng.settle_checked("sc-absent", 0, EffectState::Settled)
            .unwrap(),
        SettleOutcome::Rejected(SettleRejection::NotFound)
    ));
    let StartOutcome::Started { generation } = eng.start("sc1", 0).unwrap() else {
        panic!("expected Started");
    };
    // Illegal edge: the journal has no Parked edge (parks are engine state).
    match eng
        .settle_checked("sc1", generation, EffectState::Parked)
        .unwrap()
    {
        SettleOutcome::Rejected(SettleRejection::IllegalTransition { from, to }) => {
            assert_eq!(from, EffectState::InFlight);
            assert_eq!(to, EffectState::Parked);
        }
        other => panic!("park edge must be IllegalTransition: {other:?}"),
    }
    // Stale fence after a claim moves it.
    let ClaimOutcome::Claimed(claimed) = eng
        .claim_checked("sc1", "worker", 10_000, 0, generation)
        .unwrap()
    else {
        panic!("live claim must succeed");
    };
    match eng
        .settle_checked("sc1", generation, EffectState::Settled)
        .unwrap()
    {
        SettleOutcome::Rejected(SettleRejection::StaleGeneration { expected, actual }) => {
            assert_eq!(expected, generation);
            assert_eq!(actual, claimed.generation);
        }
        other => panic!("stale settle must be StaleGeneration: {other:?}"),
    }
    // Live settle succeeds; terminal rows never move.
    let SettleOutcome::Settled(row) = eng
        .settle_checked("sc1", claimed.generation, EffectState::Settled)
        .unwrap()
    else {
        panic!("live settle must succeed");
    };
    assert_eq!(row.state, EffectState::Settled);
    assert!(matches!(
        eng.settle_checked("sc1", claimed.generation, EffectState::Failed)
            .unwrap(),
        SettleOutcome::Rejected(SettleRejection::Terminal { .. })
    ));
}

#[test]
fn stale_attempt_projects_the_live_row() {
    let mut eng = OurEngine::new();
    let StartOutcome::Started { generation } = eng.start("sa1", 0).unwrap() else {
        panic!("expected Started");
    };
    let ClaimOutcome::Claimed(claimed) = eng
        .claim_checked("sa1", "worker", 10_000, 0, generation)
        .unwrap()
    else {
        panic!("live claim must succeed");
    };
    // Zombie attempt on the old fence: refused, but storage carries the
    // live fence instead of going stale.
    assert!(eng.attempt("sa1", generation).unwrap().is_none());
    assert_eq!(eng.query("sa1").unwrap().generation, claimed.generation);
}
