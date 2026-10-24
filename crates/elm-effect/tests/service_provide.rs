//! `Effect.provideService` in Effect 4 (`Effect.test.ts:398`, "Context.Service").
//!
//! A tag is a type; providing its service removes the tag from the
//! requirement tuple and the effect reads the value at interpretation.

use elm_effect::{Effect, Exit, Runtime, ServiceTag, use_service};

struct ATag;

impl ServiceTag for ATag {
    type Service = String;
    const ID: &'static str = "EffectTest/ATag";
}

#[test]
fn context_service_is_provided_and_read() {
    let effect: Effect<String, std::convert::Infallible, ()> =
        use_service::<ATag>().provide::<ATag, ()>("A".to_string());
    assert_eq!(Runtime::run(effect), Exit::Success("A".to_string()));
}
