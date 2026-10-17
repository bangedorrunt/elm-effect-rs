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

use std::collections::{HashMap, VecDeque};
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
        /// Source response text, when captured.
        response_text: String,
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
            Self::ToolParameterValidation {
                tool_name,
                description,
            } => {
                format!("Invalid parameters for tool '{tool_name}': {description}")
            }
            Self::InvalidToolResult {
                tool_name,
                description,
            } => {
                format!("Tool '{tool_name}' returned invalid result: {description}")
            }
            Self::ToolResultEncoding {
                tool_name,
                description,
                ..
            } => {
                format!("Failed to encode result for tool '{tool_name}': {description}")
            }
            Self::ToolConfiguration {
                tool_name,
                description,
            } => {
                format!("Invalid configuration for tool '{tool_name}': {description}")
            }
            Self::StructuredOutput { details, .. } => format!("Structured output error: {details}"),
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
    /// Wrapper tag, mirroring `AiError._tag` (the reason carries its own tag).
    pub tag: String,
    /// Structured reason.
    pub reason: AiReason,
    /// Provider module, when the caller records one.
    pub module: Option<String>,
    /// Provider method, when the caller records one.
    pub method: Option<String>,
    /// Caller description, overriding the default message when present.
    pub description: Option<String>,
    /// Whether retrying may succeed.
    pub is_retryable: bool,
}

/// `AiError.isAiError` over a possibly-absent borrowed value.
///
/// The tag guards an arbitrary value, so the port has to answer the same
/// question about the same inputs. Rust has no untyped value, so the guard
/// takes a borrowed `Any` and an `Option` slot: `None` is the tag's
/// `null`/`undefined`, which its guard answers `false` for, and a boxed standard
/// error or an unrelated struct is `false` because it is not this type. One
/// call therefore covers every assertion the tag's row makes.
pub fn is_ai_error(value: Option<&dyn std::any::Any>) -> bool {
    value
        .and_then(|value| value.downcast_ref::<AiError>())
        .is_some()
}

impl AiError {
    /// Wrapper type identity, mirroring `AiError.TypeId`.
    pub const TYPE_ID: &'static str = "AiError";

    /// Build from a reason, rendering tag/retryability from it.
    pub fn new(reason: AiReason) -> Self {
        Self {
            tag: AiError::TYPE_ID.to_owned(),
            description: None,
            is_retryable: reason.is_retryable(),
            reason,
            module: None,
            method: None,
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

    /// Record the provider module and method the call was made through.
    pub fn with_context(mut self, module: &str, method: &str) -> Self {
        self.module = Some(module.to_owned());
        self.method = Some(method.to_owned());
        self
    }

    /// Tag of the wrapped reason, mirroring `AiError._tag` on the reason.
    pub fn reason_tag(&self) -> &'static str {
        self.reason.tag()
    }

    /// `true` when this value carries the AiError wrapper tag. Rust's type
    /// system rules out the runtime-shape cases `isAiError` guards in TS.
    pub fn is_ai_error(&self) -> bool {
        self.tag == AiError::TYPE_ID
    }

    /// Retry delay carried by the reason, if any.
    pub fn retry_after_secs(&self) -> Option<u64> {
        match &self.reason {
            AiReason::RateLimit { retry_after_secs } => *retry_after_secs,
            _ => None,
        }
    }

    /// Rendered message: the reason message, prefixed `module.method:` when
    /// the caller recorded a context.
    pub fn message(&self) -> String {
        let rendered = match &self.description {
            Some(d) => self.reason.message_with_description(d),
            None => self.reason.message(),
        };
        match (&self.module, &self.method) {
            (Some(module), Some(method)) => format!("{module}.{method}: {rendered}"),
            _ => rendered,
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

/// Approval-gated once-runner: an approved tool executes the first time its
/// approval resolves and never replays on later turns (`needsApproval`
/// tools in `ai/Chat.ts`: the completed approval is terminal).
pub struct ApprovalTracker {
    completed: std::sync::Mutex<std::collections::HashSet<String>>,
}

impl ApprovalTracker {
    /// Empty tracker: no approval has completed yet.
    pub fn new() -> Self {
        Self { completed: std::sync::Mutex::new(std::collections::HashSet::new()) }
    }

    /// Run `execute` only when `approval_id` never completed; true when it ran.
    pub fn run_once(&self, approval_id: &str, execute: impl FnOnce()) -> bool {
        let mut completed = self.completed.lock().expect("approval tracker");
        if !completed.insert(approval_id.to_owned()) {
            return false;
        }
        execute();
        true
    }
}

impl Default for ApprovalTracker {
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

/// Provider-specific metadata keyed by provider name; each value is JSON
/// metadata or `null`.
pub type ProviderMetadata = HashMap<String, Option<serde_json::Value>>;

/// Token usage totals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageInfo {
    /// Input tokens consumed.
    pub prompt_tokens: u64,
    /// Output tokens produced.
    pub completion_tokens: u64,
    /// Total tokens billed.
    pub total_tokens: u64,
}

impl UsageInfo {
    /// Encode to the wire JSON shape.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "promptTokens": self.prompt_tokens,
            "completionTokens": self.completion_tokens,
            "totalTokens": self.total_tokens,
        })
    }

    /// Decode from the wire JSON shape.
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        Some(Self {
            prompt_tokens: value.get("promptTokens")?.as_u64()?,
            completion_tokens: value.get("completionTokens")?.as_u64()?,
            total_tokens: value.get("totalTokens")?.as_u64()?,
        })
    }
}

