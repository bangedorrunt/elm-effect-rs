//! The engine at the framework edge (ADR-0005).
//!
//! The engine is a synchronous kernel, like the journal. A host lifts a verb
//! into an `Effect`, provides the engine through its own `ServiceTag`, and
//! runs it at the edge. These witnesses keep that shape from drifting into a
//! second runtime.

use std::sync::{Arc, Mutex};

use elm_effect::{Effect, Exit, Runtime, ServiceTag, use_service};
use elm_effect_durable::{DurableEngine, OurEngine, StartOutcome};

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
