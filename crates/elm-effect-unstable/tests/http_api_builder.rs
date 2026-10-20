//! `packages/effect/test/httpapi/HttpApiBuilder.test.ts` builder/routing witnesses.
//!
//! Carries the rows this surface owns: handler registration and lookup
//! errors (`HttpApiBuilder.group`/`handle`/`handleAll`), path-template
//! routing with a literal action suffix, `QUERY` endpoints, array query
//! parameters, and content-type negotiation. Schema-codec, streaming/SSE,
//! `WithHeaders`, `ParseOptions`-slot, middleware, and security rows stay
//! unported: no codec, fiber, or live-server machinery exists here.

use elm_effect_unstable::http::{HttpApi, HttpApiGroup, UrlParams, build_endpoint_url};
use elm_effect_unstable::http_endpoint::{
    Endpoint, EndpointMethod, PayloadEncoding, PayloadKind, normalize_content_type,
};
use elm_effect_unstable::http_server::match_path;

/// Minimal `HttpApiBuilder.group` stand-in: handlers keyed by the endpoint
/// identifiers the group declares. Registration refuses unknown identifiers
/// and duplicates with the pin's messages; dispatch calls the stored handler.
struct Handlers {
    group: String,
    known: Vec<String>,
    handlers: Vec<(String, fn(&str) -> String)>,
}

impl Handlers {
    fn new(group: &str, known: &[&str]) -> Self {
        Self {
            group: group.to_owned(),
            known: known.iter().map(|s| s.to_string()).collect(),
            handlers: Vec::new(),
        }
    }

    fn handle(
        &mut self,
        id: &str,
        handler: fn(&str) -> String,
    ) -> Result<fn(&str) -> String, String> {
        if !self.known.iter().any(|known| known == id) {
            return Err(format!(
                "HttpApiEndpoint \"{id}\" not found in HttpApiGroup \"{}\"",
                self.group
            ));
        }
        if self.handlers.iter().any(|(registered, _)| registered == id) {
            return Err(format!(
                "Handler for HttpApiEndpoint \"{id}\" is already registered in HttpApiGroup \"{}\"",
                self.group
            ));
        }
        self.handlers.push((id.to_owned(), handler));
        Ok(handler)
    }

    fn handle_all(&mut self, entries: Vec<(&str, fn(&str) -> String)>) -> Result<(), String> {
        for (id, handler) in entries {
            self.handle(id, handler)?;
        }
        Ok(())
    }

    fn call(&self, id: &str, arg: &str) -> Option<String> {
        self.handlers
            .iter()
            .find(|(registered, _)| registered == id)
            .map(|(_, handler)| handler(arg))
    }
}

fn get_user(arg: &str) -> String {
    format!("user:{arg}")
}

fn create_user(arg: &str) -> String {
    format!("created:{arg}")
}

#[test]
fn round_trips_a_path_parameter_followed_by_a_literal_action_suffix() {
    // HttpApiBuilder.test.ts:319 — `/operations/:id:wait` binds `id`, keeps `:wait`.
    let url = build_endpoint_url(None, "", "/operations/:id:wait", &[("id", "op_1")], &[])
        .expect("template renders");
    assert_eq!(url, "/operations/op_1:wait");
    assert_eq!(
        match_path("/operations/:id:wait", "/operations/op_1:wait"),
        Some(vec![("id".to_owned(), "op_1".to_owned())])
    );
}

#[test]
fn does_not_bind_a_non_word_schema_key_in_a_literal_client_path() {
    // HttpApiBuilder.test.ts:337 — `a$b` is not a placeholder name, stays literal.
    let url = build_endpoint_url(None, "", "/x/:a$b", &[("a$b", "value")], &[])
        .expect("template renders");
    assert_eq!(url, "/x/:a$b");
}

#[test]
fn returns_the_callback_unchanged_and_supports_registration_in_a_group() {
    // HttpApiBuilder.test.ts:353 — `handle` returns the callback; the group owns it.
    let group = HttpApiGroup::make("users").add_endpoint(
        elm_effect_unstable::http::HttpApiEndpoint::get("create", "/users/:id"),
    );
    assert_eq!(
        group.endpoint("create").map(|e| e.path.as_str()),
        Some("/users/:id")
    );

    let mut handlers = Handlers::new("users", &["create"]);
    let callback: fn(&str) -> String = get_user;
    let returned = handlers.handle("create", callback).expect("registers");
    assert_eq!(returned, callback);
    assert_eq!(handlers.call("create", "42"), Some("user:42".to_owned()));
}

#[test]
fn round_trips_a_query_request_body() {
    // HttpApiBuilder.test.ts:381 — a QUERY endpoint carries a payload.
    let endpoint = Endpoint::query("run", "/search")
        .payload(vec![PayloadEncoding::new(
            "application/json",
            PayloadKind::Json,
        )])
        .build()
        .expect("QUERY endpoint validates");
    assert_eq!(endpoint.method, EndpointMethod::Query);
    assert_eq!(endpoint.path, "/search");
    assert!(endpoint.payload_entry("application/json").is_some());
}