/// HTTP request half of an [`HttpContext`].
#[derive(Clone, Debug, PartialEq)]
pub struct HttpRequestContext {
    /// Uppercase method (`POST`).
    pub method: String,
    /// Full URL.
    pub url: String,
    /// Query pairs in order.
    pub url_params: Vec<(String, String)>,
    /// Fragment, when present.
    pub hash: Option<String>,
    /// Request headers.
    pub headers: HashMap<String, String>,
}

/// HTTP response half of an [`HttpContext`].
#[derive(Clone, Debug, PartialEq)]
pub struct HttpResponseContext {
    /// Numeric status.
    pub status: u16,
    /// Response headers.
    pub headers: HashMap<String, String>,
}

/// Request/response/body bundle carried by error metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct HttpContext {
    /// The request.
    pub request: HttpRequestContext,
    /// The response.
    pub response: HttpResponseContext,
    /// Raw body text.
    pub body: String,
}

impl HttpContext {
    /// Encode to the wire JSON shape.
    pub fn to_json(&self) -> serde_json::Value {
        let params: Vec<serde_json::Value> = self
            .request
            .url_params
            .iter()
            .map(|(k, v)| serde_json::json!([k, v]))
            .collect();
        serde_json::json!({
            "request": {
                "method": self.request.method,
                "url": self.request.url,
                "urlParams": params,
                "hash": self.request.hash,
                "headers": self.request.headers,
            },
            "response": {
                "status": self.response.status,
                "headers": self.response.headers,
            },
            "body": self.body,
        })
    }

    /// Decode from the wire JSON shape.
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        let request = value.get("request")?;
        let response = value.get("response")?;
        let mut url_params = Vec::new();
        for pair in request.get("urlParams")?.as_array()? {
            let pair = pair.as_array()?;
            url_params.push((
                pair.first()?.as_str()?.to_owned(),
                pair.get(1)?.as_str()?.to_owned(),
            ));
        }
        let mut headers = HashMap::new();
        for (key, val) in request.get("headers")?.as_object()? {
            headers.insert(key.clone(), val.as_str()?.to_owned());
        }
        let mut response_headers = HashMap::new();
        for (key, val) in response.get("headers")?.as_object()? {
            response_headers.insert(key.clone(), val.as_str()?.to_owned());
        }
        Some(Self {
            request: HttpRequestContext {
                method: request.get("method")?.as_str()?.to_owned(),
                url: request.get("url")?.as_str()?.to_owned(),
                url_params,
                hash: request
                    .get("hash")
                    .and_then(|hash| hash.as_str())
                    .map(str::to_owned),
                headers,
            },
            response: HttpResponseContext {
                status: response.get("status")?.as_u64()? as u16,
                headers: response_headers,
            },
            body: value.get("body")?.as_str()?.to_owned(),
        })
    }
}

fn get_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_owned)
}

fn get_opt_string(value: &serde_json::Value, key: &str) -> Option<Option<String>> {
    match value.get(key) {
        None | Some(serde_json::Value::Null) => Some(None),
        Some(text) => text.as_str().map(|text| Some(text.to_owned())),
    }
}

