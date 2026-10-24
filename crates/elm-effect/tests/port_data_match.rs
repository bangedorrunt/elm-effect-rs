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

/// `Data.test.ts:240` — a tagged error can be caught by its tag.
#[derive(Clone, Debug, PartialEq)]
struct NotFound {
    resource: String,
}

impl std::fmt::Display for NotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NotFound: {}", self.resource)
    }
}

impl std::error::Error for NotFound {}

impl elm_effect::TaggedError for NotFound {
    fn tag(&self) -> &'static str {
        "NotFound"
    }
}

#[test]
fn tagged_error_is_caught_by_tag() {
    let missing = NotFound {
        resource: "/test".to_string(),
    };
    let caught: elm_effect::Effect<String, NotFound> = elm_effect::Effect::fail(missing)
        .catch_tag("NotFound", |error| {
            elm_effect::Effect::succeed(format!("caught: {}", error.resource))
        });
    assert_eq!(
        elm_effect::Runtime::run(caught),
        elm_effect::Exit::Success("caught: /test".to_string())
    );

    // Another tag is not this arm's business: the failure passes through.
    let other: elm_effect::Effect<String, NotFound> =
        elm_effect::Effect::fail(NotFound {
            resource: "/x".to_string(),
        })
        .catch_tag("Other", |_| elm_effect::Effect::succeed("nope".to_string()));
    assert!(matches!(
        elm_effect::Runtime::run(other),
        elm_effect::Exit::Failure(_)
    ));
}

#[test]
fn shape_area_matches_by_tag() {
    use data::{Field, Variant};
    // Data.test.ts:133 - Circle area via tag match.
    let circle = Variant::new("Circle", [("radius".to_owned(), Field::Float(5.0))]);
    let area = elm_effect::Matcher::value(circle)
        .when(
            elm_effect::Predicate::make(|shape: &Variant| shape.tag() == "Circle"),
            |shape| match shape.get("radius") {
                Some(Field::Float(radius)) => std::f64::consts::PI * radius * radius,
                _ => f64::NAN,
            },
        )
        .when(
            elm_effect::Predicate::make(|shape: &Variant| shape.tag() == "Rect"),
            |shape| match (shape.get("width"), shape.get("height")) {
                (Some(Field::Float(width)), Some(Field::Float(height))) => width * height,
                _ => f64::NAN,
            },
        )
        .or_else(f64::NAN);
    assert_eq!(area, std::f64::consts::PI * 25.0);
    // :138 - Rect area via tag match.
    let rect = Variant::new(
        "Rect",
        [
            ("width".to_owned(), Field::Float(3.0)),
            ("height".to_owned(), Field::Float(4.0)),
        ],
    );
    let area = elm_effect::Matcher::value(rect)
        .when(
            elm_effect::Predicate::make(|shape: &Variant| shape.tag() == "Circle"),
            |shape| match shape.get("radius") {
                Some(Field::Float(radius)) => std::f64::consts::PI * radius * radius,
                _ => f64::NAN,
            },
        )
        .when(
            elm_effect::Predicate::make(|shape: &Variant| shape.tag() == "Rect"),
            |shape| match (shape.get("width"), shape.get("height")) {
                (Some(Field::Float(width)), Some(Field::Float(height))) => width * height,
                _ => f64::NAN,
            },
        )
        .or_else(f64::NAN);
    assert_eq!(area, 12.0);
}

#[test]
fn match_fn_keeps_selector_arguments() {
    // Match.test.ts:6 — arms match the projected selector while bodies
    // keep the full input (both selector arguments).
    let run = |filter: &'static str| {
        let todos = vec![false, true];
        elm_effect::Matcher::value((todos, filter))
            .discriminator(|(_, f)| *f, "All", |(t, _)| t.len())
            .discriminator(
                |(_, f)| *f,
                "Active",
                |(t, _)| t.iter().filter(|completed| !**completed).count(),
            )
            .discriminator(
                |(_, f)| *f,
                "Completed",
                |(t, _)| t.iter().filter(|completed| **completed).count(),
            )
            .exhaustive()
    };
    assert_eq!(run("All"), 2);
    assert_eq!(run("Active"), 1);
    assert_eq!(run("Completed"), 1);
}

#[test]
fn match_field_value_hit_miss_missing() {
    // Match.test.ts:60 — symbol-keyed pattern properties; keys are plain
    // strings here (documented divergence), so pin field-value matching
    // with hit, mismatch, and missing cases.
    use data::{Field, Variant};
    let run = |fields: Vec<(String, Field)>| {
        elm_effect::Matcher::value(Variant::new("Obj", fields))
            .discriminator(
                |shape: &Variant| match shape.get("key") {
                    Some(Field::Str(s)) => Some(s.clone()),
                    _ => None,
                },
                Some("expected".to_string()),
                |_| "hit",
            )
            .or_else("miss")
    };
    assert_eq!(
        run(vec![(
            "key".to_string(),
            Field::Str("expected".to_string())
        )]),
        "hit"
    );
    assert_eq!(
        run(vec![("key".to_string(), Field::Str("other".to_string()))]),
        "miss"
    );
    assert_eq!(run(vec![]), "miss");
}

#[test]
fn empty_variants_match() {
    use data::Variant;
    // Data.test.ts:160 - empty variants carry their tag.
    let none = Variant::empty("None");
    assert_eq!(none.tag(), "None");
    // :165 - empty variants match by tag.
    let show_none = elm_effect::Matcher::value(none)
        .when(
            elm_effect::Predicate::make(|shape: &Variant| shape.tag() == "None"),
            |_| "none".to_owned(),
        )
        .when(
            elm_effect::Predicate::make(|shape: &Variant| shape.tag() == "Some"),
            |_| "some".to_owned(),
        )
        .or_else("unreachable".to_owned());
    assert_eq!(show_none, "none");
}
