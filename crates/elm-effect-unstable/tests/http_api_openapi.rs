//! `packages/effect/test/httpapi/OpenApi.test.ts` document-shape witnesses.
//!
//! Each test carries one row with the exact expected OpenAPI fragment. Rows
//! needing missing machinery stay excused with reasons: encoded-side optionality
//! projection (:117), codec modes (:155, :556), middleware error sets (:621,
//! :643, :668, :697, :749), header wire schemas (:474, :501, :525, :596, :780),
//! stream/text/form bodies (:292, :326, :374, :393, :424), spec caching and
//! `rawJSON`/override identity (:178 passes by construction, :205, :221).

use elm_effect_schema::{Schema, Unknown};
use elm_effect_unstable::openapi::{
    ApiEndpoint, Method, OpenApiError, ParamIn, Parameter, ResponseEntry, SecurityScheme,
    SpecBuilder, SpecCache, template_path,
};
use std::cell::Cell;

fn string() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

fn param(name: &str, location: ParamIn, required: bool) -> Parameter {
    Parameter {
        name: name.to_owned(),
        location,
        required,
        schema: string(),
    }
}

fn json_response(schema: Schema<Unknown>) -> ResponseEntry {
    ResponseEntry {
        status: None,
        content_type: "application/json".to_owned(),
        schema,
    }
}