impl AiReason {
    /// Build from a schema failure message, mirroring the
    /// `fromSchemaError` constructors, which copy `error.message`.
    /// Takes the message directly: no Schema layer exists here yet.
    pub fn invalid_output_from_message(message: &str) -> AiReason {
        AiReason::InvalidOutput {
            details: message.to_owned(),
        }
    }

    /// Build from a schema failure message plus the source response
    /// text, mirroring `StructuredOutputError.fromSchemaError`.
    pub fn structured_output_from_message(message: &str, response_text: &str) -> AiReason {
        AiReason::StructuredOutput {
            details: message.to_owned(),
            response_text: response_text.to_owned(),
        }
    }
}

impl AiReason {
    /// Encode to the tagged JSON shape.
    pub fn to_json(&self) -> serde_json::Value {
        let mut out = serde_json::Map::new();
        out.insert(
            "_tag".to_owned(),
            serde_json::Value::String(self.tag().to_owned()),
        );
        match self {
            AiReason::RateLimit { retry_after_secs } => {
                out.insert(
                    "retryAfter".to_owned(),
                    match retry_after_secs {
                        Some(secs) => serde_json::Value::from(*secs),
                        None => serde_json::Value::Null,
                    },
                );
            }
            AiReason::QuotaExhausted => {}
            AiReason::Authentication { kind } => {
                out.insert("kind".to_owned(), serde_json::Value::String(kind.clone()));
            }
            AiReason::ContentPolicy { description } => {
                out.insert(
                    "description".to_owned(),
                    match description {
                        Some(detail) => serde_json::Value::String(detail.clone()),
                        None => serde_json::Value::Null,
                    },
                );
            }
            AiReason::InvalidRequest {
                parameter,
                constraint,
            } => {
                out.insert(
                    "parameter".to_owned(),
                    serde_json::Value::String(parameter.clone()),
                );
                out.insert(
                    "constraint".to_owned(),
                    serde_json::Value::String(constraint.clone()),
                );
            }
            AiReason::InvalidOutput { details } => {
                out.insert(
                    "details".to_owned(),
                    serde_json::Value::String(details.clone()),
                );
            }
            AiReason::InternalProvider { details } => {
                out.insert(
                    "details".to_owned(),
                    serde_json::Value::String(details.clone()),
                );
            }
            AiReason::Unknown => {}
            AiReason::ToolNotFound { tool, available } => {
                out.insert("tool".to_owned(), serde_json::Value::String(tool.clone()));
                out.insert(
                    "available".to_owned(),
                    serde_json::Value::Array(
                        available
                            .iter()
                            .map(|name| serde_json::Value::String(name.clone()))
                            .collect(),
                    ),
                );
            }
            AiReason::ToolParameterValidation {
                tool_name,
                description,
            } => {
                out.insert(
                    "toolName".to_owned(),
                    serde_json::Value::String(tool_name.clone()),
                );
                out.insert(
                    "description".to_owned(),
                    serde_json::Value::String(description.clone()),
                );
            }
            AiReason::InvalidToolResult {
                tool_name,
                description,
            } => {
                out.insert(
                    "toolName".to_owned(),
                    serde_json::Value::String(tool_name.clone()),
                );
                out.insert(
                    "description".to_owned(),
                    serde_json::Value::String(description.clone()),
                );
            }
            AiReason::ToolResultEncoding {
                tool_name,
                tool_result,
                description,
            } => {
                out.insert(
                    "toolName".to_owned(),
                    serde_json::Value::String(tool_name.clone()),
                );
                out.insert(
                    "toolResult".to_owned(),
                    serde_json::Value::String(tool_result.clone()),
                );
                out.insert(
                    "description".to_owned(),
                    serde_json::Value::String(description.clone()),
                );
            }
            AiReason::ToolConfiguration {
                tool_name,
                description,
            } => {
                out.insert(
                    "toolName".to_owned(),
                    serde_json::Value::String(tool_name.clone()),
                );
                out.insert(
                    "description".to_owned(),
                    serde_json::Value::String(description.clone()),
                );
            }
            AiReason::StructuredOutput {
                details,
                response_text,
            } => {
                out.insert(
                    "details".to_owned(),
                    serde_json::Value::String(details.clone()),
                );
                out.insert(
                    "responseText".to_owned(),
                    serde_json::Value::String(response_text.clone()),
                );
            }
            AiReason::UnsupportedSchema { details } => {
                out.insert(
                    "details".to_owned(),
                    serde_json::Value::String(details.clone()),
                );
            }
            AiReason::InvalidUserInput { details } => {
                out.insert(
                    "details".to_owned(),
                    serde_json::Value::String(details.clone()),
                );
            }
            AiReason::ToolkitRequired => {}
            AiReason::NetworkError { details } => {
                out.insert(
                    "details".to_owned(),
                    serde_json::Value::String(details.clone()),
                );
            }
        }
        serde_json::Value::Object(out)
    }
}
impl AiReason {
    /// Decode from the tagged JSON shape; unknown tags fail.
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        let tag = value.get("_tag")?.as_str()?;
        Some(match tag {
            "RateLimitError" => AiReason::RateLimit {
                retry_after_secs: match value.get("retryAfter") {
                    None | Some(serde_json::Value::Null) => None,
                    Some(secs) => Some(secs.as_u64()?),
                },
            },
            "QuotaExhaustedError" => AiReason::QuotaExhausted,
            "AuthenticationError" => AiReason::Authentication {
                kind: get_string(value, "kind")?,
            },
            "ContentPolicyError" => AiReason::ContentPolicy {
                description: get_opt_string(value, "description")?,
            },
            "InvalidRequestError" => AiReason::InvalidRequest {
                parameter: get_string(value, "parameter")?,
                constraint: get_string(value, "constraint")?,
            },
            "InvalidOutputError" => AiReason::InvalidOutput {
                details: get_string(value, "details")?,
            },
            "InternalProviderError" => AiReason::InternalProvider {
                details: get_string(value, "details")?,
            },
            "UnknownError" => AiReason::Unknown,
            "ToolNotFoundError" => AiReason::ToolNotFound {
                tool: get_string(value, "tool")?,
                available: value
                    .get("available")?
                    .as_array()?
                    .iter()
                    .map(|name| name.as_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()?,
            },
            "ToolParameterValidationError" => AiReason::ToolParameterValidation {
                tool_name: get_string(value, "toolName")?,
                description: get_string(value, "description")?,
            },
            "InvalidToolResultError" => AiReason::InvalidToolResult {
                tool_name: get_string(value, "toolName")?,
                description: get_string(value, "description")?,
            },
            "ToolResultEncodingError" => AiReason::ToolResultEncoding {
                tool_name: get_string(value, "toolName")?,
                tool_result: get_string(value, "toolResult")?,
                description: get_string(value, "description")?,
            },
            "ToolConfigurationError" => AiReason::ToolConfiguration {
                tool_name: get_string(value, "toolName")?,
                description: get_string(value, "description")?,
            },
            "StructuredOutputError" => AiReason::StructuredOutput {
                details: get_string(value, "details")?,
                response_text: get_string(value, "responseText").unwrap_or_default(),
            },
            "UnsupportedSchemaError" => AiReason::UnsupportedSchema {
                details: get_string(value, "details")?,
            },
            "InvalidUserInputError" => AiReason::InvalidUserInput {
                details: get_string(value, "details")?,
            },
            "ToolkitRequiredError" => AiReason::ToolkitRequired,
            "NetworkError" => AiReason::NetworkError {
                details: get_string(value, "details")?,
            },
            _ => return None,
        })
    }
}

