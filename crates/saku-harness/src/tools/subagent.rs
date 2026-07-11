//! In-process Subagent inner loop.

use async_trait::async_trait;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::Mutex;

use super::{Tool, ToolBatchPolicy, ToolContext, ToolError, ToolResult};
use crate::config::Effort;
use crate::path::resolve_in_workspace;
use crate::provider::{ALLOWED_MODELS, is_allowed_model, is_supported_effort};
use crate::types::{ContentPart, Message, ProviderEvent, Request, Role, ToolCall};

const DEFAULT_MAX_TURNS: usize = 20;
const EXPLORE_TOOLS: &[&str] = &["read", "find", "grep", "ls", "cd", "bash"];

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SubagentMode {
    #[default]
    Explore,
    Edit,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SubagentArgs {
    task: String,
    #[serde(default)]
    mode: SubagentMode,
    #[serde(default = "default_max_turns")]
    max_turns: usize,
    model: Option<String>,
    effort: Option<Effort>,
}

fn default_max_turns() -> usize {
    DEFAULT_MAX_TURNS
}

#[derive(Default)]
pub struct SubagentTool;

impl SubagentTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for SubagentTool {
    fn name(&self) -> &str {
        "subagent"
    }

    fn description(&self) -> &str {
        "Run an isolated in-process child agent and return its final summary. Explore mode is read-only; edit mode may change the Workspace."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task": { "type": "string", "minLength": 1 },
                "mode": { "type": "string", "enum": ["explore", "edit"], "default": "explore" },
                "max_turns": { "type": "integer", "minimum": 1, "default": 20 },
                "model": {
                    "type": "string",
                    "enum": ALLOWED_MODELS,
                    "description": "Optional canonical model id; defaults to the Session model"
                },
                "effort": { "type": "string", "enum": ["minimal", "low", "medium", "high", "xhigh", "max"] }
            },
            "required": ["task"],
            "additionalProperties": false
        })
    }

    fn batch_policy(&self, arguments: &[Value]) -> ToolBatchPolicy {
        if arguments.len() <= 1 {
            return ToolBatchPolicy::Sequential;
        }
        let all_explore = arguments
            .iter()
            .all(|arguments| call_mode(arguments) == Ok(SubagentMode::Explore));
        if all_explore {
            ToolBatchPolicy::Concurrent { max: 4 }
        } else {
            ToolBatchPolicy::Reject(
                "parallel subagent batches must be explore-only; run edit subagents one at a time"
                    .into(),
            )
        }
    }

    async fn execute(&self, ctx: &ToolContext<'_>, args: Value) -> Result<ToolResult, ToolError> {
        let args: SubagentArgs = serde_json::from_value(args)
            .map_err(|error| ToolError::Message(format!("invalid subagent arguments: {error}")))?;
        if args.task.trim().is_empty() {
            return Ok(ToolResult::error("subagent task must not be empty"));
        }
        if args.max_turns == 0 {
            return Ok(ToolResult::error("subagent max_turns must be at least 1"));
        }

        let parent = ctx.session.snapshot().await;
        let model = args
            .model
            .map(|model| normalize_model_id(&model))
            .unwrap_or(parent.model);
        let effort = args.effort.unwrap_or(parent.effort);
        if !is_allowed_model(&model) {
            return Ok(ToolResult::error(format!(
                "unsupported subagent model `{model}`; allowed: {}",
                ALLOWED_MODELS.join(", ")
            )));
        }
        if !is_supported_effort(&model, effort) {
            return Ok(ToolResult::error(format!(
                "unsupported subagent effort `{effort}` for model `{model}`"
            )));
        }

        let tools = {
            let registry = ctx.session.inner.tools.lock().await;
            match args.mode {
                SubagentMode::Explore => registry.selected(EXPLORE_TOOLS),
                SubagentMode::Edit => registry.except(&["subagent"]),
            }
        };
        let definitions = tools
            .iter()
            .map(|tool| tool.definition())
            .collect::<Vec<_>>();
        let mut system = ctx.system_prompt.to_owned();
        system.push_str("\nYou are a Subagent. Complete only the supplied task, then return a concise summary to the parent. You have an isolated transcript. The subagent tool is unavailable; do not attempt to delegate.\n");
        if args.mode == SubagentMode::Explore {
            system.push_str(
                "Explore mode is analysis-only. You may use bash to inspect state and run analysis scripts, but must not mutate files, use shell redirection to write, or bypass the unavailable write/edit tools. If a mutating tool is unavailable, do not retry through bash; report that the task requires an edit Subagent.\n",
            );
        }
        let mut messages = vec![Message::user_text(args.task)];
        let mut partial = String::new();
        let mut child_cwd = ctx.cwd.clone();
        let child_snapshots = Arc::new(Mutex::new(Vec::new()));

        for _ in 0..args.max_turns {
            if *ctx.abort.borrow() {
                return Ok(ToolResult::error("subagent aborted"));
            }
            let request = Request {
                system: system.clone(),
                messages: messages.clone(),
                tools: definitions.clone(),
                model: model.clone(),
                effort,
            };
            let mut stream = ctx.session.inner.provider.complete(request);
            let mut text = String::new();
            let mut calls = Vec::new();
            while let Some(event) = stream.next().await {
                match event.map_err(|error| ToolError::Message(error.to_string()))? {
                    ProviderEvent::TextDelta(delta) => text.push_str(&delta),
                    ProviderEvent::ToolCall {
                        id,
                        name,
                        arguments,
                    } => {
                        calls.push(ToolCall {
                            id,
                            name,
                            arguments,
                        });
                    }
                    ProviderEvent::MessageComplete => break,
                    ProviderEvent::Error(message) => return Ok(ToolResult::error(message)),
                    ProviderEvent::ReasoningDelta(_) | ProviderEvent::Usage(_) => {}
                }
            }
            if !text.is_empty() {
                partial = text.clone();
            }
            messages.push(Message {
                role: Role::Assistant,
                content: if text.is_empty() {
                    Vec::new()
                } else {
                    vec![ContentPart::text(text)]
                },
                tool_call_id: None,
                tool_calls: calls.clone(),
            });
            if calls.is_empty() {
                return Ok(ToolResult::text(partial));
            }
            for call in calls {
                let result = if call.name == "subagent" {
                    ToolResult::error("nested subagents are not allowed (maximum depth is 1)")
                } else if call.name == "cd" {
                    match call.arguments.get("path").and_then(Value::as_str) {
                        Some(path) => match resolve_in_workspace(ctx.workspace, &child_cwd, path) {
                            Ok(path) if path.is_dir() => {
                                child_cwd = path;
                                ToolResult::text(format!("cwd: {}", child_cwd.display()))
                            }
                            Ok(path) => {
                                ToolResult::error(format!("not a directory: {}", path.display()))
                            }
                            Err(error) => ToolResult::error(error.to_string()),
                        },
                        None => ToolResult::error("missing string argument `path`"),
                    }
                } else if let Some(tool) = tools.iter().find(|tool| tool.name() == call.name) {
                    let child_ctx = ToolContext {
                        session: ctx.session,
                        workspace: ctx.workspace,
                        cwd: child_cwd.clone(),
                        data_dir: ctx.data_dir,
                        system_prompt: ctx.system_prompt,
                        skill_roots: ctx.skill_roots.clone(),
                        local_read_snapshots: Some(Arc::clone(&child_snapshots)),
                        abort: ctx.abort.clone(),
                        progress: None,
                    };
                    tool.execute(&child_ctx, call.arguments)
                        .await
                        .unwrap_or_else(|error| ToolResult::error(error.to_string()))
                } else {
                    let mut message = format!(
                        "tool `{}` is unavailable in {:?} mode",
                        call.name, args.mode
                    )
                    .to_lowercase();
                    if args.mode == SubagentMode::Explore {
                        message.push_str(
                            "; do not retry through bash—report that an edit subagent is required",
                        );
                    }
                    ToolResult::error(message)
                };
                messages.push(Message {
                    role: Role::Tool,
                    content: result.content,
                    tool_call_id: Some(call.id),
                    tool_calls: Vec::new(),
                });
            }
        }

        let notice = format!(
            "[subagent stopped after reaching max_turns={}]",
            args.max_turns
        );
        Ok(ToolResult::text(if partial.is_empty() {
            notice
        } else {
            format!("{partial}\n\n{notice}")
        }))
    }
}

fn normalize_model_id(model: &str) -> String {
    model.trim().to_ascii_lowercase().replace([' ', '_'], "-")
}

pub(crate) fn call_mode(arguments: &Value) -> Result<SubagentMode, String> {
    serde_json::from_value::<SubagentArgs>(arguments.clone())
        .map(|args| args.mode)
        .map_err(|error| format!("invalid subagent arguments: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_advertises_the_canonical_model_ids() {
        let schema = SubagentTool::new().parameters_schema();
        assert_eq!(schema["properties"]["model"]["enum"], json!(ALLOWED_MODELS));
    }

    #[test]
    fn model_id_normalization_accepts_common_human_formatting() {
        assert_eq!(normalize_model_id(" GPT 5.4 Mini "), "gpt-5.4-mini");
        assert_eq!(normalize_model_id("gpt_5.6_sol"), "gpt-5.6-sol");
    }
}
