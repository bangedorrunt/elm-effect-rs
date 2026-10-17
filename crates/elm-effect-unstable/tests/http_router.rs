//! `packages/effect/test/http/HttpRouter.test.ts` prefix-routing witnesses.

use elm_effect_unstable::http_server::{PrefixRouter, Response, prefix_route};

fn echo(local_url: &str) -> Response {
    Response::text(local_url)
}

fn body(response: &Response) -> String {
    String::from_utf8_lossy(&response.body).into_owned()
}

#[test]
fn normalizes_the_prefix_stored_by_prefix_route() {
    // HttpRouter.test.ts:25 — "/api/" stores as "/api".
    let route = prefix_route("GET", "/users", "/api/");
    assert_eq!(route.prefix, Some("/api".to_owned()));
}

#[test]
fn handles_prefixed_routes() {
    // HttpRouter.test.ts:34 — prefixed router dispatches and answers.
    let mut router = PrefixRouter::new().prefixed("/api");
    router.add("GET", "/hello", |_| Response::text("hi"));
    let response = router.dispatch("GET", "/api/hello").unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(body(&response), "hi");
}

#[test]
fn preserves_the_local_url_for_each_prefix() {
    // HttpRouter.test.ts:57 — every tuple: handler sees the prefix-stripped URL.
    for (prefix, request_url) in [("/", "/users"), ("/api/", "/api/users")] {
        let mut router = PrefixRouter::new().prefixed(prefix);
        router.add("GET", "/users", echo);
        let response = router.dispatch("GET", request_url).unwrap();
        assert_eq!(body(&response), "/users", "prefix {prefix}");
    }
}

#[test]
fn nests_a_prefixed_sub_router_beneath_a_prefixed_parent() {
    // HttpRouter.test.ts:67 — /api parent + /app child serve /api/app/users.
    let mut child = PrefixRouter::new().prefixed("/app");
    child.add("GET", "/users", echo);
    let mut parent = PrefixRouter::new().prefixed("/api");
    parent.mount(child);
    let response = parent.dispatch("GET", "/api/app/users").unwrap();
    assert_eq!(body(&response), "/users");
}

#[test]
fn nests_a_prefix_route_prefix_beneath_a_prefixed_parent() {
    // HttpRouter.test.ts:78 — prefixRoute("…", "/app") under parent "/api".
    let users = prefix_route("GET", "/users", "/app");
    let mut parent = PrefixRouter::new().prefixed("/api");
    parent.add_prefixed("GET", "/users", users.prefix.as_deref().unwrap(), echo);
    let response = parent.dispatch("GET", "/api/app/users").unwrap();
    assert_eq!(body(&response), "/users");
}
