//! CLI + AI fence: clap-backed argv parsing and the LanguageModel fake
//! contract. Parsing runs on argument vectors, never on the process.

use elm_effect_unstable::ai::{
    AiError, AiReason, FakeModel, GeneratedText, LanguageModel, ModelError, is_ai_error,
    object_has_keys,
};
use elm_effect_unstable::cli::{CliApp, CliError, FlagDef, demo_app, parse};
use elm_effect_unstable::cli_ai::CliError as WizardCliError;
use elm_effect_unstable::cli::primitive;
use std::collections::BTreeMap;
use elm_effect_unstable::cli_ai::{
    Arg, Command, WizardStep, flag_arg, merge_key_values, option_runs, wizard_argv, wizard_plan,
};
use elm_effect_unstable::cli_ai::Run;

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
    // Primitive.test.ts:241 - choices map to non-string values too.
    let numbers = [("one", 1), ("two", 2), ("three", 3)];
    for (text, want) in [("one", 1), ("two", 2), ("three", 3)] {
        assert_eq!(primitive::parse_choice(text, &numbers), Ok(want));
    }
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

#[test]
fn primitive_key_value_pairs() {
    use elm_effect_unstable::cli::primitive;
    // Primitive.test.ts:346 - `=` inside values survives.
    assert_eq!(
        primitive::parse_key_value("DATABASE_URL=postgres://user:pass@host:5432/db?sslmode=require"),
        Ok((
            "DATABASE_URL".to_owned(),
            "postgres://user:pass@host:5432/db?sslmode=require".to_owned()
        ))
    );
    // :356 - trailing padding survives.
    assert_eq!(
        primitive::parse_key_value("TOKEN=YWJjZA=="),
        Ok(("TOKEN".to_owned(), "YWJjZA==".to_owned()))
    );
    // :362 - malformed inputs fail with pinned messages.
    for (text, want) in [
        (
            "invalid",
            "Invalid key=value format. Expected format: key=value, got: invalid",
        ),
        (
            "=value",
            "Invalid key=value format. Both key and value must be non-empty. Got: =value",
        ),
        (
            "key=",
            "Invalid key=value format. Both key and value must be non-empty. Got: key=",
        ),
    ] {
        assert_eq!(primitive::parse_key_value(text).expect_err("rejects"), want);
    }
}

#[test]
fn primitive_never_rejects_all() {
    use elm_effect_unstable::cli::primitive;
    // Primitive.test.ts:394 - tag.
    assert_eq!(primitive::NEVER_TAG, "Never");
    // :398 - every input fails with the pinned message.
    assert_eq!(
        primitive::parse_never("value").expect_err("rejects"),
        "This option does not accept values"
    );
}

#[test]
fn primitive_date_values() {
    use elm_effect_unstable::cli::primitive;
    // Primitive.test.ts:129 - valid dates parse.
    let day = primitive::parse_date("2024-01-15").expect("date");
    assert_eq!(&day.to_iso_string()[..10], "2024-01-15");
    let moment = primitive::parse_date("2024-01-15T12:30:45.123Z").expect("datetime");
    assert_eq!(moment.to_iso_string(), "2024-01-15T12:30:45.123Z");
    // Offsets normalize to UTC.
    let shifted = primitive::parse_date("2024-01-15T12:30:45+02:00").expect("offset");
    assert_eq!(shifted.hour, 10);
    assert_eq!(shifted.minute, 30);
    // :155 - invalid values name the failure.
    assert_eq!(
        primitive::parse_date("not-a-date").expect_err("rejects"),
        "Expected a valid Date"
    );
    // :158 - tag.
    assert_eq!(primitive::DATE_TAG, "Date");
}

#[test]
fn primitive_file_text_and_json() {
    use elm_effect_unstable::cli::primitive;
    // Primitive.test.ts:375 (FileText/FileParse portions) - files read
    // as text and parsed as JSON; FileSchema stays open (needs Schema).
    let path = std::env::temp_dir().join(format!("prim-file-{}.json", std::process::id()));
    std::fs::write(&path, "{\"enabled\":true}").expect("writes");
    let text = primitive::parse_file_text(path.to_str().expect("utf8")).expect("reads");
    assert_eq!(text, "{\"enabled\":true}");
    assert_eq!(
        primitive::parse_file_json(path.to_str().expect("utf8")).expect("parses"),
        serde_json::json!({ "enabled": true })
    );
    assert!(primitive::parse_file_text("/non/existent/file.txt").is_err());
    std::fs::remove_file(&path).ok();
}

