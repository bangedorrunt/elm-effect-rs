//! `ai/LanguageModel` port (`ai/LanguageModel.ts` at the pin).
//!
//! The [`LanguageModel`] trait is the contract: text generation, object
//! generation against a schema shape, and streamed chunks. [`FakeModel`]
//! scripts responses so tests never touch a provider; no provider ships in
//! any default build (there is no provider code here at all).
//!
//! Residuals: toolkits and tool approval flows, provider options plumbing,
//! telemetry spans, tokenizer bindings, MCP/Anthropic/OpenAI surfaces, and
//! the full response/tool-call shapes (plain text + JSON strings here).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Generated text plus the stop reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratedText {
    /// Output text.
    pub text: String,
    /// Why generation stopped (`"stop"`, `"length"`, ...).
    pub finish_reason: String,
}

/// Model failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelError {
    /// Provider refused or failed.
    Provider(String),
    /// Output did not satisfy the schema shape.
    BadResponse(String),
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Provider(e) => write!(f, "provider: {e}"),
            Self::BadResponse(e) => write!(f, "bad response: {e}"),
        }
    }
}

/// Language model contract: text, schema-shaped objects, streams.
pub trait LanguageModel: Send + Sync {
    /// Generate text for `prompt`.
    fn generate_text(&self, prompt: &str) -> Result<GeneratedText, ModelError>;
    /// Generate a JSON object for `prompt` satisfying `schema_json`
    /// (top-level keys named by the schema; checked shallowly).
    fn generate_object(&self, prompt: &str, schema_json: &str) -> Result<String, ModelError>;
    /// Stream text chunks for `prompt`.
    fn stream_text(&self, prompt: &str) -> Vec<String>;
}

/// Shallow check: every key in `keys` appears as `"key"` in `json`.
pub fn object_has_keys(json: &str, keys: &[&str]) -> bool {
    let trimmed = json.trim();
    if !(trimmed.starts_with('{') && trimmed.ends_with('}')) {
        return false;
    }
    keys.iter().all(|k| json.contains(&format!("\"{k}\"")))
}

/// Scripted model for tests: queued text/object/chunk responses.
#[derive(Debug, Default)]
pub struct FakeModel {
    texts: Mutex<VecDeque<Result<GeneratedText, ModelError>>>,
    objects: Mutex<VecDeque<Result<String, ModelError>>>,
    chunks: Mutex<VecDeque<Vec<String>>>,
    calls: Mutex<Vec<String>>,
}

/// Shared scripted model.
pub type SharedFake = Arc<FakeModel>;

impl FakeModel {
    /// Empty script.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Queue one text response.
    pub fn push_text(&self, response: Result<GeneratedText, ModelError>) {
        self.texts.lock().expect("fake").push_back(response);
    }

    /// Queue one object response.
    pub fn push_object(&self, response: Result<String, ModelError>) {
        self.objects.lock().expect("fake").push_back(response);
    }

    /// Queue one chunk stream.
    pub fn push_chunks(&self, chunks: Vec<String>) {
        self.chunks.lock().expect("fake").push_back(chunks);
    }

    /// Prompts received so far.
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("fake").clone()
    }
}

impl LanguageModel for Arc<FakeModel> {
    fn generate_text(&self, prompt: &str) -> Result<GeneratedText, ModelError> {
        self.calls.lock().expect("fake").push(prompt.to_owned());
        self.texts
            .lock()
            .expect("fake")
            .pop_front()
            .unwrap_or(Ok(GeneratedText {
                text: String::new(),
                finish_reason: "stop".to_owned(),
            }))
    }

    fn generate_object(&self, prompt: &str, _schema_json: &str) -> Result<String, ModelError> {
        self.calls.lock().expect("fake").push(prompt.to_owned());
        self.objects
            .lock()
            .expect("fake")
            .pop_front()
            .unwrap_or(Ok("{}".to_owned()))
    }

    fn stream_text(&self, prompt: &str) -> Vec<String> {
        self.calls.lock().expect("fake").push(prompt.to_owned());
        self.chunks
            .lock()
            .expect("fake")
            .pop_front()
            .unwrap_or_default()
    }
}

// --- AiError port (AiError.ts at the pin): tagged reasons, messages, retryability ---

/// AI failure reason tag. Mirrors the tag's `Schema.tag` names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AiReason {
    /// Rate limited; optional retry-after in seconds.
    RateLimit {
        /// Retry delay, if the provider gave one.
        retry_after_secs: Option<u64>,
    },
    /// Quota exhausted; never retryable without user action.
    QuotaExhausted,
    /// Bad credentials or key shape.
    Authentication {
        /// Machine-readable kind (`"InvalidKey"`, ...).
        kind: String,
    },
    /// Content refused by policy, with an optional description.
    ContentPolicy {
        /// Human detail, if any.
        description: Option<String>,
    },
    /// Bad request parameter with its constraint.
    InvalidRequest {
        /// Parameter name.
        parameter: String,
        /// Constraint text.
        constraint: String,
    },
    /// Bad model output with details.
    InvalidOutput {
        /// What was wrong.
        details: String,
    },
    /// Provider-side failure with details.
    InternalProvider {
        /// Provider detail.
        details: String,
    },
    /// Unclassified failure.
    Unknown,
    /// Unknown tool name plus the available tool list.
    ToolNotFound {
        /// Requested tool.
        tool: String,
        /// Available tool names.
        available: Vec<String>,
    },
    /// Tool parameter failed validation.
    ToolParameterValidation {
        /// Detail text.
        details: String,
    },
    /// Tool returned an unusable result.
    InvalidToolResult {
        /// Detail text.
        details: String,
    },
    /// Tool result could not be encoded.
    ToolResultEncoding {
        /// Detail text.
        details: String,
    },
    /// Tool misconfigured.
    ToolConfiguration {
        /// Detail text.
        details: String,
    },
    /// Structured output did not satisfy the schema.
    StructuredOutput {
        /// Detail text.
        details: String,
    },
    /// Schema shape unsupported by the provider.
    UnsupportedSchema {
        /// Detail text.
        details: String,
    },
    /// Bad user input.
    InvalidUserInput {
        /// Detail text.
        details: String,
    },
    /// Toolkit required but missing.
    ToolkitRequired,
    /// Network failure.
    NetworkError {
        /// Detail text.
        details: String,
    },
}