impl AiError {
    /// Encode the wrapper with its reason.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "_tag": AiError::TYPE_ID,
            "module": self.module,
            "method": self.method,
            "reason": self.reason.to_json(),
            "description": self.description,
        })
    }

    /// Decode the wrapper with its reason.
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        if value.get("_tag")?.as_str()? != AiError::TYPE_ID {
            return None;
        }
        let mut error = AiError::new(AiReason::from_json(value.get("reason")?)?);
        if let Some(module) = value.get("module").and_then(|module| module.as_str()) {
            error.module = Some(module.to_owned());
        }
        if let Some(method) = value.get("method").and_then(|method| method.as_str()) {
            error.method = Some(method.to_owned());
        }
        if let Some(description) = value.get("description").and_then(|d| d.as_str()) {
            error = error.with_description(description);
        }
        Some(error)
    }
}

// --- EmbeddingModel port (ai/EmbeddingModel.ts at the pin) ---

/// Token usage for an embedding call. `None` is the tag's `undefined`.
#[derive(Clone, Debug, PartialEq)]
pub struct EmbeddingUsage {
    /// Input tokens billed, when the provider reports them.
    pub input_tokens: Option<u64>,
}

impl EmbeddingUsage {
    /// Encode to the wire shape, omitting undefined input tokens.
    pub fn to_json(&self) -> serde_json::Value {
        match self.input_tokens {
            Some(tokens) => serde_json::json!({ "inputTokens": tokens }),
            None => serde_json::json!({}),
        }
    }

