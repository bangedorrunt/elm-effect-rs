//! Foundation behavior at the `effect@4.0.0` pin.

use std::cmp::Ordering;

use elm_effect::{
    Equal, Equivalence, Hash, Order, Predicate, array_equivalence, boolean_equivalence,
    boolean_order, combine_hash, combine_ordering, compose, const_false, const_null, const_true,
    constant, flip, flip_curried, hash_f64, hash_str, identity, match_ordering, number_equivalence,
    number_order, ord_order, reverse_ordering, string_equivalence, string_order, tuple_equivalence,
    tupled, untupled,
};

#[path = "../src/data/bigint.rs"]
mod bigint;

#[path = "../src/cron.rs"]
mod cron;

#[test]
#[allow(invalid_nan_comparisons, clippy::assertions_on_constants)]
fn equal_nan_is_nan() {
    // The deliberate divergence: Equal treats NaN as equal; f64's == stays IEEE.
    assert!(f64::NAN.equal(&f64::NAN));
    assert!(f32::NAN.equal(&f32::NAN));
    assert!(f64::NAN != f64::NAN);
    assert!(!1.0f64.equal(&f64::NAN));
    assert!(!f64::NAN.equal(&f64::INFINITY));
    assert!(1.0f64.equal(&1.0));
    assert!(vec![f64::NAN].equal(&vec![f64::NAN]));
    assert!(Some(f64::NAN).equal(&Some(f64::NAN)));
    assert!(!None::<f64>.equal(&Some(1.0)));
}

#[test]
fn equal_primitives_are_structural() {
    assert!("a".to_string().equal(&"a".to_string()));
    assert!(!"a".to_string().equal(&"b".to_string()));
    assert!(true.equal(&true));
    assert!(!true.equal(&false));
    assert!(Some(1u32).equal(&Some(1u32)));
    assert!((1u32, "x".to_string()).equal(&(1u32, "x".to_string())));
    assert!(!(1u32, "x".to_string()).equal(&(1u32, "y".to_string())));
}

#[test]
fn hash_nan_matches_and_combines() {
    assert_eq!(f64::NAN.hash_value(), f64::NAN.hash_value());
    assert_eq!(hash_f64(f64::NAN), hash_f64(f64::NAN));
    assert_eq!(1u32.hash_value(), 1u32.hash_value());
    assert_eq!(hash_str("abc"), hash_str("abc"));
    assert_ne!(hash_str("abc"), hash_str("abd"));
    assert_eq!(true.hash_value(), true.hash_value());
    assert_ne!(true.hash_value(), false.hash_value());
    let pair = (1u32, "x".to_string());
    assert_eq!(
        pair.hash_value(),
        combine_hash(1u32.hash_value(), "x".to_string().hash_value())
    );
    assert_eq!(hash_f64(0.0), hash_f64(-0.0));
    assert_eq!(vec![1u32, 2].hash_value(), vec![1u32, 2].hash_value());
    assert_ne!(vec![1u32, 2].hash_value(), vec![2u32, 1].hash_value());
    assert_ne!(vec![1u32].hash_value(), vec![1u32, 2].hash_value());
    assert_ne!(true.hash_value(), hash_str("true"));
    assert_ne!(None::<u32>.hash_value(), hash_str("None"));
}

#[test]
fn order_number_nan_rules() {
    let order: Order<f64> = number_order();
    assert_eq!(order.compare(&f64::NAN, &f64::NAN), Ordering::Equal);
    assert_eq!(order.compare(&f64::NAN, &1.0), Ordering::Less);
    assert_eq!(order.compare(&1.0, &f64::NAN), Ordering::Greater);
    assert_eq!(order.compare(&1.0, &2.0), Ordering::Less);
    assert_eq!(order.compare(&2.0, &1.0), Ordering::Greater);
    // Pin behavior: ties report Greater.
    assert_eq!(order.compare(&1.0, &1.0), Ordering::Greater);
    assert_eq!(order.min(1.0, 1.0), 1.0);
    assert_eq!(order.clamp(5.0, 1.0, 3.0), 3.0);
    assert!(order.is_less_than(&1.0, &2.0));
    assert!(order.is_greater_than(&2.0, &1.0));
    assert!(order.is_less_than_or_equal(&1.0, &2.0));
    assert!(!order.is_less_than_or_equal(&2.0, &1.0));
    assert!(order.is_greater_than_or_equal(&2.0, &1.0));
    assert!(!order.is_greater_than_or_equal(&1.0, &2.0));
    assert!(order.is_between(&2.0, &1.0, &3.0));
    assert!(!order.is_between(&4.0, &1.0, &3.0));
    assert_eq!(order.max(1.0, 2.0), 2.0);
    let both = Order::combine_all(vec![number_order(), number_order()]);
    assert_eq!(both.compare(&1.0, &2.0), Ordering::Less);
}

