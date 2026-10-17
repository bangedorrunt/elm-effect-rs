//! Prompt construction from response parts (`ai/Prompt.ts` at the pin).
//!
//! Ports the assembly half of `Prompt`: parts with options-as-metadata,
//! streamed-delta folding with metadata accumulation, tool-call placement
//! (provider-executed results stay in the assistant message, the final
//! non-preliminary framework result forms the tool message), message merge,
//! and leading-system text. Schema codecs are out of scope: JSON
//! round-trips go through `serde`, which is this port's codec.

use serde_json::{Map, Value};

/// Provider metadata attached to a part.
pub type Options = Map<String, Value>;

/// Deep-merge two option maps: objects merge recursively, otherwise the
/// later value wins. This is how streamed metadata accumulates.
pub fn merge_options(into: &mut Options, extra: Options) {
    for (key, value) in extra {
        match (into.remove(&key), value) {
            (Some(Value::Object(mut into_map)), Value::Object(extra_map)) => {
                merge_options(&mut into_map, extra_map);
                into.insert(key, Value::Object(into_map));
            }
            (_, value) => {
                into.insert(key, value);
            }
        }
    }
}

/// A message role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// System instruction.
    System,
    /// End-user message.
    User,
    /// Assistant message.
    Assistant,
    /// Tool message.
    Tool,
}

/// One prompt part.
#[derive(Debug, Clone, PartialEq)]
pub enum PromptPart {
    /// Plain text.
    Text {
        /// The text.
        text: String,
        /// Provider options.
        options: Options,
    },
    /// Model reasoning.
    Reasoning {
        /// The reasoning text.
        text: String,
        /// Provider options.
        options: Options,
    },
    /// A file attachment (base64 data).
    File {
        /// Media type.
        media_type: String,
        /// Base64-encoded bytes.
        data_base64: String,
        /// Provider options.
        options: Options,
    },
    /// A tool call awaiting execution.
    ToolCall {
        /// Call id.
        id: String,
        /// Tool name.
        name: String,
        /// Raw params.
        params: Value,
        /// Executed by the provider rather than the framework.
        provider_executed: bool,
        /// Provider options.
        options: Options,
    },
    /// A tool result.
    ToolResult {
        /// Call id.
        id: String,
        /// Tool name.
        name: String,
        /// Whether the tool failed.
        is_failure: bool,
        /// The result value.
        result: Value,
        /// Executed by the provider rather than the framework.
        provider_executed: bool,
        /// A progress update rather than the final result.
        preliminary: bool,
        /// Provider options.
        options: Options,
    },
    /// An approval request attached to a tool call.
    ToolApprovalRequest {
        /// Approval id.
        approval_id: String,
        /// Owning tool call id.
        tool_call_id: String,
        /// Provider options.
        options: Options,
    },
    /// An approval decision.
    ToolApprovalResponse {
        /// Approval id.
        approval_id: String,
        /// Whether the call was approved.
        approved: bool,
        /// Provider options.
        options: Options,
    },
}

impl PromptPart {
    /// Plain text without options.
    pub fn text(text: &str) -> Self {
        Self::Text {
            text: text.to_owned(),
            options: Options::new(),
        }
    }

    /// Roles whose messages may carry this part.
    pub fn allowed_roles(&self) -> &'static [Role] {
        match self {
            Self::Text { .. } | Self::File { .. } => &[Role::User, Role::Assistant],
            Self::Reasoning { .. } | Self::ToolCall { .. } | Self::ToolApprovalRequest { .. } => {
                &[Role::Assistant]
            }
            Self::ToolResult { .. } | Self::ToolApprovalResponse { .. } => &[Role::Tool],
        }
    }
}

/// One prompt message.
#[derive(Debug, Clone, PartialEq)]
pub struct PromptMessage {
    /// Who sent it.
    pub role: Role,
    /// Its parts.
    pub parts: Vec<PromptPart>,
}

impl PromptMessage {
    /// A message with these parts.
    pub fn new(role: Role, parts: Vec<PromptPart>) -> Self {
        Self { role, parts }
    }

    /// A single text part from a raw string.
    pub fn text(role: Role, text: &str) -> Self {
        Self::new(role, vec![PromptPart::text(text)])
    }
}

/// A prompt: an ordered list of messages.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Prompt {
    /// The messages.
    pub messages: Vec<PromptMessage>,
}

impl Prompt {
    /// No messages.
    pub fn empty() -> Self {
        Self::default()
    }

    /// A prompt from these messages.
    pub fn make(messages: Vec<PromptMessage>) -> Self {
        Self { messages }
    }

