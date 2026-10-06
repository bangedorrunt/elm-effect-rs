//! CLI + AI fence: clap-backed argv parsing and the LanguageModel fake
//! contract. Parsing runs on argument vectors, never on the process.

use elm_effect_unstable::ai::{
    FakeModel, GeneratedText, LanguageModel, ModelError, object_has_keys,
};
use elm_effect_unstable::cli::{CliError, demo_app, parse};

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|s| s.to_string()).collect()
}

#[test]
fn parse_serve_defaults() {
    let app = demo_app();
    let req = parse(&app, &argv(&["demo", "serve"])).expect("parses");
    assert_eq!(req.command, ["serve".to_owned()]);
    assert_eq!(req.flags.get("port").map(String::as_str), Some("8080"));
    assert!(!req.flags.contains_key("verbose"));
    assert!(req.positional.is_empty());
}

#[test]
fn parse_flags_and_positionals() {
    let app = demo_app();
    let req = parse(
        &app,
        &argv(&["demo", "serve", "--port", "1", "-v", "a.txt"]),
    )
    .expect("parses");
    assert_eq!(req.flags.get("port").map(String::as_str), Some("1"));
    assert_eq!(req.flags.get("verbose").map(String::as_str), Some("true"));
    assert_eq!(req.positional, ["a.txt".to_owned()]);
}

#[test]
fn parse_errors_and_help() {
    let app = demo_app();
    assert!(matches!(
        parse(&app, &argv(&["demo", "nope"])),
        Err(CliError::Unknown(_))
    ));
    assert!(matches!(
        parse(&app, &argv(&["demo", "--help"])),
        Err(CliError::Help(_))
    ));
    assert!(matches!(
        parse(&app, &argv(&["demo", "serve", "--port"])),
        Err(CliError::MissingValue(_)) | Err(CliError::Unknown(_))
    ));
}

#[test]
fn ai_fake_contract() {
    let model = FakeModel::new();
    model.push_text(Ok(GeneratedText {
        text: "hi".to_owned(),
        finish_reason: "stop".to_owned(),
    }));
    model.push_object(Ok("{\"a\":1}".to_owned()));
    model.push_chunks(vec!["x".to_owned(), "y".to_owned()]);
    assert_eq!(model.generate_text("p").expect("text").text, "hi");
    let obj = model.generate_object("p", "{}").expect("object");
    assert!(object_has_keys(&obj, &["a"]));
    assert!(!object_has_keys(&obj, &["zzz"]));
    assert!(!object_has_keys("nope", &["a"]));
    assert_eq!(model.stream_text("p"), ["x".to_owned(), "y".to_owned()]);
    assert_eq!(model.calls().len(), 3);
    model.push_text(Err(ModelError::Provider("down".to_owned())));
    assert!(matches!(
        model.generate_text("p"),
        Err(ModelError::Provider(_))
    ));
}

#[test]
fn ai_error_tags_and_retryability() {
    use elm_effect_unstable::ai::{AiError, AiReason};
    let cases: Vec<(AiReason, &str, bool)> = vec![
        (
            AiReason::RateLimit {
                retry_after_secs: None,
            },
            "RateLimitError",
            true,
        ),
        (AiReason::QuotaExhausted, "QuotaExhaustedError", false),
        (
            AiReason::Authentication {
                kind: "InvalidKey".to_owned(),
            },
            "AuthenticationError",
            false,
        ),
        (
            AiReason::ContentPolicy { description: None },
            "ContentPolicyError",
            false,
        ),
        (
            AiReason::InvalidRequest {
                parameter: "temperature".to_owned(),
                constraint: "must be between 0 and 2".to_owned(),
            },
            "InvalidRequestError",
            false,
        ),
        (
            AiReason::InvalidOutput {
                details: "x".to_owned(),
            },
            "InvalidOutputError",
            true,
        ),
        (
            AiReason::InternalProvider {
                details: "x".to_owned(),
            },
            "InternalProviderError",
            true,
        ),
        (AiReason::Unknown, "UnknownError", false),
        (
            AiReason::ToolNotFound {
                tool: "t".to_owned(),
                available: vec![],
            },
            "ToolNotFoundError",
            true,
        ),
        (
            AiReason::ToolParameterValidation {
                tool_name: "T".to_owned(),
                description: "x".to_owned(),
            },
            "ToolParameterValidationError",
            true,
        ),
        (
            AiReason::InvalidToolResult {
                tool_name: "T".to_owned(),
                description: "x".to_owned(),
            },
            "InvalidToolResultError",
            false,
        ),
        (
            AiReason::ToolResultEncoding {
                tool_name: "T".to_owned(),
                tool_result: "{}".to_owned(),
                description: "x".to_owned(),
            },
            "ToolResultEncodingError",
            false,
        ),
        (
            AiReason::ToolConfiguration {
                tool_name: "T".to_owned(),
                description: "x".to_owned(),
            },
            "ToolConfigurationError",
            false,
        ),
        (
            AiReason::StructuredOutput {
                details: "x".to_owned(),
                response_text: String::new(),
            },
            "StructuredOutputError",
            true,
        ),
        (
            AiReason::UnsupportedSchema {
                details: "x".to_owned(),
            },
            "UnsupportedSchemaError",
            false,
        ),
        (
            AiReason::InvalidUserInput {
                details: "x".to_owned(),
            },
            "InvalidUserInputError",
            false,
        ),
        (AiReason::ToolkitRequired, "ToolkitRequiredError", false),
        (
            AiReason::NetworkError {
                details: "x".to_owned(),
            },
            "NetworkError",
            false,
        ),
    ];
    for (reason, tag, retryable) in cases {
        let error = AiError::new(reason);
        assert_eq!(error.reason_tag(), tag);
        assert_eq!(error.is_retryable, retryable, "retryability of {tag}");
    }
}

