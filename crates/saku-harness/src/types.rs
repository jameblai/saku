//! Shared harness types: content parts, transcript, Run/Provider events.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Effort;

/// Session-owned Provider activity that produced a Usage Record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    #[default]
    Run,
    Subagent,
    GoalEvaluator,
}

impl UsageSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Run => "Run",
            Self::Subagent => "Subagents",
            Self::GoalEvaluator => "Goal evaluator",
        }
    }
}

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

/// Token counts from a Provider turn (cache split out of billed input).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl TokenUsage {
    pub fn add_assign(&mut self, other: &TokenUsage) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
    }

    /// Prompt-side tokens useful for context fill (input + cache read + cache write).
    pub fn prompt_tokens(self) -> u64 {
        self.input + self.cache_read + self.cache_write
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct UsageAggregate {
    pub tokens: TokenUsage,
    pub estimated_cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageRecord {
    pub source: UsageSource,
    pub model: String,
    pub tokens: TokenUsage,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct UsageBySource {
    pub run: UsageAggregate,
    pub subagent: UsageAggregate,
    pub goal_evaluator: UsageAggregate,
}

impl UsageBySource {
    pub fn get_mut(&mut self, source: UsageSource) -> &mut UsageAggregate {
        match source {
            UsageSource::Run => &mut self.run,
            UsageSource::Subagent => &mut self.subagent,
            UsageSource::GoalEvaluator => &mut self.goal_evaluator,
        }
    }

    pub fn entries(self) -> [(UsageSource, UsageAggregate); 3] {
        [
            (UsageSource::Run, self.run),
            (UsageSource::Subagent, self.subagent),
            (UsageSource::GoalEvaluator, self.goal_evaluator),
        ]
    }
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
    /// Token usage for the completed Provider turn (may arrive before MessageComplete).
    Usage(TokenUsage),
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