    /// Append `right`'s messages after `left`'s.
    pub fn concat(left: &Prompt, right: Vec<PromptMessage>) -> Self {
        let mut messages = left.messages.clone();
        messages.extend(right);
        Self { messages }
    }

    /// Text appended to the leading system message, creating it when absent.
    pub fn append_system(prompt: &Prompt, text: &str) -> Self {
        Self::edit_leading_system(prompt, text, false)
    }

    /// Text prepended to the leading system message, creating it when absent.
    pub fn prepend_system(prompt: &Prompt, text: &str) -> Self {
        Self::edit_leading_system(prompt, text, true)
    }

    fn edit_leading_system(prompt: &Prompt, text: &str, prepend: bool) -> Self {
        let mut messages = prompt.messages.clone();
        match messages.first_mut() {
            Some(PromptMessage {
                role: Role::System,
                parts,
            }) => {
                let current = part_text(parts);
                let next = if prepend {
                    format!("{text}{current}")
                } else {
                    format!("{current}{text}")
                };
                *parts = vec![PromptPart::text(&next)];
            }
            _ => messages.insert(0, PromptMessage::text(Role::System, text)),
        }
        Self { messages }
    }
}

/// The concatenated text of a message's text parts.
fn part_text(parts: &[PromptPart]) -> String {
    parts
        .iter()
        .map(|part| match part {
            PromptPart::Text { text, .. } => text.clone(),
            _ => String::new(),
        })
        .collect()
}

/// One response part flowing into `from_response_parts`.
#[derive(Debug, Clone)]
pub enum ResponsePart {
    /// Start of a streamed text part.
    TextStart {
        /// Stream id.
        id: String,
        /// Part metadata.
        metadata: Options,
    },
    /// One text delta.
    TextDelta {
        /// Stream id.
        id: String,
        /// Appended text.
        delta: String,
        /// Part metadata.
        metadata: Options,
    },
    /// End of a streamed text part.
    TextEnd {
        /// Stream id.
        id: String,
        /// Part metadata.
        metadata: Options,
    },
    /// Start of a streamed reasoning part.
    ReasoningStart {
        /// Stream id.
        id: String,
        /// Part metadata.
        metadata: Options,
    },
    /// One reasoning delta.
    ReasoningDelta {
        /// Stream id.
        id: String,
        /// Appended text.
        delta: String,
        /// Part metadata.
        metadata: Options,
    },
    /// End of a streamed reasoning part.
    ReasoningEnd {
        /// Stream id.
        id: String,
        /// Part metadata.
        metadata: Options,
    },
    /// A complete text part.
    Text {
        /// The text.
        text: String,
        /// Part metadata.
        metadata: Options,
    },
    /// A complete reasoning part.
    Reasoning {
        /// The text.
        text: String,
        /// Part metadata.
        metadata: Options,
    },
    /// A complete file part.
    File {
        /// Raw bytes.
        data: Vec<u8>,
        /// Media type.
        media_type: String,
        /// Part metadata.
        metadata: Options,
    },
    /// A tool call.
    ToolCall {
        /// Call id.
        id: String,
        /// Tool name.
        name: String,
        /// Raw params.
        params: Value,
        /// Executed by the provider.
        provider_executed: bool,
        /// Part metadata.
        metadata: Options,
    },
    /// A tool result.
    ToolResult {
        /// Call id.
        id: String,
        /// Tool name.
        name: String,
        /// Whether the tool failed.
        is_failure: bool,
        /// The result value.
        result: Value,
        /// A progress update rather than the final result.
        preliminary: bool,
        /// Executed by the provider.
        provider_executed: bool,
        /// Part metadata.
        metadata: Options,
    },
}