#[test]
fn ai_error_message_shapes() {
    use elm_effect_unstable::ai::{AiError, AiReason};
    let limited = AiError::new(AiReason::RateLimit {
        retry_after_secs: Some(60),
    });
    assert!(limited.message().contains("Rate limit exceeded"));
    assert!(limited.message().contains("Retry after"));
    assert_eq!(
        AiError::new(AiReason::QuotaExhausted).message(),
        "Quota exhausted"
    );
    let keyed = AiError::new(AiReason::Authentication {
        kind: "InvalidKey".to_owned(),
    });
    assert!(keyed.message().contains("InvalidKey"));
    let policy = AiError::new(AiReason::ContentPolicy {
        description: Some("Input contains prohibited content".to_owned()),
    });
    assert!(
        policy
            .message()
            .contains("Input contains prohibited content")
    );
    let req = AiError::new(AiReason::InvalidRequest {
        parameter: "temperature".to_owned(),
        constraint: "must be between 0 and 2".to_owned(),
    });
    assert!(req.message().contains("parameter 'temperature'"));
    assert!(req.message().contains("must be between 0 and 2"));
    let out = AiError::new(AiReason::InvalidOutput {
        details: "Value 5 is invalid".to_owned(),
    });
    assert!(out.message().contains("Value 5 is invalid"));
    let tool = AiError::new(AiReason::ToolNotFound {
        tool: "UnknownTool".to_owned(),
        available: vec!["GetWeather".to_owned(), "GetTime".to_owned()],
    });
    assert!(tool.message().contains("Tool 'UnknownTool' not found"));
    assert!(
        tool.message()
            .contains("Available tools: GetWeather, GetTime")
    );
    let lonely = AiError::new(AiReason::ToolNotFound {
        tool: "UnknownTool".to_owned(),
        available: vec![],
    });
    assert!(lonely.message().contains("Available tools: none"));
    let custom = AiError::new(AiReason::Unknown).with_description("Something unexpected happened");
    assert_eq!(custom.message(), "Something unexpected happened");
    assert_eq!(AiError::new(AiReason::Unknown).message(), "Unknown error");
}

#[test]
fn chat_persists_and_loads() {
    use elm_effect_unstable::ai::{ChatMessage, ChatRole, ChatStore};
    let store = ChatStore::new();
    assert!(store.load("missing").is_err());
    store.save(
        "c1",
        vec![
            ChatMessage {
                id: String::new(),
                role: ChatRole::User,
                content: "hi".to_owned(),
            },
            ChatMessage {
                id: String::new(),
                role: ChatRole::Assistant,
                content: "hello".to_owned(),
            },
        ],
    );
    let history = store.load("c1").expect("saved");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].id, "msg-1");
    assert_eq!(
        store.latest_assistant_id("c1").as_deref(),
        Some(history[1].id.as_str())
    );
    assert_eq!(store.latest_assistant_id("missing"), None);
}

#[test]
fn chat_respects_ttl() {
    use elm_effect_unstable::ai::{ChatMessage, ChatRole, ChatStore};
    let store = ChatStore::with_ttl(std::time::Duration::from_millis(30));
    store.save(
        "c1",
        vec![ChatMessage {
            id: String::new(),
            role: ChatRole::User,
            content: "hi".to_owned(),
        }],
    );
    assert!(store.load("c1").is_ok());
    std::thread::sleep(std::time::Duration::from_millis(60));
    assert!(store.load("c1").is_err());
}

