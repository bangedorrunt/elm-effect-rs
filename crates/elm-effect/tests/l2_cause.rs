//! Law L2: the cause model owns the failure vocabulary at the crate edge.
//!
//! A cause is ordinary data: a vector of reasons. A defect is never a typed
//! error value, and an exit is data, not an effect.

use elm_effect::{Cause, Defect, Exit, FiberId, Reason};
use std::convert::Infallible;

#[test]
fn cause_reasons_classify() {
    let fail = Cause::<String>::fail("boom".to_string());
    let die = Cause::<String>::die(Defect::new("kaboom"));
    let interrupt = Cause::<String>::interrupt(None);
    let interrupt_known = Cause::<String>::interrupt(Some(FiberId(7)));

    assert!(matches!(fail.reasons.as_slice(), [Reason::Fail(_)]));
    assert!(matches!(die.reasons.as_slice(), [Reason::Die(_)]));
    assert!(matches!(
        interrupt.reasons.as_slice(),
        [Reason::Interrupt { fiber_id: None }]
    ));
    assert!(matches!(
        interrupt_known.reasons.as_slice(),
        [Reason::Interrupt { fiber_id: Some(_) }]
    ));
}

#[test]
fn cause_structural_eq() {
    let a = Cause::fail("x".to_string());
    let b = Cause::fail("x".to_string());
    assert_eq!(a, b);

    let c = Cause::fail("y".to_string());
    assert_ne!(a, c);

    // A Die with the same text is still not a Fail: the variants cannot collapse.
    // Struct literal (not `new`) keeps the backtrace deterministic.
    let d: Cause<String> = Cause::die(Defect {
        message: "x".to_string(),
        backtrace: String::new(),
    });
    assert_ne!(a, d);
    assert_eq!(
        d,
        Cause::die(Defect {
            message: "x".to_string(),
            backtrace: String::new(),
        })
    );
}

#[test]
fn cause_empty_has_no_reasons() {
    let empty: Cause<String> = Cause::empty();
    assert!(empty.reasons.is_empty());
    assert_eq!(empty.reasons, Vec::new());
    assert_eq!(empty, Cause::empty());

    // `never` is `Infallible`, not `()`: an empty cause inhabits any channel.
    let never: Cause<Infallible> = Cause::empty();
    assert!(never.reasons.is_empty());
}

#[test]
fn defect_is_not_an_error_value() {
    // A Die carries no typed error: typed handlers see nothing.
    let die = Cause::<String>::die(Defect::new("boom"));
    assert!(die.failure().is_none());
    assert_eq!(die.defects().len(), 1);
    assert_eq!(die.defects()[0].message, "boom");

    // A Fail carries a typed error and no defect.
    let fail = Cause::fail("e".to_string());
    assert_eq!(fail.failure(), Some(&"e".to_string()));
    assert!(fail.defects().is_empty());

    // Defect implements std Error (reportable) but never unifies with E.
    fn is_std_error<T: std::error::Error>(_: &T) {}
    is_std_error(&Defect::new("boom"));
}

#[test]
fn exit_is_data_not_effect() {
    // Clone + structural equality + matching: data semantics, no runtime.
    let x: Exit<i32, String> = Exit::succeed(1_i32);
    assert_eq!(x, x.clone());
    assert!(x.is_success());

    let f: Exit<i32, String> = Exit::fail("nope".to_string());
    assert!(!f.is_success());
    assert_eq!(f, f.clone());

    let d: Exit<i32, String> = Exit::die(Defect::new("kaboom"));
    assert!(!d.is_success());

    match f {
        Exit::Failure(c) => assert!(matches!(c.reasons.as_slice(), [Reason::Fail(_)])),
        Exit::Success(_) => panic!("expected failure"),
    }
}
