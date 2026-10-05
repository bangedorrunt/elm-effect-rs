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
