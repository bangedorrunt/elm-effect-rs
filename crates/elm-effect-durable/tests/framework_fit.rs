//! The engine at the framework edge (ADR-0005).
//!
//! The engine is a synchronous kernel, like the journal. A host lifts a verb
//! into an `Effect`, provides the engine through its own `ServiceTag`, and
//! runs it at the edge. These witnesses keep that shape from drifting into a
//! second runtime.

use std::sync::{Arc, Mutex};

use elm_effect::{Effect, Exit, Runtime, ServiceTag, use_service};
use elm_effect_durable::MemStorage;
use elm_effect_durable::{
    AwaitOutcome, BackoffPolicy, ContinueOutcome, CronPolicy, CronSchedule, DurableEngine,
    EngineError, GuardedStart, HeartbeatOutcome, OurEngine, RetryOutcome, RunPhase, SignalOutcome,
    StartOptions, StartOutcome, cancel_tree_laws, child_key, digest_storage_laws, engine_laws,
    fire_key, guard_laws, heartbeat_laws, lineage_laws, plan_cancel_children, plan_cron_tick,
    retry_laws, rollover_key, signal_laws, signal_storage_laws, step_key, storage_laws,
};
use elm_effect_journal::EffectState;
use elm_effect_journal::journal_lease_laws;

struct EngineTag;

impl ServiceTag for EngineTag {
    type Service = Arc<Mutex<OurEngine>>;
    const ID: &'static str = "test/durable-engine";
}

#[test]
fn engine_is_a_service_at_the_effect_edge() {
    let service = Arc::new(Mutex::new(OurEngine::new()));
    let effect = use_service::<EngineTag>().flat_map(|engine| {
        Effect::sync(move || {
            let mut engine = engine.lock().expect("engine lock");
            engine.start("framework-fit", 0)
        })
    });

    let exit = Runtime::run(effect.provide::<EngineTag, ()>(service));
    let Exit::Success(started) = exit else {
        panic!("engine start failed: {exit:?}");
    };
    assert!(matches!(started, Ok(StartOutcome::Started { .. })));
}

#[test]
fn retry_and_reconcile_at_the_effect_edge() {
    let service = Arc::new(Mutex::new(OurEngine::new()));
    let effect = use_service::<EngineTag>().flat_map(|engine| {
        Effect::sync(move || {
            let mut engine = engine.lock().expect("engine lock");
            let opts = StartOptions {
                max_attempts: 3,
                ..StartOptions::default()
            };
            let StartOutcome::Started { generation } = engine.start_with("edge-retry", 0, &opts)?
            else {
                return Err(EngineError::NotFound("edge-retry already started".into()));
            };
            let attempted = engine
                .attempt("edge-retry", generation)?
                .map(|row| row.attempts);
            engine.mark_unknown("edge-retry")?;
            let reconciled = engine
                .reconcile("edge-retry", generation, EffectState::Settled)?
                .map(|row| row.state);
            Ok((attempted, reconciled, engine.query("edge-retry")?.phase))
        })
    });

    let exit = Runtime::run(effect.provide::<EngineTag, ()>(service));
    let Exit::Success(outcome) = exit else {
        panic!("engine verbs failed: {exit:?}");
    };
    assert_eq!(
        outcome,
        Ok((Some(1), Some(EffectState::Settled), RunPhase::Settled))
    );
}

