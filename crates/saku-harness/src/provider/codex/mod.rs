//! Codex Responses API streaming Provider.

mod login;
pub mod models;

use std::sync::Arc;

use futures::stream::{self, StreamExt};
use serde_json::{Value, json};

use crate::credentials::CredentialStore;
use crate::provider::{Provider, ProviderError, ProviderStream};
use crate::types::{ContentPart, ProviderEvent, Request, Role, ToolDefinition};

pub use login::{DeviceCodeInfo, LoginError, LoginNotify, login_device_code};
use login::{chatgpt_account_id, ensure_fresh_access};
use models::{PROVIDER_ID, is_allowed_model};

const CODEX_BASE: &str = "https://chatgpt.com/backend-api";

pub struct CodexProvider {
    store: CredentialStore,
    client: reqwest::Client,
}

impl CodexProvider {
    pub fn new(store: CredentialStore) -> Self {
        Self {
            store,
            client: reqwest::Client::new(),
        }
    }

    pub fn provider_id() -> &'static str {
        PROVIDER_ID
    }
}

impl Provider for CodexProvider {
    fn complete(&self, request: Request) -> ProviderStream {
        let store = self.store.clone();
        let client = self.client.clone();
        Box::pin(async_stream(client, store, request))
    }
}

fn async_stream(
    client: reqwest::Client,
    store: CredentialStore,
    request: Request,
) -> impl StreamExt<Item = Result<ProviderEvent, ProviderError>> + Send {
    stream::once(async move { complete_inner(client, store, request).await })
        .map(|result| match result {
            Ok(events) => stream::iter(events).left_stream(),
            Err(err) => stream::iter(std::iter::once(Err(err))).right_stream(),
        })
        .flatten()
}

async fn complete_inner(
    client: reqwest::Client,
    store: CredentialStore,
    request: Request,
) -> Result<Vec<Result<ProviderEvent, ProviderError>>, ProviderError> {
    if !is_allowed_model(&request.model) {
        return Err(ProviderError::Message(format!(
            "model `{}` is not on the v1 Codex allowlist",
            request.model
        )));
    }

    let access = ensure_fresh_access(&store)
        .await
        .map_err(|e| ProviderError::Message(e.to_string()))?;
    let account_id = chatgpt_account_id(&access).ok_or_else(|| {
        ProviderError::Message("access token missing chatgpt_account_id claim".into())
    })?;

    let body = build_request_body(&request);
    let url = format!("{CODEX_BASE}/codex/responses");
    let response = client
        .post(url)
        .header("Authorization", format!("Bearer {access}"))
        .header("chatgpt-account-id", account_id)
        .header("OpenAI-Beta", "responses=experimental")
        .header("originator", "saku")
        .header("accept", "text/event-stream")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| ProviderError::Message(e.to_string()))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(ProviderError::Message(format!(
            "codex responses error ({status}): {text}"
        )));
    }

    let mut events = Vec::new();
    let mut buffer = String::new();
    let mut tool_args: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut tool_names: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut byte_stream = response.bytes_stream();

    while let Some(chunk) = byte_stream.next().await {
        let chunk = chunk.map_err(|e| ProviderError::Message(e.to_string()))?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(idx) = buffer.find("\n\n") {
            let frame = buffer[..idx].to_string();
            buffer = buffer[idx + 2..].to_string();
            for line in frame.lines() {
                let Some(data) = line.strip_prefix("data:") else {
                    continue;
                };
                let data = data.trim();
                if data.is_empty() || data == "[DONE]" {
                    continue;
                }
                let Ok(value) = serde_json::from_str::<Value>(data) else {
                    continue;
                };
                parse_sse_event(&value, &mut events, &mut tool_args, &mut tool_names);
            }
        }
    }

    if !events
        .iter()
        .any(|e| matches!(e, Ok(ProviderEvent::MessageComplete)))
    {
        events.push(Ok(ProviderEvent::MessageComplete));
    }
    Ok(events)
}

