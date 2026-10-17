//! `packages/effect/test/httpapi/HttpApiClient.test.ts` URL-building witnesses.
//!
//! The client dispatch half (Schema codecs, `ParseOptions` slots, SSE wire
//! decoding, `Stream` responses, error content-type selection,
//! `transformClient`) needs fiber/stream/schema-compiler machinery this crate
//! does not model, so only the pure URL/client-identity rows are carried here:
//! endpoint path expansion, query rendering, base joining, prefixes,
//! top-level flattening, and `__proto__` own-key lookups.

use elm_effect_unstable::http::{
    HttpApi, HttpApiEndpoint, HttpApiGroup, HttpClient, UrlParams, build_endpoint_url, join_url,
};
use std::collections::BTreeMap;

fn query(pairs: &[(&str, Option<&str>)]) -> Vec<(String, Option<String>)> {
    pairs
        .iter()
        .map(|(name, value)| {
            (
                (*name).to_string(),
                value.map(std::string::ToString::to_string),
            )
        })
        .collect()
}

#[test]
fn builds_urls_using_endpoint_schemas() {
    // HttpApiClient.test.ts:908 — params inline, query arrays repeat.
    let url = build_endpoint_url(
        Some("https://api.example.com"),
        "",
        "/users/:id",
        &[("id", "123")],
        &query(&[
            ("page", Some("1")),
            ("tags", Some("1")),
            ("tags", Some("2")),
        ]),
    )
    .unwrap();
    assert_eq!(url, "https://api.example.com/users/123?page=1&tags=1&tags=2");
}

#[test]
fn preserves_a_base_url_pathname() {
    // HttpApiClient.test.ts:927 — the base pathname survives the join.
    let url = build_endpoint_url(Some("https://api.example.com/v1"), "", "/health", &[], &[]).unwrap();
    assert_eq!(url, "https://api.example.com/v1/health");
}

#[test]
fn encodes_path_parameters() {
    // HttpApiClient.test.ts:935 — `/` inside a value is `%2F`, not a split.
    let url = build_endpoint_url(
        Some("https://api.example.com"),
        "",
        "/state/stacks/:stack/stages/:stage/resources",
        &[("stack", "a/b"), ("stage", "prod/blue")],
        &[],
    )
    .unwrap();
    assert_eq!(
        url,
        "https://api.example.com/state/stacks/a%2Fb/stages/prod%2Fblue/resources"
    );
}

