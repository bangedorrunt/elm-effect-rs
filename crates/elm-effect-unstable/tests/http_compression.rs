//! `packages/effect/test/http/HttpCompression.test.ts` witnesses (33 rows).
//!
//! Row 340 (incremental backpressure: first chunk arrives before a latch
//! release) is NOT covered here: it needs an async `Stream` runtime with
//! latch-gated interleaving, which this crate does not have.

use elm_effect_unstable::http_compression::{
    CompressionAlgorithm, CompressionOptions, CompBody, CompResponse, default_compressible,
    decompress_deflate, decompress_gzip, negotiate, parse_accept_encoding, supported_algorithms,
    vary_with, apply_compression,
};
use std::collections::HashSet;

fn big_json() -> Vec<u8> {
    "All work and no play makes Jack a dull boy. "
        .repeat(100)
        .as_bytes()
        .to_vec()
}

fn json_app() -> CompResponse {
    let body = big_json();
    let mut res = CompResponse {
        status: 200,
        headers: Vec::new(),
        body: CompBody::Bytes(body.clone()),
    };
    res.set_header("content-type", "application/json");
    res.set_header("content-length", &body.len().to_string());
    res
}

fn web_support() -> HashSet<CompressionAlgorithm> {
    supported_algorithms()
}

fn all_support() -> HashSet<CompressionAlgorithm> {
    HashSet::from([
        CompressionAlgorithm::Gzip,
        CompressionAlgorithm::Deflate,
        CompressionAlgorithm::Br,
        CompressionAlgorithm::Zstd,
    ])
}

fn get(res: CompResponse, accept: Option<&str>, method: &str) -> CompResponse {
    apply_compression(
        &CompressionOptions::default(),
        &web_support(),
        method,
        accept,
        res,
    )
}

// HttpCompression.test.ts:100 — Stream variant: override survives gzip.
#[test]
fn preserves_content_type_override_when_compressing_stream_response() {
    let mut original = json_app();
    original.set_header("content-type", "application/json");
    let compressed = get(original, Some("gzip"), "GET");
    assert_eq!(compressed.header("content-encoding"), Some("gzip"));
    assert_eq!(
        decompress_gzip(&compressed.bytes()).unwrap(),
        big_json()
    );
    assert_eq!(compressed.header("content-type"), Some("application/json"));
}

// HttpCompression.test.ts:100 — Uint8Array variant.
#[test]
fn preserves_content_type_override_when_compressing_bytes_response() {
    let mut original = json_app();
    original.set_header("content-type", "application/json");
    let compressed = get(original, Some("gzip"), "GET");
    assert_eq!(compressed.header("content-encoding"), Some("gzip"));
    assert_eq!(
        decompress_gzip(&compressed.bytes()).unwrap(),
        big_json()
    );
    assert_eq!(compressed.header("content-type"), Some("application/json"));
}

// HttpCompression.test.ts:100 — Raw variant (raw body with a text/plain type).
#[test]
fn preserves_content_type_override_when_compressing_raw_response() {
    let body = big_json();
    let mut res = CompResponse {
        status: 200,
        headers: Vec::new(),
        body: CompBody::Bytes(body.clone()),
    };
    res.set_header("content-type", "text/plain");
    res.set_header("content-type", "application/json");
    res.set_header("content-length", &body.len().to_string());
    let compressed = get(res, Some("gzip"), "GET");
    assert_eq!(compressed.header("content-encoding"), Some("gzip"));
    assert_eq!(
        decompress_gzip(&compressed.bytes()).unwrap(),
        big_json()
    );
    assert_eq!(compressed.header("content-type"), Some("application/json"));
}

// HttpCompression.test.ts:112 — header-only Content-Type on a raw response.
#[test]
fn preserves_header_only_content_type_for_raw_responses() {
    let mut plain = json_app();
    let uncompressed = get(plain.clone(), Some("identity"), "GET");
    assert_eq!(
        uncompressed.header("content-type"),
        Some("application/json")
    );
    assert_eq!(uncompressed.bytes(), big_json());
    plain = json_app();
    let compressed = get(plain, Some("gzip"), "GET");
    assert_eq!(compressed.header("content-encoding"), Some("gzip"));
    assert_eq!(
        decompress_gzip(&compressed.bytes()).unwrap(),
        big_json()
    );
    assert_eq!(
        compressed.header("content-type"),
        uncompressed.header("content-type")
    );
}

