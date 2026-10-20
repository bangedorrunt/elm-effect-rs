//! `Schema.toDifferJsonPatch` fence: diff encoded wire values into a JSON
//! Patch, apply a patch back, decode the result.
//!
//! Derives from packages/effect/test/schema/toDifferJsonPatch.test.ts. Only
//! the rows the port can represent are carried: the struct/no-op, root
//! replace, immutability, Number, container-kind, and invalid-replace rows.
//! The Date, Defect, and DateTimeUtcFromMillis rows need schemas this crate
//! does not have (no Date/Defect/DateTime codecs), and the wide roundtrip
//! row drives those same missing codecs, so they stay unported.

use elm_effect_schema::{Differ, DifferError, JsonPatch, PatchError, PatchOp, Schema, Unknown};
use std::collections::BTreeMap;

fn object(entries: Vec<(&str, Unknown)>) -> Unknown {
    Unknown::Object(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

fn num(n: f64) -> Unknown {
    Unknown::Number(n)
}

/// toDifferJsonPatch.test.ts:61 — "patch returns the same reference if nothing
/// changed (no-op patch)". Rust has no reference identity for owned values,
/// so the port asserts the observable half: an empty patch hands the struct
/// value back unchanged, and diffing a value with itself is empty.
#[test]
fn empty_patch_hands_the_struct_value_back_unchanged() {
    let differ = Differ::new(Schema::struct_([(
        "a",
        Schema::as_unknown(Schema::string()),
    )]));
    let value = differ
        .schema()
        .decode(object(vec![("a", Unknown::Text("a".to_string()))]))
        .expect("the struct decodes");
    assert!(differ.diff(&value, &value).unwrap().is_empty());
    let out = differ.patch(&value, &[]).expect("an empty patch applies");
    assert_eq!(out, value);
}

/// toDifferJsonPatch.test.ts:69 — "root replace returns the provided reference
/// (no clone)". The port asserts the observable half: patching with one root
/// replace hands back exactly the provided value.
#[test]
fn root_replace_hands_back_the_provided_value() {
    let differ = Differ::new(Schema::any());
    let new_value = object(vec![("hello", Unknown::Text("world".to_string()))]);
    let out = differ
        .patch(
            &object(vec![("old", Unknown::Bool(true))]),
            &[PatchOp::Replace {
                path: String::new(),
                value: new_value.clone(),
            }],
        )
        .expect("a root replace applies");
    assert_eq!(out, new_value);
}

/// toDifferJsonPatch.test.ts:79 — "immutability: patch does not mutate the
/// input". The input travels behind `&`, so mutation cannot compile; the test
/// pins the rest: the input is unchanged afterwards and the output is right.
#[test]
fn patch_leaves_the_input_alone_and_returns_the_patched_value() {
    let differ = Differ::new(Schema::any());
    let old = object(vec![(
        "a",
        object(vec![(
            "b",
            Unknown::Array(vec![num(1.0), num(2.0), num(3.0)]),
        )]),
    )]);
    let snapshot = old.clone();
    let out = differ
        .patch(
            &old,
            &[
                PatchOp::Replace {
                    path: "/a/b/1".to_string(),
                    value: num(9.0),
                },
                PatchOp::Remove {
                    path: "/a/b/2".to_string(),
                },
            ],
        )
        .expect("the patch applies");
    assert_eq!(old, snapshot);
    assert_eq!(
        out,
        object(vec![(
            "a",
            object(vec![("b", Unknown::Array(vec![num(1.0), num(9.0)]))])
        )])
    );
}

/// toDifferJsonPatch.test.ts:96 — "Number: distinguishes 0 and -0; treats NaN
/// as equal to NaN". The diff compares wire numbers with JavaScript
/// `Object.is` semantics, which is the one place it is not plain `==`.
#[test]
fn number_diff_uses_object_is_semantics() {
    let differ = Differ::new(Schema::number());
    assert_eq!(
        differ.diff(&0.0, &-0.0).unwrap(),
        vec![PatchOp::Replace {
            path: String::new(),
            value: num(-0.0)
        }]
    );
    assert_eq!(
        differ.diff(&-0.0, &0.0).unwrap(),
        vec![PatchOp::Replace {
            path: String::new(),
            value: num(0.0)
        }]
    );
    assert!(differ.diff(&f64::NAN, &f64::NAN).unwrap().is_empty());
    assert!(
        differ
            .diff(&f64::INFINITY, &f64::INFINITY)
            .unwrap()
            .is_empty()
    );
    assert!(
        differ
            .diff(&f64::NEG_INFINITY, &f64::NEG_INFINITY)
            .unwrap()
            .is_empty()
    );

    let back = differ
        .patch(
            &0.0,
            &[PatchOp::Replace {
                path: String::new(),
                value: num(-0.0),
            }],
        )
        .expect("a root number replace applies");
    assert_eq!(back.to_bits(), (-0.0f64).to_bits());
    let forth = differ
        .patch(
            &-0.0,
            &[PatchOp::Replace {
                path: String::new(),
                value: num(0.0),
            }],
        )
        .expect("a root number replace applies");
    assert_eq!(forth.to_bits(), 0.0f64.to_bits());
}

/// toDifferJsonPatch.test.ts:167 — "diff/patch works when root container kind
/// changes": an array becoming an object is one root replace.
#[test]
fn root_container_kind_change_is_one_root_replace() {
    let differ = Differ::new(Schema::any());
    let old = Unknown::Array(vec![]);
    let new = Unknown::Object(BTreeMap::new());
    assert_eq!(
        differ.diff(&old, &new).unwrap(),
        vec![PatchOp::Replace {
            path: String::new(),
            value: new.clone()
        }]
    );
    assert_eq!(
        differ
            .patch(&old, &differ.diff(&old, &new).unwrap())
            .expect("applies"),
        new
    );
}

/// toDifferJsonPatch.test.ts:174 — "patch throws when asked to do an invalid
/// replace (replace requires existence)".
#[test]
fn replace_of_a_missing_key_is_refused() {
    let differ = Differ::new(Schema::any());
    let error = differ
        .patch(
            &Unknown::Object(BTreeMap::new()),
            &[PatchOp::Replace {
                path: "/x".to_string(),
                value: num(1.0),
            }],
        )
        .expect_err("replacing a missing key must fail");
    assert!(
        matches!(error, DifferError::Patch(PatchError::BadTarget(_))),
        "unexpected error: {error:?}"
    );
}

/// The free functions agree with the differ: `diff` derives the patch and
/// `apply` replays it, which is what the rows above exercise through schemas.
#[test]
fn free_diff_and_apply_agree() {
    let patch: JsonPatch = elm_effect_schema::diff(
        &object(vec![("a", num(1.0))]),
        &object(vec![("a", num(2.0)), ("b", Unknown::Bool(true))]),
    );
    assert_eq!(
        patch,
        vec![
            PatchOp::Replace {
                path: "/a".to_string(),
                value: num(2.0)
            },
            PatchOp::Add {
                path: "/b".to_string(),
                value: Unknown::Bool(true)
            },
        ]
    );
    let out = elm_effect_schema::apply(&object(vec![("a", num(1.0))]), &patch)
        .expect("the derived patch applies");
    assert_eq!(
        out,
        object(vec![("a", num(2.0)), ("b", Unknown::Bool(true))])
    );
}