#[test]
fn tool_call_modes() {
    use elm_effect_unstable::ai::{FailureMode, Tool};
    let echo = Tool::new("echo", "echoes", |params| Ok(params.to_owned()));
    assert_eq!(echo.call("{\"a\":1}").expect("valid"), "{\"a\":1}");
    assert!(echo.call("nope").is_err());
    assert!(echo.call("{\"a\":NaN}").is_err());
    assert_eq!(
        echo.call_with_mode("{\"a\":1}", &FailureMode::Return)
            .expect("returns"),
        Ok("{\"a\":1}".to_owned())
    );
    let failing = Tool::new("fail", "fails", |_| {
        Err::<String, String>("boom".to_owned())
    });
    assert_eq!(
        failing
            .call_with_mode("{}", &FailureMode::Return)
            .expect("returns"),
        Err("boom".to_owned())
    );
    assert!(failing.call_with_mode("{}", &FailureMode::Error).is_err());
    assert!(failing.call_with_mode("nope", &FailureMode::Error).is_err());
}

#[test]
fn ai_error_auth_description() {
    use elm_effect_unstable::ai::{AiError, AiReason};
    // AiError.test.ts:68 — description renders after the kind suggestion.
    let described = AiError::new(AiReason::Authentication {
        kind: "InsufficientPermissions".to_owned(),
    })
    .with_description("model unavailable for this account");
    assert_eq!(
        described.message(),
        "InsufficientPermissions: Your API key lacks required permissions. model unavailable for this account"
    );
    // AiError.test.ts:79 — absent description leaves the default untouched.
    let bare = AiError::new(AiReason::Authentication {
        kind: "InsufficientPermissions".to_owned(),
    });
    assert_eq!(
        bare.message(),
        "InsufficientPermissions: Your API key lacks required permissions"
    );
    assert!(bare.description.is_none());
    // AiError.test.ts:223 — UnknownError default without description.
    assert_eq!(AiError::new(AiReason::Unknown).message(), "Unknown error");
}

#[test]
fn ai_error_tool_messages() {
    use elm_effect_unstable::ai::{AiError, AiReason};
    // AiError.test.ts:278 — tool name plus validation message.
    let params = AiError::new(AiReason::ToolParameterValidation {
        tool_name: "GetWeather".to_owned(),
        description: "Expected string".to_owned(),
    });
    assert!(
        params
            .message()
            .contains("Invalid parameters for tool 'GetWeather'")
    );
    // AiError.test.ts:286 — validation message renders.
    let validation = AiError::new(AiReason::ToolParameterValidation {
        tool_name: "GetWeather".to_owned(),
        description: "Expected string, got number".to_owned(),
    });
    assert!(validation.message().contains("Expected string, got number"));
    // AiError.test.ts:294 — non-JSON params still construct with tag and message.
    let nan = AiError::new(AiReason::ToolParameterValidation {
        tool_name: "GetWeather".to_owned(),
        description: "Expected finite number, got NaN".to_owned(),
    });
    assert_eq!(nan.reason_tag(), "ToolParameterValidationError");
    assert!(nan.message().contains("NaN"));
    // AiError.test.ts:321.
    let result = AiError::new(AiReason::InvalidToolResult {
        tool_name: "GetWeather".to_owned(),
        description: "missing required field".to_owned(),
    });
    assert!(
        result
            .message()
            .contains("Tool 'GetWeather' returned invalid result")
    );
    // AiError.test.ts:356 + 365.
    let encoding = AiError::new(AiReason::ToolResultEncoding {
        tool_name: "GetWeather".to_owned(),
        tool_result: "{}".to_owned(),
        description: "Cannot encode".to_owned(),
    });
    assert!(
        encoding
            .message()
            .contains("Failed to encode result for tool 'GetWeather'")
    );
    let circular = AiError::new(AiReason::ToolResultEncoding {
        tool_name: "GetWeather".to_owned(),
        tool_result: "{}".to_owned(),
        description: "Cannot encode circular reference".to_owned(),
    });
    assert!(
        circular
            .message()
            .contains("Cannot encode circular reference")
    );
    // AiError.test.ts:393.
    let config = AiError::new(AiReason::ToolConfiguration {
        tool_name: "OpenAiCodeInterpreter".to_owned(),
        description: "Invalid container ID format".to_owned(),
    });
    assert!(
        config
            .message()
            .contains("Invalid configuration for tool 'OpenAiCodeInterpreter'")
    );
}