#[test]
fn order_composes_lexicographically() {
    let by_first = string_order().map_input(|pair: &(String, u32)| pair.0.clone());
    let by_second = ord_order::<u32>().map_input(|pair: &(String, u32)| pair.1);
    let both = by_first.combine(by_second);
    // Distinct first members decide on the first order alone.
    assert_eq!(
        both.compare(&("a".to_string(), 9u32), &("b".to_string(), 1u32)),
        Ordering::Less
    );
    // Tied first members report Greater per the pin, which decides.
    assert_eq!(
        both.compare(&("x".to_string(), 1u32), &("x".to_string(), 2u32)),
        Ordering::Greater
    );
}

#[test]
fn equivalence_and_predicate_basics() {
    let eq: Equivalence<f64> = number_equivalence();
    assert!(eq.is_equivalent(&f64::NAN, &f64::NAN));
    assert!(!eq.is_equivalent(&1.0, &2.0));
    assert!(string_equivalence().is_equivalent(&"a".to_string(), &"a".to_string()));
    assert!(boolean_equivalence().is_equivalent(&true, &true));
    let both = number_equivalence().combine(number_equivalence());
    assert!(both.is_equivalent(&1.0, &1.0));
    let all = Equivalence::combine_all(vec![number_equivalence(), number_equivalence()]);
    assert!(all.is_equivalent(&1.0, &1.0));
    assert!(!all.is_equivalent(&1.0, &2.0));
    let by_len = string_equivalence().map_input(|v: &Vec<u32>| v.len().to_string());
    assert!(by_len.is_equivalent(&vec![1u32], &vec![2u32]));
    assert!(
        tuple_equivalence::<u32, String>()
            .is_equivalent(&(1u32, "a".to_string()), &(1u32, "a".to_string()))
    );
    assert!(array_equivalence::<u32>().is_equivalent(&vec![1u32], &vec![1u32]));
    assert!(!array_equivalence::<u32>().is_equivalent(&vec![1u32], &vec![1u32, 2u32]));

    let positive = Predicate::make(|n: &i32| *n > 0);
    let even = Predicate::make(|n: &i32| n % 2 == 0);
    assert!(positive.clone().and(even.clone()).test(&4));
    assert!(!positive.clone().and(even.clone()).test(&3));
    assert!(positive.clone().or(even.clone()).test(&3));
    assert!(positive.not().test(&-1));
    assert!(Predicate::<i32>::always().test(&0));
    assert!(!Predicate::<i32>::never().test(&0));
    assert!(Predicate::is_some().test(&Some(1)));
    assert!(Predicate::<Option<i32>>::is_none().test(&None));
    let projected = even.clone().map_input(|pair: &(i32, i32)| pair.0);
    assert!(projected.test(&(4, 1)));
}

#[test]
fn bigint_equivalence() {
    // Equivalence.test.ts:27 — BigInt equivalence compares values.
    let eq = Equivalence::make(|a: &bigint::Big, b: &bigint::Big| a == b);
    assert!(eq.is_equivalent(&bigint::Big::from_int(1), &bigint::Big::from_int(1)));
    assert!(!eq.is_equivalent(&bigint::Big::from_int(1), &bigint::Big::from_int(2)));
}

#[test]
fn struct_string_keys_equivalence() {
    // Equivalence.test.ts:85 — per-field struct equivalence.
    let eq = tuple_equivalence::<String, i32>();
    assert!(eq.is_equivalent(&("a".to_string(), 1), &("a".to_string(), 1)));
    assert!(!eq.is_equivalent(&("a".to_string(), 1), &("b".to_string(), 1)));
    assert!(!eq.is_equivalent(&("a".to_string(), 1), &("a".to_string(), 2)));
}

#[test]
fn date_equivalence_by_instant() {
    // Equivalence.test.ts:154 — dates compare by instant.
    let eq = Equivalence::make(|a: &cron::DateTime, b: &cron::DateTime| {
        a.epoch_millis() == b.epoch_millis()
    });
    let d = |s: &str| cron::DateTime::make_unsafe(s);
    assert!(eq.is_equivalent(
        &d("2020-01-01T00:00:00.000Z"),
        &d("2020-01-01T00:00:00.000Z")
    ));
    assert!(!eq.is_equivalent(
        &d("2020-01-01T00:00:00.000Z"),
        &d("2021-01-01T00:00:00.000Z")
    ));
}