#[test]
fn law_suites_run_from_the_host_edge() {
    // A sibling host verifies the port without a vendor adapter: one call
    // per suite, every law green.
    let mut store = MemStorage::new();
    for (name, outcome) in storage_laws(&mut store) {
        assert!(outcome.is_ok(), "storage law failed: {name}");
    }
    for (name, outcome) in digest_storage_laws(&mut store) {
        assert!(outcome.is_ok(), "digest storage law failed: {name}");
    }
    for (name, outcome) in signal_storage_laws(&mut store) {
        assert!(outcome.is_ok(), "signal storage law failed: {name}");
    }

    for (name, outcome) in engine_laws(&mut OurEngine::new()) {
        assert!(outcome.is_ok(), "engine law failed: {name}");
    }
    for (name, outcome) in heartbeat_laws(&mut OurEngine::new()) {
        assert!(outcome.is_ok(), "heartbeat law failed: {name}");
    }
    for (name, outcome) in retry_laws(&mut OurEngine::new()) {
        assert!(outcome.is_ok(), "retry law failed: {name}");
    }
    for (name, outcome) in signal_laws(&mut OurEngine::new()) {
        assert!(outcome.is_ok(), "signal law failed: {name}");
    }
    for (name, outcome) in cancel_tree_laws(&mut OurEngine::new()) {
        assert!(outcome.is_ok(), "cancel tree law failed: {name}");
    }
    for (name, outcome) in lineage_laws(&mut OurEngine::new()) {
        assert!(outcome.is_ok(), "lineage law failed: {name}");
    }
    for (name, outcome) in guard_laws(&mut OurEngine::new()) {
        assert!(outcome.is_ok(), "guard law failed: {name}");
    }
    for (name, outcome) in journal_lease_laws(OurEngine::new().journal()) {
        assert!(outcome.is_ok(), "journal lease law failed: {name}");
    }
}

#[test]
fn expire_lease_at_the_effect_edge() {
    let service = Arc::new(Mutex::new(OurEngine::new()));
    let effect = use_service::<EngineTag>().flat_map(|engine| {
        Effect::sync(move || {
            let mut engine = engine.lock().expect("engine lock");
            // Live lease refuses at the edge.
            let live = StartOptions {
                max_attempts: 8,
                lease_ms: 10_000,
                owner: "edge".into(),
            };
            let StartOutcome::Started { .. } = engine.start_with("edge-live", 0, &live)? else {
                return Err(EngineError::NotFound("edge-live already started".into()));
            };
            assert!(engine.expire_lease("edge-live", 0)?.is_none());

            // Expired lease expires, then reconciles to Settled.
            let StartOutcome::Started { .. } = engine.start("edge-dead", 0)? else {
                return Err(EngineError::NotFound("edge-dead already started".into()));
            };
            let updated = engine
                .expire_lease("edge-dead", 0)?
                .ok_or_else(|| EngineError::NotFound("edge-dead must expire".into()))?;
            assert_eq!(updated.state, EffectState::Unknown);
            engine.reconcile("edge-dead", updated.generation, EffectState::Settled)?;
            Ok(engine.query("edge-dead")?.phase)
        })
    });

    let exit = Runtime::run(effect.provide::<EngineTag, ()>(service));
    let Exit::Success(outcome) = exit else {
        panic!("engine verbs failed: {exit:?}");
    };
    assert_eq!(outcome, Ok(RunPhase::Settled));
}

#[test]
fn root_keys_and_serde_at_the_edge() {
    use elm_effect_durable::{EngineMsg, ParkedRow, RunRow, RunStatus, child_key, step_key};

    assert_eq!(child_key("parent", "step", 0), "parent:step:0");
    assert_eq!(step_key("run", "step"), "run:step");

    let msg = EngineMsg::Woke {
        key: "k".into(),
        overdue_ms: 0,
        park_generation: 1,
    };
    let json = serde_json::to_string(&msg).expect("serialize");
    assert_eq!(
        serde_json::from_str::<EngineMsg>(&json).expect("deserialize"),
        msg
    );
    let row = RunRow {
        key: "k".into(),
        phase: RunPhase::Running,
        generation: 1,
        until_ms: None,
        attempt: 0,
    };
    let json = serde_json::to_string(&row).expect("serialize");
    assert_eq!(
        serde_json::from_str::<RunRow>(&json).expect("deserialize"),
        row
    );
    let park = ParkedRow {
        key: "k".into(),
        until_ms: 1_000,
        park_generation: 1,
    };
    let json = serde_json::to_string(&park).expect("serialize");
    assert_eq!(
        serde_json::from_str::<ParkedRow>(&json).expect("deserialize"),
        park
    );
    let status = RunStatus {
        key: "k".into(),
        phase: RunPhase::Running,
        generation: 1,
        until_ms: None,
    };
    let json = serde_json::to_string(&status).expect("serialize");
    assert_eq!(
        serde_json::from_str::<RunStatus>(&json).expect("deserialize"),
        status
    );
}

// --- ud08: live Elm Loop + OurEngine + TestClock witnesses ---