#[test]
fn ai_error_reason_from_http_status() {
    use elm_effect_unstable::ai::{AiError, AiReason};
    // AiError.test.ts:559 — 400 maps to InvalidRequestError.
    assert_eq!(
        AiError::reason_from_http_status(400, None).reason_tag(),
        "InvalidRequestError"
    );
    // :564 — 401 maps to AuthenticationError with InvalidKey.
    let keyed = AiError::reason_from_http_status(401, None);
    assert_eq!(keyed.reason_tag(), "AuthenticationError");
    assert!(matches!(&keyed.reason, AiReason::Authentication { kind } if kind == "InvalidKey"));
    // :572 — 403 maps to AuthenticationError with InsufficientPermissions.
    let forbidden = AiError::reason_from_http_status(403, None);
    assert_eq!(forbidden.reason_tag(), "AuthenticationError");
    assert!(matches!(
        &forbidden.reason,
        AiReason::Authentication { kind } if kind == "InsufficientPermissions"
    ));
    // :580 — description threads into the 401/403 message.
    let threaded = AiError::reason_from_http_status(403, Some("User is not authorized"));
    assert_eq!(
        threaded.description.as_deref(),
        Some("User is not authorized")
    );
    assert!(threaded.message().contains("User is not authorized"));
    // :592 — 429 maps to retryable RateLimitError.
    let limited = AiError::reason_from_http_status(429, None);
    assert_eq!(limited.reason_tag(), "RateLimitError");
    assert!(limited.is_retryable);
    // :597 — 5xx maps to InternalProviderError.
    assert_eq!(
        AiError::reason_from_http_status(503, None).reason_tag(),
        "InternalProviderError"
    );
    // :612 — unknown statuses map to UnknownError.
    assert_eq!(
        AiError::reason_from_http_status(418, None).reason_tag(),
        "UnknownError"
    );
}

#[test]
fn ai_error_delegates_to_reason_with_context() {
    use elm_effect_unstable::ai::{AiError, AiReason};
    // AiError.test.ts:489 - isRetryable delegates to the reason.
    let retryable = AiError::new(AiReason::RateLimit {
        retry_after_secs: None,
    })
    .with_context("OpenAI", "completion");
    assert!(retryable.is_retryable);
    let keyed = AiError::new(AiReason::Authentication {
        kind: "InvalidKey".to_owned(),
    })
    .with_context("OpenAI", "completion");
    assert!(!keyed.is_retryable);
    // :505 - retryAfter delegates to the reason.
    assert_eq!(
        AiError::new(AiReason::RateLimit {
            retry_after_secs: Some(60),
        })
        .retry_after_secs(),
        Some(60)
    );
    assert_eq!(keyed.retry_after_secs(), None);
    // :523 - message carries module, method, and reason.
    let message = retryable.message();
    assert!(message.starts_with("OpenAI.completion:"));
    assert!(message.contains("Rate limit exceeded"));
    // :545 / :621 need a wrapper brand (separate from the reason tag);
    // Rust keeps a single tag here, so those rows stay open.
    let unknown = AiError::new(AiReason::Unknown).with_context("Test", "test");
    assert!(unknown.message().starts_with("Test.test:"));
    // Without a context the message stays the bare reason message.
    assert_eq!(AiError::new(AiReason::Unknown).message(), "Unknown error");
}

#[test]
fn ai_error_wrapper_tag_is_distinct_from_reason() {
    use elm_effect_unstable::ai::{AiError, AiReason};
    // AiError.test.ts:545 - the wrapper tag is "AiError" for every reason.
    let unknown = AiError::new(AiReason::Unknown).with_context("Test", "test");
    assert_eq!(unknown.tag, "AiError");
    assert_eq!(AiError::TYPE_ID, "AiError");
    let limited = AiError::new(AiReason::RateLimit {
        retry_after_secs: None,
    });
    assert_eq!(limited.tag, "AiError");
    assert_eq!(limited.reason_tag(), "RateLimitError");
    // :621 - isAiError holds for the wrapper.
    assert!(unknown.is_ai_error());
    assert!(limited.is_ai_error());
    // :637 - reasons are not wrappers: their tag never matches TYPE_ID.
    for reason in [
        AiReason::NetworkError {
            details: "d".to_owned(),
        },
        AiReason::QuotaExhausted,
    ] {
        assert_ne!(reason.tag(), AiError::TYPE_ID);
    }
    // :630 - non-AiError values cannot reach this API in Rust; the type
    // system rules out the TS runtime-shape cases.
}

