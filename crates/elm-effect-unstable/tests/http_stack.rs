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

#[test]
fn url_params_roundtrip_ordered_pairs() {
    use elm_effect_unstable::http::UrlParams;
    let params = UrlParams::parse("?b=2&a=1&b=3&flag");
    assert_eq!(
        params.pairs(),
        &[
            ("b".to_string(), Some("2".to_string())),
            ("a".to_string(), Some("1".to_string())),
            ("b".to_string(), Some("3".to_string())),
            ("flag".to_string(), None),
        ]
    );
    assert_eq!(params.render(), "b=2&a=1&b=3&flag");
    assert_eq!(UrlParams::parse(&params.render()), params);
}

#[test]
fn url_params_first_last_and_set_replace() {
    use elm_effect_unstable::http::UrlParams;
    let mut params = UrlParams::parse("b=2&b=3");
    assert_eq!(params.get_first("b"), Some("2"));
    assert_eq!(params.get_last("b"), Some("3"));
    assert_eq!(params.get_first("missing"), None);
    params.set("b", Some("9".to_string()));
    assert_eq!(params.pairs(), &[("b".to_string(), Some("9".to_string()))]);
    assert!(params.remove("b"));
    assert!(!params.remove("b"));
}

#[test]
fn ipnetwork_parses_strict_cidr() {
    use elm_effect_unstable::net::IpNetwork;
    let net = IpNetwork::parse("192.168.0.0/24").unwrap();
    assert_eq!(net.render(), "192.168.0.0/24");
    assert_eq!(net.prefix(), 24);
    assert!(IpNetwork::parse("192.168.0.1/24").is_none());
    assert!(IpNetwork::parse("192.168.0.0/33").is_none());
    assert!(IpNetwork::parse("nope").is_none());
}

#[test]
fn ipnetwork_first_last_count() {
    use elm_effect_unstable::net::IpNetwork;
    let net = IpNetwork::parse("10.0.0.0/24").unwrap();
    assert_eq!(net.first().to_string(), "10.0.0.0");
    assert_eq!(net.last().to_string(), "10.0.0.255");
    assert_eq!(net.address_count(), 256);
}

#[test]
fn ipnetwork_contains_addr_and_network() {
    use elm_effect_unstable::net::IpNetwork;
    use std::net::IpAddr;
    let outer = IpNetwork::parse("10.0.0.0/8").unwrap();
    let inner = IpNetwork::parse("10.1.0.0/16").unwrap();
    let addr: IpAddr = "10.1.2.3".parse().unwrap();
    let outside: IpAddr = "11.0.0.1".parse().unwrap();
    let v6: IpAddr = "::1".parse().unwrap();
    assert!(outer.contains_addr(addr));
    assert!(!outer.contains_addr(outside));
    assert!(!outer.contains_addr(v6));
    assert!(outer.contains_network(&inner));
    assert!(!inner.contains_network(&outer));
}

#[test]
fn url_parses_absolute_and_renders() {
    use elm_effect_unstable::http::Url;
    let url = Url::parse("https://user:pass@example.com:8443/a/b?q=1#frag").unwrap();
    assert_eq!(url.scheme, "https");
    assert_eq!(url.host, "example.com");
    assert_eq!(url.port, Some(8443));
    assert_eq!(url.path, "/a/b");
    assert_eq!(url.query, "q=1");
    assert_eq!(url.fragment, "frag");
    assert_eq!(url.render(), "https://user:pass@example.com:8443/a/b?q=1#frag");
    assert_eq!(Url::parse("https://example.com").unwrap().path, "/");
}

#[test]
fn url_rejects_garbage() {
    use elm_effect_unstable::http::Url;
    assert!(Url::parse("not a url").is_none());
    assert!(Url::parse(":///missing").is_none());
    assert!(Url::parse("https://example.com:abc/").is_none());
    assert!(Url::parse("https://").is_none());
}

#[test]
fn url_resolves_relative_against_base() {
    use elm_effect_unstable::http::Url;
    let base = Url::parse("https://example.com/dir/page?q=1").unwrap();
    assert_eq!(base.resolve("/other").unwrap().render(), "https://example.com/other");
    assert_eq!(
        base.resolve("next?x=2").unwrap().render(),
        "https://example.com/dir/next?x=2"
    );
    assert_eq!(
        base.resolve("https://other.com/y").unwrap().render(),
        "https://other.com/y"
    );
}

