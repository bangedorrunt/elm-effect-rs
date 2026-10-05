//! HttpApiSecurity credential parsing: bearer, custom schemes, Basic.

#[path = "../src/http_auth.rs"]
mod http_auth;

use http_auth::{decode_basic, decode_scheme_token};

#[test]
fn decodes_bearer_token_trimmed() {
    assert_eq!(
        decode_scheme_token("Bearer abc123", "bearer"),
        Some("abc123".to_string())
    );
}

#[test]
fn decodes_custom_scheme_trimmed() {
    assert_eq!(
        decode_scheme_token("Token abc123", "Token"),
        Some("abc123".to_string())
    );
}

#[test]
fn matches_schemes_case_insensitively() {
    assert_eq!(
        decode_scheme_token("bEaReR abc123", "bearer"),
        Some("abc123".to_string())
    );
}

#[test]
fn accepts_multiple_spaces_before_credentials() {
    assert_eq!(
        decode_scheme_token("Bearer   abc123", "bearer"),
        Some("abc123".to_string())
    );
}

#[test]
fn rejects_mismatched_and_malformed_schemes() {
    assert_eq!(decode_scheme_token("Basic abc123", "bearer"), None);
    assert_eq!(decode_scheme_token("Bearerabc123", "bearer"), None);
}

#[test]
fn decodes_basic_at_first_colon() {
    let credential = decode_basic("Basic YWxpY2U6c2VjcmV0OndpdGg6Y29sb25z");
    assert_eq!(credential.username, "alice");
    assert_eq!(credential.password, "secret:with:colons");
}

#[test]
fn rejects_basic_from_other_schemes_empty() {
    let credential = decode_basic("Bearer YWxpY2U6c2VjcmV0");
    assert_eq!(credential.username, "");
    assert_eq!(credential.password, "");
}