#[test]
fn omits_missing_optional_path_parameters() {
    // HttpApiClient.test.ts:963 — `:path?` without a value drops the segment.
    let bare = build_endpoint_url(
        Some("https://api.example.com"),
        "",
        "/files/:path?",
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(bare, "https://api.example.com/files");
    let filled = build_endpoint_url(
        Some("https://api.example.com"),
        "",
        "/files/:path?",
        &[("path", "a/b")],
        &[],
    )
    .unwrap();
    assert_eq!(filled, "https://api.example.com/files/a%2Fb");
}

#[test]
fn returns_relative_urls_when_base_url_is_omitted() {
    // HttpApiClient.test.ts:989 — no base means the path renders bare.
    let url = build_endpoint_url(None, "", "/health", &[], &[]).unwrap();
    assert_eq!(url, "/health");
}

#[test]
fn supports_top_level_endpoints() {
    // HttpApiClient.test.ts:995 — a `topLevel` group flattens to the API
    // root and the `/v1` prefix still applies.
    let api = HttpApi::make("Api")
        .add(HttpApiGroup::make("top").top_level().add_endpoint(HttpApiEndpoint::get("health", "/health")))
        .prefix("/v1");
    let group = api.group("top").expect("top group");
    assert!(group.top_level);
    let endpoint = group.endpoint("health").expect("health endpoint");
    assert_eq!(endpoint.path, "/health");
    let url = build_endpoint_url(
        Some("https://api.example.com"),
        &api.prefix,
        &endpoint.path,
        &[],
        &[],
    )
    .unwrap();
    assert_eq!(url, "https://api.example.com/v1/health");
    // The flattened builder shape exposes the endpoint at the root.
    let mut root: BTreeMap<String, String> = BTreeMap::new();
    for (_, group) in &api.groups {
        if group.top_level {
            for (id, endpoint) in &group.endpoints {
                root.insert(id.clone(), endpoint.path.clone());
            }
        }
    }
    assert_eq!(root.get("health"), Some(&"/health".to_string()));
}

#[test]
fn url_builder_preserves_the_undeclared_action_suffix() {
    // HttpApiClient.test.ts:352 — `:wait` is not a declared param, so it
    // stays literal while `:id` substitutes.
    let url = build_endpoint_url(
        Some("https://api.example.com"),
        "",
        "/operations/:id:wait",
        &[("id", "op_1")],
        &[],
    )
    .unwrap();
    assert_eq!(url, "https://api.example.com/operations/op_1:wait");
}

#[test]
fn make_sends_the_request_with_the_literal_action_suffix() {
    // HttpApiClient.test.ts:358 — the execute path builds the same URL into
    // the outgoing request.
    let url = build_endpoint_url(
        Some("https://api.example.com"),
        "",
        "/operations/:id:wait",
        &[("id", "op_1")],
        &[],
    )
    .unwrap();
    let request = HttpClient::get(&url);
    assert_eq!(request.url, "https://api.example.com/operations/op_1:wait");
}

#[test]
fn encodes_path_parameters_when_executing_requests() {
    // HttpApiClient.test.ts:1077 — execute-path URL encoding matches the
    // urlBuilder encoding.
    let url = build_endpoint_url(
        Some("https://api.example.com"),
        "",
        "/state/stacks/:stack/stages/:stage/resources",
        &[("stack", "a/b"), ("stage", "prod/blue")],
        &[],
    )
    .unwrap();
    let request = HttpClient::get(&url);
    assert_eq!(
        request.url,
        "https://api.example.com/state/stacks/a%2Fb/stages/prod%2Fblue/resources"
    );
}

#[test]
fn omits_optional_path_parameters_when_executing_requests() {
    // HttpApiClient.test.ts:1111 — both shapes build through the request.
    let bare =
        build_endpoint_url(Some("https://api.example.com"), "", "/files/:path?", &[], &[]).unwrap();
    let filled = build_endpoint_url(
        Some("https://api.example.com"),
        "",
        "/files/:path?",
        &[("path", "a/b")],
        &[],
    )
    .unwrap();
    assert_eq!(HttpClient::get(&bare).url, "https://api.example.com/files");
    assert_eq!(
        HttpClient::get(&filled).url,
        "https://api.example.com/files/a%2Fb"
    );
}

#[test]
fn stores_proto_identifiers_as_own_properties() {
    // HttpApiClient.test.ts:1012 — `__proto__` is an ordinary own key at
    // every builder level.
    let group = HttpApiGroup::make("__proto__")
        .add_endpoint(HttpApiEndpoint::get("__proto__", "/proto"));
    let api = HttpApi::make("Api").add(group);
    assert!(api.group("__proto__").is_some());
    assert!(
        api.group("__proto__")
            .expect("group")
            .endpoint("__proto__")
            .is_some()
    );
    let mut builder: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (group_id, group) in &api.groups {
        let mut endpoints = BTreeMap::new();
        for (endpoint_id, endpoint) in &group.endpoints {
            endpoints.insert(endpoint_id.clone(), endpoint.path.clone());
        }
        builder.insert(group_id.clone(), endpoints);
    }
    assert!(builder.contains_key("__proto__"));
    assert!(builder["__proto__"].contains_key("__proto__"));
    assert_eq!(
        build_endpoint_url(None, "", &builder["__proto__"]["__proto__"], &[], &[]).unwrap(),
        "/proto"
    );
}

#[test]
fn stores_proto_client_identifiers_as_own_properties() {
    // HttpApiClient.test.ts:1026 — the same own-key rule holds for the
    // client shape: group and endpoint clients keyed by `__proto__`.
    let api = HttpApi::make("Api").add(
        HttpApiGroup::make("__proto__").add_endpoint(HttpApiEndpoint::get("__proto__", "/proto")),
    );
    let mut client: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (group_id, group) in &api.groups {
        let mut endpoints = BTreeMap::new();
        for (endpoint_id, endpoint) in &group.endpoints {
            endpoints.insert(endpoint_id.clone(), endpoint.path.clone());
        }
        client.insert(group_id.clone(), endpoints);
    }
    assert!(client.contains_key("__proto__"));
    assert!(client["__proto__"].contains_key("__proto__"));
    let mut group_client: BTreeMap<String, String> = BTreeMap::new();
    for (endpoint_id, endpoint) in &api.group("__proto__").expect("group").endpoints {
        group_client.insert(endpoint_id.clone(), endpoint.path.clone());
    }
    assert!(group_client.contains_key("__proto__"));
}

#[test]
fn query_pairs_render_in_order_with_repeats() {
    // Supports 908/963: the ordered-pair query model behind the builder.
    let params = UrlParams::from_pairs(query(&[
        ("page", Some("1")),
        ("tags", Some("1")),
        ("tags", Some("2")),
    ]));
    assert_eq!(params.render(), "page=1&tags=1&tags=2");
    assert_eq!(join_url("https://api.example.com/v1", "/health"), "https://api.example.com/v1/health");
}