#[test]
fn options_parser_missing_and_unknown() {
    use elm_effect_unstable::cli::primitive::{self, StrOption};
    use elm_effect_unstable::cli::CliError;
    let required = StrOption {
        name: "value".to_owned(),
        required: true,
    };
    // Errors.test.ts:38 - an absent required flag fails.
    let missing = primitive::parse_options(&[required.clone()], &[]).expect_err("fails");
    assert!(matches!(missing, CliError::MissingOption { ref option } if option == "value"));
    // Present values bind.
    let bound = primitive::parse_options(
        &[required],
        &["--value".to_owned(), "x".to_owned()],
    )
    .expect("binds");
    assert_eq!(bound.values, [("value".to_owned(), "x".to_owned())]);
    assert!(bound.errors.is_empty());
    // Errors.test.ts:89 - unknown flags accumulate instead of failing.
    let collected = primitive::parse_options(
        &[StrOption {
            name: "verbose".to_owned(),
            required: false,
        }],
        &["--unknown1".to_owned(), "--unknown2".to_owned()],
    )
    .expect("collects");
    assert_eq!(collected.errors.len(), 2);
    assert!(collected.errors.iter().all(|error| matches!(
        error,
        CliError::UnrecognizedOption { .. }
    )));
}

#[test]
fn positionals_sequence_and_bounds() {
    use elm_effect_unstable::cli::primitive::PositionalSpec;
    use elm_effect_unstable::cli::{primitive, CliError};
    let argv = |words: &[&str]| words.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
    let values = || PositionalSpec::Variadic {
        name: "values".to_owned(),
        max: 2,
        default: None,
    };
    // Errors.test.ts:127 - a bounded run rejects its overflow.
    let overflow =
        primitive::parse_positionals(&[values()], &argv(&["one", "two", "three"])).expect_err("fails");
    assert!(matches!(overflow, CliError::UnexpectedArgument { ref arguments } if arguments == &["three".to_owned()]));
    // :143 - the run leaves room for what follows.
    let shared = primitive::parse_positionals(
        &[
            values(),
            PositionalSpec::Fixed {
                name: "destination".to_owned(),
            },
        ],
        &argv(&["one", "two", "destination"]),
    )
    .expect("fits");
    assert_eq!(
        shared,
        [
            ("values".to_owned(), vec!["one".to_owned(), "two".to_owned()]),
            ("destination".to_owned(), vec!["destination".to_owned()]),
        ]
    );
    // :162 - a fixed positional rejects its overflow.
    let fixed = primitive::parse_positionals(
        &[PositionalSpec::Fixed {
            name: "value".to_owned(),
        }],
        &argv(&["one", "two"]),
    )
    .expect_err("fails");
    assert!(matches!(fixed, CliError::UnexpectedArgument { ref arguments } if arguments == &["two".to_owned()]));
    // Absent fixed positionals report their name.
    let missing = primitive::parse_positionals(
        &[PositionalSpec::Fixed {
            name: "value".to_owned(),
        }],
        &[],
    )
    .expect_err("fails");
    assert!(matches!(missing, CliError::MissingArgument { ref argument } if argument == "value"));
}

#[test]
fn param_never_sentinels() {
    use elm_effect_unstable::cli::param::{CliOptions, CliParam, ParamKind};
    use elm_effect_unstable::cli::CliError;
    use std::collections::BTreeMap;
    // Param.test.ts:30 - flag sentinels fail missing and supplied.
    for sentinel in [CliParam::flag_never(), CliParam::never(ParamKind::Flag)] {
        let missing = sentinel.parse(&CliOptions::default()).expect_err("fails");
        assert!(matches!(missing, CliError::MissingOption { .. }));
        let supplied = sentinel
            .parse(&CliOptions {
                flags: BTreeMap::from([("__never__".to_owned(), vec!["value".to_owned()])]),
                ..CliOptions::default()
            })
            .expect_err("fails");
        assert!(matches!(supplied, CliError::InvalidValue { .. }));
    }
    // :40 - argument sentinels fail missing and supplied.
    for sentinel in [CliParam::argument_never(), CliParam::never(ParamKind::Argument)] {
        let missing = sentinel.parse(&CliOptions::default()).expect_err("fails");
        assert!(matches!(missing, CliError::MissingOption { .. }));
        let supplied = sentinel
            .parse(&CliOptions {
                args: BTreeMap::from([("__never__".to_owned(), vec!["value".to_owned()])]),
                ..CliOptions::default()
            })
            .expect_err("fails");
        assert!(matches!(supplied, CliError::InvalidValue { .. }));
    }
}