#[test]
fn record_string_keys_equivalence() {
    // Equivalence.test.ts:123 — record equality needs the same keys
    // with equivalent values on both sides.
    use std::collections::BTreeMap;
    let eq = Equivalence::make(|a: &BTreeMap<String, i32>, b: &BTreeMap<String, i32>| a == b);
    let full = || BTreeMap::from([("a".to_string(), 1), ("b".to_string(), 2)]);
    assert!(eq.is_equivalent(&full(), &full()));
    let mut changed = full();
    changed.insert("b".to_string(), 3);
    assert!(!eq.is_equivalent(&full(), &changed));
    let mut missing = full();
    missing.remove("b");
    assert!(!eq.is_equivalent(&full(), &missing));
    assert!(!eq.is_equivalent(&missing, &full()));
}

#[test]
fn function_and_ordering_basics() {
    assert_eq!(identity(3), 3);
    assert_eq!(constant(7)(), 7);
    assert!(const_true()());
    assert!(!const_false()());
    assert_eq!(flip(|a: u32, b: u32| a - b)(1, 3), 2);
    assert_eq!(compose(|a: u32| a + 1, |b: u32| b * 2)(3), 8);

    assert_eq!(reverse_ordering(Ordering::Less), Ordering::Greater);
    assert_eq!(
        combine_ordering(Ordering::Equal, Ordering::Less),
        Ordering::Less
    );
    assert_eq!(
        match_ordering(Ordering::Greater, || "lt", || "eq", || "gt"),
        "gt"
    );
    assert_eq!(
        string_order().compare(&"a".to_string(), &"b".to_string()),
        Ordering::Less
    );
    assert_eq!(boolean_order().compare(&false, &true), Ordering::Less);
    assert_eq!(
        boolean_order().reverse().compare(&false, &true),
        Ordering::Greater
    );
}

#[test]
fn predicate_and_stops_at_first_failure() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let probe = calls.clone();
    let failing = Predicate::make(|_: &u32| false);
    let counting = Predicate::make(move |_: &u32| {
        probe.fetch_add(1, Ordering::SeqCst);
        true
    });
    assert!(!failing.and(counting).test(&1));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let probe2 = calls.clone();
    let counting2 = Predicate::make(move |_: &u32| {
        probe2.fetch_add(1, Ordering::SeqCst);
        true
    });
    assert!(counting2.and(Predicate::make(|_: &u32| true)).test(&1));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn predicate_or_needs_any_and_not_flips() {
    assert!(
        Predicate::make(|_: &u32| false)
            .or(Predicate::make(|_: &u32| true))
            .test(&0)
    );
    assert!(
        !Predicate::make(|_: &u32| false)
            .or(Predicate::make(|_: &u32| false))
            .test(&0)
    );
    assert!(!Predicate::make(|_: &u32| true).not().test(&0));
    assert!(Predicate::make(|_: &u32| false).not().test(&0));
}

#[test]
fn predicate_xor_needs_exactly_one() {
    assert!(
        Predicate::make(|_: &u32| true)
            .xor(Predicate::make(|_: &u32| false))
            .test(&0)
    );
    assert!(
        !Predicate::make(|_: &u32| true)
            .xor(Predicate::make(|_: &u32| true))
            .test(&0)
    );
    assert!(
        !Predicate::make(|_: &u32| false)
            .xor(Predicate::make(|_: &u32| false))
            .test(&0)
    );
}

