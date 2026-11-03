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
    DurableEngine, EngineError, OurEngine, RunPhase, StartOptions, StartOutcome, engine_laws,
    storage_laws,
};
use elm_effect_journal::EffectState;

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
    let mut engine = OurEngine::new();
    let laws = engine_laws(&mut engine);
    assert!(!laws.is_empty(), "engine laws must not be empty");
    for (name, outcome) in laws {
        assert!(outcome.is_ok(), "engine law failed: {name}");
    }
}
