//! Search tools: find, grep (FFF), ls.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::index::SharedIndex;
use crate::path::resolve_in_workspace;
use crate::tools::file::arg_string;
use crate::tools::{Tool, ToolContext, ToolError, ToolResult};

const DEFAULT_FIND_LIMIT: usize = 100;
const DEFAULT_GREP_LIMIT: usize = 50;
const MAX_OUTPUT_CHARS: usize = 30_000;

pub fn search_tools(index: SharedIndex) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(FindTool {
            index: Arc::clone(&index),
        }),
        Arc::new(GrepTool {
            index: Arc::clone(&index),
        }),
        Arc::new(LsTool),
    ]
}

pub struct FindTool {
    index: SharedIndex,
}

pub struct GrepTool {
    index: SharedIndex,
}

pub struct LsTool;

fn truncate_output(mut text: String) -> String {
    if text.chars().count() > MAX_OUTPUT_CHARS {
        text = text.chars().take(MAX_OUTPUT_CHARS).collect();
        text.push_str("\n...[truncated]");
    }
    text
}

#[async_trait]
impl Tool for FindTool {
    fn name(&self) -> &str {
        "find"
    }

    fn description(&self) -> &str {
        "Find files in the Workspace by fuzzy/glob-ish pattern (FFF). Prefer this over bash find."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string" },
                "limit": { "type": "integer", "description": "Max results (default 100)" }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _ctx: &ToolContext<'_>, args: Value) -> Result<ToolResult, ToolError> {
        let pattern = arg_string(&args, "pattern")?;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_FIND_LIMIT);
        self.index
            .ensure_ready()
            .await
            .map_err(|e| ToolError::Message(e.to_string()))?;
        let index = Arc::clone(&self.index);
        let paths = tokio::task::spawn_blocking(move || index.find(&pattern, limit))
            .await
            .map_err(|e| ToolError::Message(e.to_string()))?
            .map_err(|e| ToolError::Message(e.to_string()))?;
        if paths.is_empty() {
            return Ok(ToolResult::text("(no matches)"));
        }
        Ok(ToolResult::text(truncate_output(paths.join("\n"))))
    }
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search file contents in the Workspace (FFF). Prefer this over bash grep."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string" },
                "limit": { "type": "integer", "description": "Max matches (default 50)" }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _ctx: &ToolContext<'_>, args: Value) -> Result<ToolResult, ToolError> {
        let pattern = arg_string(&args, "pattern")?;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_GREP_LIMIT);
        self.index
            .ensure_ready()
            .await
            .map_err(|e| ToolError::Message(e.to_string()))?;
        let index = Arc::clone(&self.index);
        let lines = tokio::task::spawn_blocking(move || index.grep(&pattern, limit))
            .await
            .map_err(|e| ToolError::Message(e.to_string()))?
            .map_err(|e| ToolError::Message(e.to_string()))?;
        if lines.is_empty() {
            return Ok(ToolResult::text("(no matches)"));
        }
        Ok(ToolResult::text(truncate_output(lines.join("\n"))))
    }
}

#[async_trait]
impl Tool for LsTool {
    fn name(&self) -> &str {
        "ls"
    }

    fn description(&self) -> &str {
        "List directory entries alphabetically (thin readdir; not FFF)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Directory to list (default: Working Directory)" }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, ctx: &ToolContext<'_>, args: Value) -> Result<ToolResult, ToolError> {
        let state = ctx.session.snapshot().await;
        let path_arg = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let memory = crate::memory::memory_path(&ctx.session.inner.data_dir);
        let dir = resolve_in_workspace(
            &ctx.session.inner.workspace,
            &state.cwd,
            path_arg,
            Some(&memory),
        )
        .map_err(|e| ToolError::Message(e.to_string()))?;
        if !dir.is_dir() {
            return Ok(ToolResult::error(format!(
                "not a directory: {}",
                dir.display()
            )));
        }
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .map_err(|e| ToolError::Message(e.to_string()))?
            .filter_map(|e| e.ok())
            .map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    format!("{name}/")
                } else {
                    name
                }
            })
            .collect();
        names.sort();
        if names.is_empty() {
            Ok(ToolResult::text("(empty)"))
        } else {
            Ok(ToolResult::text(names.join("\n")))
        }
    }
}
