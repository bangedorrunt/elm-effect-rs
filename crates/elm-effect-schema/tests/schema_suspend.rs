//! Slice 7: suspended (lazy/recursive) schemas.
//! Ports packages/effect/test/schema/Schema.test.ts suspend cells by exact call.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use elm_effect_schema::{Field, Schema, Unknown};

fn obj(pairs: &[(&str, Unknown)]) -> Unknown {
    Unknown::Object(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    )
}

// 3994 + 6932: the factory does not run until the first decode/encode call.
#[test]
fn suspend_stays_lazy_until_decode() {
    let calls = Arc::new(AtomicUsize::new(0));
    let probe = calls.clone();
    let schema = Schema::<Unknown>::suspend(move || {
        probe.fetch_add(1, Ordering::SeqCst);
        Schema::unknown()
    });
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "factory runs before any call"
    );
    assert_eq!(schema.decode(Unknown::Null), Ok(Unknown::Null));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

// 6932: suspend resolves a binding initialized after the schema is built.
#[test]
fn suspend_reads_late_initialized_binding() {
    let expected: Arc<Mutex<Unknown>> = Arc::new(Mutex::new(Unknown::Null));
    let reader = expected.clone();
    let schema = Schema::<Unknown>::suspend(move || {
        let want = reader.lock().unwrap().clone();
        Schema::literal(want)
    });
    *expected.lock().unwrap() = Unknown::Text("late".to_string());
    assert_eq!(
        schema.decode(Unknown::Text("late".to_string())),
        Ok(Unknown::Text("late".to_string()))
    );
    assert!(schema.decode(Unknown::Null).is_err());
}

fn node_schema() -> Schema<BTreeMap<String, Unknown>> {
    Schema::struct_shape(vec![
        Field::required("value", Schema::as_unknown(Schema::string())),
        Field::optional(
            "next",
            Schema::<Unknown>::suspend(|| Schema::as_unknown(node_schema())),
        ),
    ])
}

// 3994: recursive values stay lazy — each level builds on demand while decoding.
#[test]
fn suspend_recursive_shape_decodes_nested() {
    let schema = Schema::as_unknown(node_schema());
    let nested = obj(&[
        ("value", Unknown::Text("a".to_string())),
        ("next", obj(&[("value", Unknown::Text("b".to_string()))])),
    ]);
    let decoded = schema.decode(nested.clone()).expect("nested node decodes");
    assert_eq!(
        schema.try_encode(&decoded).unwrap(),
        nested,
        "recursive roundtrip is exact"
    );
    assert!(
        schema
            .decode(obj(&[("value", Unknown::Number(1.0))]))
            .is_err()
    );
}

// 4586 + 4627: candidate selection never forces a suspended loser after a win.
#[test]
fn union_skips_suspended_loser_after_success() {
    let schema = Schema::union(
        Schema::literal(Unknown::Text("hit".to_string())),
        Schema::<Unknown>::suspend(|| panic!("suspended loser must stay lazy after a winner")),
    );
    assert_eq!(
        schema.decode(Unknown::Text("hit".to_string())),
        Ok(Unknown::Text("hit".to_string()))
    );
    assert_eq!(
        schema.try_encode(&Unknown::Text("hit".to_string())),
        Ok(Unknown::Text("hit".to_string()))
    );
}

// 4586, second half: the pin counts the factory's calls, and the count only
// comes out right if a suspend resolves once. Its third leg pins the same
// count for the `oneOf` mode, where an ambiguous value does force the member
// (exactly one member must match, so the winner cannot short-circuit).
#[test]
fn suspend_resolves_once_across_calls_that_reach_it() {
    let evaluations = Arc::new(AtomicUsize::new(0));
    let probe = Arc::clone(&evaluations);
    let suspended: Schema<Unknown> = Schema::<Unknown>::suspend(move || {
        probe.fetch_add(1, Ordering::SeqCst);
        Schema::as_unknown(Schema::string())
    });
    let any_of = Schema::union(Schema::literal(Unknown::Text("a".to_string())), suspended);

    assert_eq!(
        any_of.decode(Unknown::Text("b".to_string())),
        Ok(Unknown::Text("b".to_string()))
    );
    assert_eq!(
        evaluations.load(Ordering::SeqCst),
        1,
        "first reach resolves"
    );
    assert_eq!(
        any_of.decode(Unknown::Text("c".to_string())),
        Ok(Unknown::Text("c".to_string()))
    );
    assert_eq!(
        evaluations.load(Ordering::SeqCst),
        1,
        "the pin resolves the thunk once (SchemaAST.ts:3863), not per decode"
    );
}

#[test]
fn one_of_forces_a_suspended_member_once() {
    let evaluations = Arc::new(AtomicUsize::new(0));
    let probe = Arc::clone(&evaluations);
    let suspended: Schema<Unknown> = Schema::<Unknown>::suspend(move || {
        probe.fetch_add(1, Ordering::SeqCst);
        Schema::as_unknown(Schema::string())
    });
    let one_of = Schema::<Unknown>::union_with_mode(
        vec![Schema::literal(Unknown::Text("a".to_string())), suspended],
        "oneOf",
    );

    assert_eq!(
        one_of
            .decode(Unknown::Text("a".to_string()))
            .expect_err("two members accept \"a\"")
            .to_string(),
        "expected exactly one member to match at $, got string"
    );
    assert_eq!(
        evaluations.load(Ordering::SeqCst),
        1,
        "exactly-one-member counting forces the member, but only once"
    );
}

// 1393: suspending a schema keeps its inner checks — they still reject.
#[test]
fn suspend_preserves_inner_refinement() {
    let schema = Schema::<Unknown>::suspend(|| {
        Schema::as_unknown(Schema::string()).refine(
            "non-empty",
            |unknown| !matches!(unknown, Unknown::Text(text) if text.is_empty()),
        )
    });
    assert_eq!(
        schema.decode(Unknown::Text("ok".to_string())),
        Ok(Unknown::Text("ok".to_string()))
    );
    assert!(schema.decode(Unknown::Text(String::new())).is_err());
    assert!(schema.decode(Unknown::Null).is_err());
}

// 10664: a struct field can be declared with a suspended member schema.
#[test]
fn struct_field_using_suspend() {
    let schema = Schema::struct_shape(vec![
        Field::required("name", Schema::as_unknown(Schema::string())),
        Field::required(
            "child",
            Schema::<Unknown>::suspend(|| Schema::as_unknown(Schema::string())),
        ),
    ]);
    let input = obj(&[
        ("name", Unknown::Text("p".to_string())),
        ("child", Unknown::Text("c".to_string())),
    ]);
    let decoded = schema
        .decode(input.clone())
        .expect("suspended field decodes");
    assert_eq!(schema.try_encode(&decoded).unwrap(), input);
}
