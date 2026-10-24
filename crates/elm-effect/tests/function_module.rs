//! `Function.test.ts` rows: the surface the ledger called a convention gap.
//!
//! Where the Rust call shape differs from the pin's, the difference is the
//! calling convention, not the law. `pipe` and `flow` are macros because Rust
//! has no variadic function, and a `flow` head that takes several arguments
//! takes them as one tuple because a Rust closure has a fixed arity.
//!
//! The `dual` block stays excused: one constructor serving both call orders
//! needs runtime arity dispatch, which this typed API does not have.

use std::cell::Cell;
use std::sync::Arc;

use elm_effect::{
    apply, compose, const_false, const_null, const_true, const_unit, flip_curried, flow, hole,
    identity, memoize, memoize_idempotent, pipe, sk, tupled, untupled,
};

#[test]
fn function_apply() {
    // :81 - applies a value to a function.
    assert_eq!(apply("hello", |s: &str| s.len()), 5);
    // :86 - works with `pipe`: the applied function is one step in a chain.
    assert_eq!(pipe!(3, |n: i32| apply(n, |m: i32| m * 2)), 6);
}

#[test]
fn function_constants() {
    // :153/:159 - `constUndefined` and `constVoid` are one value in Rust.
    let nothing = const_unit();
    assert_eq!(nothing(), ());
    assert_eq!(nothing(), ());
}

#[test]
fn function_compose_with_pipe() {
    // :179 - `compose` is a data-last step: `pipe(2, compose(inc, double))`.
    let inc = |n: i32| n + 1;
    let double = |n: i32| n * 2;
    assert_eq!(pipe!(2, compose(inc, double)), 6);
}

#[test]
fn function_pipe() {
    // `compose`/`identity` are the chain's own steps; the rows under test are
    // the chain's order and typing, not the bodies.
    let inc = compose(identity::<i32>, |n: i32| n + 1);
    let double = |n: i32| n * 2;

    // :209 - a value with no functions is unchanged.
    assert_eq!(pipe!(5), 5);
    // :213 - one function.
    assert_eq!(pipe!(5, inc), 6);
    // :217 - two functions run left to right.
    assert_eq!(pipe!(5, inc, double), 12);
    // :221 - a long chain keeps the order.
    assert_eq!(
        pipe!(
            1,
            |n: i32| n + 1,
            |n: i32| n + 1,
            |n: i32| n + 1,
            |n: i32| n + 1,
            |n: i32| n + 1,
            |n: i32| n + 1,
            |n: i32| n + 1,
            |n: i32| n + 1,
            |n: i32| n + 1
        ),
        10
    );
    // :239 - the value's type changes from step to step.
    assert_eq!(pipe!(5, |n: i32| n.to_string(), |s: String| s.len()), 1);
}

#[test]
fn function_flow() {
    // :250 - `flow` of one function is that function.
    let f = compose(identity::<i32>, |n: i32| n + 1);
    let one = flow!(f);
    assert_eq!(one(1), 2);

    // :261 - three functions compose left to right.
    let three = flow!(|s: &str| s.len(), |n: usize| n * 2, |n: usize| n + 1);
    assert_eq!(three("ab"), 5);

    // :272 - up to nine functions.
    let nine = flow!(
        |n: i32| n + 1,
        |n: i32| n + 1,
        |n: i32| n + 1,
        |n: i32| n + 1,
        |n: i32| n + 1,
        |n: i32| n + 1,
        |n: i32| n + 1,
        |n: i32| n + 1,
        |n: i32| n + 1
    );
    assert_eq!(nine(0), 9);

    // :287 - the head takes the arguments, the tail takes its one result.
    let head = flow!(|(a, b): (i32, i32)| a + b, |n: i32| n * 2);
    assert_eq!(head((1, 2)), 6);
}

#[test]
fn function_hole() {
    // :297 - `hole` panics when it is reached, naming itself.
    let caught = std::panic::catch_unwind(hole::<u8>).expect_err("hole must panic");
    let message = caught
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| caught.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("");
    assert!(message.contains("hole"), "message was {message:?}");
}

#[test]
fn function_sk() {
    // :303 - SK returns its second argument.
    assert_eq!(sk(0, "hello"), "hello");
    assert_eq!(sk("a", 42), 42);
}