#[test]
fn url_query_params_roundtrip() {
    use elm_effect_unstable::http::{Url, UrlParams};
    let mut url = Url::parse("https://example.com/s?a=1").unwrap();
    let mut params = url.url_params();
    params.append("b".to_string(), Some("2".to_string()));
    url.set_url_params(&params);
    assert_eq!(url.query, "a=1&b=2");
    let appended = Url::parse("https://example.com/s?a=1#frag").unwrap();
    assert_eq!(appended.render(), "https://example.com/s?a=1#frag");
}

#[test]
fn url_setters_mutate_parts() {
    use elm_effect_unstable::http::Url;
    let url = Url::parse("https://user:pass@example.com:8443/a?q=1#frag").unwrap().mutate(|u| {
        u.set_hash("other");
        u.set_pathname("b");
        u.set_search("?x=2");
        u.set_port(None);
        u.set_protocol("http:");
        u.set_username("admin");
        u.set_password("s3cret");
    });
    assert_eq!(url.render(), "http://admin:s3cret@example.com/b?x=2#other");
    assert!(url.redacted().contains("admin:***@"));
    assert!(!url.redacted().contains("s3cret"));
}

#[test]
fn url_host_setters_split_hostname_and_port() {
    use elm_effect_unstable::http::Url;
    let mut url = Url::parse("https://example.com:8443/a").unwrap();
    url.set_hostname("other.com");
    assert_eq!(url.port, Some(8443));
    url.set_host("third.com:9000");
    assert_eq!(url.host.as_str(), "third.com");
    assert_eq!(url.port, Some(9000));
    let before = url.clone();
    url.set_href("not a url");
    assert_eq!(url, before);
    url.set_href("http://plain.com/");
    assert_eq!(url.render(), "http://plain.com/");
}

#[test]
fn url_paths_join_without_doubling_slashes() {
    use elm_effect_unstable::http::join_url;
    assert_eq!(join_url("https://a.com/api", "users"), "https://a.com/api/users");
    assert_eq!(join_url("https://a.com/api", "/users"), "https://a.com/api/users");
    assert_eq!(join_url("https://a.com/api/", "users"), "https://a.com/api/users");
    assert_eq!(join_url("https://a.com/api", ""), "https://a.com/api");
}

#[test]
fn request_headers_remove_and_missing_noop() {
    let mut request = elm_effect_unstable::http::HttpClient::get("https://a.com/")
        .header("X-A", "1")
        .header("x-a", "2");
    assert!(request.remove_header("X-A"));
    assert!(!request.remove_header("X-A"));
    assert!(!request.remove_header("absent"));
    assert!(request.headers.is_empty());
}

#[test]
fn request_hash_stores_option_and_removes() {
    let mut request = elm_effect_unstable::http::HttpClient::get("https://a.com/");
    assert_eq!(request.hash, None);
    request.set_hash("frag");
    assert_eq!(request.hash, Some("frag".to_string()));
    request.remove_hash();
    assert_eq!(request.hash, None);
}

#[test]
fn ipinterface_keeps_host_bits() {
    use elm_effect_unstable::net::IpInterface;
    let iface = IpInterface::parse("192.168.1.7/24").unwrap();
    assert_eq!(iface.address().to_string(), "192.168.1.7");
    assert_eq!(iface.render(), "192.168.1.7/24");
    assert_eq!(iface.network().render(), "192.168.1.0/24");
    assert!(IpInterface::parse("192.168.1.7/33").is_none());
    assert!(IpInterface::parse("nope").is_none());
}

#[test]
fn ipinterface_bare_defaults_full_width() {
    use elm_effect_unstable::net::{IpInterface, IpNetwork};
    use std::net::IpAddr;
    let v4: IpAddr = "10.0.0.9".parse().unwrap();
    let v6: IpAddr = "::1".parse().unwrap();
    assert_eq!(IpInterface::from_addr(v4).prefix(), 32);
    assert_eq!(IpInterface::from_addr(v6).prefix(), 128);
    let net = IpNetwork::parse("10.0.0.0/8").unwrap();
    assert!(net.contains_addr(IpInterface::from_addr(v4).address()));
    assert_eq!(IpInterface::from_addr(v4).network(), IpNetwork::parse("10.0.0.9/32").unwrap());
}

