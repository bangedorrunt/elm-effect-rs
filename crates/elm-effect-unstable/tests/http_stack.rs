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

#[test]
fn headers_are_case_insensitive_multimap() {
    use elm_effect_unstable::http::Headers;
    let mut headers = Headers::new();
    headers.append("X-A", "1");
    headers.append("x-a", "2");
    assert_eq!(headers.get("X-A"), Some("2"));
    assert_eq!(headers.get_all("x-a"), vec!["1", "2"]);
    headers.set("X-A", "3");
    assert_eq!(headers.get_all("X-A"), vec!["3"]);
    assert!(headers.has("x-a"));
    assert!(headers.remove("X-A"));
    assert!(!headers.has("x-a"));
}

#[test]
fn cookies_parse_header_and_set_cookie() {
    use elm_effect_unstable::http::Cookie;
    let pairs = Cookie::parse_header("a=1; b=\"two\"");
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[1].value, "two");
    let set = Cookie::parse_set_cookie("id=7; Path=/; HttpOnly; SameSite=Lax; Max-Age=60").unwrap();
    assert_eq!(set.path.as_deref(), Some("/"));
    assert!(set.http_only && !set.secure);
    assert_eq!(set.max_age, Some(60));
    assert!(set.to_set_cookie().contains("HttpOnly"));
}

#[test]
fn etag_renders_and_matches_weakly() {
    use elm_effect_unstable::http::Etag;
    let strong = Etag::strong_for(b"body");
    assert!(strong.render().starts_with('"'));
    let weak = Etag::weak_for(b"body");
    assert!(weak.render().starts_with("W/"));
    assert!(strong.is_fresh(&weak.render()));
    assert!(strong.is_fresh("*"));
    assert!(!strong.is_fresh("\"other\""));
}

#[test]
fn http_body_renders_with_content_type() {
    use elm_effect_unstable::http::HttpBody;
    assert_eq!(HttpBody::Empty.render(), Vec::<u8>::new());
    assert_eq!(HttpBody::Text("hi".into()).content_type(), Some("text/plain; charset=utf-8"));
    assert_eq!(
        HttpBody::Form(vec![("a".into(), "b c".into())]).render(),
        b"a=b+c".to_vec()
    );
}