    /// Decode from the wire shape; missing or null input tokens is `None`.
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        match value.get("inputTokens") {
            None | Some(serde_json::Value::Null) => Some(Self { input_tokens: None }),
            Some(tokens) => Some(Self {
                input_tokens: Some(tokens.as_u64()?),
            }),
        }
    }
}

/// One embedding vector.
#[derive(Clone, Debug, PartialEq)]
pub struct Embedding {
    /// The vector.
    pub vector: Vec<f64>,
}

/// Single-input embed response.
#[derive(Clone, Debug, PartialEq)]
pub struct EmbedResponse {
    /// The vector.
    pub vector: Vec<f64>,
    /// Usage reported by the provider.
    pub usage: EmbeddingUsage,
}

/// Multi-input embed response.
#[derive(Clone, Debug, PartialEq)]
pub struct EmbedManyResponse {
    /// One embedding per input, in order.
    pub embeddings: Vec<Embedding>,
    /// Usage reported by the provider.
    pub usage: EmbeddingUsage,
}

/// Provider response: one vector per input plus usage.
pub type ProviderResponse = (Vec<Vec<f64>>, EmbeddingUsage);

/// Embedding model: `embed`/`embedMany` over a provider `embedMany`.
pub struct EmbeddingModel {
    embed_many: Box<dyn Fn(&[String]) -> Result<ProviderResponse, AiError> + Send + Sync>,
}

impl EmbeddingModel {
    /// Build from a provider `embedMany` function.
    pub fn new(
        embed_many: impl Fn(&[String]) -> Result<ProviderResponse, AiError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            embed_many: Box::new(embed_many),
        }
    }

    /// Embed one input. Fails with `InvalidOutputError` unless the provider
    /// returns exactly one vector.
    pub fn embed(&self, input: &str) -> Result<EmbedResponse, AiError> {
        let (results, usage) = (self.embed_many)(&[input.to_owned()])?;
        if results.len() != 1 {
            return Err(AiError::new(AiReason::InvalidOutput {
                details: format!("expected 1 embedding, got {}", results.len()),
            }));
        }
        Ok(EmbedResponse {
            vector: results.into_iter().next().unwrap_or_default(),
            usage,
        })
    }

    /// Embed many inputs in order. Empty input bypasses the provider.
    /// Fails with `InvalidOutputError` on a result/input length mismatch.
    pub fn embed_many(&self, inputs: &[&str]) -> Result<EmbedManyResponse, AiError> {
        if inputs.is_empty() {
            return Ok(EmbedManyResponse {
                embeddings: Vec::new(),
                usage: EmbeddingUsage { input_tokens: None },
            });
        }
        let owned: Vec<String> = inputs.iter().map(|input| (*input).to_owned()).collect();
        let (results, usage) = (self.embed_many)(&owned)?;
        if results.len() != owned.len() {
            return Err(AiError::new(AiReason::InvalidOutput {
                details: format!(
                    "expected {} embeddings, got {}",
                    owned.len(),
                    results.len()
                ),
            }));
        }
        Ok(EmbedManyResponse {
            embeddings: results
                .into_iter()
                .map(|vector| Embedding { vector })
                .collect(),
            usage,
        })
    }
}

// --- ResponseIdTracker port (ai/ResponseIdTracker.ts at the pin): incremental prompts ---

use std::sync::atomic::{AtomicU64, Ordering};

static TRACK_UID: AtomicU64 = AtomicU64::new(1);

fn next_track_uid() -> u64 {
    TRACK_UID.fetch_add(1, Ordering::Relaxed)
}

/// Role of a tracked prompt message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrackRole {
    /// System instruction.
    System,
    /// End-user message.
    User,
    /// Assistant message.
    Assistant,
    /// Tool result message.
    Tool,
}