#[test]
fn ansi_cursor_sequences() {
    use elm_effect_unstable::cli::cursor_to;
    // Ansi.test.ts:4 - horizontal absolute, one-indexed.
    assert_eq!(cursor_to(0, None), "\x1b[1G");
    // Ansi.test.ts:8 - cursor position, one-indexed row and column.
    assert_eq!(cursor_to(2, Some(3)), "\x1b[4;3H");
}

#[test]
fn lexer_tokenizes_argv() {
    use elm_effect_unstable::cli::{LexToken, lex_argv};
    // Lexer.test.ts:5 - every equals sign survives in the inline value.
    let lexed = lex_argv(&["--query=left=right"]);
    assert_eq!(
        lexed.tokens,
        vec![LexToken::LongOption {
            name: "query".to_owned(),
            raw: "--query=left=right".to_owned(),
            value: Some("left=right".to_owned()),
        }]
    );
    assert!(lexed.trailing_operands.is_empty());
    // :16 - negative numbers lex as values.
    let lexed = lex_argv(&["--lon", "-3.70", "-2", "-.5", "-1e3"]);
    assert_eq!(
        lexed.tokens,
        vec![
            LexToken::LongOption {
                name: "lon".to_owned(),
                raw: "--lon".to_owned(),
                value: None,
            },
            LexToken::Value { value: "-3.70".to_owned() },
            LexToken::Value { value: "-2".to_owned() },
            LexToken::Value { value: "-.5".to_owned() },
            LexToken::Value { value: "-1e3".to_owned() },
        ]
    );
    // :28 - short options that are not negative numbers split per char.
    let lexed = lex_argv(&["-1a", "-e"]);
    assert_eq!(
        lexed.tokens,
        vec![
            LexToken::ShortOption { flag: "1".to_owned(), raw: "-1".to_owned(), value: None },
            LexToken::ShortOption { flag: "a".to_owned(), raw: "-a".to_owned(), value: None },
            LexToken::ShortOption { flag: "e".to_owned(), raw: "-e".to_owned(), value: None },
        ]
    );
}

#[test]
fn user_error_message_fallback_chain() {
    use elm_effect_unstable::cli::{UserCause, UserError};
    // Errors.test.ts:299 - the user-facing message wins over the cause.
    let error = UserError::new(UserCause::Error("internal details".to_owned()))
        .with_message("Could not deploy the application");
    assert_eq!(error.message(), "Could not deploy the application");
    // :308 - an Error cause message is the fallback.
    let error = UserError::new(UserCause::Error("Connection refused".to_owned()));
    assert_eq!(error.message(), "Connection refused");
    // :316 - a string cause is the fallback.
    let error = UserError::new(UserCause::Text("Connection refused".to_owned()));
    assert_eq!(error.message(), "Connection refused");
    // :322 - causes without a message get the generic fallback.
    let error = UserError::new(UserCause::Opaque);
    assert_eq!(error.message(), "An error occurred");
    // :328 - empty messages fall through to the next fallback.
    let error = UserError::new(UserCause::Error("Connection refused".to_owned())).with_message("");
    assert_eq!(error.message(), "Connection refused");
    let error = UserError::new(UserCause::Text("".to_owned()));
    assert_eq!(error.message(), "An error occurred");
    // :339 - the runtime reports user errors before rendering.
    assert!(UserError::new(UserCause::Text("failed".to_owned())).is_error_reported());
}

