//! HTTP stack fence: client, api routes, net addresses.
//! Mirrors packages/effect/test/http/, http-api/ and net/ behaviors
//! that do not need a live server (see residuals for the rest).

use elm_effect_unstable::http::{Api, HttpClient, HttpMethod, NetAddr};

#[test]
fn client_builds_get_requests() {
    let request = HttpClient::get("https://example.com/items?page=2");
    assert_eq!(request.method, HttpMethod::GET);
    assert_eq!(request.url, "https://example.com/items?page=2");
    assert!(request.headers.is_empty());
}

#[test]
fn client_post_carries_body() {
    let request = HttpClient::post("https://example.com/items", r#"{"a":1}"#);
    assert_eq!(request.method, HttpMethod::POST);
    assert_eq!(request.body, Some(br#"{"a":1}"#.to_vec()));
}

#[test]
fn client_headers_accumulate() {
    let request = HttpClient::get("https://example.com")
        .header("accept", "application/json")
        .header("x-n", "1");
    assert_eq!(request.headers.len(), 2);
    assert_eq!(
        request.headers[0],
        ("accept".to_string(), "application/json".to_string())
    );
}

#[test]
fn api_routes_match_method_and_path() {
    let api = Api::new().route(HttpMethod::GET, "/items", "list").route(
        HttpMethod::POST,
        "/items",
        "create",
    );
    assert_eq!(api.lookup(&HttpMethod::GET, "/items"), Some("list"));
    assert_eq!(api.lookup(&HttpMethod::DELETE, "/items"), None);
    assert_eq!(api.lookup(&HttpMethod::GET, "/missing"), None);
}

#[test]
fn api_path_params_extract() {
    let api = Api::new().route(HttpMethod::GET, "/items/:id", "one");
    assert_eq!(
        api.lookup(&HttpMethod::GET, "/items/42"),
        Some("one"),
        "param segments match and the route resolves"
    );
    assert_eq!(api.lookup(&HttpMethod::GET, "/items"), None);
}

#[test]
fn net_addr_parses_host_port() {
    let addr = NetAddr::parse("example.com:8080").unwrap();
    assert_eq!(addr.host, "example.com");
    assert_eq!(addr.port, 8080);
    assert!(NetAddr::parse("no-port").is_err());
    assert!(NetAddr::parse("bad:port").is_err());
}
