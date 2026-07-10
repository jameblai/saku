//! Shared harness types: content parts, transcript, Run/Provider events.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Effort;

/// Multimodal content shared across turns, tool results, and Provider requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    Image { mime: String, bytes: Vec<u8> },
}

impl ContentPart {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn image(mime: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self::Image {
            mime: mime.into(),
            bytes,
        }
    }
}

/// One user invocation into a Session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserTurn {
    pub text: String,
    pub images: Vec<ContentPart>,
}

impl UserTurn {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            images: Vec::new(),
        }
    }
}

/// Role in the Session transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    #[serde(rename = "tool_result")]
    Tool,
}

/// A tool call requested by the assistant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// One message in the Harness transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
}

impl Message {
    pub fn user_text(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: vec![ContentPart::text(text)],
            tool_call_id: None,
            tool_calls: Vec::new(),
        }
    }

    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: vec![ContentPart::text(text)],
            tool_call_id: None,
            tool_calls: Vec::new(),
        }
    }

    pub fn from_user_turn(turn: &UserTurn) -> Self {
        let mut content = Vec::new();
        if !turn.text.is_empty() {
            content.push(ContentPart::text(&turn.text));
        }
        content.extend(turn.images.iter().cloned());
        Self {
            role: Role::User,
            content,
            tool_call_id: None,
            tool_calls: Vec::new(),
        }
    }
}

/// Events streamed on a [`crate::session::RunHandle`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunEvent {
    RunStarted,
    Queued,
    Dequeued,
    TextDelta { text: String },
    ReasoningDelta { text: String },
    ToolStarted { name: String, args: Value },
    ToolProgress { message: String },
    ToolFinished { name: String, ok: bool },
    AssistantFinished,
    RunFinished,
    RunError { message: String },
    RunAborted,
}

/// Events from a Provider completion stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderEvent {
    TextDelta(String),
    ReasoningDelta(String),
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
    },
    MessageComplete,
    Error(String),
}

/// Tool definition exposed to the Provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// Request sent to a Provider.
#[derive(Debug, Clone)]
pub struct Request {
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub model: String,
    pub effort: Effort,
}