/// One part of a tracked prompt message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrackPart {
    /// Plain text.
    Text(String),
    /// Tool call awaiting execution.
    ToolCall {
        /// Call id.
        id: String,
        /// Tool name.
        name: String,
        /// Raw params.
        params: String,
    },
    /// Approval request attached to an assistant tool call.
    ApprovalRequest {
        /// Approval id.
        approval_id: String,
        /// Owning tool call id.
        tool_call_id: String,
    },
    /// Approval decision attached to a tool message.
    ApprovalResponse {
        /// Approval id.
        approval_id: String,
        /// Whether the call was approved.
        approved: bool,
    },
    /// Executed tool result.
    ToolResult {
        /// Call id.
        id: String,
        /// Tool name.
        name: String,
        /// Result text.
        result: String,
    },
}

/// One prompt message with a stable identity. Cloning preserves the uid, so
/// spreading a previously sent prompt (as the TS tests do) keeps the tracker's
/// object-identity mapping intact. Equality ignores the uid: two messages with
/// the same role and parts are the same content even when minted separately.
#[derive(Clone, Debug, Eq)]
pub struct TrackMessage {
    /// Stable identity minted at construction.
    pub uid: u64,
    /// Who sent it.
    pub role: TrackRole,
    /// Message parts.
    pub parts: Vec<TrackPart>,
}

impl PartialEq for TrackMessage {
    fn eq(&self, other: &Self) -> bool {
        self.role == other.role && self.parts == other.parts
    }
}

impl TrackMessage {
    fn fresh(role: TrackRole, parts: Vec<TrackPart>) -> Self {
        Self { uid: next_track_uid(), role, parts }
    }

    /// System text message.
    pub fn system(text: &str) -> Self {
        Self::fresh(TrackRole::System, vec![TrackPart::Text(text.to_owned())])
    }

    /// User text message.
    pub fn user(text: &str) -> Self {
        Self::fresh(TrackRole::User, vec![TrackPart::Text(text.to_owned())])
    }

    /// Assistant text message.
    pub fn assistant(text: &str) -> Self {
        Self::fresh(TrackRole::Assistant, vec![TrackPart::Text(text.to_owned())])
    }

    /// Assistant message carrying a tool call plus its approval request.
    pub fn assistant_tool_call_with_approval(
        tool_call_id: &str,
        tool_name: &str,
        params: &str,
        approval_id: &str,
    ) -> Self {
        Self::fresh(
            TrackRole::Assistant,
            vec![
                TrackPart::ToolCall {
                    id: tool_call_id.to_owned(),
                    name: tool_name.to_owned(),
                    params: params.to_owned(),
                },
                TrackPart::ApprovalRequest {
                    approval_id: approval_id.to_owned(),
                    tool_call_id: tool_call_id.to_owned(),
                },
            ],
        )
    }

    /// Tool message carrying an approval decision.
    pub fn tool_approval_response(approval_id: &str, approved: bool) -> Self {
        Self::fresh(
            TrackRole::Tool,
            vec![TrackPart::ApprovalResponse { approval_id: approval_id.to_owned(), approved }],
        )
    }

    /// Tool message carrying an executed result.
    pub fn tool_result(tool_call_id: &str, tool_name: &str, result: &str) -> Self {
        Self::fresh(
            TrackRole::Tool,
            vec![TrackPart::ToolResult {
                id: tool_call_id.to_owned(),
                name: tool_name.to_owned(),
                result: result.to_owned(),
            }],
        )
    }
}

/// Tracks which prompt messages produced each provider response id, so a later
/// turn can send `previousResponseId` plus only the messages after the latest
/// assistant turn. Mirrors `ResponseIdTracker.make`: prefix must be fully
/// recognized, otherwise state clears and the caller sends the full prompt.
pub struct ResponseIdTracker {
    seen: Mutex<HashMap<u64, String>>,
}

impl ResponseIdTracker {
    /// Empty tracker.
    pub fn new() -> Self {
        Self { seen: Mutex::new(HashMap::new()) }
    }

    /// Drop all tracked state (session drop recovery).
    pub fn clear(&self) {
        self.seen.lock().expect("tracker").clear();
    }

    /// Record that `prompt` produced `response_id`.
    pub fn mark_messages(&self, prompt: &[TrackMessage], response_id: &str) {
        let mut seen = self.seen.lock().expect("tracker");
        for message in prompt {
            seen.insert(message.uid, response_id.to_owned());
        }
    }

