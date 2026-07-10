use async_trait::async_trait;
use serde_json::{Value, json};

use super::{arg_string, ok_text, resolve_tool_path};
use crate::tools::{Tool, ToolContext, ToolError, ToolResult};

pub struct EditTool;

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Replace an exact old_string with new_string in an existing file. Requires a matching Read Snapshot."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "old_string": { "type": "string" },
                "new_string": { "type": "string" }
            },
            "required": ["path", "old_string", "new_string"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, ctx: &ToolContext<'_>, args: Value) -> Result<ToolResult, ToolError> {
        let path_arg = arg_string(&args, "path")?;
        let old = arg_string(&args, "old_string")?;
        let new = arg_string(&args, "new_string")?;
        let path = resolve_tool_path(ctx.workspace, &ctx.cwd, &path_arg)?;
        if !path.exists() {
            return Ok(ToolResult::error(format!(
                "file does not exist: {}",
                path.display()
            )));
        }
        ctx.session
            .assert_fresh_snapshot(&path)
            .await
            .map_err(ToolError::Message)?;
        let content =
            std::fs::read_to_string(&path).map_err(|e| ToolError::Message(e.to_string()))?;
        let matches: Vec<_> = content.match_indices(&old).collect();
        if matches.is_empty() {
            return Ok(ToolResult::error("old_string not found in file"));
        }
        if matches.len() > 1 {
            return Ok(ToolResult::error(
                "old_string matched multiple times; make it unique",
            ));
        }
        let updated = content.replacen(&old, &new, 1);
        std::fs::write(&path, &updated).map_err(|e| ToolError::Message(e.to_string()))?;
        ctx.session
            .record_read_snapshot(&path)
            .await
            .map_err(ToolError::Message)?;
        Ok(ok_text("ok"))
    }
}
