//! `Equal` for the values the trait can reach: primitives, arrays, nesting, and
//! the IEEE-754 special values. Rows that need a dynamic JS object, a cycle,
//! or a Proxy are not representable and are excused in the ledger.

use elm_effect::Equal;

#[test]
fn equal_primitives_are_structural() {
    // Equal.test.ts:378, :383, :388 — numbers, strings and booleans compare
    // by value, which is what `==` already means for these types.
    assert!(1.5f64.equal(&1.5));
    assert!(!1.5f64.equal(&2.5));
    assert!("hello".to_string().equal(&"hello".to_string()));
    assert!(!"hello".to_string().equal(&"world".to_string()));
    assert!(true.equal(&true));
    assert!(!true.equal(&false));
}

#[test]
fn equal_arrays_compare_length_then_elements() {
    // Equal.test.ts:349, :360, :364, :370 — identical arrays match, empty
    // arrays match, and a difference in length or in an element does not.
    assert!(vec![1, 2, 3].equal(&vec![1, 2, 3]));
    assert!(Vec::<i32>::new().equal(&Vec::<i32>::new()));
    assert!(!vec![1, 2, 3].equal(&vec![1, 2, 4]));
    assert!(!vec![1, 2, 3].equal(&vec![1, 2]));
}

#[test]
fn equal_nesting_is_deep() {
    // Equal.test.ts:493, :499 — nesting is compared all the way down, in both
    // container kinds.
    assert!(vec![vec![1, 2], vec![3]].equal(&vec![vec![1, 2], vec![3]]));
    assert!(
        Some(vec![Some(1), None]).equal(&Some(vec![Some(1), None])),
        "Option nesting is deep too"
    );
    // :505, :511 — a difference at any depth is a difference.
    assert!(!vec![vec![1, 2], vec![3]].equal(&vec![vec![1, 9], vec![3]]));
    assert!(!Some(vec![Some(1), None]).equal(&Some(vec![Some(2), None])));
}

#[test]
fn equal_treats_nan_as_equal_to_nan() {
    // Equal.test.ts:520, :530, :537, :544, :551 — the trait's one deviation
    // from IEEE: NaN equals NaN, so it can be a key or a member. Infinity and
    // -Infinity stay distinct from each other and from every finite value.
    let nan = f64::NAN;
    assert!(nan.equal(&f64::NAN));
    assert!(!nan.equal(&1.0));
    assert!(!nan.equal(&f64::INFINITY));
    assert!(f64::INFINITY.equal(&f64::INFINITY));
    assert!(f64::NEG_INFINITY.equal(&f64::NEG_INFINITY));
    assert!(!f64::INFINITY.equal(&f64::NEG_INFINITY));
    assert!(!f64::INFINITY.equal(&1.0));

    // :576, :583, :610, :617 — the special values behave the same nested.
    assert!(vec![f64::INFINITY, 1.0].equal(&vec![f64::INFINITY, 1.0]));
    assert!(Some(f64::NEG_INFINITY).equal(&Some(f64::NEG_INFINITY)));
    assert!(!vec![f64::INFINITY, 1.0].equal(&vec![f64::NEG_INFINITY, 1.0]));
    assert!(vec![nan, f64::INFINITY].equal(&vec![f64::NAN, f64::INFINITY]));
    assert!(!vec![nan, f64::NEG_INFINITY].equal(&vec![f64::NAN, f64::INFINITY]));
}

#[test]
fn equal_reaches_option_and_tuple() {
    // Equal.test.ts:434 — a value that implements the trait compares through
    // it, and Option and tuple carry it.
    assert!(Some(1).equal(&Some(1)));
    assert!(!Some(1).equal(&Some(2)));
    assert!(None::<i32>.equal(&None::<i32>));
    assert!(!Some(1).equal(&None));
    assert!((1, "a".to_string()).equal(&(1, "a".to_string())));
    assert!(!(1, "a".to_string()).equal(&(2, "a".to_string())));
}
