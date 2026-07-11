use async_trait::async_trait;
use serde_json::{Value, json};

use super::{arg_string, ok_text, resolve_tool_path};
use crate::tools::{Tool, ToolContext, ToolError, ToolResult};

pub struct WriteTool;

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Write contents to a path. Creates new files freely; overwriting an existing file requires a matching Read Snapshot."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, ctx: &ToolContext<'_>, args: Value) -> Result<ToolResult, ToolError> {
        let path_arg = arg_string(&args, "path")?;
        let content = arg_string(&args, "content")?;
        let path = resolve_tool_path(ctx.workspace, &ctx.cwd, &path_arg)?;
        if path.exists() {
            ctx.assert_fresh_snapshot(&path)
                .await
                .map_err(ToolError::Message)?;
        } else if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ToolError::Message(e.to_string()))?;
        }
        std::fs::write(&path, &content).map_err(|e| ToolError::Message(e.to_string()))?;
        ctx.record_read_snapshot(&path)
            .await
            .map_err(ToolError::Message)?;
        Ok(ok_text("ok"))
    }
}