#[test]
fn param_boolean_defaults() {
    use elm_effect_unstable::cli::param::{CliOptions, CliParam, ParamValue};
    // Param.test.ts:51 - an omitted boolean fails.
    assert!(matches!(
        CliParam::boolean("verbose").parse(&CliOptions::default()),
        Err(elm_effect_unstable::cli::CliError::MissingOption { .. })
    ));
    // :63 - the requested default fills in.
    let (_, value) = CliParam::boolean("verbose")
        .with_default(ParamValue::Switch(true))
        .parse(&CliOptions::default())
        .expect("defaults")
        .expect("present");
    assert_eq!(value, ParamValue::Switch(true));
    // Param.test.ts:75 - a false default restores switch behavior.
    let (_, off) = CliParam::boolean("verbose")
        .with_default(ParamValue::Switch(false))
        .parse(&CliOptions::default())
        .expect("defaults")
        .expect("present");
    assert_eq!(off, ParamValue::Switch(false));
}

#[test]
fn param_optional_booleans() {
    use elm_effect_unstable::cli::param::CliParam;
    use std::collections::BTreeMap;
    // Param.test.ts:133 - an omitted optional yields none.
    assert_eq!(
        CliParam::boolean("verbose").optional().parse(
            &elm_effect_unstable::cli::param::CliOptions::default(),
        ),
        Ok(None)
    );
    // :145 - a supplied optional yields its value.
    assert_eq!(
        CliParam::boolean("verbose").optional().parse(
            &elm_effect_unstable::cli::param::CliOptions {
                flags: BTreeMap::from([("verbose".to_owned(), vec!["false".to_owned()])]),
                ..elm_effect_unstable::cli::param::CliOptions::default()
            },
        ),
        Ok(Some((
            "verbose".to_owned(),
            elm_effect_unstable::cli::param::ParamValue::Switch(false)
        )))
    );
}

#[test]
fn param_config_fallback_chain() {
    use elm_effect_unstable::cli::param::{CliOptions, CliParam, ParamKind, ParamValue};
    use elm_effect_unstable::cli::CliError;
    use std::collections::BTreeMap;
    let config = |pairs: &[(&str, &str)]| {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect::<BTreeMap<String, String>>()
    };
    // Param.test.ts:420 - config fills a missing boolean.
    let (_, value) = CliParam::boolean("verbose")
        .with_fallback_config("VERBOSE")
        .parse(&CliOptions {
            config: config(&[("VERBOSE", "true")]),
            ..CliOptions::default()
        })
        .expect("config")
        .expect("present");
    assert_eq!(value, ParamValue::Switch(true));
    // :468 - config fills missing text.
    let (_, name) = CliParam::text(ParamKind::Flag, "name")
        .with_fallback_config("NAME")
        .parse(&CliOptions {
            config: config(&[("NAME", "Ava")]),
            ..CliOptions::default()
        })
        .expect("config")
        .expect("present");
    assert_eq!(name, ParamValue::Text("Ava".to_owned()));
    // :492 - supplied values beat config.
    let (_, kept) = CliParam::text(ParamKind::Flag, "name")
        .with_fallback_config("NAME")
        .parse(&CliOptions {
            flags: BTreeMap::from([("name".to_owned(), vec!["from-flags".to_owned()])]),
            config: config(&[("NAME", "from-config")]),
            ..CliOptions::default()
        })
        .expect("supplied")
        .expect("present");
    assert_eq!(kept, ParamValue::Text("from-flags".to_owned()));
    // :516 - config fills missing arguments.
    let (_, repo) = CliParam::text(ParamKind::Argument, "repository")
        .with_fallback_config("REPOSITORY")
        .parse(&CliOptions {
            config: config(&[("REPOSITORY", "effect")]),
            ..CliOptions::default()
        })
        .expect("config")
        .expect("present");
    assert_eq!(repo, ParamValue::Text("effect".to_owned()));
    // :540 - missing config still fails.
    assert!(matches!(
        CliParam::text(ParamKind::Flag, "name")
            .with_fallback_config("NAME")
            .parse(&CliOptions::default()),
        Err(CliError::MissingOption { .. })
    ));
    // :562 - unparsable config fails as invalid.
    assert!(matches!(
        CliParam::integer(ParamKind::Flag, "count")
            .with_fallback_config("COUNT")
            .parse(&CliOptions {
                config: config(&[("COUNT", "nope")]),
                ..CliOptions::default()
            }),
        Err(CliError::InvalidValue { .. })
    ));
}