#[test]
fn primitive_value_parsers() {
    use elm_effect_unstable::cli::primitive;
    // Primitive.test.ts:65/66 - true values.
    for text in ["true", "1", "y", "yes", "on"] {
        assert_eq!(primitive::parse_bool(text), Ok(true), "{text}");
    }
    // :75 - false values.
    for text in ["false", "0", "n", "no", "off"] {
        assert_eq!(primitive::parse_bool(text), Ok(false), "{text}");
    }
    // :84 - anything else fails with the expected set.
    assert_eq!(
        primitive::parse_bool("invalid"),
        Err("Expected \"true\" | \"yes\" | \"on\" | \"1\" | \"y\" | \"false\" | \"no\" | \"off\" | \"0\" | \"n\"".to_owned())
    );
    // :91/:121 - tags.
    assert_eq!(primitive::BOOLEAN_TAG, "Boolean");
    assert_eq!(primitive::FINITE_TAG, "Finite");
    // :98/99 - finite numbers parse.
    for (text, want) in [("42", 42.0), ("3.14", 3.14), ("-42.5", -42.5), ("0", 0.0), ("1e3", 1000.0)] {
        assert_eq!(primitive::parse_finite(text), Ok(want), "{text}");
    }
    // :108/:113 - the rest fail naming finite numbers.
    // Primitive.test.ts:166 - valid integers parse.
    for (text, want) in [
        ("42", 42),
        ("-123", -123),
        ("0", 0),
        ("9007199254740991", 9007199254740991),
        ("1e3", 1000),
    ] {
        assert_eq!(primitive::parse_int(text), Ok(want), "{text}");
    }
    // :175 - non-integers and non-numbers fail with tagged messages.
    for text in ["3.14", "not-a-number"] {
        let err = primitive::parse_int(text).expect_err("rejects");
        assert!(
            err == "Expected an integer"
                || err == "Expected a string representing a finite number",
            "{text}"
        );
    }
    // :182 - tag.
    assert_eq!(primitive::INT_TAG, "Int");
    for text in ["not-a-number", "NaN", "Infinity", "-Infinity", "1e309"] {
        let err = primitive::parse_finite(text).expect_err("rejects");
        assert!(err.contains("finite number"), "{text}");
    }
}

#[test]
fn provider_metadata_roundtrip() {
    use elm_effect_unstable::ai::ProviderMetadata;
    use serde_json::json;
    // AiError.test.ts:655 - metadata encodes and decodes round-trip.
    let mut metadata = ProviderMetadata::new();
    metadata.insert("name".to_owned(), Some(json!("OpenAI")));
    metadata.insert("errorCode".to_owned(), Some(json!("rate_limit_exceeded")));
    metadata.insert("errorType".to_owned(), Some(json!("rate_limit")));
    metadata.insert("requestId".to_owned(), Some(json!("req_123abc")));
    let text = serde_json::to_string(&metadata).expect("encodes");
    let back: ProviderMetadata = serde_json::from_str(&text).expect("decodes");
    assert_eq!(back, metadata);
    let mut nulled = ProviderMetadata::new();
    nulled.insert("anthropic".to_owned(), None);
    let text = serde_json::to_string(&nulled).expect("encodes");
    assert_eq!(
        serde_json::from_str::<ProviderMetadata>(&text).expect("decodes"),
        nulled
    );
}

#[test]
fn usage_info_roundtrip() {
    use elm_effect_unstable::ai::UsageInfo;
    // AiError.test.ts:670 - usage totals encode and decode round-trip.
    let usage = UsageInfo {
        prompt_tokens: 100,
        completion_tokens: 50,
        total_tokens: 150,
    };
    let text = serde_json::to_string(&usage.to_json()).expect("encodes");
    let back = UsageInfo::from_json(&serde_json::from_str(&text).expect("json")).expect("decodes");
    assert_eq!(back, usage);
}

#[test]
fn http_context_roundtrip() {
    use elm_effect_unstable::ai::{HttpContext, HttpRequestContext, HttpResponseContext};
    use std::collections::HashMap;
    // AiError.test.ts:684 - request/response/body encode and decode round-trip.
    let mut headers = HashMap::new();
    headers.insert("Content-Type".to_owned(), "application/json".to_owned());
    let context = HttpContext {
        request: HttpRequestContext {
            method: "POST".to_owned(),
            url: "https://api.example.com/v1/chat".to_owned(),
            url_params: vec![("model".to_owned(), "gpt-4".to_owned())],
            hash: None,
            headers: headers.clone(),
        },
        response: HttpResponseContext { status: 200, headers },
        body: "{\"result\": \"success\"}".to_owned(),
    };
    let text = serde_json::to_string(&context.to_json()).expect("encodes");
    let back = HttpContext::from_json(&serde_json::from_str(&text).expect("json")).expect("decodes");
    assert_eq!(back, context);
}

