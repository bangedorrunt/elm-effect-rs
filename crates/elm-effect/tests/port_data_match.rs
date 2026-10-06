//! Foundation behavior at the `effect@4.0.0` pin: data, matching, filters,
//! combiners, reducers, differs.

use elm_effect::{
    Combiner, Differ, Equal, Filter, Hash, Matcher, Predicate, Reducer, Tagged, ord_order, tagged,
};

#[path = "../src/data/data.rs"]
mod data;

#[test]
fn tagged_structs_match_by_tag() {
    let circle = tagged("circle", 1u32);
    let square = tagged("square", 2u32);
    assert!(circle.equal(&tagged("circle", 1u32)));
    assert!(!circle.equal(&square));
    assert!(!circle.equal(&tagged("circle", 2u32)));
    assert_eq!(circle.hash_value(), tagged("circle", 1u32).hash_value());

    let area = Matcher::value(circle)
        .discriminator(|c: &Tagged<u32>| c.tag, "circle", |c| c.value * 3)
        .discriminator(|c: &Tagged<u32>| c.tag, "square", |c| c.value * 4)
        .exhaustive();
    assert_eq!(area, 3);

    let missing = Matcher::value(square)
        .discriminator(|c: &Tagged<u32>| c.tag, "circle", |c| c.value)
        .option();
    assert_eq!(missing, None);
    assert_eq!(
        Matcher::value(tagged("circle", 1u32))
            .discriminator_starts_with(|c: &Tagged<u32>| c.tag.to_string(), "cir", |c| c.value)
            .exhaustive(),
        1
    );
    assert_eq!(
        Matcher::value(tagged("square", 2u32))
            .discriminator_starts_with(|c: &Tagged<u32>| c.tag.to_string(), "cir", |c| c.value)
            .option(),
        None
    );
    assert_eq!(
        Matcher::value(7i32)
            .when(Predicate::make(|n: &i32| *n > 10), |_| "big")
            .when_or(
                Predicate::make(|n: &i32| *n == 7),
                Predicate::make(|n: &i32| *n == 8),
                |_| "lucky"
            )
            .or_else("other"),
        "lucky"
    );
    assert_eq!(
        Matcher::value(1i32)
            .when_and(
                Predicate::make(|n: &i32| *n > 0),
                Predicate::make(|n: &i32| *n < 10),
                |_| "small positive"
            )
            .exhaustive(),
        "small positive"
    );
}

#[test]
fn filters_narrow() {
    let positive = Filter::<i32, i32, i32>::from_predicate(Predicate::make(|n: &i32| *n > 0));
    assert_eq!(positive.check(&1), Ok(1));
    assert_eq!(positive.check(&-1), Err(-1));
    assert!(positive.clone().to_predicate().test(&1));

    let small = Filter::<i32, i32, i32>::equals(2);
    let both = positive.and(small);
    assert_eq!(both.check(&2), Ok(2));
    assert_eq!(both.check(&4), Err(4));
    assert_eq!(both.check(&-2), Err(-2));

    let either = Filter::<i32, i32, i32>::equals(1).or(Filter::<i32, i32, i32>::equals(2));
    assert_eq!(either.check(&2), Ok(2));
    assert_eq!(either.check(&3), Err(3));

    let mapped = Filter::<i32, i32, i32>::equals(1).map_fail(|e| format!("no: {e}"));
    assert_eq!(mapped.check(&9), Err("no: 9".to_string()));
}

#[test]
fn combiners_fold() {
    let sum = Combiner::make(|a: &u32, b: &u32| a + b);
    assert_eq!(sum.combine(&1, &2), 3);
    assert_eq!(sum.flip().combine(&1, &2), 3);
    assert_eq!(Combiner::first().combine(&1u32, &2u32), 1);
    assert_eq!(Combiner::last().combine(&1u32, &2u32), 2);
    assert_eq!(Combiner::constant(9u32).combine(&1, &2), 9);
    let (sa, sb) = ("a".to_string(), "b".to_string());
    assert_eq!(
        Combiner::<String>::intercalate(", ").combine(&sa, &sb),
        "a, b"
    );
    assert_eq!(Combiner::min(ord_order()).combine(&1u32, &2u32), 1);
    assert_eq!(Combiner::max(ord_order()).combine(&1u32, &2u32), 2);

    let total = Reducer::make(0u32, |a: &u32, b: &u32| a + b);
    assert_eq!(total.empty(), 0);
    assert_eq!(total.fold(&[1, 2, 3]), 6);
    assert_eq!(total.flip().fold(&[1, 2, 3]), 6);
}

#[test]
fn differ_patches_round_trip() {
    let differ = Differ::<u32, Option<u32>>::replace();
    assert_eq!(differ.empty(), None);
    assert_eq!(differ.diff(&1, &1), None);
    assert_eq!(differ.diff(&1, &2), Some(2));
    assert_eq!(differ.combine(&None, &Some(3)), Some(3));
    assert_eq!(differ.patch(&1, &Some(2)), 2);
    assert_eq!(differ.patch(&1, &None), 1);

    let order_differ = Differ::make(
        0u32,
        |a: &u32, b: &u32| b.saturating_sub(*a),
        |a: &u32, b: &u32| a + b,
        |a: &u32, p: &u32| a + p,
    );
    let patch = order_differ.diff(&5u32, &7u32);
    assert_eq!(order_differ.patch(&5u32, &patch), 7);
}

#[test]
fn data_struct_pipe_and_fields() {
    use data::{Field, Struct};
    // Data.test.ts:29/79 - .pipe() applies and returns.
    let person = Struct::new([("name", Field::Str("C".to_owned())), ("age", Field::Int(1))]);
    let name = person.pipe(|s| match s.get("name") {
        Some(Field::Str(name)) => name.clone(),
        _ => String::new(),
    });
    assert_eq!(name, "C");
    // :35/:73 - fieldless structs construct.
    let empty = Struct::new(Vec::<(String, Field)>::new());
    assert!(empty.field_names().is_empty());
    // :41 - fields are enumerable.
    let ordered = Struct::new([("b", Field::Int(2)), ("a", Field::Int(1))]);
    let mut names = ordered.field_names();
    names.sort();
    assert_eq!(names, ["a", "b"]);
}

#[test]
fn tagged_errors_act_like_errors() {
    use data::{Field, TaggedError};
    // Data.test.ts:179/235 - tagged errors are std errors.
    let err = TaggedError::new("MyError", "boom", Vec::<(String, Field)>::new());
    let boxed: Box<dyn std::error::Error> = Box::new(err);
    assert_eq!(boxed.to_string(), "MyError: boom");
    // :190 - they carry a stack trace.
    let err = TaggedError::new("MyError", "boom", Vec::<(String, Field)>::new());
    assert!(!err.stack().is_empty());
    // :196/:250 - fieldless errors construct.
    let bare = TaggedError::new("Bare", "x", Vec::<(String, data::Field)>::new());
    assert_eq!(bare.tag(), "Bare");
    // :225/:257 - the tag is set and doubles as the name.
    assert_eq!(bare.name(), "Bare");
    // :262 - different tags are not equal.
    let other = TaggedError::new("Other", "x", Vec::<(String, Field)>::new());
    assert_ne!(bare, other);
    assert_eq!(
        bare,
        TaggedError::new("Bare", "x", Vec::<(String, Field)>::new())
    );
}
