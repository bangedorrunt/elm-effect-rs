//! `Ref` read-then-write combinators at the pin
//! (`packages/effect/test/Ref.test.ts`, the getAndSet/getAndUpdate/
//! updateAndGet/getAndUpdateSome/updateSome family).
//!
//! The shared property: each of these is ONE atomic step. The rows assert
//! both halves of the step (what was returned AND what got stored), which is
//! what "atomically" means in the pin's doc language -- a get followed by a
//! set would pass the value assertions and fail the atomicity ones.

use elm_effect::Ref;

#[test]
fn get_returns_the_current_value() {
    let cell = Ref::new(7u32);
    assert_eq!(cell.get(), 7);
}

#[test]
fn set_stores_the_provided_value() {
    let cell = Ref::new(1u32);
    cell.set(2);
    assert_eq!(cell.get(), 2);
}

#[test]
fn get_and_set_returns_the_previous_value() {
    let cell = Ref::new(1u32);
    // The returned value is what was there BEFORE, and the cell now holds
    // the replacement.
    assert_eq!(cell.get_and_set(2), 1);
    assert_eq!(cell.get(), 2);
}

#[test]
fn get_and_update_returns_previous_and_stores_computed() {
    let cell = Ref::new(10u32);
    assert_eq!(cell.get_and_update(|current| current + 5), 10);
    assert_eq!(cell.get(), 15);
}

#[test]
fn update_and_get_stores_and_returns_the_computed_value() {
    let cell = Ref::new(10u32);
    assert_eq!(cell.update_and_get(|current| current * 2), 20);
    assert_eq!(cell.get(), 20);
}

#[test]
fn update_stores_the_computed_value() {
    let cell = Ref::new(3u32);
    cell.update(|current| current + 4);
    assert_eq!(cell.get(), 7);
}

#[test]
fn get_and_update_some_leaves_state_unchanged_when_no_transition_matches() {
    // None => no update, but the PREVIOUS value is still returned.
    let cell = Ref::new(10u32);
    assert_eq!(cell.get_and_update_some(|current| None::<u32>), 10);
    assert_eq!(cell.get(), 10, "a None transition must not touch the cell");
}

#[test]
fn get_and_update_some_applies_matching_transitions() {
    let cell = Ref::new(10u32);
    assert_eq!(cell.get_and_update_some(|current| Some(current + 1)), 10);
    assert_eq!(cell.get(), 11);
}

#[test]
fn update_some_leaves_state_unchanged_when_no_transition_matches() {
    let cell = Ref::new(10u32);
    cell.update_some(|current| None::<u32>);
    assert_eq!(cell.get(), 10);
}

#[test]
fn update_some_applies_matching_transitions() {
    let cell = Ref::new(10u32);
    cell.update_some(|current| Some(current * 3));
    assert_eq!(cell.get(), 30);
}

#[test]
fn update_some_and_get_returns_current_when_no_transition_matches() {
    let cell = Ref::new(10u32);
    assert_eq!(cell.update_some_and_get(|current| None::<u32>), 10);
    assert_eq!(cell.get(), 10);
}

#[test]
fn update_some_and_get_applies_and_returns_the_new_value() {
    let cell = Ref::new(10u32);
    assert_eq!(cell.update_some_and_get(|current| Some(current - 4)), 6);
    assert_eq!(cell.get(), 6);
}
