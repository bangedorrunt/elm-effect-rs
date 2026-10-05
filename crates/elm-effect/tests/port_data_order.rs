//! hi5 tail slice: Order struct/tuple/date + Ordering reducer cells.
//! Ports packages/effect/test/Order.test.ts and Ordering.test.ts rows
//! left uncovered by the area beads.

use std::cmp::Ordering;

#[path = "../src/cron.rs"]
mod cron;

use cron::DateTime;
use elm_effect::{
    Order, Reducer, combine_ordering, number_order, ord_order,
};

// Ordering.test.ts:6 — Ordering folds as a reducer: first decided wins.
#[test]
fn ordering_reducer_folds_verdicts() {
    let verdicts = Reducer::make(Ordering::Equal, |a: &Ordering, b: &Ordering| {
        combine_ordering(*a, *b)
    });
    assert_eq!(verdicts.fold(&[Ordering::Less, Ordering::Equal]), Ordering::Less);
    assert_eq!(
        verdicts.fold(&[Ordering::Equal, Ordering::Greater]),
        Ordering::Greater
    );
    assert_eq!(
        verdicts.fold(&[Ordering::Less, Ordering::Greater]),
        Ordering::Less,
        "an early verdict survives later ones"
    );
    assert_eq!(
        verdicts.fold(&[Ordering::Equal, Ordering::Equal]),
        Ordering::Equal
    );
}

struct Person {
    name: String,
    age: i32,
}

fn person_order() -> Order<Person> {
    // NOTE: `string_order` reports ties as Greater per its pin, which would
    // stop fallthrough; struct fields use `ord_order` so ties stay Equal.
    Order::combine_all(vec![
        ord_order::<String>().map_input(|person: &Person| person.name.clone()),
        ord_order::<i32>().map_input(|person: &Person| person.age),
    ])
}

// Order.test.ts:6 — Struct compares field by field, first difference wins.
#[test]
fn order_struct_compares_fieldwise() {
    let order = person_order();
    let ann = Person { name: "ann".to_string(), age: 30 };
    let bob = Person { name: "bob".to_string(), age: 20 };
    let ann_older = Person { name: "ann".to_string(), age: 40 };
    assert_eq!(order.compare(&ann, &bob), Ordering::Less);
    assert_eq!(order.compare(&bob, &ann), Ordering::Greater);
    assert_eq!(
        order.compare(&ann, &ann_older),
        Ordering::Less,
        "tied name falls through to age"
    );
    assert_eq!(
        order.compare(&ann, &Person { name: "ann".to_string(), age: 30 }),
        Ordering::Equal
    );
}

// Order.test.ts:13 — Tuple compares element by element.
#[test]
fn order_tuple_compares_pairwise() {
    let order = Order::combine_all(vec![
        ord_order::<String>().map_input(|pair: &(String, i32)| pair.0.clone()),
        ord_order::<i32>().map_input(|pair: &(String, i32)| pair.1),
    ]);
    assert_eq!(
        order.compare(&("a".to_string(), 9), &("b".to_string(), 1)),
        Ordering::Less
    );
    assert_eq!(
        order.compare(&("a".to_string(), 9), &("a".to_string(), 10)),
        Ordering::Less,
        "tied first element falls through to second"
    );
    assert_eq!(
        order.compare(&("a".to_string(), 9), &("a".to_string(), 9)),
        Ordering::Equal
    );
}

// Order.test.ts:40 — Date compares by timestamp.
#[test]
fn order_date_compares_by_timestamp() {
    let order = ord_order::<DateTime>();
    let earlier = DateTime::from_epoch_millis(1_700_000_000_000);
    let later = DateTime::from_epoch_millis(1_700_000_001_000);
    assert_eq!(order.compare(&earlier, &later), Ordering::Less);
    assert_eq!(order.compare(&later, &earlier), Ordering::Greater);
    assert_eq!(
        order.compare(&earlier, &DateTime::from_epoch_millis(1_700_000_000_000)),
        Ordering::Equal
    );
    // number_order is the same shape over raw millis the Date wraps.
    assert_eq!(
        number_order().compare(&1_700_000_000_000.0, &1_700_000_001_000.0),
        Ordering::Less
    );
}