impl AiReason {
    /// Tag name, mirroring `Schema.tag`.
    pub fn tag(&self) -> &'static str {
        match self {
            Self::RateLimit { .. } => "RateLimitError",
            Self::QuotaExhausted => "QuotaExhaustedError",
            Self::Authentication { .. } => "AuthenticationError",
            Self::ContentPolicy { .. } => "ContentPolicyError",
            Self::InvalidRequest { .. } => "InvalidRequestError",
            Self::InvalidOutput { .. } => "InvalidOutputError",
            Self::InternalProvider { .. } => "InternalProviderError",
            Self::Unknown => "UnknownError",
            Self::ToolNotFound { .. } => "ToolNotFoundError",
            Self::ToolParameterValidation { .. } => "ToolParameterValidationError",
            Self::InvalidToolResult { .. } => "InvalidToolResultError",
            Self::ToolResultEncoding { .. } => "ToolResultEncodingError",
            Self::ToolConfiguration { .. } => "ToolConfigurationError",
            Self::StructuredOutput { .. } => "StructuredOutputError",
            Self::UnsupportedSchema { .. } => "UnsupportedSchemaError",
            Self::InvalidUserInput { .. } => "InvalidUserInputError",
            Self::ToolkitRequired => "ToolkitRequiredError",
            Self::NetworkError { .. } => "NetworkError",
        }
    }

    /// Whether retrying may succeed.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::RateLimit { .. }
            | Self::InternalProvider { .. }
            | Self::InvalidOutput { .. }
            | Self::StructuredOutput { .. }
            | Self::ToolNotFound { .. }
            | Self::ToolParameterValidation { .. } => true,
            _ => false,
        }
    }

    /// Human message, mirroring the tag's `message` getters.
    pub fn message(&self) -> String {
        match self {
            Self::RateLimit { retry_after_secs } => match retry_after_secs {
                Some(secs) => format!("Rate limit exceeded. Retry after {secs}s"),
                None => "Rate limit exceeded".to_owned(),
            },
            Self::QuotaExhausted => "Quota exhausted".to_owned(),
            Self::Authentication { kind } => format!("Authentication failed: {kind}"),
            Self::ContentPolicy { description } => match description {
                Some(d) => format!("Content policy violation: {d}"),
                None => "Content policy violation".to_owned(),
            },
            Self::InvalidRequest { parameter, constraint } => {
                format!("Invalid request: parameter '{parameter}' {constraint}")
            }
            Self::InvalidOutput { details } => format!("Invalid output: {details}"),
            Self::InternalProvider { details } => format!("Internal provider error: {details}"),
            Self::Unknown => "Unknown error".to_owned(),
            Self::ToolNotFound { tool, available } => {
                let list = if available.is_empty() {
                    "none".to_owned()
                } else {
                    available.join(", ")
                };
                format!("Tool '{tool}' not found. Available tools: {list}")
            }
            Self::ToolParameterValidation { details } => {
                format!("Tool parameter validation failed: {details}")
            }
            Self::InvalidToolResult { details } => format!("Invalid tool result: {details}"),
            Self::ToolResultEncoding { details } => format!("Tool result encoding failed: {details}"),
            Self::ToolConfiguration { details } => format!("Tool misconfigured: {details}"),
            Self::StructuredOutput { details } => format!("Structured output error: {details}"),
            Self::UnsupportedSchema { details } => format!("Unsupported schema: {details}"),
            Self::InvalidUserInput { details } => format!("Invalid user input: {details}"),
            Self::ToolkitRequired => "Toolkit required".to_owned(),
            Self::NetworkError { details } => format!("Network error: {details}"),
        }
    }
}

/// Tagged AI failure: a reason with its message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiError {
    /// Machine tag (mirrors `_tag`).
    pub tag: String,
    /// Structured reason.
    pub reason: AiReason,
    /// Caller description, overriding the default message when present.
    pub description: Option<String>,
    /// Whether retrying may succeed.
    pub is_retryable: bool,
}

impl AiError {
    /// Build from a reason, rendering tag/retryability from it.
    pub fn new(reason: AiReason) -> Self {
        Self {
            tag: reason.tag().to_owned(),
            description: None,
            is_retryable: reason.is_retryable(),
            reason,
        }
    }

    /// Attach a description: the message renders it unchanged.
    pub fn with_description(mut self, description: &str) -> Self {
        self.description = Some(description.to_owned());
        self
    }

    /// Rendered message: the description when present, else the default.
    pub fn message(&self) -> String {
        match &self.description {
            Some(d) => d.clone(),
            None => self.reason.message(),
        }
    }
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message())
    }
}
