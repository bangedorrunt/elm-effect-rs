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
        /// Invoked tool name.
        tool_name: String,
        /// Validation detail.
        description: String,
    },
    /// Tool returned an unusable result.
    InvalidToolResult {
        /// Invoked tool name.
        tool_name: String,
        /// Result detail.
        description: String,
    },
    /// Tool result could not be encoded.
    ToolResultEncoding {
        /// Invoked tool name.
        tool_name: String,
        /// Unencodable result text.
        tool_result: String,
        /// Encoding detail.
        description: String,
    },
    /// Tool misconfigured.
    ToolConfiguration {
        /// Invoked tool name.
        tool_name: String,
        /// Configuration detail.
        description: String,
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
        matches!(
            self,
            Self::RateLimit { .. }
                | Self::InternalProvider { .. }
                | Self::InvalidOutput { .. }
                | Self::StructuredOutput { .. }
                | Self::ToolNotFound { .. }
                | Self::ToolParameterValidation { .. }
        )
    }

    /// Human message, mirroring the tag's `message` getters.
    pub fn message(&self) -> String {
        match self {
            Self::RateLimit { retry_after_secs } => match retry_after_secs {
                Some(secs) => format!("Rate limit exceeded. Retry after {secs}s"),
                None => "Rate limit exceeded".to_owned(),
            },
            Self::QuotaExhausted => "Quota exhausted".to_owned(),
            Self::Authentication { kind } => {
                let suggestion = match kind.as_str() {
                    "InvalidKey" => "Verify your API key is correct",
                    "ExpiredKey" => "Your API key has expired. Generate a new one",
                    "MissingKey" => "No API key provided. Set the appropriate environment variable",
                    "InsufficientPermissions" => "Your API key lacks required permissions",
                    _ => "Authentication failed. Check your credentials",
                };
                format!("{kind}: {suggestion}")
            }
            Self::ContentPolicy { description } => match description {
                Some(d) => format!("Content policy violation: {d}"),
                None => "Content policy violation".to_owned(),
            },
            Self::InvalidRequest {
                parameter,
                constraint,
            } => {
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
            Self::ToolParameterValidation { tool_name, description } => {
                format!("Invalid parameters for tool '{tool_name}': {description}")
            }
            Self::InvalidToolResult { tool_name, description } => {
                format!("Tool '{tool_name}' returned invalid result: {description}")
            }
            Self::ToolResultEncoding { tool_name, description, .. } => {
                format!("Failed to encode result for tool '{tool_name}': {description}")
            }
            Self::ToolConfiguration { tool_name, description } => {
                format!("Invalid configuration for tool '{tool_name}': {description}")
            },
            Self::StructuredOutput { details } => format!("Structured output error: {details}"),
            Self::UnsupportedSchema { details } => format!("Unsupported schema: {details}"),
            Self::InvalidUserInput { details } => format!("Invalid user input: {details}"),
            Self::ToolkitRequired => "Toolkit required".to_owned(),
            Self::NetworkError { details } => format!("Network error: {details}"),
        }
    }

    /// Compose an attached description the way the tag's `message` getter
    /// does: appended after the kind suggestion for authentication errors,
    /// replacing the default everywhere else.
    pub fn message_with_description(&self, description: &str) -> String {
        match self {
            Self::Authentication { .. } => format!("{}. {description}", self.message()),
            _ => description.to_owned(),
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

    /// Attach a description, composed per reason by [`AiError::message`].
    pub fn with_description(mut self, description: &str) -> Self {
        self.description = Some(description.to_owned());
        self
    }

    /// Classify an HTTP status into an error, mirroring `reasonFromHttpStatus`.
    pub fn reason_from_http_status(status: u16, description: Option<&str>) -> Self {
        let reason = match status {
            400 => AiReason::InvalidRequest {
                parameter: String::new(),
                constraint: String::new(),
            },
            401 => AiReason::Authentication {
                kind: "InvalidKey".to_owned(),
            },
            403 => AiReason::Authentication {
                kind: "InsufficientPermissions".to_owned(),
            },
            429 => AiReason::RateLimit {
                retry_after_secs: None,
            },
            _ if status >= 500 => AiReason::InternalProvider {
                details: "Server error".to_owned(),
            },
            _ => AiReason::Unknown,
        };
        let mut error = AiError::new(reason);
        if let Some(description) = description {
            error = error.with_description(description);
        }
        error
    }

    /// Rendered message: the description composed per reason when present,
    /// else the default.
    pub fn message(&self) -> String {
        match &self.description {
            Some(d) => self.reason.message_with_description(d),
            None => self.reason.message(),
        }
    }
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message())
    }
}

// --- Chat port (ai/Chat.ts at the pin): persisted history with TTL ---

/// Message role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatRole {
    /// End-user message.
    User,
    /// Assistant message.
    Assistant,
    /// Tool result message.
    Tool,
}

/// One chat message with a stable identifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMessage {
    /// Stable id (`msg-N` in encounter order unless set explicitly).
    pub id: String,
    /// Who sent it.
    pub role: ChatRole,
    /// Text content.
    pub content: String,
}

/// Missing chat error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatNotFound {
    /// Requested chat id.
    pub chat_id: String,
}

