//! `Atom.readable` on the write context's read half: a node whose value comes
//! from a function that reads other atoms.

use elm_effect_unstable::reactivity::{readable, Atom, Registry};

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