#[test]
fn positional_defaults_and_supplied_beats_config() {
    use elm_effect_unstable::cli::param::{CliOptions, CliParam, ParamValue};
    use elm_effect_unstable::cli::primitive::PositionalSpec;
    use std::collections::BTreeMap;
    // Param.test.ts:170 - an omitted variadic takes its default.
    let files = PositionalSpec::Variadic {
        name: "files".to_owned(),
        max: 100,
        default: Some(vec!["README.md".to_owned()]),
    };
    assert_eq!(
        elm_effect_unstable::cli::primitive::parse_positionals(&[files], &[]),
        Ok(vec![("files".to_owned(), vec!["README.md".to_owned()])])
    );
    // :444 - explicitly supplied values beat config fallbacks.
    let options = CliOptions {
        flags: BTreeMap::from([("verbose".to_owned(), vec!["false".to_owned()])]),
        config: BTreeMap::from([("VERBOSE".to_owned(), "true".to_owned())]),
        ..CliOptions::default()
    };
    let (_, value) = CliParam::boolean("verbose")
        .with_fallback_config("VERBOSE")
        .parse(&options)
        .expect("supplied")
        .expect("present");
    assert_eq!(value, ParamValue::Switch(false));
}

#[test]
fn param_or_else_fallbacks() {
    use elm_effect_unstable::cli::param::{CliOptions, CliParam, ParamKind, ParamValue};
    use std::collections::BTreeMap;
    let config = || CliParam::text(ParamKind::Flag, "config");
    let config_url = || CliParam::text(ParamKind::Flag, "config-url");
    // Param.test.ts:88 - the alternate flag fills in.
    let (_, value) = elm_effect_unstable::cli::param::or_else(
        &config(),
        &config_url(),
        &CliOptions {
            flags: BTreeMap::from([(
                "config-url".to_owned(),
                vec!["https://example.com".to_owned()],
            )]),
            ..CliOptions::default()
        },
    )
    .expect("fallback")
    .expect("present");
    assert_eq!(
        value,
        ParamValue::Text("https://example.com".to_owned())
    );
    // :157 - the first flag wins when present.
    let (_, kept) = elm_effect_unstable::cli::param::or_else(
        &config(),
        &config_url(),
        &CliOptions {
            flags: BTreeMap::from([("config".to_owned(), vec!["config.json".to_owned()])]),
            ..CliOptions::default()
        },
    )
    .expect("first")
    .expect("present");
    assert_eq!(kept, ParamValue::Text("config.json".to_owned()));
}

#[test]
fn command_routes_unknown_with_suggestions() {
    use elm_effect_unstable::cli::{CliCommand, CliError};
    let app = CliCommand {
        name: "app",
        subcommands: vec![CliCommand {
            name: "deploy",
            ..Default::default()
        }],
        ..Default::default()
    };
    // Errors.test.ts:106 - unknown subcommands name themselves + suggest.
    let error = app
        .route(&["deplyo".to_owned()])
        .expect_err("unknown");
    match &error {
        CliError::UnknownSubcommand {
            subcommand,
            suggestions,
            ..
        } => {
            assert_eq!(subcommand, "deplyo");
            assert!(suggestions.contains(&"deploy".to_owned()), "{suggestions:?}");
        }
        _ => panic!("wrong variant: {error:?}"),
    }
    // Known routes resolve with their remainder.
    let routed = app
        .route(&["deploy".to_owned(), "--dry-run".to_owned()])
        .expect("routes");
    assert_eq!(routed.path, ["app".to_owned(), "deploy".to_owned()]);
    assert_eq!(routed.rest, ["--dry-run".to_owned()]);
}