#[test]
fn preserves_literal_action_suffixes_in_path_templates() {
    // OpenApi.test.ts:73 — `:id:wait` templates the id and keeps `:wait`.
    let mut endpoint = ApiEndpoint::new("wait", Method::Post, "/operations/:id:wait");
    endpoint.parameters.push(param("id", ParamIn::Path, true));
    let spec = SpecBuilder::new("Api").endpoint(endpoint).build().unwrap();
    assert_eq!(
        spec["paths"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        vec!["/operations/{id}:wait"]
    );
    assert_eq!(
        spec["paths"]["/operations/{id}:wait"]["post"]["parameters"],
        serde_json::json!([
            {"name": "id", "in": "path", "required": true, "schema": {"type": "string"}}
        ])
    );
}

#[test]
fn keeps_action_paths_literal_when_no_parameter_schema_is_declared() {
    // OpenApi.test.ts:89 — no leading `:`, so no substitution and no params.
    let endpoint = ApiEndpoint::new("resolve", Method::Post, "/offers:resolve");
    let spec = SpecBuilder::new("Api").endpoint(endpoint).build().unwrap();
    assert_eq!(
        spec["paths"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        vec!["/offers:resolve"]
    );
    assert_eq!(
        spec["paths"]["/offers:resolve"]["post"]["parameters"],
        serde_json::json!([])
    );
}

#[test]
fn preserves_parameter_schemas_when_a_transform_reorders_them() {
    // OpenApi.test.ts:99 — the transform sees the assembled operation.
    fn reverse(mut operation: serde_json::Value) -> serde_json::Value {
        let params = operation["parameters"].take();
        operation["parameters"] = match params {
            serde_json::Value::Array(mut params) => {
                params.reverse();
                serde_json::Value::Array(params)
            }
            other => other,
        };
        operation
    }
    let mut endpoint = ApiEndpoint::new("list", Method::Get, "/list");
    endpoint.parameters.push(Parameter {
        name: "first".to_owned(),
        location: ParamIn::Query,
        required: true,
        schema: Schema::as_unknown(Schema::literal(Unknown::Text("first".to_owned()))),
    });
    endpoint.parameters.push(Parameter {
        name: "second".to_owned(),
        location: ParamIn::Query,
        required: true,
        schema: Schema::as_unknown(Schema::literal(Unknown::Text("second".to_owned()))),
    });
    endpoint.transform = Some(reverse);
    let spec = SpecBuilder::new("Api").endpoint(endpoint).build().unwrap();
    assert_eq!(
        spec["paths"]["/list"]["get"]["parameters"],
        serde_json::json!([
            {"name": "second", "in": "query", "required": true,
             "schema": {"type": "string", "enum": ["second"]}},
            {"name": "first", "in": "query", "required": true,
             "schema": {"type": "string", "enum": ["first"]}}
        ])
    );
}

#[test]
fn returns_fresh_spec_instances_without_a_cache() {
    // OpenApi.test.ts:178 — every build returns owned values, so mutating one
    // document never leaks into the next.
    let endpoint = ApiEndpoint::new("get", Method::Get, "/resource");
    let builder = SpecBuilder::new("Api").endpoint(endpoint);
    let mut first = builder.build().unwrap();
    first["info"]["title"] = serde_json::json!("mutated");
    first["paths"]["/resource"]["get"]["operationId"] = serde_json::json!("mutated");
    assert_ne!(first, builder.build().unwrap());
    let second = builder.build().unwrap();
    assert_eq!(second["info"]["title"], serde_json::json!("Api"));
    assert_eq!(
        second["paths"]["/resource"]["get"]["operationId"],
        serde_json::json!("get")
    );
}

#[test]
fn preserves_every_declared_payload_content_type() {
    // OpenApi.test.ts:233 — declared (non-normalized) content types stay as keys.
    let profile_a = "Application/Vnd.Effect+JSON; Profile=A";
    let profile_b = "application/vnd.effect+json; profile=b";
    let mut endpoint = ApiEndpoint::new("create", Method::Post, "/create");
    endpoint.payloads.push((
        profile_a.to_owned(),
        Schema::as_unknown(Schema::struct_of(vec![("a".to_owned(), string())])),
    ));
    endpoint.payloads.push((
        profile_b.to_owned(),
        Schema::as_unknown(Schema::struct_of(vec![("b".to_owned(), string())])),
    ));
    let spec = SpecBuilder::new("Api").endpoint(endpoint).build().unwrap();
    let content = &spec["paths"]["/create"]["post"]["requestBody"]["content"];
    assert_eq!(
        content[profile_a]["schema"],
        serde_json::json!({
            "type": "object",
            "properties": {"a": {"type": "string"}},
            "required": ["a"],
            "additionalProperties": false,
        })
    );
    assert_eq!(
        content[profile_b]["schema"],
        serde_json::json!({
            "type": "object",
            "properties": {"b": {"type": "string"}},
            "required": ["b"],
            "additionalProperties": false,
        })
    );
}

#[test]
fn preserves_get_and_query_schemas_on_the_same_path() {
    // OpenApi.test.ts:267 — GET stays a method; QUERY nests under the
    // additional-operations map.
    fn summarize(mut operation: serde_json::Value) -> serde_json::Value {
        assert_eq!(
            operation["requestBody"]["content"]["application/json"]["schema"],
            serde_json::json!({"type": "string"})
        );
        operation["summary"] = serde_json::json!("Search");
        operation
    }
    let mut get = ApiEndpoint::new("list", Method::Get, "/search");
    get.success
        .push(json_response(Schema::as_unknown(Schema::checked(
            Schema::as_unknown(Schema::number()),
            "isFinite",
            vec![],
            vec![("expected", "a finite number")],
        ))));
    let mut query = ApiEndpoint::new("search", Method::Query, "/search");
    query
        .payloads
        .push(("application/json".to_owned(), string()));
    query
        .success
        .push(json_response(Schema::as_unknown(Schema::boolean())));
    query.transform = Some(summarize);
    let spec = SpecBuilder::new("Api")
        .endpoint(get)
        .endpoint(query)
        .build()
        .unwrap();
    assert_eq!(spec["openapi"], serde_json::json!("3.1.0"));
    let item = &spec["paths"]["/search"];
    assert_eq!(
        item.as_object().unwrap().keys().collect::<Vec<_>>(),
        vec!["get", "x-oai-additionalOperations"]
    );
    assert_eq!(
        item["get"]["responses"]["200"]["content"]["application/json"]["schema"],
        serde_json::json!({"type": "number"})
    );
    let query_op = &item["x-oai-additionalOperations"]["QUERY"];
    assert_eq!(query_op["summary"], serde_json::json!("Search"));
    assert_eq!(
        query_op["responses"]["200"]["content"]["application/json"]["schema"],
        serde_json::json!({"type": "boolean"})
    );
}

#[test]
fn emits_an_anyof_for_two_different_endpoint_errors_sharing_a_status() {
    // OpenApi.test.ts:725 — two identified errors on 500 become an anyOf.
    fn identified(name: &str, field: &str) -> Schema<Unknown> {
        Schema::as_unknown(Schema::annotated_with(
            Schema::struct_of(vec![(field.to_owned(), string())]),
            vec![("identifier", name)],
        ))
    }
    let mut endpoint = ApiEndpoint::new("error", Method::Get, "/error");
    endpoint
        .error
        .push(json_response(identified("FirstError", "code")));
    endpoint
        .error
        .push(json_response(identified("SecondError", "reason")));
    let spec = SpecBuilder::new("Api").endpoint(endpoint).build().unwrap();
    assert_eq!(
        spec["paths"]["/error"]["get"]["responses"]["500"]["content"]["application/json"]["schema"],
        serde_json::json!({
            "anyOf": [
                {"$ref": "#/components/schemas/FirstError"},
                {"$ref": "#/components/schemas/SecondError"}
            ]
        })
    );
}

#[test]
fn rejects_duplicate_method_and_path_pairs() {
    // OpenApi.test.ts:804
    let spec = SpecBuilder::new("Api")
        .endpoint(ApiEndpoint::new("first", Method::Get, "/shared"))
        .endpoint(ApiEndpoint::new("second", Method::Get, "/shared"))
        .build();
    assert_eq!(
        spec,
        Err(OpenApiError::DuplicateOperation {
            method: "GET".to_owned(),
            path: "/shared".to_owned(),
        })
    );
    assert_eq!(
        spec.unwrap_err().to_string(),
        "Duplicate OpenAPI operation for GET /shared"
    );
}

#[test]
fn rejects_equivalent_templated_method_and_path_pairs() {
    // OpenApi.test.ts:815 — `:id` and `:userId` template alike; the report
    // names the colliding endpoint's template.
    let spec = SpecBuilder::new("Api")
        .endpoint(ApiEndpoint::new("first", Method::Get, "/users/:id"))
        .endpoint(ApiEndpoint::new("second", Method::Get, "/users/:userId"))
        .build();
    assert_eq!(
        spec.unwrap_err().to_string(),
        "Duplicate OpenAPI operation for GET /users/{userId}"
    );
}

#[test]
fn rejects_duplicate_operation_identifiers() {
    // OpenApi.test.ts:826
    let mut first = ApiEndpoint::new("first", Method::Get, "/first");
    first.operation_id = Some("shared".to_owned());
    let mut second = ApiEndpoint::new("second", Method::Get, "/second");
    second.operation_id = Some("shared".to_owned());
    let spec = SpecBuilder::new("Api")
        .endpoint(first)
        .endpoint(second)
        .build();
    assert_eq!(
        spec.unwrap_err().to_string(),
        "Duplicate OpenAPI operationId: shared"
    );
}

#[test]
fn allows_operations_without_identifiers() {
    // OpenApi.test.ts:837 — a transform can drop the operationId entirely.
    fn drop_id(mut operation: serde_json::Value) -> serde_json::Value {
        operation
            .as_object_mut()
            .expect("operation")
            .remove("operationId");
        operation
    }
    let mut first = ApiEndpoint::new("first", Method::Get, "/first");
    first.transform = Some(drop_id);
    let mut second = ApiEndpoint::new("second", Method::Get, "/second");
    second.transform = Some(drop_id);
    let spec = SpecBuilder::new("Api")
        .endpoint(first)
        .endpoint(second)
        .build()
        .unwrap();
    for path in ["/first", "/second"] {
        let operation = &spec["paths"][path]["get"];
        assert!(operation.is_object());
        assert!(!operation.as_object().unwrap().contains_key("operationId"));
    }
}

fn secured(id: &str, path: &str, name: &str, scheme: SecurityScheme) -> ApiEndpoint {
    let mut endpoint = ApiEndpoint::new(id, Method::Get, path);
    endpoint.security.push((name.to_owned(), scheme));
    endpoint
}

#[test]
fn rejects_incompatible_security_schemes_with_the_same_name() {
    // OpenApi.test.ts:860 — a header key and a query key both named `auth`.
    let spec = SpecBuilder::new("Api")
        .endpoint(secured(
            "first",
            "/first",
            "auth",
            SecurityScheme::ApiKey {
                name: "x-api-key".to_owned(),
                location: "header".to_owned(),
            },
        ))
        .endpoint(secured(
            "second",
            "/second",
            "auth",
            SecurityScheme::ApiKey {
                name: "api_key".to_owned(),
                location: "query".to_owned(),
            },
        ))
        .build();
    assert_eq!(
        spec.unwrap_err().to_string(),
        "Conflicting OpenAPI security scheme: auth"
    );
}

#[test]
fn deduplicates_equivalent_security_schemes_with_the_same_name() {
    // OpenApi.test.ts:866
    let key = || SecurityScheme::ApiKey {
        name: "x-api-key".to_owned(),
        location: "header".to_owned(),
    };
    let spec = SpecBuilder::new("Api")
        .endpoint(secured("first", "/first", "auth", key()))
        .endpoint(secured("second", "/second", "auth", key()))
        .build()
        .unwrap();
    assert_eq!(
        spec["components"]["securitySchemes"]["auth"],
        serde_json::json!({"type": "apiKey", "name": "x-api-key", "in": "header"})
    );
}

#[test]
fn deduplicates_api_key_header_names_case_insensitively() {
    // OpenApi.test.ts:878 — the first declaration's casing wins.
    let spec = SpecBuilder::new("Api")
        .endpoint(secured(
            "first",
            "/first",
            "auth",
            SecurityScheme::ApiKey {
                name: "X-API-Key".to_owned(),
                location: "header".to_owned(),
            },
        ))
        .endpoint(secured(
            "second",
            "/second",
            "auth",
            SecurityScheme::ApiKey {
                name: "x-api-key".to_owned(),
                location: "header".to_owned(),
            },
        ))
        .build()
        .unwrap();
    assert_eq!(
        spec["components"]["securitySchemes"]["auth"],
        serde_json::json!({"type": "apiKey", "name": "X-API-Key", "in": "header"})
    );
}

#[test]
fn deduplicates_http_security_schemes_case_insensitively() {
    // OpenApi.test.ts:890 — `Token` then `token` collapse onto the first.
    let spec = SpecBuilder::new("Api")
        .endpoint(secured(
            "first",
            "/first",
            "auth",
            SecurityScheme::Http {
                scheme: "Token".to_owned(),
            },
        ))
        .endpoint(secured(
            "second",
            "/second",
            "auth",
            SecurityScheme::Http {
                scheme: "token".to_owned(),
            },
        ))
        .build()
        .unwrap();
    assert_eq!(
        spec["components"]["securitySchemes"]["auth"],
        serde_json::json!({"type": "http", "scheme": "Token"})
    );
}

#[test]
fn stores_proto_security_scheme_names_as_own_properties() {
    // OpenApi.test.ts:901 — maps hold `__proto__` as an ordinary key. (The
    // `Object.getPrototypeOf` half is JS-runtime-specific and has no port.)
    let spec = SpecBuilder::new("Api")
        .endpoint(secured(
            "first",
            "/first",
            "__proto__",
            SecurityScheme::ApiKey {
                name: "x-api-key".to_owned(),
                location: "header".to_owned(),
            },
        ))
        .build()
        .unwrap();
    let schemes = spec["components"]["securitySchemes"].as_object().unwrap();
    assert!(schemes.contains_key("__proto__"));
    assert_eq!(schemes.keys().collect::<Vec<_>>(), vec!["__proto__"]);
    assert_eq!(
        schemes["__proto__"],
        serde_json::json!({"type": "apiKey", "name": "x-api-key", "in": "header"})
    );
}

#[test]
fn rejects_incompatible_proto_security_schemes() {
    // OpenApi.test.ts:921
    let spec = SpecBuilder::new("Api")
        .endpoint(secured(
            "first",
            "/first",
            "__proto__",
            SecurityScheme::ApiKey {
                name: "x-api-key".to_owned(),
                location: "header".to_owned(),
            },
        ))
        .endpoint(secured(
            "second",
            "/second",
            "__proto__",
            SecurityScheme::ApiKey {
                name: "api_key".to_owned(),
                location: "query".to_owned(),
            },
        ))
        .build();
    assert_eq!(
        spec.unwrap_err().to_string(),
        "Conflicting OpenAPI security scheme: __proto__"
    );
}

#[test]
fn templates_action_suffixes_but_leaves_plain_colons_alone() {
    // Unit cover for `template_path` beyond the two row witnesses.
    assert_eq!(
        template_path("/operations/:id:wait"),
        "/operations/{id}:wait"
    );
    assert_eq!(template_path("/offers:resolve"), "/offers:resolve");
    assert_eq!(template_path("/users/:id"), "/users/{id}");
}

// OpenApiRepresentation.test.ts rows. The port has no `Schema.Date`
// constructor with a canonical codec, so declaration schemas ride on
// identified strings here: the compiler maps the `date` opaque kind to
// `{"type": "string"}` the same way, and every assertion below holds on the
// assembly (alias pairs, inlining, shared components), not on the codec.

fn declared(name: &str) -> Schema<Unknown> {
    Schema::as_unknown(Schema::annotated_with(
        Schema::string(),
        vec![("identifier", name)],
    ))
}

fn success_of(schema: Schema<Unknown>) -> ApiEndpoint {
    let mut endpoint = ApiEndpoint::new("date", Method::Get, "/date");
    endpoint.success.push(json_response(schema));
    endpoint
}

#[test]
fn uses_canonical_json_codecs_for_additional_declaration_schemas() {
    // OpenApiRepresentation.test.ts:6 — an additional declaration aliases its
    // `Encoded` form.
    let spec = SpecBuilder::new("Api")
        .additional_schema(declared("AdditionalDate"))
        .build()
        .unwrap();
    assert_eq!(
        spec["components"]["schemas"],
        serde_json::json!({
            "AdditionalDate": {"$ref": "#/components/schemas/AdditionalDateEncoded"},
            "AdditionalDateEncoded": {"type": "string"}
        })
    );
}

#[test]
fn applies_the_reference_policy_without_removing_additional_components() {
    // OpenApiRepresentation.test.ts:16 — the policy inlines the definition at
    // every use, but the additional component itself stays.
    let spec = SpecBuilder::new("Api")
        .additional_schema(declared("AdditionalDate"))
        .endpoint(success_of(declared("AdditionalDate")))
        .reference_policy(|_| false)
        .build()
        .unwrap();
    assert_eq!(
        spec["components"]["schemas"],
        serde_json::json!({"AdditionalDate": {"type": "string"}})
    );
    assert_eq!(
        spec["paths"]["/date"]["get"]["responses"]["200"]["content"]["application/json"]["schema"],
        serde_json::json!({"type": "string"})
    );
}

#[test]
fn uses_canonical_json_codecs_for_response_declaration_schemas() {
    // OpenApiRepresentation.test.ts:34 — a declaration response compiles to
    // its wire shape.
    let spec = SpecBuilder::new("Api")
        .endpoint(success_of(Schema::as_unknown(Schema::string())))
        .build()
        .unwrap();
    assert_eq!(
        spec["paths"]["/date"]["get"]["responses"]["200"]["content"]["application/json"]["schema"],
        serde_json::json!({"type": "string"})
    );
}

#[test]
fn projects_request_and_response_schemas_to_the_encoded_side() {
    // OpenApiRepresentation.test.ts:86 — `FiniteFromString` wires as a string.
    // (The compiler spells this opaque kind on the encoded side already.)
    let wire = Schema::as_unknown(Schema::finite_from_string());
    let mut endpoint = ApiEndpoint::new("create", Method::Post, "/create");
    endpoint
        .payloads
        .push(("application/json".to_owned(), wire.clone()));
    endpoint.success.push(json_response(wire));
    let spec = SpecBuilder::new("Api").endpoint(endpoint).build().unwrap();
    assert_eq!(
        spec["paths"]["/create"]["post"]["requestBody"]["content"]["application/json"]["schema"],
        serde_json::json!({"type": "string"})
    );
    assert_eq!(
        spec["paths"]["/create"]["post"]["responses"]["200"]["content"]["application/json"]["schema"],
        serde_json::json!({"type": "string"})
    );
}

#[test]
fn shares_definitions_and_returns_cached_copies_by_api_identity() {
    // OpenApiRepresentation.test.ts:131 — one shared definition, and every
    // build returns owned values, so two builds compare equal but mutate
    // independently.
    let shared = Schema::as_unknown(Schema::annotated_with(
        Schema::struct_of(vec![(
            "value".to_owned(),
            Schema::as_unknown(Schema::finite_from_string()),
        )]),
        vec![("identifier", "Shared")],
    ));
    let mut endpoint = ApiEndpoint::new("shared", Method::Post, "/shared");
    endpoint.payloads.push((
        "application/json".to_owned(),
        Schema::as_unknown(Schema::struct_of(vec![
            ("first".to_owned(), shared.clone()),
            ("second".to_owned(), shared.clone()),
        ])),
    ));
    endpoint.success.push(json_response(shared));
    let builder = SpecBuilder::new("Api").endpoint(endpoint);
    let first = builder.build().unwrap();
    let second = builder.build().unwrap();
    assert_eq!(first, second);
    assert_eq!(
        first["components"]["schemas"]["Shared"],
        serde_json::json!({
            "type": "object",
            "properties": {"value": {"type": "string"}},
            "required": ["value"],
            "additionalProperties": false,
        })
    );
    let mut owned = first.clone();
    owned["info"]["title"] = serde_json::json!("mutated");
    assert_eq!(second["info"]["title"], serde_json::json!("Api"));
}

#[test]
fn retains_referenced_parameter_schemas_without_the_parameter_container() {
    // OpenApiRepresentation.test.ts:155 — the query struct decomposes into
    // parameters; the `Query` container never emits itself.
    let filter = Schema::as_unknown(Schema::annotated_with(
        Schema::struct_of(vec![("value".to_owned(), string())]),
        vec![("identifier", "Filter")],
    ));
    let query = Schema::as_unknown(Schema::annotated_with(
        Schema::struct_of(vec![("filter".to_owned(), filter)]),
        vec![("identifier", "Query")],
    ));
    let mut endpoint = ApiEndpoint::new("list", Method::Get, "/list");
    endpoint.query_structs.push(query);
    let spec = SpecBuilder::new("Api").endpoint(endpoint).build().unwrap();
    assert_eq!(
        spec["paths"]["/list"]["get"]["parameters"],
        serde_json::json!([
            {"name": "filter", "in": "query", "required": true,
             "schema": {"$ref": "#/components/schemas/Filter"}}
        ])
    );
    let schemas = spec["components"]["schemas"].as_object().unwrap();
    assert_eq!(schemas.keys().collect::<Vec<_>>(), vec!["Filter"]);
    assert_eq!(
        schemas["Filter"],
        serde_json::json!({
            "type": "object",
            "properties": {"value": {"type": "string"}},
            "required": ["value"],
            "additionalProperties": false,
        })
    );
}

#[test]
fn caches_generated_specs_by_api_and_options_identity() {
    // OpenApiRepresentation.test.ts:179 — one build per (api, options) key; a
    // repeated key clones the stored document instead of rebuilding.
    fn build_spec() -> serde_json::Value {
        let mut endpoint = ApiEndpoint::new("value", Method::Get, "/value");
        endpoint.success.push(ResponseEntry {
            status: None,
            content_type: "application/json".to_owned(),
            schema: Schema::as_unknown(Schema::struct_of(vec![(
                "value".to_owned(),
                Schema::as_unknown(Schema::string()),
            )])),
        });
        SpecBuilder::new("Api").endpoint(endpoint).build().unwrap()
    }
    let cache = SpecCache::new();
    let calls = Cell::new(0);
    let first = cache.spec("Api", 1, || {
        calls.set(calls.get() + 1);
        build_spec()
    });
    assert!(calls.get() > 0);
    let calls_after_first = calls.get();
    let second = cache.spec("Api", 1, || {
        calls.set(calls.get() + 1);
        build_spec()
    });
    assert_eq!(calls.get(), calls_after_first);
    assert_eq!(first, second);
    cache.spec("Api", 2, || {
        calls.set(calls.get() + 1);
        build_spec()
    });
    assert!(calls.get() > calls_after_first);
}
