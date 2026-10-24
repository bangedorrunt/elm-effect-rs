//! `Atom.subscriptionRef`: an atom whose value is a `SubscriptionRef`'s
//! current value, with pushes drained by `Registry::tick`.

use elm_effect::SubscriptionRef;
use elm_effect_unstable::reactivity::{subscription_ref, Registry};
use std::sync::Arc;

#[test]
fn reads_see_the_ref_and_writes_set_it() {
    let watched = Arc::new(SubscriptionRef::new(0_i64));
    let writable = subscription_ref(Arc::clone(&watched));
    let registry = Registry::make();
    assert_eq!(registry.get(&writable.atom()), 0);

    writable.write(&registry, 9);
    assert_eq!(watched.get(), 9, "a write reaches the ref");
    assert_eq!(registry.get(&writable.atom()), 9, "a write also reaches the atom");
}

#[test]
fn a_value_set_on_the_ref_arrives_on_the_next_tick() {
    let watched = Arc::new(SubscriptionRef::new(0_i64));
    let writable = subscription_ref(Arc::clone(&watched));
    let registry = Registry::make();
    assert_eq!(registry.get(&writable.atom()), 0);

    watched.set(5);
    assert_eq!(
        registry.get(&writable.atom()),
        0,
        "no push path: the atom holds its last value until a tick"
    );

    registry.tick();
    assert_eq!(registry.get(&writable.atom()), 5);
}

#[test]
fn every_assignment_between_ticks_is_delivered_in_order() {
    let watched = Arc::new(SubscriptionRef::new(0_i64));
    let writable = subscription_ref(Arc::clone(&watched));
    let registry = Registry::make();
    assert_eq!(registry.get(&writable.atom()), 0);

    watched.set(1);
    watched.set(2);
    registry.tick();
    assert_eq!(registry.get(&writable.atom()), 2, "the pump drains the queue");
}
