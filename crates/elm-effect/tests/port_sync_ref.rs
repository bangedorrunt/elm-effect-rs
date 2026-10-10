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

#[test]
fn get_and_update_effect_with_failure_yields_the_failure() {
    // "getAndUpdateEffect - with failure": the effect fails, so the outcome
    // is the typed failure rather than a stored value.
    let cell = SynchronizedRef::new("value".to_string());
    let effect: Effect<String, String, ()> = cell.get_and_update_effect(|current: &String| {
        (
            current.clone(),
            Effect::<(), String, ()>::fail("failure".to_string()),
        )
    });
    match Runtime::run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"failure".to_string())),
        other => panic!("expected a typed failure, got {other:?}"),
    }
}

#[test]
fn get_and_update_some_effect_happy_path_keeps_active() {
    // "getAndUpdateSomeEffect - happy path": no transition matches, so the
    // cell keeps Active and the PREVIOUS value is returned.
    let cell = SynchronizedRef::new("Active".to_string());
    let effect: Effect<String, String, ()> = cell.get_and_update_some_effect(|state: &String| {
        if state == "Closed" {
            Effect::succeed(Some("Changed".to_string()))
        } else {
            Effect::succeed(None)
        }
    });
    assert!(matches!(Runtime::run(effect), Exit::Success(ref s) if s == "Active"));
    assert_eq!(cell.get(), "Active");
}

#[test]
fn get_and_update_some_effect_twice() {
    // "getAndUpdateSomeEffect - twice": an unconditional Some stores, and the
    // result is the value BEFORE that step.
    let cell = SynchronizedRef::new("Active".to_string());
    let first: Effect<String, String, ()> = cell.get_and_update_some_effect(|state: &String| {
        if state == "Active" {
            Effect::succeed(Some("Changed".to_string()))
        } else {
            Effect::succeed(None)
        }
    });
    let second: Effect<String, String, ()> = cell.get_and_update_some_effect(|state: &String| {
        match state.as_str() {
            "Closed" => Effect::succeed(Some("Active".to_string())),
            "Changed" => Effect::succeed(Some("Closed".to_string())),
            _ => Effect::succeed(None),
        }
    });
    assert!(matches!(Runtime::run(first), Exit::Success(ref s) if s == "Active"));
    assert!(matches!(Runtime::run(second), Exit::Success(ref s) if s == "Changed"));
    assert_eq!(cell.get(), "Closed");
}

#[test]
fn get_and_update_some_effect_with_failure_yields_the_failure() {
    // "getAndUpdateSomeEffect - with failure": the effect fails before any
    // update, so the cell is untouched and the typed failure is returned.
    let cell = SynchronizedRef::new("Active".to_string());
    let effect: Effect<String, String, ()> = cell.get_and_update_some_effect(|state: &String| {
        if state == "Active" {
            Effect::fail("failure".to_string())
        } else {
            Effect::succeed(None)
        }
    });
    match Runtime::run(effect) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"failure".to_string())),
        other => panic!("expected a typed failure, got {other:?}"),
    }
    assert_eq!(cell.get(), "Active", "a failed effect must not update the cell");
}

#[test]
fn get_and_update_some_updates_the_backing_ref() {
    // "getAndUpdateSome updates the backing ref": the pure conditional
    // variant returns the value before the update.
    let cell = SynchronizedRef::new(1u32);
    let previous = cell.get_and_update_some(|n: &u32| Some(n + 1));
    assert_eq!(previous, 1);
    assert_eq!(cell.get(), 2);
}

#[test]
fn get_and_update_some_effect_interrupt_parent_fiber_and_update() {
    // "getAndUpdateSomeEffect - interrupt parent fiber and update": the cell
    // is created by a child fiber that then parks. Interrupting that fiber
    // must not poison the cell, and the later update still lands.
    use elm_effect::{Cause, ForkOpts};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    let rt = Runtime::new();
    let slot: Arc<Mutex<Option<SynchronizedRef<String>>>> = Arc::new(Mutex::new(None));
    let created = Arc::new(AtomicBool::new(false));

    let (slot_c, created_c) = (Arc::clone(&slot), Arc::clone(&created));
    let child = rt.fork_detach_with(
        move |ctx| -> Exit<(), String> {
            *slot_c.lock().unwrap() = Some(SynchronizedRef::new("Active".to_string()));
            created_c.store(true, Ordering::SeqCst);
            while !ctx.is_interrupted() {
                std::thread::park_timeout(Duration::from_millis(1));
            }
            Exit::Failure(Cause::interrupt(None))
        },
        ForkOpts::immediate(),
    );

    while !created.load(Ordering::SeqCst) {
        std::thread::park_timeout(Duration::from_millis(1));
    }
    child.interrupt();

    let cell = slot.lock().unwrap().take().expect("child stored the cell");
    let effect: Effect<String, String, ()> = cell.update_and_get_effect(|_current: &String| {
        Effect::succeed("Closed".to_string())
    });
    let result = Runtime::run(effect);
    assert_eq!(result, Exit::Success("Closed".to_string()));
    assert_eq!(cell.get(), "Closed");
}