#[test]
fn function_memoize() {
    let calls = Cell::new(0);
    let f = memoize(|n: &u32| {
        calls.set(calls.get() + 1);
        n * 2
    });

    // :310 - the same reference answers from the cache.
    let key = Arc::new(5u32);
    assert_eq!(f(&key), 10);
    assert_eq!(f(&key), 10);
    assert_eq!(calls.get(), 1);

    // :322 - a different reference with the same value recomputes.
    let other = Arc::new(5u32);
    assert_eq!(f(&other), 10);
    assert_eq!(calls.get(), 2);
}

#[test]
fn function_memoize_idempotent() {
    // :335 - the output is cached as a fixed point.
    let calls = Cell::new(0);
    let input = Arc::new(String::from("input"));
    let output = Arc::new(String::from("output"));
    let f = memoize_idempotent(|s: &Arc<String>| {
        calls.set(calls.get() + 1);
        if s.as_str() == "input" {
            Arc::clone(&output)
        } else {
            Arc::clone(s)
        }
    });

    assert!(Arc::ptr_eq(&f(&input), &output));
    assert!(Arc::ptr_eq(&f(&output), &output));
    assert_eq!(calls.get(), 1);

    // :349 - an input that is already the fixed point is cached on the first
    // call, and the identity result is not stored twice.
    let calls = Cell::new(0);
    let fixed = Arc::new(7u32);
    let g = memoize_idempotent(|s: &Arc<u32>| {
        calls.set(calls.get() + 1);
        Arc::clone(s)
    });
    assert!(Arc::ptr_eq(&g(&fixed), &fixed));
    assert!(Arc::ptr_eq(&g(&fixed), &fixed));
    assert_eq!(calls.get(), 1);
}

#[test]
fn function_identity_rows() {
    // :93, :101, :108, :114, :120, :126 - identity returns its argument
    // itself: primitives by value, objects by the same reference.
    assert_eq!(identity(42), 42);
    assert_eq!(identity("text"), "text");
    assert!(identity(true));
    assert_eq!(identity(()), ());

    let shared = Arc::new(String::from("shared"));
    let returned = identity(Arc::clone(&shared));
    assert!(
        Arc::ptr_eq(&returned, &shared),
        "the same object comes back"
    );

    // The same call answers the same value every time; identity keeps no
    // state.
    assert_eq!(identity(7), identity(7));
    assert!(Arc::ptr_eq(&identity(Arc::clone(&shared)), &shared));
}

#[test]
fn function_constant_rows() {
    // :135, :141, :147 - the three constant thunks answer always true,
    // always false, and null (None in Rust).
    let yes = const_true();
    assert!(yes());
    assert!(yes());
    let no = const_false();
    assert!(!no());
    assert!(!no());
    let null = const_null::<String>();
    assert_eq!(null, None);
    assert_eq!(null, None);
}

#[test]
fn function_flip_curried_row() {
    // :165 - the curried flip still applies the original function's first
    // argument second.
    let subtract = |a: i32| move |b: i32| a - b;
    assert_eq!(subtract(10)(3), 7);
    let flipped = flip_curried(subtract);
    assert_eq!(flipped(3)(10), 7, "flipped(b)(a) is f(a)(b)");
    assert_eq!(flipped(3)(10), 7, "the captured argument replays");
}

#[test]
fn function_data_first_compose_runs_left_to_right() {
    // :175 - compose's input goes through the first function first.
    let inc = |n: i32| n + 1;
    let double = |n: i32| n * 2;
    assert_eq!(compose(inc, double)(3), 8);
    assert_eq!(compose(double, inc)(3), 7);
    assert_eq!(pipe!(3, inc, double), 8);
}

#[test]
fn function_tupled_and_untupled_rows() {
    // :193, :201 - the two adapters are inverses of each other.
    let add = |a: i32, b: i32| a + b;
    assert_eq!(tupled(add)((2, 3)), 5);
    assert_eq!(untupled(tupled(add))(2, 3), 5);

    let pair = |(a, b): (i32, i32)| a * b;
    assert_eq!(untupled(pair)(4, 5), 20);
    assert_eq!(tupled(untupled(pair))((4, 5)), 20);
}

#[test]
fn function_composes_two_functions() {
    // :255 - a two-function flow is compose with the same left-to-right order.
    let f = flow!(|n: i32| n + 1, |n: i32| n * 2);
    assert_eq!(f(3), 8);
    assert_eq!(flow!(|n: i32| n * 2, |n: i32| n + 1)(3), 7);
}
