//! Writable atoms: the write half of `Atom.writable`, with the
//! `WriteContext` that `optimistic`, `swr` and the subscription atoms build
//! on.
//!
//! A write payload type can differ from the value type, which is why the
//! write is a registered behaviour by atom id rather than part of the node.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use elm_effect_unstable::reactivity::{Atom, Registry, writable};

#[test]
fn a_write_reaches_the_atom_through_set_self() {
    // Atom.writable's `setSelf`: the write decides what the atom reads.
    let counter = writable(0i64, |context, delta: i64| {
        context.set_self(delta * 2);
    });
    let registry = Registry::make();
    assert_eq!(registry.get(counter.atom()), 0);

    counter.write(&registry, 21);
    assert_eq!(registry.get(counter.atom()), 42, "the write set the value");
}

#[test]
fn a_write_can_read_and_write_another_atom() {
    // `WriteContext.get` / `.set`: a write sees the current value and can
    // update a sibling, which is what optimistic writes do.
    let source = Atom::make(10i64);
    let derived = Atom::make(0i64);
    let source_for_write = source.clone();
    let derived_for_write = derived.clone();
    let bump = writable(0i64, move |context, _: ()| {
        let current = context.get(&source_for_write);
        context.set(&derived_for_write, current + 1);
        context.set_self(current);
    });
    let registry = Registry::make();

    bump.write(&registry, ());
    assert_eq!(registry.get(bump.atom()), 10);
    assert_eq!(registry.get(&derived), 11);
}

#[test]
fn writes_inside_a_batch_notify_once() {
    // A write is a set, so batching applies to it unchanged.
    let counter = writable(0i64, |context, value: i64| {
        context.set_self(value);
    });
    let registry = Registry::make();
    let seen = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&seen);
    let _sub = registry.subscribe(counter.atom(), move |_| {
        count.fetch_add(1, Ordering::SeqCst);
    });

    registry.batch(|| {
        counter.write(&registry, 1);
        counter.write(&registry, 2);
    });
    assert_eq!(seen.load(Ordering::SeqCst), 1);
    assert_eq!(registry.get(counter.atom()), 2);
}