// HttpCompression.test.ts:128 — gzip happy path.
#[test]
fn compresses_a_compressible_response_with_gzip() {
    let compressed = get(json_app(), Some("gzip"), "GET");
    assert_eq!(compressed.status, 200);
    assert_eq!(compressed.header("content-encoding"), Some("gzip"));
    assert_eq!(compressed.header("vary"), Some("Accept-Encoding"));
    assert_eq!(compressed.header("content-length"), None);
    assert_eq!(
        decompress_gzip(&compressed.bytes()).unwrap(),
        big_json()
    );
}

// HttpCompression.test.ts:138 — deflate on request.
#[test]
fn compresses_with_deflate_when_requested() {
    let compressed = get(json_app(), Some("deflate"), "GET");
    assert_eq!(compressed.header("content-encoding"), Some("deflate"));
    assert_eq!(
        decompress_deflate(&compressed.bytes()).unwrap(),
        big_json()
    );
}

// HttpCompression.test.ts:145 — server order beats client order.
#[test]
fn server_preference_order_picks_br_over_gzip_when_supported() {
    let picked = negotiate(
        Some("gzip, br"),
        &CompressionOptions::default().algorithms,
        &all_support(),
    );
    assert_eq!(picked, Some(CompressionAlgorithm::Br));
}

// HttpCompression.test.ts:153 — q-values gate, server order ranks.
#[test]
fn client_q_values_decide_acceptability_only_server_order_ranks() {
    let picked = negotiate(
        Some("gzip;q=1, br;q=0.5"),
        &CompressionOptions::default().algorithms,
        &all_support(),
    );
    assert_eq!(picked, Some(CompressionAlgorithm::Br));
}

// HttpCompression.test.ts:161 — q=0 is unacceptable.
#[test]
fn q_zero_makes_a_coding_unacceptable() {
    let res = get(json_app(), Some("gzip;q=0"), "GET");
    assert_eq!(res.header("content-encoding"), None);
    assert_eq!(res.header("vary"), Some("Accept-Encoding"));
    assert_eq!(res.bytes(), big_json());
}

// HttpCompression.test.ts:168 — fall through past q=0.
#[test]
fn falls_through_server_order_past_q_zero_codings() {
    let res = get(json_app(), Some("gzip;q=0, deflate"), "GET");
    assert_eq!(res.header("content-encoding"), Some("deflate"));
}

// HttpCompression.test.ts:173 — wildcard matches.
#[test]
fn wildcard_matches_unlisted_codings() {
    let res = get(json_app(), Some("*"), "GET");
    assert_eq!(res.header("content-encoding"), Some("gzip"));
}

// HttpCompression.test.ts:178 — wildcard q=0 disables.
#[test]
fn wildcard_with_q_zero_disables_unlisted_codings() {
    let res = get(json_app(), Some("*;q=0"), "GET");
    assert_eq!(res.status, 200);
    assert_eq!(res.header("content-encoding"), None);
    assert_eq!(res.bytes(), big_json());
}

// HttpCompression.test.ts:185 — nothing acceptable: identity 200, never 406.
#[test]
fn sends_identity_with_200_when_nothing_is_acceptable() {
    let res = get(json_app(), Some("br, identity;q=0"), "GET");
    assert_eq!(res.status, 200);
    assert_eq!(res.header("content-encoding"), None);
    assert_eq!(res.header("vary"), Some("Accept-Encoding"));
    assert_eq!(res.bytes(), big_json());
}

// HttpCompression.test.ts:193 — malformed q means absent.
#[test]
fn treats_a_malformed_accept_encoding_as_absent() {
    assert_eq!(parse_accept_encoding("gzip;q=oops"), None);
    let res = get(json_app(), Some("gzip;q=oops"), "GET");
    assert_eq!(res.header("content-encoding"), None);
    assert_eq!(res.bytes(), big_json());
}

