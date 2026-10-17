//! `packages/effect/test/http/HttpMiddleware.test.ts` CORS + logger witnesses.

use elm_effect_unstable::http_server::{CorsOptions, CorsRequest, apply_cors, log_annotations};

fn allowed() -> CorsOptions {
    CorsOptions::allow_origins(["https://client.example", "https://other.example"])
}

fn get(origin: &str) -> CorsRequest {
    CorsRequest { origin: Some(origin.to_owned()), is_preflight: false }
}

#[test]
fn adds_origin_when_vary_is_absent() {
    // HttpMiddleware.test.ts:19 — no Vary on the response yet.
    let (allow_origin, vary) = apply_cors(&allowed(), &get("https://client.example"), None);
    assert_eq!(allow_origin.as_deref(), Some("https://client.example"));
    assert_eq!(vary.as_deref(), Some("Origin"));
}

#[test]
fn preserves_existing_vary_dimensions_when_adding_origin() {
    // HttpMiddleware.test.ts:19
    let (allow_origin, vary) =
        apply_cors(&allowed(), &get("https://client.example"), Some("Accept-Language"));
    assert_eq!(allow_origin.as_deref(), Some("https://client.example"));
    assert_eq!(vary.as_deref(), Some("Accept-Language, Origin"));
}

#[test]
fn preserves_other_dimensions_without_duplicating_a_mixed_case_origin() {
    // HttpMiddleware.test.ts:19 — oRiGiN already covers Origin.
    let (allow_origin, vary) = apply_cors(
        &allowed(),
        &get("https://client.example"),
        Some("Accept-Language, oRiGiN"),
    );
    assert_eq!(allow_origin.as_deref(), Some("https://client.example"));
    assert_eq!(vary.as_deref(), Some("Accept-Language, oRiGiN"));
}

#[test]
fn preserves_wildcard_vary() {
    // HttpMiddleware.test.ts:19
    let (allow_origin, vary) = apply_cors(&allowed(), &get("https://client.example"), Some("*"));
    assert_eq!(allow_origin.as_deref(), Some("https://client.example"));
    assert_eq!(vary.as_deref(), Some("*"));
}

#[test]
fn preserves_both_vary_dimensions_for_preflight_requests() {
    // HttpMiddleware.test.ts:60 — OPTIONS preflight varies on both dimensions.
    let request = CorsRequest {
        origin: Some("https://client.example".to_owned()),
        is_preflight: true,
    };
    let (_, vary) = apply_cors(&allowed(), &request, None);
    let mut dimensions: Vec<String> = vary
        .unwrap()
        .split(',')
        .map(|member| member.trim().to_lowercase())
        .collect();
    dimensions.sort();
    assert_eq!(dimensions, vec!["access-control-request-headers", "origin"]);
}

#[test]
fn varies_by_origin_when_the_request_origin_is_rejected() {
    // HttpMiddleware.test.ts:82 — rejected origin: no allow-origin, Vary: Origin.
    let (allow_origin, vary) = apply_cors(&allowed(), &get("https://rejected.example"), None);
    assert_eq!(allow_origin, None);
    assert_eq!(vary.as_deref(), Some("Origin"));
}

#[test]
fn preserves_vary_when_allowing_all_origins() {
    // HttpMiddleware.test.ts:99 — allow-all echoes `*` and leaves Vary alone.
    let (allow_origin, vary) = apply_cors(
        &CorsOptions::allow_all(),
        &get("https://client.example"),
        Some("Accept-Language"),
    );
    assert_eq!(allow_origin.as_deref(), Some("*"));
    assert_eq!(vary.as_deref(), Some("Accept-Language"));
}

#[test]
fn annotates_method_path_and_status_without_query_or_hash() {
    // HttpMiddleware.test.ts:119
    let annotations = log_annotations("GET", "http://localhost:3000/todos/1?foo=bar#top", 204);
    assert_eq!(
        annotations,
        vec![
            ("http.method".to_owned(), "GET".to_owned()),
            ("http.url".to_owned(), "/todos/1".to_owned()),
            ("http.status".to_owned(), "204".to_owned()),
        ]
    );
}
