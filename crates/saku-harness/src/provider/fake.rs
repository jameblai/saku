//! Scripted fake Provider for Harness tests.

use std::collections::VecDeque;
use std::sync::Mutex;

use futures::stream;
use serde_json::Value;

use super::{Provider, ProviderError, ProviderStream};
use crate::types::{ProviderEvent, Request, ToolCall};

/// One scripted assistant turn.
#[derive(Debug, Clone)]
pub enum ScriptedResponse {
    /// Stream text deltas then complete (no tool calls).
    Text(String),
    /// Emit tool calls (arguments as JSON values) then complete.
    ToolCalls(Vec<ToolCall>),
    /// Fail the completion.
    Error(String),
}

/// Records requests and returns scripted responses in order.
#[derive(Debug, Default)]
pub struct FakeProvider {
    responses: Mutex<VecDeque<ScriptedResponse>>,
    requests: Mutex<Vec<Request>>,
}

impl FakeProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, response: ScriptedResponse) {
        self.responses.lock().expect("lock").push_back(response);
    }

    pub fn push_text(&self, text: impl Into<String>) {
        self.push(ScriptedResponse::Text(text.into()));
    }

    pub fn push_error(&self, message: impl Into<String>) {
        self.push(ScriptedResponse::Error(message.into()));
    }

    pub fn push_tool_calls(&self, calls: Vec<ToolCall>) {
        self.push(ScriptedResponse::ToolCalls(calls));
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().expect("lock").clone()
    }

    pub fn last_request(&self) -> Option<Request> {
        self.requests.lock().expect("lock").last().cloned()
    }
}

impl Provider for FakeProvider {
    fn complete(&self, request: Request) -> ProviderStream {
        self.requests.lock().expect("lock").push(request);
        let next = self.responses.lock().expect("lock").pop_front();
        let events: Vec<Result<ProviderEvent, ProviderError>> = match next {
            Some(ScriptedResponse::Text(text)) => {
                let mut out = Vec::new();
                // Chunk into small deltas so adapters exercise streaming.
                for chunk in split_deltas(&text) {
                    out.push(Ok(ProviderEvent::TextDelta(chunk)));
                }
                out.push(Ok(ProviderEvent::MessageComplete));
                out
            }
            Some(ScriptedResponse::ToolCalls(calls)) => {
                let mut out = Vec::new();
                for call in calls {
                    out.push(Ok(ProviderEvent::ToolCall {
                        id: call.id,
                        name: call.name,
                        arguments: call.arguments,
                    }));
                }
                out.push(Ok(ProviderEvent::MessageComplete));
                out
            }
            Some(ScriptedResponse::Error(message)) => {
                vec![Err(ProviderError::Message(message))]
            }
            None => vec![Err(ProviderError::Message(
                "FakeProvider: no scripted response left".into(),
            ))],
        };
        Box::pin(stream::iter(events))
    }
}

fn split_deltas(text: &str) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    // Prefer word-ish chunks; fall back to whole string.
    let mut chunks = Vec::new();
    let mut buf = String::new();
    for (i, ch) in text.chars().enumerate() {
        buf.push(ch);
        if buf.len() >= 8 || ch == ' ' || i == text.chars().count() - 1 {
            chunks.push(std::mem::take(&mut buf));
        }
    }
    if !buf.is_empty() {
        chunks.push(buf);
    }
    chunks
}

/// Helper for tests building tool-call scripts.
pub fn tool_call(id: &str, name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    }
}
