//! `Data.$match` data-first: matching a tagged struct directly, the
//! `toJSON` field shape, and a Data class used as an Effect failure.

use elm_effect::{Effect, Exit, Matcher, Runtime};

#[path = "../src/data/data.rs"]
mod data;

use data::{Field, Struct};

fn circle(radius: f64) -> Struct {
    Struct::tagged("Circle", [("radius".to_string(), Field::Float(radius))])
}

fn rect() -> Struct {
    Struct::tagged("Rect", [("w".to_string(), Field::Float(0.0))])
}

#[test]
fn matches_a_tagged_struct_directly_data_first() {
    // Data.test.ts:143 — `$match` takes the value first, so both arms are
    // named by tag and the matching one receives the value.
    let area = |shape: &Struct| {
        Matcher::value(shape.clone())
            .discriminator(
                |s: &Struct| s.tag().unwrap_or_default().to_string(),
                "Circle".to_string(),
                |s| match s.get("radius") {
                    Some(Field::Float(r)) => *r * 2.0,
                    _ => f64::NAN,
                },
            )
            .discriminator(
                |s: &Struct| s.tag().unwrap_or_default().to_string(),
                "Rect".to_string(),
                |_| 0.0,
            )
            .exhaustive()
    };
    assert_eq!(area(&circle(1.0)), 2.0);
    assert_eq!(area(&rect()), 0.0);
}

#[test]
fn to_json_fields_carries_the_tag_and_every_field() {
    // Data.test.ts:212 — `toJSON` includes the fields, tag included.
    let e = Struct::tagged(
        "MyError",
        [
            ("code".to_string(), Field::Int(1)),
            ("message".to_string(), Field::Str("test".to_string())),
        ],
    );
    let json = e.to_json_fields();
    assert_eq!(json[0], ("_tag".to_string(), "\"MyError\"".to_string()));
    assert!(json.contains(&("code".to_string(), "1".to_string())));
    assert!(json.contains(&("message".to_string(), "\"test\"".to_string())));
    // fields are alphabetical, not insertion order (a BTreeMap holds them)
    assert_eq!(e.field_names(), vec!["code", "message"]);
}

#[test]
fn a_data_error_yields_as_an_effect_failure() {
    // Data.test.ts:202 — yielding a Data error inside a program fails the
    // program, so the exit is a Failure carrying that error.
    let not_found = Struct::tagged(
        "NotFound",
        [("resource".to_string(), Field::Str("/test".to_string()))],
    );

    let exit = Runtime::run(Effect::<(), Struct, ()>::fail(not_found.clone()));
    match exit {
        Exit::Failure(cause) => assert_eq!(cause.find_error(), Some(&not_found)),
        other => panic!("expected a Failure, got {other:?}"),
    }
}