    /// Incremental fragment when the prompt extends tracked history.
    pub fn prepare(&self, prompt: &[TrackMessage]) -> Option<(String, Vec<TrackMessage>)> {
        let mut seen = self.seen.lock().expect("tracker");
        let any_tracked = prompt.iter().any(|m| seen.contains_key(&m.uid));
        if !any_tracked {
            seen.clear();
            return None;
        }
        let last_assistant = prompt.iter().rposition(|m| m.role == TrackRole::Assistant)?;
        if last_assistant == 0 && prompt.len() <= 1 {
            seen.clear();
            return None;
        }
        let mut response_id: Option<String> = None;
        for message in &prompt[..last_assistant] {
            match seen.get(&message.uid) {
                Some(id) => response_id = Some(id.clone()),
                None => {
                    seen.clear();
                    return None;
                }
            }
        }
        let response_id = response_id?;
        let suffix = prompt[last_assistant + 1..].to_vec();
        if suffix.is_empty() {
            seen.clear();
            return None;
        }
        Some((response_id, suffix))
    }
}

impl Default for ResponseIdTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Provider call as seen by the fake: the sent prompt plus incremental fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderCall {
    /// Prompt sent to the provider (stripped when approvals resolve).
    pub prompt: Vec<TrackMessage>,
    /// Prior response id, when the turn continued tracked history.
    pub previous_response_id: Option<String>,
    /// Messages after the latest assistant turn, when tracking applied.
    pub incremental_prompt: Option<Vec<TrackMessage>>,
}

/// Chunk of a scripted text/stream response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrackChunk {
    /// Provider response id.
    Meta(String),
    /// Text payload.
    Text(String),
    /// Stop reason marker.
    Finish,
}

impl TrackChunk {
    /// Whether this chunk is the stop marker.
    pub fn is_finish(&self) -> bool {
        matches!(self, Self::Finish)
    }
}

/// Scripted model recording [`ProviderCall`]s and returning queued response
/// ids with text/finish chunks. The queue order is the response-id order.
pub struct TrackerModel {
    ids: Mutex<VecDeque<String>>,
    calls: Mutex<Vec<ProviderCall>>,
}