// HttpCompression.test.ts:199 — out-of-range q means absent.
#[test]
fn treats_an_out_of_range_q_value_as_absent() {
    assert_eq!(parse_accept_encoding("gzip;q=1.5"), None);
    let res = get(json_app(), Some("gzip;q=1.5"), "GET");
    assert_eq!(res.header("content-encoding"), None);
    assert_eq!(res.bytes(), big_json());
}

// HttpCompression.test.ts:205 — no header, no compression, but Vary.
#[test]
fn does_not_compress_without_an_accept_encoding_header() {
    let res = get(json_app(), None, "GET");
    assert_eq!(res.header("content-encoding"), None);
    assert_eq!(res.header("vary"), Some("Accept-Encoding"));
    assert_eq!(res.bytes(), big_json());
}

// HttpCompression.test.ts:212 — below minSize: skip, keep Vary + length.
#[test]
fn skips_bodies_below_min_size_but_still_sets_vary() {
    let mut res = CompResponse {
        status: 200,
        headers: Vec::new(),
        body: CompBody::Bytes(b"{}".to_vec()),
    };
    res.set_header("content-type", "application/json");
    res.set_header("content-length", "2");
    let out = get(res, Some("gzip"), "GET");
    assert_eq!(out.header("content-encoding"), None);
    assert_eq!(out.header("vary"), Some("Accept-Encoding"));
    assert_eq!(out.header("content-length"), Some("2"));
    assert_eq!(out.bytes(), b"{}".to_vec());
}

// HttpCompression.test.ts:221 — custom minSize.
#[test]
fn respects_a_custom_min_size() {
    let mut res = CompResponse {
        status: 200,
        headers: Vec::new(),
        body: CompBody::Bytes(b"{}".to_vec()),
    };
    res.set_header("content-type", "application/json");
    res.set_header("content-length", "2");
    let out = apply_compression(
        &CompressionOptions {
            min_size: 1,
            ..CompressionOptions::default()
        },
        &web_support(),
        "GET",
        Some("gzip"),
        res,
    );
    assert_eq!(out.header("content-encoding"), Some("gzip"));
}

// HttpCompression.test.ts:227 — malformed content lengths are ignored.
#[test]
fn ignores_malformed_content_lengths_when_checking_min_size() {
    for content_length in ["1e3", "1.5"] {
        let body = big_json();
        let mut res = CompResponse {
            status: 200,
            headers: Vec::new(),
            body: CompBody::Bytes(body),
        };
        res.set_header("content-type", "application/json");
        res.set_header("content-length", content_length);
        let out = get(res, Some("gzip"), "GET");
        assert_eq!(out.header("content-encoding"), Some("gzip"));
    }
}

// HttpCompression.test.ts:240 — incompressible type: skip, no Vary.
#[test]
fn skips_non_compressible_content_types_without_vary() {
    let mut res = CompResponse {
        status: 200,
        headers: Vec::new(),
        body: CompBody::Bytes(vec![0x89; 2048]),
    };
    res.set_header("content-type", "image/png");
    res.set_header("content-length", "2048");
    let out = get(res, Some("gzip"), "GET");
    assert_eq!(out.header("content-encoding"), None);
    assert_eq!(out.header("vary"), None);
}

// HttpCompression.test.ts:249 — custom predicate.
#[test]
fn supports_a_custom_compressible_predicate() {
    fn png_only(content_type: &str) -> bool {
        content_type == "image/png"
    }
    let mut res = CompResponse {
        status: 200,
        headers: Vec::new(),
        body: CompBody::Bytes(vec![0x89; 2048]),
    };
    res.set_header("content-type", "image/png");
    res.set_header("content-length", "2048");
    let out = apply_compression(
        &CompressionOptions {
            compressible: png_only,
            ..CompressionOptions::default()
        },
        &web_support(),
        "GET",
        Some("gzip"),
        res,
    );
    assert_eq!(out.header("content-encoding"), Some("gzip"));
}

