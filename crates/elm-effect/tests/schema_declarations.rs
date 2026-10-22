//! The `builtInRevivers` rows whose constructors live in this crate:
//! `Schema.ByteSize` (:966), `Schema.CauseReason` (:840), `Schema.Cause`
//! (:849) and `Schema.Exit` (:867).
//!
//! Each test is the pin's `assertDeclarationReviver`: convert → `to_json` →
//! `from_json` → revive with the revivers → convert again → compare. The
//! comparison is what makes the claim evidence rather than a name check, and
//! the revived schema must still answer its constructor's code.
//!
//! The schema crate cannot host these: `ByteSize`, `Cause` and `Exit` are this
//! crate's own types, and the dependency runs the other way.

use elm_effect::{cause_reason_schema, cause_schema, declaration_revivers, exit_schema};
use elm_effect_schema::representation_doc::{
    Document, ReprError, from_json, from_representation_with_declarations, to_json,
    to_representation,
};
use elm_effect_schema::{Schema, Unknown};

/// The pin's `assertDeclarationReviver`: the constructor writes its id and
/// payload, a persisted one is a `Declaration` node, and the revived schema
/// converts back to the same document.
fn assert_declaration_reviver(
    schema: Schema<Unknown>,
    id: &str,
    expect_payload: elm_effect_schema::serde_json::Value,
    expect_code: &str,
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
    let revivers = declaration_revivers();
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
    assert_eq!(
        revived.to_code(),
        expect_code,
        "{id}: the revived schema still renders its constructor"
    );
}

/// `builtInRevivers.test.ts:966` `revives ByteSize`.
#[test]
fn revives_byte_size() {
    assert_declaration_reviver(
        Schema::as_unknown(elm_effect::byte_size()),
        "effect/schema/ByteSize",
        elm_effect_schema::serde_json::Value::Null,
        "Schema.ByteSize",
    );
}

/// `builtInRevivers.test.ts:840` `revives CauseReason`: the two type parameters
/// come back in the order the constructor declares them.
#[test]
fn revives_cause_reason() {
    assert_declaration_reviver(
        Schema::as_unknown(cause_reason_schema(
            Schema::string(),
            Schema::number(),
        )),
        "effect/schema/CauseReason",
        elm_effect_schema::serde_json::Value::Null,
        "Schema.CauseReason(Schema.String, Schema.Number)",
    );
}

/// `builtInRevivers.test.ts:849` `revives Cause`.
#[test]
fn revives_cause() {
    assert_declaration_reviver(
        Schema::as_unknown(cause_schema(Schema::string(), Schema::number())),
        "effect/schema/Cause",
        elm_effect_schema::serde_json::Value::Null,
        "Schema.Cause(Schema.String, Schema.Number)",
    );
}

/// `builtInRevivers.test.ts:867` `revives Exit(value, error, defect)`.
#[test]
fn revives_exit() {
    assert_declaration_reviver(
        Schema::as_unknown(exit_schema(
            Schema::string(),
            Schema::number(),
            Schema::as_unknown(Schema::boolean()),
        )),
        "effect/schema/Exit",
        elm_effect_schema::serde_json::Value::Null,
        "Schema.Exit(Schema.String, Schema.Number, Schema.Boolean)",
    );
}

/// A document whose parameter count does not match the constructor is refused
/// by id, rather than building a schema from a shifted parameter list.
#[test]
fn a_shifted_parameter_list_is_refused() {
    let schema = Schema::as_unknown(cause_schema(Schema::string(), Schema::number()));
    let mut json = to_json(&to_representation(&schema).expect("converts")).expect("to_json");
    json["representation"]["typeParameters"] = elm_effect_schema::serde_json::json!([{
        "_tag": "String"
    }]);
    let reread: Document = from_json(&json).expect("from_json");
    let Err(error) = from_representation_with_declarations(&reread, &[], &declaration_revivers())
    else {
        panic!("a one-parameter Cause must not revive");
    };
    assert!(
        error.message.contains("takes 2 type parameter(s), got 1"),
        "{error}"
    );
}