use elm_effect::{ClockImpl, Duration, ManagedRuntime, TestClock};
use elm_effect_durable::{EngineMsg, JoinOutcome, ParkOutcome, plan_join};
use elm_effect_elm::{
    Boot, Cmd, Durability, Failure, Interpret, InterpretError, Loop, Producer, Program, Update,
};
use elm_effect_journal::durable_log::MemLog;
use elm_effect_journal::saga::{SagaStep, SagaStepStatus, compensate_key, plan_saga_rollback};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq)]
struct PipeModel {
    wokes: u64,
    last_overdue_ms: u64,
    done: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum PipeMsg {
    Woke { overdue_ms: u64 },
    Done,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum PipeCmd {
    Work { key: String },
}

impl Cmd for PipeCmd {
    fn durability(&self) -> Durability {
        match self {
            Self::Work { key } => Durability::Durable { key: key.clone() },
        }
    }
}

struct Pipe;

impl Program for Pipe {
    type Model = PipeModel;
    type Msg = PipeMsg;
    type Cmd = PipeCmd;
    type Err = InterpretError;

    fn init(_boot: &Boot) -> Update<PipeModel, PipeCmd> {
        Update::quiet(PipeModel {
            wokes: 0,
            last_overdue_ms: 0,
            done: false,
        })
    }

    fn update(model: &PipeModel, msg: &PipeMsg) -> Update<PipeModel, PipeCmd> {
        match msg {
            PipeMsg::Woke { overdue_ms } => Update::quiet(PipeModel {
                wokes: model.wokes + 1,
                last_overdue_ms: *overdue_ms,
                done: model.done,
            }),
            PipeMsg::Done => Update::quiet(PipeModel {
                wokes: model.wokes,
                last_overdue_ms: model.last_overdue_ms,
                done: true,
            }),
        }
    }

    fn failure(_failure: &Failure<InterpretError>) -> PipeMsg {
        PipeMsg::Done
    }
}

struct PipeRun;

impl Interpret<Pipe> for PipeRun {
    fn interpret(_cmd: &PipeCmd) -> Effect<PipeMsg, InterpretError> {
        Effect::sync(|| PipeMsg::Done)
    }
}

fn pipe_app() -> Loop<Pipe, PipeRun> {
    Loop::<Pipe, PipeRun>::boot(
        &Boot::default(),
        ManagedRuntime::new(),
        Arc::new(MemLog::new()),
        Producer::new("ud08"),
    )
    .expect("boot")
}

fn started_here(engine: &mut OurEngine, key: &str) -> u64 {
    match engine.start(key, 0).expect("start") {
        StartOutcome::Started { generation } => generation,
        StartOutcome::Already { .. } => panic!("fresh key must start: {key}"),
    }
}

#[test]
fn pipe_commands_carry_durable_keys() {
    let cmd = PipeCmd::Work {
        key: "ud08-pipe".into(),
    };
    let Durability::Durable { key } = cmd.durability() else {
        panic!("pipe work must be durable");
    };
    assert_eq!(key, "ud08-pipe");
}

#[test]
fn elm_loop_wakes_through_test_clock_and_settles() {
    let clock = TestClock::new();
    clock.set_time(0);
    let mut engine = OurEngine::new();
    let generation = match engine
        .start("ud08-pipe", clock.now_millis())
        .expect("start")
    {
        StartOutcome::Started { generation } => generation,
        StartOutcome::Already { .. } => panic!("fresh key must start"),
    };
    let out = engine
        .park_until("ud08-pipe", 5_000, 60, 0, None)
        .expect("park");
    assert!(
        matches!(out, ParkOutcome::Wait { .. }),
        "future deadline parks: {out:?}"
    );

    // Time moves only through the clock; the engine reads what the host passes.
    clock.adjust(Duration::from_millis(6_000));
    let now_ms = clock.now_millis();
    assert_eq!(now_ms, 6_000);
    let wakes = engine.drain_due(now_ms).expect("drain");
    assert_eq!(wakes.len(), 1, "exactly one wake fires");
    let EngineMsg::Woke {
        key, overdue_ms, ..
    } = &wakes[0];
    assert_eq!(key, "ud08-pipe");
    assert_eq!(*overdue_ms, 1_000);

    // The wake crosses into the Elm fold as an ordinary message.
    let mut app = pipe_app();
    app.send(PipeMsg::Woke {
        overdue_ms: *overdue_ms,
    })
    .expect("send");
    assert_eq!(app.model().wokes, 1);
    assert_eq!(app.model().last_overdue_ms, 1_000);

    // The finished work settles the run; the fold observes completion.
    engine
        .settle("ud08-pipe", generation, EffectState::Settled)
        .expect("settle")
        .expect("live fence settles");
    assert_eq!(
        engine.query("ud08-pipe").expect("query").phase,
        RunPhase::Settled
    );
    app.send(PipeMsg::Done).expect("send");
    assert!(app.model().done);
}

#[test]
fn fan_out_join_pending_then_all_settled_and_failed() {
    let mut engine = OurEngine::new();
    let obt_a = child_key("ud08-parent", "fetch", 0);
    let obt_b = child_key("ud08-parent", "render", 0);
    let _ = engine.start("ud08-parent", 0).expect("parent starts");
    let gen_a = started_here(&mut engine, &obt_a);
    let gen_b = started_here(&mut engine, &obt_b);
    let _ = engine
        .park_until(&obt_b, 10_000, 60, 0, None)
        .expect("park one child");

    // One Running plus one Parked child is still pending, in slice order.
    let phases = vec![
        (obt_a.clone(), engine.query(&obt_a).expect("query").phase),
        (obt_b.clone(), engine.query(&obt_b).expect("query").phase),
    ];
    assert_eq!(
        plan_join(&phases),
        JoinOutcome::Pending {
            remaining: vec![obt_a.clone(), obt_b.clone()]
        }
    );

    // Every child settled is AllSettled.
    engine
        .settle(&obt_a, gen_a, EffectState::Settled)
        .expect("settle a")
        .expect("live fence");
    engine
        .settle(&obt_b, gen_b, EffectState::Settled)
        .expect("settle b")
        .expect("live fence");
    let phases = vec![
        (obt_a.clone(), engine.query(&obt_a).expect("query").phase),
        (obt_b.clone(), engine.query(&obt_b).expect("query").phase),
    ];
    assert_eq!(plan_join(&phases), JoinOutcome::AllSettled);

    // Terminal failure wins over pending: the running child never masks it.
    let obt_c = child_key("ud08-parent2", "fetch", 0);
    let obt_d = child_key("ud08-parent2", "render", 0);
    let _ = engine.start("ud08-parent2", 0).expect("parent2 starts");
    started_here(&mut engine, &obt_c);
    started_here(&mut engine, &obt_d);
    engine.cancel(&obt_d, 0).expect("cancel one child");
    let phases = vec![
        (obt_c.clone(), engine.query(&obt_c).expect("query").phase),
        (obt_d.clone(), engine.query(&obt_d).expect("query").phase),
    ];
    assert_eq!(
        plan_join(&phases),
        JoinOutcome::Failed {
            failed: vec![],
            cancelled: vec![obt_d.clone()],
            unknown: vec![],
        }
    );
}

#[test]
fn saga_forward_failure_rolls_back_lifo_through_engine() {
    let mut engine = OurEngine::new();
    let run = "ud08-saga";
    let charge = step_key(run, "charge");
    let reserve = step_key(run, "reserve");
    let ship = step_key(run, "ship");
    let gen_charge = started_here(&mut engine, &charge);
    let gen_reserve = started_here(&mut engine, &reserve);
    let gen_ship = started_here(&mut engine, &ship);

    // Two forward steps settle; the third fails and triggers the rollback.
    engine
        .settle(&charge, gen_charge, EffectState::Settled)
        .expect("settle charge")
        .expect("live fence");
    engine
        .settle(&reserve, gen_reserve, EffectState::Settled)
        .expect("settle reserve")
        .expect("live fence");
    engine
        .settle(&ship, gen_ship, EffectState::Failed)
        .expect("fail ship")
        .expect("live fence");
    assert_eq!(engine.query(&ship).expect("query").phase, RunPhase::Failed);

    let steps = vec![
        SagaStep {
            step_key: charge.clone(),
            compensate_key: compensate_key(run, "charge"),
            status: SagaStepStatus::Completed,
        },
        SagaStep {
            step_key: reserve.clone(),
            compensate_key: compensate_key(run, "reserve"),
            status: SagaStepStatus::Completed,
        },
        SagaStep {
            step_key: ship.clone(),
            compensate_key: compensate_key(run, "ship"),
            status: SagaStepStatus::Failed,
        },
    ];
    let plan = plan_saga_rollback(&steps);
    // Strict LIFO over the completed steps; the failed step has nothing to undo.
    assert_eq!(
        plan.compensations,
        vec![
            compensate_key(run, "reserve"),
            compensate_key(run, "charge"),
        ]
    );
    assert!(!plan.all_compensated);

    // Every compensation is its own durable run through the engine.
    for compensation in &plan.compensations {
        let generation = started_here(&mut engine, compensation);
        engine
            .settle(compensation, generation, EffectState::Settled)
            .expect("settle compensation")
            .expect("live fence");
        assert_eq!(
            engine.query(compensation).expect("query").phase,
            RunPhase::Settled,
            "{compensation} must settle"
        );
    }

    // A rollback that crashed halfway resumes idempotently: compensated
    // steps never re-emit.
    let resumed = vec![
        SagaStep {
            step_key: charge.clone(),
            compensate_key: compensate_key(run, "charge"),
            status: SagaStepStatus::Compensated,
        },
        SagaStep {
            step_key: reserve.clone(),
            compensate_key: compensate_key(run, "reserve"),
            status: SagaStepStatus::Completed,
        },
        SagaStep {
            step_key: ship.clone(),
            compensate_key: compensate_key(run, "ship"),
            status: SagaStepStatus::Failed,
        },
    ];
    assert_eq!(
        plan_saga_rollback(&resumed).compensations,
        vec![compensate_key(run, "reserve")]
    );
}

#[test]
fn cron_tick_fires_once_into_elm_loop() {
    // Half a minute into a minute: fresh under both policies.
    let base = 1_700_000_000_000u64;
    let now_ms = base - (base % 60_000) + 30_000;
    let tick_ms = now_ms - 30_000;
    for policy in [CronPolicy::Latest, CronPolicy::Skip { grace_ms: 60_000 }] {
        let schedule = CronSchedule::parse("* * * * *").expect("valid cron");
        let fire =
            plan_cron_tick("ud08-cron", &schedule, policy, None, now_ms).expect("fresh tick fires");
        assert_eq!(fire.tick_ms, tick_ms);
        assert_eq!(fire.key, fire_key("ud08-cron", tick_ms));

        let mut engine = OurEngine::new();
        assert!(
            matches!(
                engine.start(&fire.key, now_ms),
                Ok(StartOutcome::Started { .. })
            ),
            "first firing dispatches under {policy:?}"
        );
        assert!(
            matches!(
                engine.start(&fire.key, now_ms),
                Ok(StartOutcome::Already { .. })
            ),
            "replay re-attaches under {policy:?}"
        );

        // The fired run is drivable from the Elm fold like any other run.
        let mut app = pipe_app();
        app.send(PipeMsg::Done).expect("send");
        assert!(app.model().done);
        assert_eq!(
            engine.query(&fire.key).expect("query").phase,
            RunPhase::Running
        );
    }
}

#[test]
fn composed_durable_parity_workflow_in_elm_loop() {
    use elm_effect::{ClockImpl, TestClock};

    let clock = TestClock::new();
    let mut engine = OurEngine::new();

    // 1. Guarded start with replay digest
    let run_key = rollover_key("workflow-comp", 0);
    let opts = StartOptions {
        max_attempts: 5,
        lease_ms: 10_000,
        owner: "worker-comp".into(),
    };
    let generation = match engine
        .start_guarded(&run_key, clock.now_millis(), &opts, "digest-v1")
        .expect("start_guarded")
    {
        GuardedStart::Started { generation } => generation,
        other => panic!("expected started, got {other:?}"),
    };
    // Replay with same digest re-attaches
    assert!(matches!(
        engine
            .start_guarded(&run_key, clock.now_millis(), &opts, "digest-v1")
            .expect("replay"),
        GuardedStart::Already { .. }
    ));

    // 2. Latched signal before await
    assert!(matches!(
        engine
            .signal(&run_key, "sig-approve", "payload-ok", clock.now_millis())
            .expect("signal"),
        SignalOutcome::Latched { wake: None }
    ));
    // Await consumes the latched signal without parking
    match engine
        .await_signal(&run_key, 60_000, 3600, clock.now_millis(), None)
        .expect("await_signal")
    {
        AwaitOutcome::Signalled(rows) => {
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].signal_id, "sig-approve");
            assert_eq!(rows[0].payload, "payload-ok");
        }
        AwaitOutcome::Parked(_) => panic!("expected signalled, got parked"),
    }

