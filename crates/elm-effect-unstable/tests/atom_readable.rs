//! `Atom.readable` on the write context's read half: a node whose value comes
//! from a function that reads other atoms.

use elm_effect_unstable::reactivity::{readable, Atom, Registry};
use std::sync::{Arc, Mutex};

#[test]
fn readable_reads_several_source_atoms() {
    let left = Atom::make(2_i64);
    let right = Atom::make(3_i64);
    let sum = readable(move |context| context.get(&left) + context.get(&right));
    let registry = Registry::make();
    assert_eq!(registry.get(&sum), 5);
}

#[test]
fn readable_rebuilds_on_refresh_and_not_before() {
    // Ceiling, asserted so it cannot rot silently: a readable node tracks no
    // dependency edges, so a source change alone does NOT rebuild it. The
    // explicit refresh is what re-reads.
    let source = Atom::make(1_i64);
    let fed = source.clone();
    let doubled = readable(move |context| context.get(&fed) * 2);
    let registry = Registry::make();
    assert_eq!(registry.get(&doubled), 2);
    registry.set(&source, 10);
    assert_eq!(
        registry.get(&doubled),
        2,
        "no dependency edge means no automatic rebuild; this is the documented ceiling"
    );
    registry.refresh(&doubled);
    assert_eq!(registry.get(&doubled), 20);
}

#[test]
fn self_value_sees_the_previous_build() {
    let step = Atom::make(1_i64);
    let fed = step.clone();
    let counted = readable(move |context| context.self_value::<i64>().unwrap_or(0) + context.get(&fed));
    let registry = Registry::make();
    assert_eq!(registry.get(&counted), 1);
    registry.refresh(&counted);
    assert_eq!(registry.get(&counted), 2);
}

#[test]
fn a_readable_subscription_lives_until_its_finalizer_cancels_it() {
    // The pin wires a read's subscriptions to the node's finalizers, so the
    // subscription outlives the read but not the node. Two reads must not
    // double-subscribe either, which is why this wires once.
    let source = Atom::make(1_i64);
    let seen: Arc<Mutex<Vec<i64>>> = Arc::new(Mutex::new(Vec::new()));
    let wired = Arc::new(Mutex::new(false));
    let fed = source.clone();
    let sink = Arc::clone(&seen);
    let guard = Arc::clone(&wired);
    let derived = readable(move |context| {
        let mut wired = guard.lock().unwrap();
        if !*wired {
            *wired = true;
            let sink = Arc::clone(&sink);
            let subscription = context.subscribe(&fed, move |value| {
                sink.lock().unwrap().push(value)
            });
            context.add_finalizer(move || subscription.cancel());
        }
        drop(wired);
        context.get(&fed)
    })
    .auto_dispose();
    let registry = Registry::make();
    assert_eq!(registry.get(&derived), 1);
    registry.set(&source, 2);
    assert_eq!(*seen.lock().unwrap(), vec![2]);
    // No listeners of its own, so the disposal pass releases the node and
    // runs its finalizer.
    registry.tick();
    registry.set(&source, 3);
    assert_eq!(
        *seen.lock().unwrap(),
        vec![2],
        "the finalizer must cancel the subscription when the node is released"
    );
}
