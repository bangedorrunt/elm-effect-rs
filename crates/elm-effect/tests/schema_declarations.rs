//! The `builtInRevivers` rows whose constructors live in this crate:
//! `Schema.ByteSize` (:966), `Schema.Chunk` (:921), `Schema.RegExp` (:930),
//! `Schema.Graph` (:894), `Schema.Date` (:948), `Schema.CauseReason` (:840),
//! `Schema.Cause` (:849) and `Schema.Exit` (:867).
//!
//! Each test is the pin's `assertDeclarationReviver`: convert → `to_json` →
//! `from_json` → revive with the revivers → convert again → compare. The
//! comparison is what makes the claim evidence rather than a name check, and
//! the revived schema must still answer its constructor's code.
//!
//! The schema crate cannot host these: `ByteSize`, `Graph`, `DateTime`, `Cause`
//! and `Exit` are this crate's own types, and the dependency runs the other way.

use elm_effect::{
    File, Kind, cause_reason_schema, cause_schema, date, declaration_revivers, exit_schema, file,
    graph,
};
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

/// `builtInRevivers.test.ts:921` `revives Chunk`.
#[test]
fn revives_chunk() {
    assert_declaration_reviver(
        Schema::as_unknown(elm_effect::chunk(Schema::string())),
        "effect/schema/Chunk",
        elm_effect_schema::serde_json::Value::Null,
        "Schema.Chunk(Schema.String)",
    );
}

/// `builtInRevivers.test.ts:930` `revives RegExp`.
#[test]
fn revives_regexp() {
    assert_declaration_reviver(
        Schema::as_unknown(elm_effect::regexp()),
        "effect/schema/RegExp",
        elm_effect_schema::serde_json::Value::Null,
        "Schema.RegExp",
    );
}

/// `builtInRevivers.test.ts:894` `revives Graph`: the kind rides in the payload
/// while node and edge are the two type parameters.
#[test]
fn revives_graph() {
    assert_declaration_reviver(
        Schema::as_unknown(graph(Kind::Directed, Schema::string(), Schema::number())),
        "effect/schema/Graph",
        elm_effect_schema::serde_json::Value::String("directed".to_string()),
        "Schema.Graph(\"directed\", Schema.String, Schema.Number)",
    );
}

/// The pin's documented example (`Schema.Graph`): the wire object decodes into a
/// graph and encodes back to the same object, indexes included.
#[test]
fn a_graph_decodes_and_encodes_the_pin_wire_form() {
    let schema = graph(Kind::Directed, Schema::string(), Schema::number());
    let wire = elm_effect_schema::serde_json::json!({
        "type": "directed",
        "nodes": [{ "index": 0, "data": "A" }, { "index": 1, "data": "B" }],
        "edges": [{ "index": 0, "source": 0, "target": 1, "data": 1 }]
    });
    let value = schema
        .decode(Unknown::from_json(wire.clone()))
        .expect("the pin's own example decodes");
    assert_eq!(value.node_count(), 2);
    assert_eq!(value.edge_count(), 1);
    assert_eq!(
        schema.try_encode(&value).expect("encodes"),
        Unknown::from_json(wire),
        "the wire form round-trips",
    );
}

/// `builtInRevivers.test.ts:948` `revives Date`.
#[test]
fn revives_date() {
    assert_declaration_reviver(
        Schema::as_unknown(date()),
        "effect/schema/Date",
        elm_effect_schema::serde_json::Value::Null,
        "Schema.Date",
    );
}

/// `Schema.Date`'s JSON codec is `toJSON`/`dateFromString`: the text is the
/// instant, a bare civil date reads as UTC, and a non-ISO string is refused.
#[test]
fn a_date_codec_reads_and_writes_the_iso_text() {
    let schema = date();
    let millis = schema
        .decode(Unknown::from_json(elm_effect_schema::serde_json::json!(
            "2024-01-01T00:00:00.123Z"
        )))
        .expect("an ISO instant decodes");
    assert_eq!(millis.epoch_millis(), 1_704_067_200_123);
    assert_eq!(
        schema.try_encode(&millis).expect("encodes"),
        Unknown::Text("2024-01-01T00:00:00.123Z".to_string()),
    );

    // The pin reads a bare civil string as UTC (`DateTime.make`), and the value
    // it answers is a millisecond instant, so the text is the normalized one.
    let bare = schema
        .decode(Unknown::from_json(elm_effect_schema::serde_json::json!(
            "2024-01-01"
        )))
        .expect("a civil date decodes");
    assert_eq!(
        schema.try_encode(&bare).expect("encodes"),
        Unknown::Text("2024-01-01T00:00:00.000Z".to_string()),
    );

    let error = schema
        .decode(Unknown::Text("not a date".to_string()))
        .expect_err("garbage is refused");
    assert!(
        error
            .to_string()
            .contains("a string that will be decoded as a Date"),
        "{error}"
    );
}

