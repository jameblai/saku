//! Memory tool: full-replace `MEMORY.md` under the Data Dir (ADR 0020).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::memory::{memory_path, validate_memory_write};
use crate::tools::file::arg_string;
use crate::tools::{Tool, ToolContext, ToolError, ToolResult};

pub fn memory_tools() -> Vec<Arc<dyn Tool>> {
    vec![Arc::new(MemoryTool)]
}

pub struct MemoryTool;

#[async_trait]
impl Tool for MemoryTool {
    fn name(&self) -> &str {
        "memory"
    }

    fn description(&self) -> &str {
        "Full-replace Memory (`MEMORY.md` under the Data Dir). Pass the complete new contents; \
         empty string clears Memory. Cap is 2200 characters. Call this when retaining durable facts \
         across Sessions — do not claim you remembered unless this call succeeded."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "Full Memory contents after this call (empty clears)"
                }
            },
            "required": ["content"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, ctx: &ToolContext<'_>, args: Value) -> Result<ToolResult, ToolError> {
        let content = arg_string(&args, "content")?;
        if let Err(err) = validate_memory_write(&content) {
            return Ok(ToolResult::error(err.to_string()));
        }

        let path = memory_path(ctx.data_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ToolError::Message(e.to_string()))?;
        }
        std::fs::write(&path, &content).map_err(|e| ToolError::Message(e.to_string()))?;
        Ok(ToolResult::text("ok"))
    }
}