impl TrackerModel {
    /// Script `ids` in call order.
    pub fn new(ids: Vec<&str>) -> Self {
        Self {
            ids: ids.into_iter().map(str::to_owned).collect::<VecDeque<_>>().into(),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Prompts recorded so far.
    pub fn calls(&self) -> Vec<ProviderCall> {
        self.calls.lock().expect("tracker model").clone()
    }

    fn next_id(&self) -> String {
        self.ids
            .lock()
            .expect("tracker model")
            .pop_front()
            .unwrap_or_else(|| "resp_fallback".to_owned())
    }

    fn record(
        &self,
        tracker: Option<&ResponseIdTracker>,
        prompt: Vec<TrackMessage>,
    ) -> Vec<TrackChunk> {
        let (previous_response_id, incremental_prompt) = tracker
            .and_then(|t| t.prepare(&prompt))
            .map(|(id, suffix)| (Some(id), Some(suffix)))
            .unwrap_or((None, None));
        let response_id = self.next_id();
        self.calls.lock().expect("tracker model").push(ProviderCall {
            prompt: prompt.clone(),
            previous_response_id,
            incremental_prompt,
        });
        if let Some(t) = tracker {
            t.mark_messages(&prompt, &response_id);
        }
        vec![
            TrackChunk::Meta(response_id.clone()),
            TrackChunk::Text(format!("assistant-{response_id}")),
            TrackChunk::Finish,
        ]
    }

    /// Text turn with optional tracker (object turns share this path).
    pub fn generate_text(
        &self,
        tracker: Option<&ResponseIdTracker>,
        prompt: Vec<TrackMessage>,
    ) -> Vec<TrackChunk> {
        self.record(tracker, prompt)
    }

    /// Object turn: same tracking, schema shape is plain text here.
    pub fn generate_object(
        &self,
        tracker: Option<&ResponseIdTracker>,
        prompt: Vec<TrackMessage>,
        _schema: &str,
    ) -> Vec<TrackChunk> {
        self.record(tracker, prompt)
    }

    /// Streamed turn: records like text, returns metadata plus finish.
    pub fn stream_text(
        &self,
        tracker: Option<&ResponseIdTracker>,
        prompt: Vec<TrackMessage>,
    ) -> Vec<TrackChunk> {
        let (previous_response_id, incremental_prompt) = tracker
            .and_then(|t| t.prepare(&prompt))
            .map(|(id, suffix)| (Some(id), Some(suffix)))
            .unwrap_or((None, None));
        let response_id = self.next_id();
        self.calls.lock().expect("tracker model").push(ProviderCall {
            prompt: prompt.clone(),
            previous_response_id,
            incremental_prompt,
        });
        if let Some(t) = tracker {
            t.mark_messages(&prompt, &response_id);
        }
        vec![TrackChunk::Meta(response_id), TrackChunk::Finish]
    }

    /// Streamed turn with a toolkit present: no tool calls return, so the
    /// stream still ends in a finish part.
    pub fn stream_text_with_toolkit(
        &self,
        tracker: Option<&ResponseIdTracker>,
        prompt: Vec<TrackMessage>,
        _has_toolkit: bool,
    ) -> Vec<TrackChunk> {
        let (previous_response_id, incremental_prompt) = tracker
            .and_then(|t| t.prepare(&prompt))
            .map(|(id, suffix)| (Some(id), Some(suffix)))
            .unwrap_or((None, None));
        self.calls.lock().expect("tracker model").push(ProviderCall {
            prompt,
            previous_response_id,
            incremental_prompt,
        });
        vec![TrackChunk::Finish]
    }

    /// Text turn with one pre-resolved approval: executes the approved
    /// `ApprovalTool` once, appends its result, strips approval artifacts,
    /// then tracks the stripped prompt.
    pub fn generate_text_with_approval(
        &self,
        tracker: Option<&ResponseIdTracker>,
        prompt: Vec<TrackMessage>,
    ) -> Vec<TrackChunk> {
        let mut approved_calls: Vec<(String, String)> = Vec::new();
        let mut approved_ids: Vec<String> = Vec::new();
        for message in &prompt {
            for part in &message.parts {
                if let TrackPart::ApprovalRequest { approval_id, tool_call_id } = part {
                    let approved = prompt.iter().flat_map(|m| &m.parts).any(|p| {
                        matches!(
                            p,
                            TrackPart::ApprovalResponse { approval_id: id, approved: true }
                            if id == approval_id
                        )
                    });
                    if approved {
                        let name = prompt
                            .iter()
                            .flat_map(|m| &m.parts)
                            .find_map(|p| match p {
                                TrackPart::ToolCall { id, name, .. } if id == tool_call_id => {
                                    Some(name.clone())
                                }
                                _ => None,
                            })
                            .unwrap_or_default();
                        approved_calls.push((tool_call_id.clone(), name));
                        approved_ids.push(approval_id.clone());
                    }
                }
            }
        }
        let mut stripped: Vec<TrackMessage> = Vec::new();
        for message in &prompt {
            match message.role {
                TrackRole::Assistant => {
                    let kept: Vec<TrackPart> = message
                        .parts
                        .iter()
                        .filter(|p| match p {
                            TrackPart::ApprovalRequest { approval_id, .. } => {
                                !approved_ids.contains(approval_id)
                            }
                            _ => true,
                        })
                        .cloned()
                        .collect();
                    if !kept.is_empty() {
                        stripped.push(TrackMessage {
                            uid: message.uid,
                            role: message.role.clone(),
                            parts: kept,
                        });
                    }
                }
                TrackRole::Tool => {
                    let kept: Vec<TrackPart> = message
                        .parts
                        .iter()
                        .filter(|p| match p {
                            TrackPart::ApprovalResponse { approval_id, .. } => {
                                !approved_ids.contains(approval_id)
                            }
                            _ => true,
                        })
                        .cloned()
                        .collect();
                    if !kept.is_empty() {
                        stripped.push(TrackMessage {
                            uid: message.uid,
                            role: message.role.clone(),
                            parts: kept,
                        });
                    }
                }
                _ => stripped.push(message.clone()),
            }
        }
        for (tool_call_id, name) in &approved_calls {
            stripped.push(TrackMessage::tool_result(tool_call_id, name, "approved-result"));
        }
        self.record(tracker, stripped)
    }
}