// HttpCompression.test.ts:260 — pre-encoded responses pass through.
#[test]
fn leaves_already_encoded_responses_untouched() {
    let body = big_json();
    let mut res = CompResponse {
        status: 200,
        headers: Vec::new(),
        body: CompBody::Bytes(body.clone()),
    };
    res.set_header("content-type", "application/json");
    res.set_header("content-encoding", "br");
    res.set_header("content-length", &body.len().to_string());
    let out = get(res, Some("gzip"), "GET");
    assert_eq!(out.header("content-encoding"), Some("br"));
    assert_eq!(out.header("vary"), None);
    assert_eq!(out.bytes(), body);
}

// HttpCompression.test.ts:273 — null Raw body: no stamps at all.
#[test]
fn does_not_stamp_headers_when_a_raw_body_cannot_be_transformed() {
    let mut res = CompResponse {
        status: 200,
        headers: Vec::new(),
        body: CompBody::RawNull,
    };
    res.set_header("content-type", "application/json");
    res.set_header("etag", "\"abc\"");
    let out = get(res, Some("gzip"), "GET");
    assert_eq!(out.header("content-encoding"), None);
    assert_eq!(out.header("vary"), None);
    assert_eq!(out.header("etag"), Some("\"abc\""));
    assert!(out.bytes().is_empty());
}

// HttpCompression.test.ts:287 — identity opt-out is stripped.
#[test]
fn strips_a_content_encoding_identity_opt_out_before_sending() {
    let body = big_json();
    let mut res = CompResponse {
        status: 200,
        headers: Vec::new(),
        body: CompBody::Bytes(body.clone()),
    };
    res.set_header("content-type", "application/json");
    res.set_header("content-encoding", "identity");
    res.set_header("content-length", &body.len().to_string());
    let out = get(res, Some("gzip"), "GET");
    assert_eq!(out.header("content-encoding"), None);
    assert_eq!(out.bytes(), body);
}

// HttpCompression.test.ts:299 — Cache-Control: no-transform wins.
#[test]
fn honors_cache_control_no_transform() {
    let body = big_json();
    let mut res = CompResponse {
        status: 200,
        headers: Vec::new(),
        body: CompBody::Bytes(body.clone()),
    };
    res.set_header("content-type", "application/json");
    res.set_header("cache-control", "public, no-transform");
    res.set_header("content-length", &body.len().to_string());
    let out = get(res, Some("gzip"), "GET");
    assert_eq!(out.header("content-encoding"), None);
    assert_eq!(out.bytes(), body);
}

// HttpCompression.test.ts:311 — 204/304/206 pass through.
#[test]
fn leaves_204_304_and_206_responses_untouched() {
    for status in [204u16, 304] {
        let mut res = CompResponse {
            status,
            headers: Vec::new(),
            body: CompBody::Empty,
        };
        res.set_header("content-length", "0");
        let out = get(res, Some("gzip"), "GET");
        assert_eq!(out.status, status);
        assert_eq!(out.header("content-encoding"), None);
        assert_eq!(out.header("vary"), None);
    }
    let body = big_json();
    let mut partial = CompResponse {
        status: 206,
        headers: Vec::new(),
        body: CompBody::Bytes(body.clone()),
    };
    partial.set_header("content-type", "application/json");
    partial.set_header("content-length", &body.len().to_string());
    let out = get(partial, Some("gzip"), "GET");
    assert_eq!(out.status, 206);
    assert_eq!(out.header("content-encoding"), None);
    assert_eq!(out.bytes(), body);
}

// HttpCompression.test.ts:333 — HEAD mirrors GET headers, empty body.
#[test]
fn mirrors_get_headers_for_head_requests_without_a_body() {
    let out = get(json_app(), Some("gzip"), "HEAD");
    assert_eq!(out.header("content-encoding"), Some("gzip"));
    assert_eq!(out.header("vary"), Some("Accept-Encoding"));
    assert!(out.bytes().is_empty());
}

// HttpCompression.test.ts:377 — strong ETag weakens.
#[test]
fn weakens_a_strong_etag_on_compressed_responses() {
    let mut res = json_app();
    res.set_header("etag", "\"abc\"");
    let out = get(res, Some("gzip"), "GET");
    assert_eq!(out.header("etag"), Some("W/\"abc\""));
}