fn parse_sse_event(
    value: &Value,
    events: &mut Vec<Result<ProviderEvent, ProviderError>>,
    tool_args: &mut std::collections::HashMap<String, String>,
    tool_names: &mut std::collections::HashMap<String, String>,
) {
    let Some(ty) = value.get("type").and_then(|v| v.as_str()) else {
        return;
    };
    match ty {
        "response.output_text.delta" | "response.refusal.delta" => {
            if let Some(delta) = value.get("delta").and_then(|v| v.as_str()) {
                events.push(Ok(ProviderEvent::TextDelta(delta.to_string())));
            }
        }
        "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
            if let Some(delta) = value.get("delta").and_then(|v| v.as_str()) {
                events.push(Ok(ProviderEvent::ReasoningDelta(delta.to_string())));
            }
        }
        "response.output_item.added" => {
            if let Some(item) = value.get("item")
                && item.get("type").and_then(|v| v.as_str()) == Some("function_call")
            {
                let call_id = item
                    .get("call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let args = item
                    .get("arguments")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                tool_names.insert(call_id.clone(), name);
                tool_args.insert(call_id, args);
            }
        }
        "response.function_call_arguments.delta" => {
            let call_id = value
                .get("call_id")
                .or_else(|| value.get("item_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if let Some(delta) = value.get("delta").and_then(|v| v.as_str()) {
                tool_args
                    .entry(call_id.to_string())
                    .or_default()
                    .push_str(delta);
            }
        }
        "response.function_call_arguments.done" => {
            let call_id = value
                .get("call_id")
                .or_else(|| value.get("item_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if let Some(args) = value.get("arguments").and_then(|v| v.as_str()) {
                tool_args.insert(call_id.to_string(), args.to_string());
            }
        }
        "response.output_item.done" => {
            if let Some(item) = value.get("item")
                && item.get("type").and_then(|v| v.as_str()) == Some("function_call")
            {
                let call_id = item
                    .get("call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .or_else(|| tool_names.get(&call_id).map(String::as_str))
                    .unwrap_or("unknown")
                    .to_string();
                let args_str = item
                    .get("arguments")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .or_else(|| tool_args.get(&call_id).cloned())
                    .unwrap_or_else(|| "{}".into());
                let arguments = serde_json::from_str(&args_str).unwrap_or(json!({}));
                events.push(Ok(ProviderEvent::ToolCall {
                    id: call_id,
                    name,
                    arguments,
                }));
            }
        }
        "response.completed" | "response.incomplete" | "response.done" => {
            events.push(Ok(ProviderEvent::MessageComplete));
        }
        "error" | "response.failed" => {
            let message = value
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .or_else(|| value.get("message").and_then(|m| m.as_str()))
                .unwrap_or("codex stream error")
                .to_string();
            events.push(Err(ProviderError::Message(message)));
        }
        _ => {}
    }
}

fn build_request_body(request: &Request) -> Value {
    let mut input = Vec::new();
    for msg in &request.messages {
        match msg.role {
            Role::User => {
                input.push(json!({
                    "type": "message",
                    "role": "user",
                    "content": content_parts_to_input(&msg.content),
                }));
            }
            Role::Assistant => {
                let content = content_parts_to_output(&msg.content);
                if !content.is_empty() {
                    input.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "status": "completed",
                        "content": content,
                    }));
                }
                for call in &msg.tool_calls {
                    input.push(json!({
                        "type": "function_call",
                        "call_id": call.id,
                        "name": call.name,
                        "arguments": call.arguments.to_string(),
                    }));
                }
            }
            Role::Tool => {
                let text = msg
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ContentPart::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": msg.tool_call_id.clone().unwrap_or_default(),
                    "output": text,
                }));
            }
        }
    }

    let tools: Vec<Value> = request.tools.iter().map(tool_def_to_codex).collect();

    let mut body = json!({
        "model": request.model,
        "store": false,
        "stream": true,
        "instructions": request.system,
        "input": input,
        "tool_choice": "auto",
        "parallel_tool_calls": true,
        "text": { "verbosity": "medium" },
        "include": ["reasoning.encrypted_content"],
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }
    body["reasoning"] = json!({
        "effort": request.effort.as_str(),
        "summary": "auto",
    });
    body
}

fn content_parts_to_input(parts: &[ContentPart]) -> Vec<Value> {
    parts
        .iter()
        .map(|p| match p {
            ContentPart::Text { text } => json!({
                "type": "input_text",
                "text": text,
            }),
            ContentPart::Image { mime, bytes } => {
                let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
                json!({
                    "type": "input_image",
                    "image_url": format!("data:{mime};base64,{b64}"),
                })
            }
        })
        .collect()
}

/// Assistant message content for the Responses API (output items, not input).
fn content_parts_to_output(parts: &[ContentPart]) -> Vec<Value> {
    parts
        .iter()
        .filter_map(|p| match p {
            ContentPart::Text { text } => Some(json!({
                "type": "output_text",
                "text": text,
                "annotations": [],
            })),
            // Assistant turns do not replay images as output parts.
            ContentPart::Image { .. } => None,
        })
        .collect()
}

fn tool_def_to_codex(tool: &ToolDefinition) -> Value {
    json!({
        "type": "function",
        "name": tool.name,
        "description": tool.description,
        "parameters": tool.parameters,
    })
}

/// Shared constructor used by the binary.
pub fn create_codex_provider(store: CredentialStore) -> Arc<dyn Provider> {
    Arc::new(CodexProvider::new(store))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Effort;
    use crate::types::Message;

    fn sample_request(messages: Vec<Message>) -> Request {
        Request {
            system: "sys".into(),
            messages,
            tools: Vec::new(),
            model: "gpt-5.5".into(),
            effort: Effort::Medium,
        }
    }

    #[test]
    fn follow_up_assistant_history_uses_output_text_not_input_text() {
        // Repro: after a successful first turn, the next Run replays the assistant
        // message. Codex rejects assistant content typed as input_text (400:
        // "Supported values are: 'output_text' and 'refusal'").
        let body = build_request_body(&sample_request(vec![
            Message::user_text("whats up"),
            Message::assistant_text("Hey!"),
            Message::user_text("Hello?"),
        ]));
        let input = body["input"].as_array().expect("input array");
        assert_eq!(input.len(), 3);

        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"][0]["type"], "input_text");

        assert_eq!(input[1]["role"], "assistant");
        assert_eq!(
            input[1]["content"][0]["type"],
            "output_text",
            "assistant history must use output_text (got {:?})",
            input[1]["content"][0]["type"]
        );
        assert_ne!(input[1]["content"][0]["type"], "input_text");

        assert_eq!(input[2]["role"], "user");
        assert_eq!(input[2]["content"][0]["type"], "input_text");
    }

    #[test]
    fn user_only_turn_still_uses_input_text() {
        let body = build_request_body(&sample_request(vec![Message::user_text("hi")]));
        let input = body["input"].as_array().expect("input array");
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["content"][0]["type"], "input_text");
    }
}