/// `builtInRevivers.test.ts:984` `revives File`.
#[test]
fn revives_file() {
    assert_declaration_reviver(
        Schema::as_unknown(file()),
        "effect/schema/File",
        elm_effect_schema::serde_json::Value::Null,
        "Schema.File",
    );
}

/// `Schema.File`'s JSON codec is `{ data, type, name, lastModified }` with the
/// bytes Base64-encoded, and a `data` the decoder cannot read is refused at its
/// own key.
#[test]
fn a_file_codec_reads_and_writes_the_wire_object() {
    let schema = file();
    let wire = elm_effect_schema::serde_json::json!({
        "data": "aGVsbG8=",
        "type": "text/plain",
        "name": "greeting.txt",
        "lastModified": 1_704_067_200_000_i64
    });
    let value = schema
        .decode(Unknown::from_json(wire.clone()))
        .expect("the pin's shape decodes");
    assert_eq!(value.data(), b"hello");
    assert_eq!(value.name(), "greeting.txt");
    assert_eq!(value.media_type(), "text/plain");
    assert_eq!(value.last_modified(), 1_704_067_200_000);
    assert_eq!(value.size(), 5);
    assert_eq!(
        schema.try_encode(&value).expect("encodes"),
        Unknown::from_json(wire),
        "the wire object round-trips",
    );

    let error = schema
        .decode(Unknown::from_json(elm_effect_schema::serde_json::json!({
            "data": "aGVsbG8",
            "type": "text/plain",
            "name": "greeting.txt",
            "lastModified": 0
        })))
        .expect_err("unpadded Base64 is the malformed form");
    assert!(error.to_string().contains("a valid Base64 string"), "{error}");

    assert_eq!(File::new(b"hi".to_vec(), "a", "text/plain", 0).size(), 2);
}

/// The three rules the graph itself enforces: indexes increase, endpoints are
/// live nodes, and the wire kind is the one the schema declared.
#[test]
fn a_graph_refuses_a_snapshot_the_graph_cannot_hold() {
    let schema = graph(Kind::Directed, Schema::string(), Schema::number());
    let cases = [
        (
            elm_effect_schema::serde_json::json!({
                "type": "directed",
                "nodes": [{ "index": 1, "data": "A" }, { "index": 1, "data": "B" }],
                "edges": []
            }),
            "strictly increasing",
        ),
        (
            elm_effect_schema::serde_json::json!({
                "type": "directed",
                "nodes": [{ "index": 0, "data": "A" }],
                "edges": [{ "index": 0, "source": 0, "target": 3, "data": 1 }]
            }),
            "Node 3 does not exist",
        ),
        (
            elm_effect_schema::serde_json::json!({ "type": "undirected", "nodes": [], "edges": [] }),
            "the directed graph kind",
        ),
    ];
    for (wire, expected) in cases {
        let error = schema
            .decode(Unknown::from_json(wire))
            .expect_err("the snapshot is not a directed graph");
        assert!(error.to_string().contains(expected), "{expected}: {error}");
    }
}

/// The kind is a declaration payload, so a document that names neither kind has
/// no constructor to rebuild.
#[test]
fn a_graph_document_without_a_kind_payload_is_refused() {
    let schema = Schema::as_unknown(graph(Kind::Undirected, Schema::string(), Schema::number()));
    let mut json = to_json(&to_representation(&schema).expect("converts")).expect("to_json");
    json["representation"]["representation"]["payload"] =
        elm_effect_schema::serde_json::Value::Null;
    let reread: Document = from_json(&json).expect("from_json");
    let Err(error) = from_representation_with_declarations(&reread, &[], &declaration_revivers())
    else {
        panic!("a graph without a kind must not revive");
    };
    assert!(
        error
            .message
            .contains("takes a directed/undirected payload"),
        "{error}"
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
