//! hi5 tail slice: SynchronizedRef.modifySomeEffect cells.
//! Ports packages/effect/test/SynchronizedRef.modifySomeEffect.test.ts rows
//! left uncovered by the area beads.
#![cfg(feature = "tokio")]

use elm_effect::{Effect, Exit, Runtime, SynchronizedRef};

// Row 5: the callback form applies the update and yields its result.
#[test]
fn modify_some_effect_applies_update_and_yields_result() {
    let cell = SynchronizedRef::new(10u32);
    let effect: Effect<String, String, ()> =
        cell.modify_some_effect("fallback".to_string(), |current: &u32| {
            Some((current + 1, format!("was {current}")))
        });
    let exit = Runtime::run(effect);
    assert!(matches!(exit, Exit::Success(ref s) if s == "was 10"));
    assert_eq!(cell.get(), 11);
}

// Row 16: None preserves the value and yields the fallback.
#[test]
fn modify_some_effect_none_preserves_value_with_fallback() {
    let cell = SynchronizedRef::new(10u32);
    let effect: Effect<String, String, ()> =
        cell.modify_some_effect("fallback".to_string(), |_: &u32| None);
    let exit = Runtime::run(effect);
    assert!(matches!(exit, Exit::Success(ref s) if s == "fallback"));
    assert_eq!(cell.get(), 10, "None leaves the stored value untouched");
}

#[test]
fn synchronised_ref_get_returns_current() {
    let cell = elm_effect::SynchronizedRef::new(5u32);
    assert_eq!(cell.get(), 5);
}

#[test]
fn get_and_update_effect_returns_previous_and_stores_update() {
    // SynchronizedRef row "getAndUpdateEffect - happy path": the RESULT is
    // the previous value and the cell now holds the update. Distinct from
    // update_effect, which returns the effect's own result.
    let cell = elm_effect::SynchronizedRef::new(1u32);
    let effect: Effect<u32, String, ()> =
        cell.get_and_update_effect(|current: &u32| (current + 10, Effect::succeed(99)));
    let exit = elm_effect::Runtime::run(effect);
    assert!(matches!(exit, Exit::Success(1)), "returns the PREVIOUS value");
    assert_eq!(cell.get(), 11, "stores the update");
}