#[test]
fn function_combinators() {
    // Function.test.ts:93/101 - identity returns values and references.
    assert_eq!(identity(3), 3);
    let obj = vec![1];
    assert!(std::ptr::eq(identity(&obj), &obj));
    // :108/:114 - satisfies/cast are identity at runtime.
    assert_eq!(identity(5), 5);
    assert_eq!(identity(42), 42);
    // :120 - constant returns the value every time.
    let c = constant(42);
    assert_eq!(c(), 42);
    assert_eq!(c(), 42);
    // :135/:141/:147 - nullary constants.
    assert!(const_true()());
    assert!(!const_false()());
    assert_eq!(const_null::<i32>(), None);
    // :165 - flip reverses curried arguments.
    fn subtract_len(a: i64) -> Box<dyn Fn(&str) -> i64> {
        Box::new(move |s: &str| a - s.len() as i64)
    }
    assert_eq!(flip_curried(subtract_len)("aaa")(2), -1);
    // :175 - compose runs left to right.
    let increment = |n: i32| n + 1;
    let double = |n: i32| n * 2;
    assert_eq!(compose(increment, double)(2), 6);
    // :193/:201 - tupled and untupled convert both ways.
    fn add(a: i32, b: i32) -> i32 {
        a + b
    }
    assert_eq!(tupled(add)((1, 2)), 3);
    assert_eq!(untupled(|pair: (i32, i32)| pair.0 + pair.1)(1, 2), 3);
}

// ---------------------------------------------------------------------------
// Predicate guards over the dynamic Value domain
// (Predicate.test.ts :19-:549)
// ---------------------------------------------------------------------------

use elm_effect::predicate::{
    Key, Value, has_property, is_tagged, is_tuple_of, is_tuple_of_at_least,
};

/// An object value from pairs, so the guard tests never name a record type:
/// the pin's rows construct plain objects.
fn obj(pairs: Vec<(&str, Value)>) -> Value {
    Value::Object(
        pairs
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect(),
    )
}

#[test]
fn compose_narrows_through_both_checks() {
    // :19 — compose(isString, isNonEmptyString): a value passes only when it
    // is a string AND non-empty; null and "" fail.
    let composed =
        Predicate::<Value>::make(|v: &Value| matches!(v, Value::String(s) if !s.is_empty()));
    assert!(composed.test(&Value::String("a".into())));
    assert!(!composed.test(&Value::Null));
    assert!(!composed.test(&Value::String(String::new())));
}

#[test]
fn compose_with_a_plain_predicate_narrows_then_tests() {
    // :27 — compose(isString, isLong): the second check runs on the narrowed
    // value, so a non-string never reaches it.
    fn is_long(s: &str) -> bool {
        s.len() > 2
    }
    let composed =
        Predicate::<Value>::make(|v: &Value| matches!(v, Value::String(s) if is_long(s)));
    assert!(composed.test(&Value::String("abcd".into())));
    assert!(!composed.test(&Value::String("a".into())));
    assert!(!composed.test(&Value::Number(1.0)));
}

#[test]
fn tuple_of_predicates_requires_every_element() {
    // :61 — Predicate.Tuple([isPositive, isNegative]) over a pair.
    let p = Predicate::<i64>::tuple(vec![
        Predicate::make(|n: &i64| *n > 0),
        Predicate::make(|n: &i64| *n < 0),
    ]);
    assert!(p.test(&vec![1, -1]));
    assert!(!p.test(&vec![1, 1]));
    assert!(!p.test(&vec![-1, -1]));
    assert!(!p.test(&vec![-1, 1]));
}

#[test]
fn struct_of_predicates_ignores_extra_keys_and_stops_early() {
    // :89/:98 — every field must pass; extra keys are ignored; the first
    // failing field stops the walk.
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted_calls = std::sync::Arc::clone(&calls);
    let counted = Predicate::<i64>::make(move |n: &i64| {
        counted_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        *n > 0
    });
    let p = Predicate::<i64>::struct_(vec![
        ("a".to_string(), counted),
        ("b".to_string(), Predicate::make(|n: &i64| *n < 0)),
    ]);
    let mut record: std::collections::BTreeMap<String, i64> = Default::default();
    record.insert("a".to_string(), 1);
    record.insert("b".to_string(), -1);
    assert!(p.test(&record));
    let mut record: std::collections::BTreeMap<String, i64> = Default::default();
    record.insert("a".to_string(), -1);
    record.insert("extra".to_string(), 7);
    assert!(!p.test(&record));
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the failing field stops the walk"
    );
}

#[test]
fn is_truthy_uses_javascript_truthiness() {
    assert!(elm_effect::predicate::is_truthy(&Value::Bool(true)));
    assert!(!elm_effect::predicate::is_truthy(&Value::Bool(false)));
    assert!(elm_effect::predicate::is_truthy(&Value::String("a".into())));
    assert!(!elm_effect::predicate::is_truthy(&Value::String(
        String::new()
    )));
    assert!(elm_effect::predicate::is_truthy(&Value::Number(1.0)));
    assert!(!elm_effect::predicate::is_truthy(&Value::Number(0.0)));
    assert!(elm_effect::predicate::is_truthy(&Value::BigInt(1)));
    assert!(!elm_effect::predicate::is_truthy(&Value::BigInt(0)));
}