#[test]
fn round_trips_array_query_parameters_with_one_or_more_values() {
    // HttpApiBuilder.test.ts:428 — repeated names survive render and parse.
    for values in [vec!["a", "b"], vec!["a"]] {
        let params = UrlParams::from_pairs(
            values
                .iter()
                .map(|v| ("ids".to_owned(), Some(v.to_string())))
                .collect(),
        );
        let back = UrlParams::parse(&params.render());
        let got: Vec<&str> = back
            .pairs()
            .iter()
            .filter(|(name, _)| name == "ids")
            .map(|(_, value)| value.as_deref().unwrap_or(""))
            .collect();
        assert_eq!(got, values);
    }
}

#[test]
fn round_trips_mixed_case_media_types_with_declared_and_received_parameters() {
    // HttpApiBuilder.test.ts:487 — keys compare lowercased, parameters stripped.
    assert_eq!(
        normalize_content_type("Application/Vnd.Effect+JSON; profile=declared"),
        "application/vnd.effect+json"
    );
    let endpoint = Endpoint::post("create", "/create")
        .payload(vec![PayloadEncoding::new(
            "Application/Vnd.Effect+JSON; profile=declared",
            PayloadKind::Json,
        )])
        .build()
        .expect("payload validates");
    assert!(
        endpoint
            .payload_entry(&normalize_content_type(
                "application/vnd.effect+json; profile=received"
            ))
            .is_some()
    );
}

#[test]
fn round_trips_custom_form_urlencoded_media_types() {
    // HttpApiBuilder.test.ts:527 — a vendor form type keys by normalized name.
    let encoding = PayloadEncoding::new("application/vnd.effect.form", PayloadKind::Form);
    assert_eq!(encoding.key(), "application/vnd.effect.form");
    assert_eq!(encoding.kind, PayloadKind::Form);
}

#[test]
fn registers_handle_all_handlers_at_runtime() {
    // HttpApiBuilder.test.ts:1567 — one call registers every endpoint handler.
    let api = HttpApi::make("Api").add(HttpApiGroup::make("users").add_endpoint(
        elm_effect_unstable::http::HttpApiEndpoint::get("getUser", "/users"),
    ));
    let group = api.group("users").expect("group registered");
    let mut handlers = Handlers::new(
        "users",
        &group
            .endpoints
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
    );
    handlers
        .handle_all(vec![("getUser", get_user as fn(&str) -> String)])
        .expect("handleAll registers");
    // `createUser` is unknown to this group, so it refuses at runtime.
    assert!(handlers.handle("createUser", create_user).is_err());
    assert_eq!(
        handlers.call("getUser", "user-1"),
        Some("user:user-1".to_owned())
    );
}

#[test]
fn rejects_unknown_endpoint_identifiers() {
    // HttpApiBuilder.test.ts:1621 — `missing` and `toString` are both unknown.
    let mut handlers = Handlers::new("users", &["getUser"]);
    for identifier in ["missing", "toString"] {
        let error = handlers
            .handle(identifier, get_user)
            .expect_err("unknown refuses");
        assert_eq!(
            error,
            format!("HttpApiEndpoint \"{identifier}\" not found in HttpApiGroup \"users\"")
        );
    }
}

#[test]
fn rejects_duplicate_handle_and_handle_all_registrations() {
    // HttpApiBuilder.test.ts:1649 — the second registration fails either way.
    let mut direct = Handlers::new("users", &["getUser"]);
    direct.handle("getUser", get_user).expect("first registers");
    assert_eq!(
        direct
            .handle("getUser", create_user)
            .expect_err("duplicate refuses"),
        "Handler for HttpApiEndpoint \"getUser\" is already registered in HttpApiGroup \"users\""
    );

    let mut bulk = Handlers::new("users", &["getUser"]);
    bulk.handle_all(vec![("getUser", get_user)])
        .expect("first registers");
    assert_eq!(
        bulk.handle_all(vec![("getUser", create_user)])
            .expect_err("duplicate refuses"),
        "Handler for HttpApiEndpoint \"getUser\" is already registered in HttpApiGroup \"users\""
    );
}

#[test]
fn executes_effectful_handler_builders() {
    // HttpApiBuilder.test.ts:1717 — the builder closure runs before dispatch.
    let build = || -> Result<Handlers, String> {
        let mut handlers = Handlers::new("users", &["getUser"]);
        handlers.handle("getUser", get_user)?;
        Ok(handlers)
    };
    let handlers = build().expect("builder closure runs");
    assert_eq!(
        handlers.call("getUser", "user-1"),
        Some("user:user-1".to_owned())
    );
}
