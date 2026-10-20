//! The construction adapters' contract when a constructor default DIES rather
//! than failing with a schema issue: `make` and `makeOption` may only report a
//! schema issue, so a defect is re-thrown with the adapter's own wording and
//! kept attached.
//!
//!     cargo test -p elm-effect-schema --test schema_constructor_die

use std::collections::BTreeMap;

use elm_effect_schema::class::class;
use elm_effect_schema::{
    AdapterError, CONSTRUCTOR_DIE, ConstructError, Field, OPTION_DIE, Schema, SchemaError, Unknown,
};

fn map(pairs: &[(&str, Unknown)]) -> BTreeMap<String, Unknown> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.clone()))
        .collect()
}

fn obj(pairs: &[(&str, Unknown)]) -> Unknown {
    Unknown::Object(map(pairs))
}

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

/// A struct whose one field carries the constructor default under test, wrapped
/// the way a struct field always is.
fn defaulted_struct(failure: ConstructError) -> Schema<BTreeMap<String, Unknown>> {
    Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(
            Schema::<String>::string().with_constructor_default(move || Err(failure.clone())),
        ),
    )])
}

/// Schema.test.ts:3640 — "should throw an error when the cause contains both a
/// schema issue and a defect": the constructor adapter may only throw schema
/// issues, so the default's mixed cause is reported with the adapter's wording
/// and the defect survives.
#[test]
fn make_reports_a_died_default_with_the_constructor_adapter_message() {
    // The pin's cause holds a schema issue AND a defect. Construction keeps
    // both, so the mixed evidence is inspectable by whoever catches it.
    let mixed = ConstructError::Die {
        defect: text("defect"),
        issues: vec![SchemaError::DefaultFailed {
            path: "$".to_string(),
            message: "schema issue".to_string(),
        }],
    };
    let schema = defaulted_struct(mixed);

    // Construction prefixes the issue to the field, and keeps the defect.
    match schema.construct(obj(&[])).unwrap_err() {
        ConstructError::Die { defect, issues } => {
            assert_eq!(defect, text("defect"));
            assert_eq!(issues.len(), 1, "the issue that rode beside the defect");
            assert_eq!(
                issues[0].to_string(),
                "schema issue at $.a",
                "the issue lands on the field, not the root"
            );
        }
        other => panic!("a defect must not be reported as an issue: {other:?}"),
    }

    // `make` is an adapter: it cannot report a die as an issue, so it reports
    // its own contract message and keeps the defect.
    let error = schema.make(map(&[])).unwrap_err();
    assert_eq!(error.to_string(), CONSTRUCTOR_DIE);
    match &error {
        AdapterError::Die {
            message, defect, ..
        } => {
            assert_eq!(*message, CONSTRUCTOR_DIE);
            assert_eq!(defect, &text("defect"));
        }
        other => panic!("the adapter must not report a die as an issue: {other:?}"),
    }

    // The control: a default that fails with a pure schema issue is reported as
    // itself, which is what makes the adapter's refusal above meaningful.
    let pure = defaulted_struct(ConstructError::Issue(SchemaError::DefaultFailed {
        path: "$".to_string(),
        message: "ctor default failed".to_string(),
    }));
    let issue = pure.make(map(&[])).unwrap_err();
    assert!(matches!(issue, AdapterError::Issue(_)));
    assert!(issue.to_string().contains("ctor default failed"));
    assert!(issue.to_string().contains("$.a"));

    // Decoding never runs a constructor default, so the same input fails as a
    // missing key instead of reaching the die at all.
    assert!(schema.decode(obj(&[])).is_err());
}

/// Schema.test.ts:3712 — "should preserve mixed schema issue and defect
/// causes" (`makeEffect`): the exit the pin hands back has the defect *and* the
/// issue beside it, so `Cause.findError` still finds the issue. This port's
/// `make` is that adapter one hop earlier, and the refusal keeps the issues.
#[test]
fn make_keeps_the_issues_beside_a_died_default() {
    let mixed = ConstructError::Die {
        defect: text("defect"),
        issues: vec![SchemaError::DefaultFailed {
            path: "$".to_string(),
            message: "schema issue".to_string(),
        }],
    };
    let schema = defaulted_struct(mixed);

    let error = schema.make(map(&[])).unwrap_err();
    assert_eq!(error.to_string(), CONSTRUCTOR_DIE);
    match &error {
        AdapterError::Die {
            message,
            defect,
            issues,
        } => {
            assert_eq!(*message, CONSTRUCTOR_DIE);
            assert_eq!(defect, &text("defect"));
            assert_eq!(issues.len(), 1, "the issue that rode beside the defect");
            assert_eq!(issues[0].to_string(), "schema issue at $.a");
        }
        other => panic!("the adapter must not report a die as an issue: {other:?}"),
    }
    // The pin's `Cause.findError(error.cause)`.
    assert_eq!(
        error.issue().map(ToString::to_string),
        Some("schema issue at $.a".to_string())
    );

    // The control: an issue-only failure keeps answering as that issue.
    let pure = defaulted_struct(ConstructError::Issue(SchemaError::DefaultFailed {
        path: "$".to_string(),
        message: "schema issue".to_string(),
    }));
    let plain = pure.make(map(&[])).unwrap_err();
    assert!(matches!(plain, AdapterError::Issue(_)));
    assert_eq!(
        plain.issue().map(ToString::to_string),
        Some("schema issue at $.a".to_string())
    );
}

/// Schema.test.ts:3670 — "should throw an error when the cause is not a schema
/// issue": `makeOption` may only answer `None` for schema issues, so a died
/// default is thrown for both the struct and the class over it.
#[test]
fn make_option_refuses_a_died_default_instead_of_answering_none() {
    let schema = defaulted_struct(ConstructError::die(text("make defect")));

    let error = schema.make_option(obj(&[])).unwrap_err();
    assert_eq!(error.to_string(), OPTION_DIE);
    match &error {
        AdapterError::Die {
            message, defect, ..
        } => {
            assert_eq!(*message, OPTION_DIE);
            assert_eq!(defect, &text("make defect"));
        }
        other => panic!("the option adapter must not answer none for a die: {other:?}"),
    }

    // The class adapter shares the wording, and asks with an empty object
    // because `A.makeOption()` takes no props.
    let a = class(
        "A",
        vec![(
            "a",
            Schema::as_unknown(
                Schema::<String>::string()
                    .with_constructor_default(|| Err(ConstructError::die(text("make defect")))),
            ),
        )],
    );
    let class_error = a.make_option(Unknown::Object(BTreeMap::new())).unwrap_err();
    assert_eq!(class_error.to_string(), OPTION_DIE);
    assert!(matches!(class_error, AdapterError::Die { .. }));

    // The control: a check failure is a schema issue, which is exactly what
    // `makeOption` answers `None` for.
    let checked = Schema::as_unknown(Schema::struct_shape(vec![Field::required(
        "a",
        Schema::as_unknown(Schema::finite_from_string().refine("greater than 0", |n| *n > 0.0)),
    )]));
    assert_eq!(checked.make_option(obj(&[("a", text("-1"))])), Ok(None));
}
