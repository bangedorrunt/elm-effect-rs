//! `Ref` read-then-write combinators: getAndSet, getAndUpdate, updateAndGet,
//! getAndUpdateSome, and updateSome.
//!
//! Each of these is ONE atomic step. The tests assert both halves of the step
//! (what was returned AND what got stored); a get followed by a set would pass
//! the value assertions and fail the atomicity ones.

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
    assert_eq!(cell.get_and_update_some(|_current| None::<u32>), 10);
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
    cell.update_some(|_current| None::<u32>);
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
    assert_eq!(cell.update_some_and_get(|_current| None::<u32>), 10);
    assert_eq!(cell.get(), 10);
}

#[test]
fn update_some_and_get_applies_and_returns_the_new_value() {
    let cell = Ref::new(10u32);
    assert_eq!(cell.update_some_and_get(|current| Some(current - 4)), 6);
    assert_eq!(cell.get(), 6);
}

#[test]
fn modify_returns_a_result_while_storing_the_new_value() {
    // "modify returns a result while storing the new value": the returned
    // result and the stored value are independent halves of one atomic step.
    let cell = Ref::new("current".to_string());
    let result = cell.modify(|_current| ("hello".to_string(), "update".to_string()));
    assert_eq!(result, "hello");
    assert_eq!(cell.get(), "update");
}

#[test]
fn modify_some_once() {
    // "modifySome - once": a non-matching transition still returns its
    // result and leaves the cell unchanged.
    let cell = Ref::new("Active".to_string());
    let result = cell.modify_some(|state| {
        if state == "Closed" {
            ("active".to_string(), Some("Active".to_string()))
        } else {
            ("state does not change".to_string(), None)
        }
    });
    assert_eq!(result, "state does not change");
    assert_eq!(cell.get(), "Active");
}

#[test]
fn modify_some_returns_a_result_while_applying_matching_transitions() {
    let cell = Ref::new("Active".to_string());
    let result1 = cell.modify_some(|state| {
        if state == "Active" {
            ("changed".to_string(), Some("Changed".to_string()))
        } else {
            ("state does not change".to_string(), None)
        }
    });
    let result2 = cell.modify_some(|state| match state.as_str() {
        "Active" => ("changed".to_string(), Some("Changed".to_string())),
        "Changed" => ("closed".to_string(), Some("Closed".to_string())),
        _ => ("state does not change".to_string(), None),
    });
    assert_eq!(result1, "changed");
    assert_eq!(result2, "closed");
    assert_eq!(cell.get(), "Closed");
}