#[test]
fn guards_detect_runtime_shapes() {
    use elm_effect::predicate::*;
    // :237/:247 — Set and Map instances, and only those.
    assert!(is_set(&Value::Set(vec![Value::Number(1.0)])));
    assert!(is_set(&Value::Set(Vec::new())));
    assert!(!is_set(&obj(Vec::new())));
    assert!(!is_set(&Value::Null));
    assert!(!is_set(&Value::Undefined));
    assert!(is_map(&Value::Map(Vec::new())));
    assert!(!is_map(&obj(Vec::new())));
    assert!(!is_map(&Value::Null));
    assert!(!is_map(&Value::Undefined));

    // :256/:264/:272/:280/:287 — the primitive tags, with NaN still a number
    // and the boxed String not a string.
    assert!(is_string(&Value::String("a".into())));
    assert!(!is_string(&Value::Number(1.0)));
    assert!(is_number(&Value::Number(2.0)));
    assert!(is_number(&Value::Number(f64::NAN)));
    assert!(!is_number(&Value::String("2".into())));
    assert!(is_boolean(&Value::Bool(true)));
    assert!(is_boolean(&Value::Bool(false)));
    assert!(!is_boolean(&Value::String("true".into())));
    assert!(is_bigint(&Value::BigInt(1)));
    assert!(!is_bigint(&Value::Number(1.0)));
    assert!(is_symbol(&Value::Symbol("a".into())));

    // :294 — property keys are strings, numbers and symbols.
    assert!(is_property_key(&Value::String("a".into())));
    assert!(is_property_key(&Value::Number(1.0)));
    assert!(is_property_key(&Value::Symbol("a".into())));
    assert!(!is_property_key(&Value::Bool(true)));

    // :304/:311/:319/:327/:335 — function, undefined and null.
    assert!(is_function(&Value::Function));
    assert!(!is_function(&obj(Vec::new())));
    assert!(is_undefined(&Value::Undefined));
    assert!(!is_undefined(&Value::Null));
    assert!(!is_not_undefined(&Value::Undefined));
    assert!(is_null(&Value::Null));
    assert!(!is_null(&Value::Undefined));
    assert!(!is_not_null(&Value::Null));

    // :343/:352/:361/:370 — nullish, never, unknown.
    assert!(is_nullish(&Value::Null) && is_nullish(&Value::Undefined));
    assert!(!is_nullish(&obj(Vec::new())));
    assert!(!is_nullish(&Value::Array(Vec::new())));
    assert!(is_not_nullish(&obj(Vec::new())));
    assert!(!is_not_nullish(&Value::Null) && !is_not_nullish(&Value::Undefined));
    assert!(!is_never(&Value::Null) && !is_never(&obj(Vec::new())));
    assert!(is_unknown(&Value::Null) && is_unknown(&Value::Undefined));

    // :379/:388/:400/:411 — the object family.
    assert!(is_object_or_array(&obj(Vec::new())));
    assert!(is_object_or_array(&Value::Array(Vec::new())));
    assert!(!is_object_or_array(&Value::Null));
    assert!(!is_object_or_array(&Value::Function));
    assert!(is_object(&obj(Vec::new())));
    assert!(!is_object(&Value::Array(Vec::new())));
    assert!(!is_object(&Value::Null) && !is_object(&Value::Undefined));
    assert!(!is_object(&Value::Function));
    assert!(is_readonly_object(&obj(Vec::new())));
    assert!(!is_readonly_object(&Value::Array(Vec::new())));
    assert!(is_object_keyword(&obj(Vec::new())));
    assert!(is_object_keyword(&Value::Array(Vec::new())));
    assert!(is_object_keyword(&Value::Function));
    assert!(!is_object_keyword(&Value::Null) && !is_object_keyword(&Value::Undefined));
    assert!(!is_object_keyword(&Value::Symbol("a".into())));

    // :463/:471/:479/:520 — the instance guards.
    assert!(is_error(&Value::Error("boom".into())));
    assert!(!is_error(&Value::Null) && !is_error(&obj(Vec::new())));
    assert!(is_uint8array(&Value::Uint8Array(Vec::new())));
    assert!(!is_uint8array(&Value::Null) && !is_uint8array(&obj(Vec::new())));
    assert!(is_date(&Value::Date(0)));
    assert!(!is_date(&Value::Null) && !is_date(&obj(Vec::new())));
    assert!(is_regexp(&Value::RegExp("a".into())));
    assert!(!is_regexp(&Value::Null) && !is_regexp(&Value::String("a".into())));

    // :487 — iterables are arrays, strings, sets and maps; nothing else.
    assert!(is_iterable(&Value::Array(Vec::new())));
    assert!(is_iterable(&Value::String(String::new())));
    assert!(is_iterable(&Value::Set(Vec::new())));
    assert!(is_iterable(&Value::Map(Vec::new())));
    assert!(!is_iterable(&Value::Null));
    assert!(!is_iterable(&obj(Vec::new())));

    // :499/:510 — promise-like needs then; a promise needs then and catch.
    let thenable = {
        let mut o = Record::new();
        o.insert("then".to_string(), Value::Function);
        Value::Object(o)
    };
    let then_catch = {
        let mut o = Record::new();
        o.insert("then".to_string(), Value::Function);
        o.insert("catch".to_string(), Value::Function);
        Value::Object(o)
    };
    assert!(is_promise(&then_catch));
    assert!(!is_promise(&thenable));
    assert!(!is_promise(&obj(Vec::new())));
    assert!(is_promise_like(&thenable));
    assert!(is_promise_like(&Value::Promise));
    assert!(!is_promise_like(&obj(Vec::new())));
    assert!(!is_promise_like(&Value::Null));
}