#[test]
fn ai_reasons_roundtrip_json() {
    use elm_effect_unstable::ai::{AiError, AiReason};
    let reasons = vec![
        AiReason::RateLimit { retry_after_secs: Some(60) },
        AiReason::RateLimit { retry_after_secs: None },
        AiReason::QuotaExhausted,
        AiReason::Authentication { kind: "InvalidKey".to_owned() },
        AiReason::ContentPolicy { description: Some("d".to_owned()) },
        AiReason::InvalidRequest { parameter: "p".to_owned(), constraint: "c".to_owned() },
        AiReason::InvalidOutput { details: "d".to_owned() },
        AiReason::InternalProvider { details: "d".to_owned() },
        AiReason::Unknown,
        AiReason::ToolNotFound { tool: "t".to_owned(), available: vec!["a".to_owned()] },
        AiReason::ToolParameterValidation { tool_name: "t".to_owned(), description: "d".to_owned() },
        AiReason::InvalidToolResult { tool_name: "t".to_owned(), description: "d".to_owned() },
        AiReason::ToolResultEncoding {
            tool_name: "t".to_owned(),
            tool_result: "r".to_owned(),
            description: "d".to_owned(),
        },
        AiReason::ToolConfiguration { tool_name: "t".to_owned(), description: "d".to_owned() },
        AiReason::StructuredOutput { details: "d".to_owned(), response_text: String::new() },
        AiReason::UnsupportedSchema { details: "d".to_owned() },
        AiReason::InvalidUserInput { details: "d".to_owned() },
        AiReason::ToolkitRequired,
        AiReason::NetworkError { details: "d".to_owned() },
    ];
    for reason in &reasons {
        let text = serde_json::to_string(&reason.to_json()).expect("encodes");
        let back = AiReason::from_json(&serde_json::from_str(&text).expect("json")).expect("decodes");
        assert_eq!(&back, reason);
    }
    // AiError.test.ts:708 - RateLimitError keeps its tag and retry info.
    let limited = AiReason::RateLimit { retry_after_secs: Some(60) };
    let back = AiReason::from_json(&limited.to_json()).expect("decodes");
    assert_eq!(back.tag(), "RateLimitError");
    assert!(matches!(back, AiReason::RateLimit { retry_after_secs: Some(60) }));
    // :719 - the union dispatches on _tag.
    let auth = AiReason::Authentication { kind: "InvalidKey".to_owned() };
    let back = AiReason::from_json(&auth.to_json()).expect("decodes");
    assert_eq!(back.tag(), "AuthenticationError");
    // :734 - the wrapper round-trips with its reason.
    let error = AiError::new(AiReason::Unknown).with_context("Test", "test");
    let text = serde_json::to_string(&error.to_json()).expect("encodes");
    let back = AiError::from_json(&serde_json::from_str(&text).expect("json")).expect("decodes");
    assert_eq!(back.tag, "AiError");
    assert_eq!(back.reason_tag(), "UnknownError");
    assert_eq!(back, error);
}

#[test]
fn ai_errors_from_schema_messages() {
    use elm_effect_unstable::ai::{AiError, AiReason};
    // AiError.test.ts:185 - InvalidOutputError carries the schema message.
    let invalid = AiReason::invalid_output_from_message("Expected string but got number");
    assert_eq!(invalid.tag(), "InvalidOutputError");
    // AiError.test.ts:444 - StructuredOutputError keeps message and response.
    let structured =
        AiReason::structured_output_from_message("Expected string", "{\"name\":123}");
    assert_eq!(structured.tag(), "StructuredOutputError");
    // AiError.test.ts:88 - descriptions survive JSON round-trips.
    let described = AiError::new(AiReason::Authentication {
        kind: "ExpiredKey".to_owned(),
    })
    .with_description("The security token included in the request is expired");
    let text = serde_json::to_string(&described.to_json()).expect("encodes");
    let back = AiError::from_json(&serde_json::from_str(&text).expect("json")).expect("decodes");
    assert_eq!(
        back.description.as_deref(),
        Some("The security token included in the request is expired")
    );
    assert_eq!(back.tag, "AiError");
}