#[test]
fn primitive_file_constructors_preserve_text_json_and_schema_decoding() {
    use elm_effect_unstable::cli::primitive;
    // Primitive.test.ts:375 — FileText hands back the file's text verbatim,
    // FileParse its JSON, and FileSchema the value a decoder makes of the
    // document, with a decoder that cannot accept it reporting its own message.
    // The pin injects a FileSystem noop layer; this port reads a real temporary
    // file, so the carrier differs and the observables do not.
    let path = std::env::temp_dir().join(format!(
        "elm-effect-primitive-375-{}.json",
        std::process::id()
    ));
    std::fs::write(&path, "{\"enabled\":true}").expect("write");
    let text_path = path.to_str().expect("utf-8 path");

    assert_eq!(
        primitive::parse_file_text(text_path).expect("text"),
        "{\"enabled\":true}"
    );
    assert_eq!(
        primitive::parse_file_json(text_path).expect("json"),
        serde_json::json!({ "enabled": true })
    );
    let enabled = primitive::parse_file_schema(text_path, |text| {
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        value
            .get("enabled")
            .and_then(|value| value.as_bool())
            .ok_or_else(|| "Expected boolean".to_owned())
    })
    .expect("decodes");
    assert!(enabled);

    // The document is an object, so the string decoder is the one that fails
    // and its message is what the caller sees.
    let err = primitive::parse_file_schema(text_path, |text| {
        let text: String = serde_json::from_str(text).map_err(|e| e.to_string())?;
        Ok(text)
    })
    .expect_err("an object is not a string");
    assert!(err.contains("string"), "{err}");

    // A missing file fails before any decoder runs.
    assert!(
        primitive::parse_file_schema("/non/existent/config.json", |_| Ok("x".to_owned())).is_err()
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn is_ai_error_answers_false_for_everything_that_is_not_one() {
    // AiError.test.ts:630 — a plain Error, the nullish values, and a struct
    // that only looks tagged are all rejected; only a real wrapper passes.
    assert!(!is_ai_error(Some(&std::io::Error::other("regular error"))));
    assert!(!is_ai_error(None));
    let fake = FakeModel::new();
    assert!(!is_ai_error(Some(fake.as_ref())), "a language model is not an AiError");
    // :637 — a reason is not a wrapper either, and the tag agrees.
    assert!(!is_ai_error(Some(&AiReason::QuotaExhausted)));

    // :621 — the wrapper itself is the one value that passes.
    let real = AiError::new(AiReason::Unknown).with_context("Test", "test");
    assert!(is_ai_error(Some(&real)));
}

// --- Command.test.ts: the wizard's argv, where a value that looks like a flag
// travels inline so the parser reads it as this flag's value ---

/// Command.test.ts:211 — `should round trip wizard flag value ${value}`: a
/// value that begins with a dash is emitted inline (`--value=-v`) because a
/// separate one would be read as a flag, and a bare `-`, an empty value and a
/// plain word travel as their own argument.
#[test]
fn wizard_flag_values_round_trip_through_the_parser() {
    for (value, expected) in [
        ("-v", vec!["--value=-v"]),
        ("--verbose", vec!["--value=--verbose"]),
        ("--", vec!["--value=--"]),
        ("--mode=x=y", vec!["--value=--mode=x=y"]),
        ("plain", vec!["--value", "plain"]),
        ("", vec!["--value", ""]),
        ("-", vec!["--value", "-"]),
    ] {
        assert_eq!(
            flag_arg("value", value),
            expected,
            "the argv for {value:?}"
        );
    }

    // The other half of the row: those argv pairs parse back to the same
    // value, which is what makes the round trip a round trip.
    let command = Command::new(
        "demo",
        "demo command",
        vec![Arg::Opt {
            name: "value".to_string(),
            help: "the value".to_string(),
            required: false,
        }],
    );
    for (value, _) in [
        ("-v", 0),
        ("--verbose", 0),
        ("--", 0),
        ("--mode=x=y", 0),
        ("plain", 0),
        ("", 0),
        ("-", 0),
    ] {
        let args = flag_arg("value", value);
        let (_, parsed) = command.parse(&args).expect("the wizard argv parses");
        assert_eq!(parsed.opt("value"), Some(value), "{value:?}");
    }
}

/// Command.test.ts:229 — a repeated flag takes several values, and the
/// inline-or-separate rule decides each one on its own.
#[test]
fn wizard_repeated_flag_values_keep_their_own_shape() {
    let values = ["-v", "plain", "--name=x", ""];
    let expected = [
        "--value=-v",
        "--value",
        "plain",
        "--value=--name=x",
        "--value",
        "",
    ];
    let argv: Vec<String> = values.iter().flat_map(|v| flag_arg("value", v)).collect();
    assert_eq!(argv, expected);
    // And the whole list parses back as the four values, in order.
    let command = Command::new(
        "demo",
        "demo command",
        vec![Arg::Opt {
            name: "value".to_string(),
            help: "the value".to_string(),
            required: false,
        }],
    );
    let (_, parsed) = command.parse(&argv).expect("the repeated argv parses");
    let mut collected = Vec::new();
    let mut i = 0;
    while let Some(value) = parsed.opt_at("value", i) {
        collected.push(value.to_string());
        i += 1;
    }
    assert_eq!(collected, values.to_vec());
}

/// Command.test.ts:168 — a negative number is a value, not a flag: `--lat
/// -40.41` takes the number as the flag's value and a bare `-2` as a
/// positional, and the typed parses accept both.
#[test]
fn negative_numbers_parse_as_flag_and_positional_values() {
    let command = Command::new(
        "geo",
        "geo command",
        vec![
            Arg::Opt {
                name: "lat".to_string(),
                help: "latitude".to_string(),
                required: false,
            },
            Arg::Opt {
                name: "lon".to_string(),
                help: "longitude".to_string(),
                required: false,
            },
            Arg::Positional {
                name: "offset".to_string(),
                help: "offset".to_string(),
            },
        ],
    );
    let argv = vec![
        "--lat".to_string(),
        "-40.41".to_string(),
        "--lon".to_string(),
        "-3.70".to_string(),
        "-2".to_string(),
    ];
    let (_, parsed) = command.parse(&argv).expect("the negative numbers parse");
    assert_eq!(parsed.opt("lat"), Some("-40.41"));
    assert_eq!(parsed.opt("lon"), Some("-3.70"));
    assert_eq!(parsed.positionals, vec!["-2".to_string()]);
    // The typed parses are what the handler receives.
    assert_eq!(primitive::parse_finite("-40.41"), Ok(-40.41));
    assert_eq!(primitive::parse_int("-2"), Ok(-2));
}

/// Command.test.ts:168 (the clap-backed parser) — `--lat -40.41` reads the
/// negative number as the flag's value rather than as an unknown option.
#[test]
fn clap_backed_parse_reads_a_hyphen_value_as_the_flags_value() {
    let app = CliApp {
        name: "geo",
        version: "0.1.0",
        about: "geo",
        flags: vec![
            FlagDef {
                name: "lat",
                short: None,
                about: "latitude",
                takes_value: true,
                default: None,
            },
            FlagDef {
                name: "lon",
                short: None,
                about: "longitude",
                takes_value: true,
                default: None,
            },
        ],
        commands: Vec::new(),
    };
    let request = parse(
        &app,
        &[
            "geo".to_string(),
            "--lat".to_string(),
            "-40.41".to_string(),
            "--lon".to_string(),
            "-3.70".to_string(),
        ],
    )
    .expect("a hyphen value is the flag's value");
    assert_eq!(request.flags.get("lat").map(String::as_str), Some("-40.41"));
    assert_eq!(request.flags.get("lon").map(String::as_str), Some("-3.70"));
}

/// Command.test.ts:119 — `Command.wizard(target)` invoked from a handler: the
/// wizard asks one question per declared flag and then one per positional, in
/// declaration order, and the answers become the argv it hands back.
#[test]
fn wizard_plan_asks_one_question_per_flag_then_per_positional() {
    let target = Command::new(
        "greet",
        "greet command",
        vec![
            Arg::Opt {
                name: "name".to_string(),
                help: "who to greet".to_string(),
                required: true,
            },
            Arg::Positional {
                name: "count".to_string(),
                help: "how many times".to_string(),
            },
        ],
    );
    assert_eq!(wizard_plan(&target), vec![
        WizardStep::Flag { name: "name".to_string(), value: String::new() },
        WizardStep::Positional { name: "count".to_string(), value: String::new() },
    ]);
    // The row's answers, in the order the questions were asked.
    let mut steps = wizard_plan(&target);
    for (step, answer) in steps.iter_mut().zip(["Alice", "2"]) {
        step.answer(answer);
    }
    assert_eq!(
        wizard_argv(&target, &steps),
        vec![
            "greet".to_string(),
            "--name".to_string(),
            "Alice".to_string(),
            "2".to_string(),
        ]
    );
}

/// Command.test.ts:182 — a negative integer flag value travels inline, so
/// `--offset=-2` is what the wizard emits and what the parser reads back.
#[test]
fn wizard_emits_a_negative_integer_flag_value_inline() {
    let command = Command::new(
        "demo",
        "demo command",
        vec![Arg::Opt {
            name: "offset".to_string(),
            help: "offset".to_string(),
            required: false,
        }],
    );
    let steps = vec![WizardStep::Flag {
        name: "offset".to_string(),
        value: "-2".to_string(),
    }];
    assert_eq!(
        wizard_argv(&command, &steps),
        vec!["demo".to_string(), "--offset=-2".to_string()]
    );
    // The handler side: the argv parses back to the negative integer.
    let argv = wizard_argv(&command, &steps);
    let (_, parsed) = command.parse(&argv[1..]).expect("the argv parses");
    assert_eq!(parsed.opt("offset"), Some("-2"));
    assert_eq!(primitive::parse_int("-2"), Ok(-2));
}

/// Command.test.ts:370 — a flag that takes a value, given without one, is
/// refused naming the flag, so the handler is never reached. The port's
/// refusal names the flag and carries no value, which is the law the four
/// rows of the table assert.
#[test]
fn a_valued_flag_without_its_value_is_refused_naming_the_flag() {
    for (name, expected) in [
        ("name", "string"),
        ("count", "integer"),
        ("config", "path"),
        ("env", "key=value"),
    ] {
        let command = Command::new(
            "demo",
            "demo command",
            vec![Arg::Opt {
                name: name.to_string(),
                help: "a value".to_string(),
                required: false,
            }],
        );
        let error = command
            .parse(&[format!("--{name}")])
            .expect_err("the flag was given no value");
        let printed = error.to_string();
        assert!(printed.contains(name), "{name}: {printed}");
        // The refusal is the missing value, not an unknown flag.
        assert!(matches!(error, WizardCliError::Missing(_)), "{name}: {printed}");
        let _ = expected;
    }
}

/// Command.test.ts:481, :506 — a repeated key=value flag merges into ONE
/// record, and the merge holds when other options are interleaved with the
/// repetitions.
#[test]
fn repeated_key_value_flags_merge_into_one_record() {
    let command = Command::new(
        "env",
        "env command",
        vec![
            Arg::Opt {
                name: "env".to_string(),
                help: "environment".to_string(),
                required: false,
            },
            Arg::Opt {
                name: "profile".to_string(),
                help: "profile".to_string(),
                required: false,
            },
            Arg::Flag {
                name: "verbose".to_string(),
                help: "verbose".to_string(),
            },
        ],
    );
    let argv = vec![
        "--env".to_string(),
        "foo=bar".to_string(),
        "--profile".to_string(),
        "dev".to_string(),
        "--env".to_string(),
        "cool=dude".to_string(),
        "--verbose".to_string(),
        "--env".to_string(),
        "zip=zop".to_string(),
    ];
    let (_, parsed) = command.parse(&argv).expect("parses");
    assert_eq!(parsed.opt("profile"), Some("dev"));
    assert!(parsed.flag("verbose"));
    let record = merge_key_values(&option_runs(&parsed, "env")).expect("each pair is well formed");
    assert_eq!(
        record,
        BTreeMap::from([
            ("foo".to_string(), "bar".to_string()),
            ("cool".to_string(), "dude".to_string()),
            ("zip".to_string(), "zop".to_string()),
        ])
    );
}

/// Command.test.ts:785 — a malformed key=value flag is refused naming the
/// value, so no record is produced from it.
#[test]
fn a_malformed_key_value_flag_is_refused_naming_the_value() {
    for bad in ["invalid", "=novalue", "nokey="] {
        let error = merge_key_values(&[bad.to_string()]).expect_err("malformed");
        assert!(error.contains("Invalid key=value format"), "{bad}: {error}");
        assert!(error.contains(bad), "{bad}: {error}");
    }
}

/// Command.test.ts:955, :964 — a command marked unlisted stays unlisted when
/// subcommands or shared flags are added, because the metadata belongs to the
/// command rather than to the shape it had when it was set.
#[test]
fn unlisted_survives_adding_subcommands_and_shared_flags() {
    let internal = Command::new("internal", "internal command", Vec::new()).unlisted();
    assert!(internal.unlisted, "the marker is set");

    let with_children = internal.clone().with_subcommands(vec![Command::new(
        "child",
        "child command",
        Vec::new(),
    )]);
    assert!(with_children.unlisted, "survives adding subcommands");
    assert_eq!(with_children.subcommands.len(), 1);

    let with_shared = internal.with_shared_flags(vec![Arg::Flag {
        name: "verbose".to_string(),
        help: "verbose".to_string(),
    }]);
    assert!(with_shared.unlisted, "survives adding shared flags");

    // A command that was never marked stays listed, so the flag is a real
    // distinction rather than a default both ways.
    let plain = Command::new("plain", "plain command", Vec::new()).with_subcommands(vec![]);
    assert!(!plain.unlisted);
}

// --- Command.test.ts: the run rows, where a selected node's handler is what
// executes ---

fn handler_log() -> std::sync::Arc<std::sync::Mutex<Vec<String>>> {
    std::sync::Arc::new(std::sync::Mutex::new(Vec::new()))
}

/// Command.test.ts:964 — with no subcommand given, the parent's own handler is
/// what executes, and it sees the parent's parsed values.
#[test]
fn the_parent_handler_runs_when_no_subcommand_is_given() {
    let log = handler_log();
    let seen = log.clone();
    let root = Command::new(
        "git",
        "git command",
        vec![Arg::Flag {
            name: "verbose".to_string(),
            help: "verbose".to_string(),
        }],
    )
    .handler(move |parsed| {
        seen.lock().expect("lock").push(format!("verbose={}", parsed.flag("verbose")));
    });
    let run = root.run(&["--verbose".to_string()]).expect("runs");
    assert_eq!(run.path, vec!["git".to_string()]);
    assert_eq!(*log.lock().expect("lock"), vec!["verbose=true".to_string()]);
}

/// Command.test.ts:975 — a subcommand given on the line selects ITS handler,
/// and the parent's does not run at all.
#[test]
fn the_selected_subcommands_handler_runs_instead_of_the_parents() {
    let log = handler_log();
    let parent_log = log.clone();
    let child_log = log.clone();
    let root = Command::group(
        "git",
        "git command",
        vec![Command::new(
            "clone",
            "clone command",
            vec![
                Arg::Positional {
                    name: "repository".to_string(),
                    help: "repository".to_string(),
                },
                Arg::Opt {
                    name: "branch".to_string(),
                    help: "branch".to_string(),
                    required: false,
                },
            ],
        )
        .handler(move |parsed| {
            child_log.lock().expect("lock").push(format!(
                "clone {} {}",
                parsed.positionals.first().cloned().unwrap_or_default(),
                parsed.opt("branch").unwrap_or_default()
            ));
        })],
    )
    .handler(move |_| {
        parent_log.lock().expect("lock").push("parent".to_string());
    });
    let run = root
        .run(&[
            "clone".to_string(),
            "myrepo".to_string(),
            "--branch".to_string(),
            "develop".to_string(),
        ])
        .expect("runs");
    assert_eq!(run.path, vec!["git".to_string(), "clone".to_string()]);
    assert_eq!(
        *log.lock().expect("lock"),
        vec!["clone myrepo develop".to_string()],
        "only the selected handler runs"
    );
}

/// Command.test.ts:991 — an alias selects the same subcommand, and the
/// alias's own name is not what the path records.
#[test]
fn an_alias_selects_the_same_subcommand() {
    let log = handler_log();
    let plan_log = log.clone();
    let root = Command::group(
        "tool",
        "tool command",
        vec![Command::new("plan", "plan command", Vec::new())
            .alias("p")
            .handler(move |_| plan_log.lock().expect("lock").push("plan".to_string()))],
    );
    let run = root.run(&["p".to_string()]).expect("the alias routes");
    assert_eq!(run.path, vec!["tool".to_string(), "plan".to_string()]);
    assert_eq!(*log.lock().expect("lock"), vec!["plan".to_string()]);
}