#[test]
fn has_property_answers_string_and_symbol_keys() {
    // :426/:435 — a present key answers true in both forms (direct and
    // curried); a missing key, a null/undefined/function value, and an
    // absent symbol key answer false. The port models a symbol as its
    // registry key, so a symbol-keyed property is a record entry under it.
    let object = obj(vec![("a", Value::Number(1.0)), ("k", Value::Number(2.0))]);
    assert!(has_property(&object, &Key::String("a".into())));
    assert!(elm_effect::predicate::has_property_with(Key::String("a".into())).test(&object));
    assert!(has_property(&object, &Key::Symbol("k".into())));
    assert!(elm_effect::predicate::has_property_with(Key::Symbol("k".into())).test(&object));
    assert!(!has_property(&obj(Vec::new()), &Key::String("a".into())));
    assert!(!has_property(&Value::Null, &Key::String("a".into())));
    assert!(!has_property(&Value::Undefined, &Key::String("a".into())));
    assert!(!has_property(&Value::Function, &Key::String("a".into())));
    assert!(!has_property(&obj(Vec::new()), &Key::Symbol("k".into())));
}

#[test]
fn is_tagged_matches_the_tag_field() {
    // :451 — only an object whose _tag equals the tag matches; both forms.
    let tagged = obj(vec![("_tag", Value::String("a".into()))]);
    assert!(!is_tagged(&Value::Number(1.0), "a"));
    assert!(!is_tagged(&obj(Vec::new()), "a"));
    assert!(!is_tagged(&tagged, "b"));
    assert!(is_tagged(&tagged, "a"));
    assert!(elm_effect::predicate::is_tagged_with("a").test(&tagged));
}

#[test]
fn is_tuple_of_checks_exact_and_minimum_length() {
    // :528/:534 — exact length, both forms.
    let three = Value::Array(vec![
        Value::Number(1.0),
        Value::Number(2.0),
        Value::Number(3.0),
    ]);
    assert!(is_tuple_of(&three, 3));
    assert!(!is_tuple_of(&three, 4));
    assert!(!is_tuple_of(&three, 2));
    assert!(
        elm_effect::predicate::is_tuple_of_with(2).test(&Value::Array(vec![
            Value::String("a".into()),
            Value::String("b".into())
        ]))
    );
    assert!(
        !elm_effect::predicate::is_tuple_of_with(2)
            .test(&Value::Array(vec![Value::String("a".into())]))
    );

    // :543/:549 — minimum length, both forms.
    assert!(is_tuple_of_at_least(&three, 3));
    assert!(is_tuple_of_at_least(&three, 2));
    assert!(!is_tuple_of_at_least(&three, 4));
    assert!(
        elm_effect::predicate::is_tuple_of_at_least_with(2).test(&Value::Array(vec![
            Value::String("a".into()),
            Value::String("b".into()),
            Value::String("c".into())
        ]))
    );
    assert!(
        !elm_effect::predicate::is_tuple_of_at_least_with(2)
            .test(&Value::Array(vec![Value::String("a".into())]))
    );
}
