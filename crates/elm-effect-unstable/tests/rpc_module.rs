//! Wave-12 Rpc fence: envelopes, router, group, middleware, fake pair.
//! Loads `rpc.rs` by path until the `rpc` feature lands in the unstable
//! manifest (BoldSummit-held). Transport/socket/Effect rows stay
//! `unported` with reasons in the close report.

#[path = "../src/rpc.rs"]
mod rpc;

use rpc::{Exit, FakePair, Group, Json, Message, Middleware, Router};

#[test]
fn envelope_preserves_request_id() {
    for id in [
        Json::Str("a%b".to_string()),
        Json::Num("42".to_string()),
        Json::Num("0".to_string()),
        Json::Null,
    ] {
        let req = Message::Request {
            id: id.clone(),
            method: "add".to_string(),
            payload: Json::Arr(vec![Json::Num("1".to_string())]),
        };
        let back = Message::decode(&req.encode()).unwrap();
        assert_eq!(back, req);
    }
}

#[test]
fn success_response_decodes_clean() {
    let text = r#"{"_tag":"Exit","requestId":"7","exit":{"_tag":"Success","value":42}}"#;
    match Message::decode(text).unwrap() {
        Message::Response { id, exit } => {
            assert_eq!(id, Json::Str("7".to_string()));
            assert_eq!(exit, Exit::Success(Json::Num("42".to_string())));
        }
        other => panic!("expected response, got {other:?}"),
    }
}

#[test]
fn decode_ignores_unknown_fields() {
    // Inherited/unknown properties never leak into the model.
    let text = r#"{"_tag":"Exit","requestId":1,"exit":{"_tag":"Success","value":[],"__proto__":{"x":1}},"extra":true}"#;
    match Message::decode(text).unwrap() {
        Message::Response { exit, .. } => {
            assert_eq!(exit, Exit::Success(Json::Arr(vec![])));
        }
        other => panic!("expected response, got {other:?}"),
    }
}

#[test]
fn json_decode_array_rules() {
    // Arrays stay flat; single payloads wrap on the request path.
    let req = Message::decode(r#"{"_tag":"Request","requestId":1,"method":"m","payload":[1,2]}"#)
        .unwrap();
    match req {
        Message::Request {
            payload: Json::Arr(items),
            ..
        } => assert_eq!(items.len(), 2),
        other => panic!("expected array payload, got {other:?}"),
    }
    let req =
        Message::decode(r#"{"_tag":"Request","requestId":1,"method":"m","payload":1}"#).unwrap();
    match req {
        Message::Request {
            payload: Json::Arr(items),
            ..
        } => assert_eq!(items.len(), 1),
        other => panic!("expected wrapped payload, got {other:?}"),
    }
}

#[test]
fn defect_and_interrupt_exits() {
    let text = r#"{"_tag":"Exit","requestId":1,"exit":{"_tag":"Defect","message":"boom"}}"#;
    assert_eq!(
        Message::decode(text).unwrap(),
        Message::Response {
            id: Json::Num("1".to_string()),
            exit: Exit::Defect("boom".to_string())
        }
    );
    // Interrupt exits use null for a missing message elsewhere; here the
    // bare Interrupt tag decodes directly.
    let text = r#"{"_tag":"Exit","requestId":1,"exit":{"_tag":"Interrupt"}}"#;
    assert!(matches!(
        Message::decode(text).unwrap(),
        Message::Response {
            exit: Exit::Interrupt,
            ..
        }
    ));
}

#[test]
fn router_single_handler() {
    let mut router = Router::new();
    router.route("add", |payload| match payload {
        Json::Arr(items) => Exit::Success(Json::Num(items.len().to_string())),
        _ => Exit::Failure(Json::Str("bad input".to_string())),
    });
    let out = router.dispatch(&Message::Request {
        id: Json::Num("1".to_string()),
        method: "add".to_string(),
        payload: Json::Arr(vec![Json::Num("1".to_string())]),
    });
    match out {
        Message::Response {
            exit: Exit::Success(Json::Num(n)),
            ..
        } => assert_eq!(n, "1"),
        other => panic!("expected success, got {other:?}"),
    }
    // Unknown methods use the defect schema.
    let out = router.dispatch(&Message::Request {
        id: Json::Num("2".to_string()),
        method: "missing".to_string(),
        payload: Json::Null,
    });
    assert!(matches!(
        out,
        Message::Response {
            exit: Exit::Defect(_),
            ..
        }
    ));
}

#[test]
fn group_stream_schemas_empty() {
    let mut group = Group::new();
    group.route("add", "adds numbers");
    assert_eq!(group.routes.len(), 1);
    // Streams need the stream wave: explicit empty set, not a silent skip.
    assert!(group.stream_schemas().is_empty());
}

#[test]
fn middleware_wraps_dispatch() {
    let mut pair = FakePair::new();
    pair.route("ping", |_| Exit::Success(Json::Str("pong".to_string())));
    pair.middleware(Middleware::new(|msg, next| match next(msg) {
        Message::Response {
            id,
            exit: Exit::Success(v),
        } => Message::Response {
            id,
            exit: Exit::Success(Json::Arr(vec![v])),
        },
        other => other,
    }));
    match pair.call("ping", Json::Null).unwrap() {
        Message::Response {
            exit: Exit::Success(Json::Arr(_)),
            ..
        } => {}
        other => panic!("expected wrapped success, got {other:?}"),
    }
}

#[test]
fn json_codec_round_trips() {
    for text in [
        r#"{"a":[1,true,"x",null],"b":{}}"#,
        r#""esc \"q\"""#,
        "42",
        "true",
        "null",
    ] {
        let value = Json::decode(text).unwrap();
        assert_eq!(Json::decode(&value.encode()).unwrap(), value, "{text}");
    }
    assert!(Json::decode("{bad}").is_err());
    assert!(Json::decode("[1,]").is_err());
}