/// Base64 without padding surprises: the pin's `Uint8Array` file data travels
/// as standard base64 text.
fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let mut word = 0u32;
        for (index, byte) in chunk.iter().enumerate() {
            word |= (*byte as u32) << (16 - 8 * index);
        }
        let digits = [18, 12, 6, 0];
        for (position, shift) in digits.into_iter().enumerate() {
            if position <= chunk.len() {
                out.push(ALPHABET[((word >> shift) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A streamed text or reasoning part under construction.
struct StreamAccumulator {
    id: String,
    reasoning: bool,
    text: String,
    options: Options,
}

impl StreamAccumulator {
    fn finish(self) -> PromptPart {
        if self.reasoning {
            PromptPart::Reasoning {
                text: self.text,
                options: self.options,
            }
        } else {
            PromptPart::Text {
                text: self.text,
                options: self.options,
            }
        }
    }
}

/// Build a prompt from response parts: streamed deltas fold into one part per
/// stream id (metadata accumulates), file parts carry base64 data, tool calls
/// stay in arrival order in the assistant message, provider-executed results
/// stay beside their call, and each call id contributes only its final
/// non-preliminary framework result to the tool message.
pub fn from_response_parts(parts: Vec<ResponsePart>) -> Prompt {
    let mut assistant: Vec<AssistantSlot> = Vec::new();
    let mut stream_index: Vec<(String, bool, usize)> = Vec::new();
    let mut tool_results: Vec<(String, PromptPart)> = Vec::new();

    for part in parts {
        match part {
            ResponsePart::TextStart { id, metadata } | ResponsePart::TextEnd { id, metadata } => {
                open_stream(
                    &mut assistant,
                    &mut stream_index,
                    id,
                    false,
                    String::new(),
                    metadata,
                );
            }
            ResponsePart::TextDelta {
                id,
                delta,
                metadata,
            } => {
                open_stream(
                    &mut assistant,
                    &mut stream_index,
                    id,
                    false,
                    delta,
                    metadata,
                );
            }
            ResponsePart::ReasoningStart { id, metadata }
            | ResponsePart::ReasoningEnd { id, metadata } => {
                open_stream(
                    &mut assistant,
                    &mut stream_index,
                    id,
                    true,
                    String::new(),
                    metadata,
                );
            }
            ResponsePart::ReasoningDelta {
                id,
                delta,
                metadata,
            } => {
                open_stream(&mut assistant, &mut stream_index, id, true, delta, metadata);
            }
            ResponsePart::Text { text, metadata } => {
                assistant.push(AssistantSlot::Complete(PromptPart::Text {
                    text,
                    options: metadata,
                }));
            }
            ResponsePart::Reasoning { text, metadata } => {
                assistant.push(AssistantSlot::Complete(PromptPart::Reasoning {
                    text,
                    options: metadata,
                }));
            }
            ResponsePart::File {
                data,
                media_type,
                metadata,
            } => {
                assistant.push(AssistantSlot::Complete(PromptPart::File {
                    media_type,
                    data_base64: base64_encode(&data),
                    options: metadata,
                }));
            }
            ResponsePart::ToolCall {
                id,
                name,
                params,
                provider_executed,
                metadata,
            } => {
                assistant.push(AssistantSlot::Complete(PromptPart::ToolCall {
                    id,
                    name,
                    params,
                    provider_executed,
                    options: metadata,
                }));
            }
            ResponsePart::ToolResult {
                id,
                name,
                is_failure,
                result,
                preliminary,
                provider_executed,
                metadata,
            } => {
                let part = PromptPart::ToolResult {
                    id: id.clone(),
                    name,
                    is_failure,
                    result,
                    provider_executed,
                    preliminary,
                    options: metadata,
                };
                if provider_executed {
                    assistant.push(AssistantSlot::Complete(part));
                } else if preliminary {
                    continue;
                } else if let Some(slot) = tool_results.iter_mut().find(|(call, _)| *call == id) {
                    slot.1 = part;
                } else {
                    tool_results.push((id, part));
                }
            }
        }
    }

    let assistant_parts = assistant.into_iter().map(AssistantSlot::finish).collect();
    let mut messages = vec![PromptMessage::new(Role::Assistant, assistant_parts)];
    if !tool_results.is_empty() {
        messages.push(PromptMessage::new(
            Role::Tool,
            tool_results.into_iter().map(|(_, part)| part).collect(),
        ));
    }
    Prompt::make(messages)
}

enum AssistantSlot {
    Complete(PromptPart),
    Stream(StreamAccumulator),
}

impl AssistantSlot {
    fn finish(self) -> PromptPart {
        match self {
            Self::Complete(part) => part,
            Self::Stream(acc) => acc.finish(),
        }
    }
}

fn open_stream(
    assistant: &mut Vec<AssistantSlot>,
    stream_index: &mut Vec<(String, bool, usize)>,
    id: String,
    reasoning: bool,
    delta: String,
    mut metadata: Options,
) {
    let position = match stream_index
        .iter()
        .find(|(known, kind, _)| *known == id && *kind == reasoning)
    {
        Some((_, _, position)) => *position,
        None => {
            assistant.push(AssistantSlot::Stream(StreamAccumulator {
                id: id.clone(),
                reasoning,
                text: String::new(),
                options: Options::new(),
            }));
            let position = assistant.len() - 1;
            stream_index.push((id, reasoning, position));
            position
        }
    };
    if let AssistantSlot::Stream(acc) = &mut assistant[position] {
        acc.text.push_str(&delta);
        merge_options(&mut acc.options, metadata);
    }
}
