//! `Atom.optimistic`: reads follow a base atom, a write hands over a
//! transition atom, and the base is refreshed once the transition settles.

use elm_effect_unstable::reactivity::{async_atom, optimistic, AsyncResult, Atom, Registry};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn success_value<T: Clone, E>(result: &AsyncResult<T, E>) -> Option<T> {
    match result {
        AsyncResult::Success { value, .. } => Some(value.clone()),
        _ => None,
    }
}

#[test]
fn a_settled_transition_refreshes_the_base() {
    // The base answers 1, then 2 on refresh. The transition answers 7 but
    // must not become the value: the pin refreshes the base instead.
    let calls = Arc::new(AtomicUsize::new(0));
    let fed = Arc::clone(&calls);
    let base = async_atom(move || {
        let call = fed.fetch_add(1, Ordering::SeqCst) as i64;
        Ok::<_, String>(call + 1)
    });
    let writable = optimistic(&base);
    let registry = Registry::make();
    assert_eq!(success_value(&registry.get(&writable.atom())), Some(1));

    let transition = async_atom(|| Ok::<_, String>(7_i64));
    writable.write(&registry, transition);

    assert_eq!(calls.load(Ordering::SeqCst), 2, "the base was refreshed once");
    assert_eq!(
        success_value(&registry.get(&writable.atom())),
        Some(2),
        "the base's new answer wins over the transition's value"
    );
}

#[test]
fn a_waiting_transition_value_is_shown_immediately() {
    // The mutation writes a success already marked waiting, carrying the
    // optimistic value; the read must surface that value, not the base's old
    // one, while the transition is in flight.
    let base = async_atom(|| Ok::<_, String>(1_i64));
    let writable = optimistic(&base);
    let registry = Registry::make();
    assert_eq!(success_value(&registry.get(&writable.atom())), Some(1));

    let transition = Atom::make(AsyncResult::<i64, String>::success_waiting(9));
    writable.write(&registry, transition);

    assert_eq!(success_value(&registry.get(&writable.atom())), Some(9));
}

#[test]
fn a_failed_transition_restores_the_last_value_without_refreshing_the_base() {
    let calls = Arc::new(AtomicUsize::new(0));
    let fed = Arc::clone(&calls);
    let base = async_atom(move || {
        let call = fed.fetch_add(1, Ordering::SeqCst) as i64;
        Ok::<_, String>(call + 1)
    });
    let writable = optimistic(&base);
    let registry = Registry::make();
    assert_eq!(success_value(&registry.get(&writable.atom())), Some(1));

    let transition = async_atom(|| Err::<i64, String>("rejected".to_string()));
    writable.write(&registry, transition);

    assert_eq!(calls.load(Ordering::SeqCst), 1, "a failure is not a reason to refetch");
    assert_eq!(
        success_value(&registry.get(&writable.atom())),
        Some(1),
        "a failed transition leaves the last good value in place"
    );
}
