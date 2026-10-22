//! The `builtInRevivers` declaration rows: a schema the pin persists as a
//! declaration must convert to its own node, come back by id, and re-convert to
//! the same JSON.
//!
//! Each test is the port's version of the pin's `assertDeclarationReviver`:
//! convert → `to_json` → `from_json` → revive with the built-in revivers →
//! convert again → compare. The comparison is what makes the claim round-trip
//! evidence rather than a name check.

use elm_effect_schema::representation_doc::{
    DeclarationReviver, Document, ReprError, from_json, from_representation_with_declarations,
    to_json, to_representation,
};
use elm_effect_schema::{Schema, Unknown};

/// The pin's `assertDeclarationReviver`, minus the JS `ast` accessors: the port
/// exposes the identity as `Schema::declaration`.
fn assert_declaration_reviver(
    schema: Schema<Unknown>,
    id: &str,
    expect_payload: serde_json::Value,
) {
    let declaration = schema
        .declaration()
        .unwrap_or_else(|| panic!("{id}: the schema must carry a declaration"));
    assert_eq!(declaration.id, id, "the constructor writes its own id");
    assert_eq!(
        declaration.payload, expect_payload,
        "{id}: the payload the constructor writes"
    );

    let document = to_representation(&schema).expect("converts");
    let json = to_json(&document).expect("to_json");
    assert_eq!(
        json["representation"]["_tag"], "Declaration",
        "{id}: a declaration converts to its own node"
    );

    let reread: Document = from_json(&json).expect("from_json");
    let revivers: Vec<DeclarationReviver> = elm_effect_schema::builtin_declaration_revivers();
    let revived = from_representation_with_declarations(&reread, &[], &revivers)
        .unwrap_or_else(|error: ReprError| panic!("{id}: revive failed: {error}"));

    let revived_json = to_json(&to_representation(&revived).expect("re-converts")).expect("to_json");
    assert_eq!(revived_json, json, "{id}: the revived document round-trips");
    assert_eq!(
        revived
            .declaration()
            .map(|declaration| declaration.id.as_str()),
        Some(id),
        "{id}: the revived schema keeps the identity"
    );
    // The pin checks that the revived node has a code generator; here that is
    // the repr's code answer, which must still name the constructor.
    assert!(
        revived.to_code().contains(id.rsplit('/').next().unwrap_or(id)),
        "{id}: the revived schema still renders its constructor: {}",
        revived.to_code()
    );
}

/// builtInRevivers.test.ts:813 `revives Option`.
#[test]
fn revives_option() {
    assert_declaration_reviver(
        Schema::as_unknown(Schema::option(Schema::string())),
        "effect/schema/Option",
        serde_json::Value::Null,
    );
}

/// builtInRevivers.test.ts:822 `revives Result`.
#[test]
fn revives_result() {
    assert_declaration_reviver(
        Schema::as_unknown(Schema::result(Schema::string(), Schema::number())),
        "effect/schema/Result",
        serde_json::Value::Null,
    );
}

/// builtInRevivers.test.ts:831 `revives Redacted`.
#[test]
fn revives_redacted() {
    assert_declaration_reviver(
        Schema::redacted(Schema::as_unknown(Schema::string()), None),
        "effect/schema/Redacted",
        serde_json::Value::Null,
    );
}

/// builtInRevivers.test.ts:876 `revives ReadonlyMap`.
#[test]
fn revives_readonly_map() {
    assert_declaration_reviver(
        Schema::as_unknown(Schema::<Vec<(Unknown, Unknown)>>::readonly_map(
            Schema::as_unknown(Schema::string()),
            Schema::as_unknown(Schema::number()),
        )),
        "effect/schema/ReadonlyMap",
        serde_json::Value::Null,
    );
}

/// builtInRevivers.test.ts:885 `revives HashMap`.
#[test]
fn revives_hash_map() {
    assert_declaration_reviver(
        Schema::as_unknown(Schema::<Vec<(Unknown, Unknown)>>::hash_map(
            Schema::as_unknown(Schema::string()),
            Schema::as_unknown(Schema::number()),
        )),
        "effect/schema/HashMap",
        serde_json::Value::Null,
    );
}

/// builtInRevivers.test.ts:903 `revives ReadonlySet`.
#[test]
fn revives_readonly_set() {
    assert_declaration_reviver(
        Schema::as_unknown(Schema::<Vec<Unknown>>::readonly_set(Schema::as_unknown(
            Schema::string(),
        ))),
        "effect/schema/ReadonlySet",
        serde_json::Value::Null,
    );
}

/// builtInRevivers.test.ts:1065 `revives Json`.
#[test]
fn revives_json() {
    assert_declaration_reviver(Schema::json(), "effect/schema/Json", serde_json::Value::Null);
}

/// The pin's `Missing reviver for {id}`: a declaration with no registered
/// reviver fails by id rather than guessing at its shape.
#[test]
fn a_declaration_without_a_reviver_fails_by_id() {
    let schema = Schema::as_unknown(Schema::option(Schema::string()));
    let document = to_representation(&schema).expect("converts");
    let Err(error) = from_representation_with_declarations(&document, &[], &[]) else {
        panic!("no reviver registered must fail");
    };
    assert!(
        error.message.contains("Missing reviver for effect/schema/Option"),
        "{error}"
    );
}

/// A reviver receives parameters in the order the constructor declares them:
/// `Result` puts success first, and a two-parameter document that arrives
/// swings right must read right.
#[test]
fn a_declaration_reviver_keeps_parameter_order() {
    let schema = Schema::as_unknown(Schema::result(Schema::string(), Schema::number()));
    let json = to_json(&to_representation(&schema).expect("converts")).expect("to_json");
    let revivers = elm_effect_schema::builtin_declaration_revivers();
    let revived = from_representation_with_declarations(
        &from_json(&json).expect("from_json"),
        &[],
        &revivers,
    )
    .expect("revives");

    // Success is a string, failure a number: a swapped pair would refuse both.
    assert!(revived.decode(Unknown::from_json(serde_json::json!({ "_tag": "Success", "success": "a" }))).is_ok());
    assert!(revived.decode(Unknown::from_json(serde_json::json!({ "_tag": "Failure", "failure": 1 }))).is_ok());
    assert!(revived.decode(Unknown::from_json(serde_json::json!({ "_tag": "Success", "success": 1 }))).is_err());
}
