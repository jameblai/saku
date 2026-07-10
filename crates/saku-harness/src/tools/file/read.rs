use async_trait::async_trait;
use serde_json::{Value, json};

use super::{ok_text, record_snapshot, resolve_tool_path};
use crate::tools::{Tool, ToolContext, ToolError, ToolResult};

pub struct ReadTool;

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read a file under the Workspace (or Memory). Records a Read Snapshot for later edit/write."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path relative to Working Directory or absolute under Workspace" }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, ctx: &ToolContext<'_>, args: Value) -> Result<ToolResult, ToolError> {
        let path_arg = super::arg_string(&args, "path")?;
        let path = resolve_tool_path(ctx.session, &path_arg).await?;
        let bytes = std::fs::read(&path).map_err(|e| ToolError::Message(e.to_string()))?;
        record_snapshot(ctx.session, &path).await?;

        if crate::vision::is_image_path(&path) {
            let (resized, mime) = crate::vision::resize_for_provider(&bytes, None)
                .map_err(|e| ToolError::Message(e.to_string()))?;
            return Ok(ToolResult {
                content: vec![crate::types::ContentPart::image(mime, resized)],
                is_error: false,
                details: None,
            });
        }

        let text = String::from_utf8_lossy(&bytes).into_owned();
        Ok(ok_text(text))
    }
}