#[test]
fn cli_error_taxonomy_tags() {
    use elm_effect_unstable::cli::CliError;
    // Errors.test.ts:28 - the UnknownSubcommand class name is its tag.
    let unknown = CliError::UnknownSubcommand {
        subcommand: "deplyo".to_owned(),
        parent: None,
        suggestions: vec!["deploy".to_owned()],
    };
    assert_eq!(unknown.tag(), "UnknownSubcommand");
    match &unknown {
        CliError::UnknownSubcommand {
            subcommand,
            suggestions,
            ..
        } => {
            assert_eq!(subcommand, "deplyo");
            assert_eq!(suggestions, &["deploy".to_owned()]);
        }
        _ => panic!("wrong variant"),
    }
    assert_eq!(
        CliError::MissingOption {
            option: "value".to_owned()
        }
        .tag(),
        "MissingOption"
    );
    assert_eq!(
        CliError::UnrecognizedOption {
            option: "--verboes".to_owned(),
            command: None,
            suggestions: vec!["--verbose".to_owned()],
        }
        .tag(),
        "UnrecognizedOption"
    );
    assert_eq!(
        CliError::DuplicateOption {
            option: "--verbose".to_owned(),
            parent_command: vec!["root".to_owned()],
            child_command: vec!["root".to_owned(), "serve".to_owned()],
        }
        .tag(),
        "DuplicateOption"
    );
    assert_eq!(
        CliError::MissingArgument {
            argument: "input".to_owned()
        }
        .tag(),
        "MissingArgument"
    );
    assert_eq!(
        CliError::UnexpectedArgument {
            arguments: vec!["extra".to_owned()]
        }
        .tag(),
        "UnexpectedArgument"
    );
    assert_eq!(
        CliError::InvalidValue {
            option: "--port".to_owned(),
            value: "abc".to_owned(),
            expected: "an integer".to_owned(),
            kind: "option".to_owned(),
        }
        .tag(),
        "InvalidValue"
    );
}

#[test]
fn primitive_string_and_choice() {
    use elm_effect_unstable::cli::primitive;
    // Primitive.test.ts:190 - strings pass through untouched.
    for text in ["hello", "", " spaces ", "123", "special!@#$%"] {
        assert_eq!(primitive::parse_string(text), text);
    }
    // :199 - tag.
    assert_eq!(primitive::STRING_TAG, "String");
    // :213 - valid choices map to their outputs.
    let colors = [("red", "RED"), ("green", "GREEN"), ("blue", "BLUE")];
    for (text, want) in [("red", "RED"), ("green", "GREEN"), ("blue", "BLUE")] {
        assert_eq!(primitive::parse_choice(text, &colors).as_deref(), Ok(want));
    }
    // :220 - the rest fail naming the allowed inputs.
    for text in ["yellow", "purple", ""] {
        let err = primitive::parse_choice(text, &colors).expect_err("rejects");
        assert_eq!(err, "\"red\" | \"green\" | \"blue\"", "{text}");
    }
    // :231 - tag.
    assert_eq!(primitive::CHOICE_TAG, "Choice");
}

#[test]
fn primitive_path_resolution() {
    use elm_effect_unstable::cli::primitive::{self, PathKind};
    // Primitive.test.ts:252 - paths resolve without requiring existence.
    let relative = primitive::parse_path("./test.txt", PathKind::Either, false).expect("resolves");
    assert!(relative.contains("test.txt"), "{relative}");
    assert!(std::path::Path::new(&relative).is_absolute(), "{relative}");
    assert_eq!(
        primitive::parse_path("/absolute/path", PathKind::Either, false).expect("kept"),
        "/absolute/path"
    );
    let nested =
        primitive::parse_path("relative/path", PathKind::Either, false).expect("resolves");
    assert!(nested.contains("relative/path"), "{nested}");
    // :265 - tag.
    assert_eq!(primitive::PATH_TAG, "Path");
    // :269 - a required file that is missing fails naming the path.
    let missing = primitive::parse_path("/non/existent/file.txt", PathKind::File, true)
        .expect_err("rejects");
    assert_eq!(missing, "Path does not exist: /non/existent/file.txt");
    // Present paths pass through existence checks.
    let manifest = env!("CARGO_MANIFEST_DIR");
    assert!(
        primitive::parse_path(
            &format!("{manifest}/Cargo.toml"),
            PathKind::File,
            true
        )
        .is_ok()
    );
    // :294 - a required directory that is missing fails too.
    let missing_dir =
        primitive::parse_path("/non/existent/dir", PathKind::Directory, true).expect_err("rejects");
    assert_eq!(missing_dir, "Path does not exist: /non/existent/dir");
    assert!(primitive::parse_path(manifest, PathKind::Directory, true).is_ok());
}

#[test]
fn primitive_redacted_hides_values() {
    use elm_effect_unstable::cli::primitive;
    // Primitive.test.ts:323 - secrets parse and render redacted.
    let secret = primitive::parse_redacted("secret-value");
    assert_eq!(secret.value(), "secret-value");
    assert_eq!(format!("{secret}"), "<redacted>");
    assert_eq!(format!("{secret:?}"), "<redacted>");
    // :332 - tag.
    assert_eq!(primitive::REDACTED_TAG, "Redacted");
    // :336 - empty strings parse too.
    let empty = primitive::parse_redacted("");
    assert_eq!(empty.value(), "");
    assert_eq!(format!("{empty}"), "<redacted>");
}
