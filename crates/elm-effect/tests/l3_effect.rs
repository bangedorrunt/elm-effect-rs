//! Law L1: building an [`Effect`](crate::Effect) runs nothing.
//!
//! Construction and `drop` perform no I/O, no sleep, no state change. Every
//! constructor and combinator is covered: if any of them ran work eagerly,
//! its counter would be non-zero after the drop.

use elm_effect::Effect;
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn counter() -> (Arc<AtomicUsize>, impl Fn() -> i32 + Send + 'static) {
    let n = Arc::new(AtomicUsize::new(0));
    let thunk = {
        let n = n.clone();
        move || {
            n.fetch_add(1, Ordering::SeqCst);
            1
        }
    };
    (n, thunk)
}

#[test]
fn description_drop_runs_nothing() {
    let (n, thunk) = counter();
    let effect: Effect<i32, String> = Effect::sync(thunk);
    drop(effect);
    assert_eq!(n.load(Ordering::SeqCst), 0);
}

#[test]
fn combinator_descriptions_drop_runs_nothing() {
    let (n1, t1) = counter();
    let (n2, t2) = counter();
    let base: Effect<i32, String> = Effect::sync(t1);
    let chained = base
        .flat_map(move |a| Effect::sync(t2).map(move |b| a + b))
        .map(|x| x * 2);
    drop(chained);
    assert_eq!(n1.load(Ordering::SeqCst), 0);
    assert_eq!(n2.load(Ordering::SeqCst), 0);

    // Narrowing combinators are lazy too.
    let (n3, t3) = counter();
    let narrowed: Effect<i32, String> = Effect::sync(t3);
    let _ = narrowed.result();
    let _ = Effect::<i32, String>::fail("e".to_string()).or_die();
    let _ = Effect::<i32, String>::succeed(1).exit();
    assert_eq!(n3.load(Ordering::SeqCst), 0);
}

#[test]
fn or_die_type_is_infallible() {
    let narrowed: Effect<i32, Infallible> = Effect::fail("e".to_string()).or_die();
    drop(narrowed);
}