// HttpCompression.test.ts:388 — weak ETag stays.
#[test]
fn leaves_a_weak_etag_unchanged_on_compressed_responses() {
    let mut res = json_app();
    res.set_header("etag", "W/\"abc\"");
    let out = get(res, Some("gzip"), "GET");
    assert_eq!(out.header("etag"), Some("W/\"abc\""));
}

// HttpCompression.test.ts:399 — skipped responses keep their ETag.
#[test]
fn leaves_the_etag_of_skipped_responses_unchanged() {
    let mut res = json_app();
    res.set_header("etag", "\"abc\"");
    let out = get(res, Some("identity"), "GET");
    assert_eq!(out.header("etag"), Some("\"abc\""));
}

// HttpCompression.test.ts:410 — preferred-but-unsupported falls through.
#[test]
fn falls_through_to_a_supported_algorithm_when_the_preferred_is_unavailable() {
    let mut opts = CompressionOptions::default();
    opts.algorithms = vec![
        CompressionAlgorithm::Zstd,
        CompressionAlgorithm::Br,
        CompressionAlgorithm::Gzip,
    ];
    let out = apply_compression(&opts, &web_support(), "GET", Some("zstd, br, gzip"), json_app());
    assert_eq!(out.header("content-encoding"), Some("gzip"));
    assert_eq!(decompress_gzip(&out.bytes()).unwrap(), big_json());
}

// HttpCompression.test.ts:418 — Vary appends.
#[test]
fn appends_accept_encoding_to_an_existing_vary_header() {
    let mut res = json_app();
    res.set_header("vary", "Origin");
    let out = get(res, Some("gzip"), "GET");
    assert_eq!(out.header("vary"), Some("Origin, Accept-Encoding"));
}

// HttpCompression.test.ts:429 — Vary dedup + star.
#[test]
fn does_not_duplicate_accept_encoding_in_vary_and_leaves_vary_star_alone() {
    let mut existing = json_app();
    existing.set_header("vary", "accept-encoding");
    let out = get(existing, Some("gzip"), "GET");
    assert_eq!(out.header("vary"), Some("accept-encoding"));

    let mut star = json_app();
    star.set_header("vary", "*");
    let out = get(star, Some("gzip"), "GET");
    assert_eq!(out.header("vary"), Some("*"));
}

// HttpCompression.test.ts:453 — cookies survive.
#[test]
fn preserves_cookies_on_compressed_responses() {
    let mut res = json_app();
    res.headers
        .push(("set-cookie".to_owned(), "session=abc".to_owned()));
    let out = get(res, Some("gzip"), "GET");
    assert_eq!(out.header("content-encoding"), Some("gzip"));
    assert!(out
        .headers
        .iter()
        .any(|(n, v)| n == "set-cookie" && v == "session=abc"));
}

// Unit: default compressible predicate mirrors TS (text/*, json/+json, …).
#[test]
fn default_compressible_predicate_covers_text_and_structured_types() {
    assert!(default_compressible("text/plain"));
    assert!(default_compressible("text/event-stream"));
    assert!(default_compressible("application/json"));
    assert!(default_compressible("application/json; charset=utf-8"));
    assert!(default_compressible("application/ld+json"));
    assert!(default_compressible("application/xml"));
    assert!(default_compressible("image/svg+xml"));
    assert!(!default_compressible("image/png"));
    assert!(!default_compressible("application/octet-stream"));
}

// Unit: vary helper keeps star and existing dimensions verbatim.
#[test]
fn vary_with_keeps_star_and_matching_dimensions_verbatim() {
    assert_eq!(vary_with(None, "Accept-Encoding"), "Accept-Encoding");
    assert_eq!(vary_with(Some("*"), "Accept-Encoding"), "*");
    assert_eq!(
        vary_with(Some("accept-encoding"), "Accept-Encoding"),
        "accept-encoding"
    );
    assert_eq!(
        vary_with(Some("Origin"), "Accept-Encoding"),
        "Origin, Accept-Encoding"
    );
}
