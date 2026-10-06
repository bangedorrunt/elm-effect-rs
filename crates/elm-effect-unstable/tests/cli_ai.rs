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
        assert_eq!(error.tag, tag);
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
    assert_eq!(nan.tag, "ToolParameterValidationError");
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
        AiError::reason_from_http_status(400, None).tag,
        "InvalidRequestError"
    );
    // :564 — 401 maps to AuthenticationError with InvalidKey.
    let keyed = AiError::reason_from_http_status(401, None);
    assert_eq!(keyed.tag, "AuthenticationError");
    assert!(matches!(&keyed.reason, AiReason::Authentication { kind } if kind == "InvalidKey"));
    // :572 — 403 maps to AuthenticationError with InsufficientPermissions.
    let forbidden = AiError::reason_from_http_status(403, None);
    assert_eq!(forbidden.tag, "AuthenticationError");
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
    assert_eq!(limited.tag, "RateLimitError");
    assert!(limited.is_retryable);
    // :597 — 5xx maps to InternalProviderError.
    assert_eq!(
        AiError::reason_from_http_status(503, None).tag,
        "InternalProviderError"
    );
    // :612 — unknown statuses map to UnknownError.
    assert_eq!(
        AiError::reason_from_http_status(418, None).tag,
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