    // 3. Lease Heartbeat
    clock.adjust(elm_effect::Duration::from_millis(5_000));
    let now_ms = clock.now_millis();
    let now_i64 = i64::try_from(now_ms).expect("i64 ms");
    match engine
        .heartbeat(&run_key, generation, "worker-comp", 10_000, now_i64)
        .expect("heartbeat")
    {
        HeartbeatOutcome::Renewed { lease_deadline, .. } => {
            assert_eq!(lease_deadline, now_i64 + 10_000);
        }
        other => panic!("expected renewed, got {other:?}"),
    }

    // 4. Retry Backoff through Park
    let backoff = BackoffPolicy::Exponential {
        base_ms: 1_000,
        factor: 2,
        cap_ms: 8_000,
    };
    match engine
        .retry_after(&run_key, generation, &backoff, 60, clock.now_millis(), None)
        .expect("retry_after")
    {
        RetryOutcome::Parked {
            attempt, until_ms, ..
        } => {
            assert_eq!(attempt, 1);
            assert_eq!(until_ms, clock.now_millis() + 1_000);
        }
        other => panic!("expected parked, got {other:?}"),
    }
    assert_eq!(
        engine.query(&run_key).expect("query").phase,
        RunPhase::Parked
    );

    // TestClock advance wakes the parked run
    clock.adjust(elm_effect::Duration::from_millis(1_000));
    let wakes = engine.drain_due(clock.now_millis()).expect("drain_due");
    assert_eq!(wakes.len(), 1);
    assert!(matches!(&wakes[0], EngineMsg::Woke { key, .. } if key == &run_key));
    assert_eq!(
        engine.query(&run_key).expect("query").phase,
        RunPhase::Running
    );