#[test]
fn strict_ip_parses_widths_and_octets() {
    use elm_effect_unstable::net::StrictIp;
    let v4 = StrictIp::parse("192.168.1.7").unwrap();
    assert_eq!(v4.width(), 32);
    assert_eq!(v4.octets(), vec![192, 168, 1, 7]);
    assert_eq!(v4.render(), "192.168.1.7");
    let v6 = StrictIp::parse("2001:db8::1").unwrap();
    assert_eq!(v6.width(), 128);
    assert_eq!(v6.octets().len(), 16);
    assert_eq!(v6.render(), "2001:db8::1");
}

#[test]
fn strict_ip_rejects_malformed() {
    use elm_effect_unstable::net::StrictIp;
    for bad in ["", "1.2.3", "1.2.3.4.5", "256.1.1.1", "01.2.3.4", "1.2.3.x", " 1.2.3.4", "1.2.3.4 ", ":::"] {
        assert!(StrictIp::parse(bad).is_none(), "{bad}");
    }
}

#[test]
fn strict_ip_classifies_bits() {
    use elm_effect_unstable::net::{IpClass, StrictIp};
    assert_eq!(StrictIp::parse("127.0.0.1").unwrap().classify(), IpClass::Loopback);
    assert_eq!(StrictIp::parse("10.1.2.3").unwrap().classify(), IpClass::Private);
    assert_eq!(StrictIp::parse("224.0.0.1").unwrap().classify(), IpClass::Multicast);
    assert_eq!(StrictIp::parse("8.8.8.8").unwrap().classify(), IpClass::Global);
    assert_eq!(StrictIp::parse("::1").unwrap().classify(), IpClass::Loopback);
}

#[test]
fn cookie_expired_renders_cleared_set_cookie() {
    use elm_effect_unstable::http::Cookie;
    let expired = Cookie::expired("session");
    let rendered = expired.to_set_cookie();
    assert!(rendered.contains("Max-Age=0"), "{rendered}");
    assert!(rendered.contains("Expires="), "{rendered}");
    let back = Cookie::parse_set_cookie(&rendered).unwrap();
    assert_eq!(back.max_age, Some(0));
    assert_eq!(back.name, "session");
}

#[test]
fn cookies_reject_invalid_names() {
    use elm_effect_unstable::http::{Cookie, Cookies};
    assert!(Cookie::is_valid_name("session_id-1.2~x"));
    for bad in ["", "has space", "semi;colon", "quo\"te", "back\\slash", "contr\x7fol"] {
        assert!(!Cookie::is_valid_name(bad), "{bad}");
    }
    assert!(Cookie::parse_pair("bad name=1").is_none());
    assert!(Cookie::parse_set_cookie("bad name=1; Path=/").is_none());
    let jar = Cookies::parse_header("a=1; bad name=2; b=3");
    assert_eq!(jar.get_value("a"), Some("1"));
    assert_eq!(jar.get_value("b"), Some("3"));
    assert_eq!(jar.get_value("missing"), None);
    assert_eq!(jar.get("a").unwrap().name, "a");
}

#[test]
fn cookies_from_pairs_rejects_invalid_names() {
    use elm_effect_unstable::http::Cookies;
    let jar = Cookies::from_pairs(vec![
        ("ok".to_string(), "1".to_string()),
        ("bad name".to_string(), "2".to_string()),
    ]);
    assert_eq!(jar.get_value("ok"), Some("1"));
    assert_eq!(jar.get_value("bad name"), None);
    assert!(!jar.is_empty());
    assert!(Cookies::new().is_empty());
}

#[test]
fn ipinterface_equality_and_throwing_parse() {
    use elm_effect_unstable::net::IpInterface;
    assert_eq!(
        IpInterface::parse("10.0.0.1/8").unwrap(),
        IpInterface::parse("10.0.0.1/8").unwrap()
    );
    assert_ne!(
        IpInterface::parse("10.0.0.1/8").unwrap(),
        IpInterface::parse("10.0.0.1/16").unwrap()
    );
    assert_eq!(IpInterface::must_parse("10.0.0.1/8").render(), "10.0.0.1/8");
}