impl std::fmt::Display for ChatNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "chat not found: {}", self.chat_id)
    }
}

struct StoredChat {
    history: Vec<ChatMessage>,
    /// Last write time, for TTL expiry.
    written_at: std::time::Instant,
    next_id: u64,
}

/// Persisted chat histories with an optional time to live.
pub struct ChatStore {
    chats: std::sync::Mutex<std::collections::HashMap<String, StoredChat>>,
    ttl: Option<std::time::Duration>,
}

impl ChatStore {
    /// Empty store with no expiry.
    pub fn new() -> Self {
        Self {
            chats: std::sync::Mutex::new(std::collections::HashMap::new()),
            ttl: None,
        }
    }

    /// Empty store expiring chats `ttl` after their last write.
    pub fn with_ttl(ttl: std::time::Duration) -> Self {
        Self {
            chats: std::sync::Mutex::new(std::collections::HashMap::new()),
            ttl: Some(ttl),
        }
    }

    fn expired(&self, written_at: std::time::Instant) -> bool {
        match self.ttl {
            None => false,
            Some(ttl) => written_at.elapsed() >= ttl,
        }
    }

    /// Persist `history` under `chat_id`, assigning `msg-N` ids to messages
    /// without one (preserving explicit ids).
    pub fn save(&self, chat_id: &str, history: Vec<ChatMessage>) {
        let mut chats = self.chats.lock().expect("chat store");
        let slot = chats
            .entry(chat_id.to_owned())
            .or_insert_with(|| StoredChat {
                history: Vec::new(),
                written_at: std::time::Instant::now(),
                next_id: 1,
            });
        for mut message in history {
            if message.id.is_empty() {
                message.id = format!("msg-{}", slot.next_id);
                slot.next_id += 1;
            }
            slot.history.push(message);
        }
        slot.written_at = std::time::Instant::now();
    }

    /// Load history, or [`ChatNotFound`] when missing or expired.
    pub fn load(&self, chat_id: &str) -> Result<Vec<ChatMessage>, ChatNotFound> {
        let mut chats = self.chats.lock().expect("chat store");
        match chats.get(chat_id) {
            Some(stored) if !self.expired(stored.written_at) => Ok(stored.history.clone()),
            _ => {
                chats.remove(chat_id);
                Err(ChatNotFound {
                    chat_id: chat_id.to_owned(),
                })
            }
        }
    }

    /// Identifier of the most recent assistant message, if any. New
    /// assistant-bound identifiers prefer this over a fresh counter, so
    /// follow-ups keep the thread's identity.
    pub fn latest_assistant_id(&self, chat_id: &str) -> Option<String> {
        let chats = self.chats.lock().expect("chat store");
        chats.get(chat_id)?.history.iter().rev().find_map(|m| {
            if m.role == ChatRole::Assistant {
                Some(m.id.clone())
            } else {
                None
            }
        })
    }
}

impl Default for ChatStore {
    fn default() -> Self {
        Self::new()
    }
}

// --- Tool port (ai/Tool.ts at the pin): named handlers, failure modes ---

/// How handler failures surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FailureMode {
    /// Return the failure as a tool result.
    Return,
    /// Raise the failure as an error.
    Error,
}

/// Tool call failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolError {
    /// Failing tool name.
    pub tool: String,
    /// Failure detail.
    pub message: String,
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.tool, self.message)
    }
}

/// Handler over raw JSON parameter strings.
type HandlerFn = Box<dyn Fn(&str) -> Result<String, String> + Send + Sync>;

/// Named handler over raw JSON parameter strings.
pub struct Tool {
    /// Tool name.
    pub name: String,
    /// Human description.
    pub description: String,
    handler: HandlerFn,
}

impl Tool {
    /// Build from a name, description, and parameter handler.
    pub fn new(
        name: &str,
        description: &str,
        handler: impl Fn(&str) -> Result<String, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.to_owned(),
            description: description.to_owned(),
            handler: Box::new(handler),
        }
    }

    /// Call with JSON params. Malformed params (including `NaN`, which is
    /// not valid JSON) fail before the handler runs.
    pub fn call(&self, params_json: &str) -> Result<String, ToolError> {
        let fail = |message: String| ToolError {
            tool: self.name.clone(),
            message,
        };
        let trimmed = params_json.trim();
        if !(trimmed.starts_with('{') && trimmed.ends_with('}')) {
            return Err(fail("invalid parameters: not a JSON object".to_owned()));
        }
        if trimmed.contains("NaN") {
            return Err(fail("invalid parameters: NaN is not valid JSON".to_owned()));
        }
        (self.handler)(params_json).map_err(fail)
    }

    /// Call under a failure mode: `Return` hands failures back as tool
    /// results, `Error` raises them.
    pub fn call_with_mode(
        &self,
        params_json: &str,
        mode: &FailureMode,
    ) -> Result<Result<String, String>, ToolError> {
        match self.call(params_json) {
            Ok(output) => Ok(Ok(output)),
            Err(error) => match mode {
                FailureMode::Return => Ok(Err(error.message.clone())),
                FailureMode::Error => Err(error),
            },
        }
    }
}