    // 5. Child tree fan-out and structured child cancellation
    let child_a = format!("{run_key}:child:a");
    let child_b = format!("{run_key}:child:b");
    engine.start(&child_a, clock.now_millis()).expect("start a");
    let gen_b = match engine.start(&child_b, clock.now_millis()).expect("start b") {
        StartOutcome::Started { generation } => generation,
        _ => panic!("fresh child_b must start"),
    };
    engine
        .settle(&child_b, gen_b, EffectState::Settled)
        .expect("settle b");
    let plan = plan_cancel_children(&[
        (child_a.clone(), RunPhase::Running),
        (child_b.clone(), RunPhase::Settled),
    ]);
    assert_eq!(plan, vec![child_a.clone()]);
    let report = engine
        .cancel_children(&[child_a.clone(), child_b.clone()], clock.now_millis())
        .expect("cancel_children");
    assert_eq!(report.cancelled, vec![child_a.clone()]);
    assert_eq!(report.already_terminal, vec![child_b.clone()]);

    // 6. Lineage Rollover (continue_as_new)
    let next_key = rollover_key("workflow-comp", 1);
    match engine
        .continue_as_new(&run_key, generation, &next_key, clock.now_millis(), &opts)
        .expect("continue_as_new")
    {
        ContinueOutcome::Continued { started, closed } => {
            assert!(matches!(started, StartOutcome::Started { .. }));
            assert_eq!(closed.state, EffectState::Settled);
        }
        ContinueOutcome::Rejected(r) => panic!("expected continued, got {r:?}"),
    }
    assert_eq!(
        engine.query(&run_key).expect("query").phase,
        RunPhase::Settled
    );
    assert_eq!(
        engine.query(&next_key).expect("query").phase,
        RunPhase::Running
    );
}
