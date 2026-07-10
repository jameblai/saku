//! File tools: read, edit, write.

mod edit;
mod read;
mod write;

use std::path::{Path, PathBuf};

use crate::path::{PathError, resolve_in_workspace};
use crate::tools::{Tool, ToolError, ToolResult};

pub use edit::EditTool;
pub use read::ReadTool;
pub use write::WriteTool;

pub fn file_tools() -> Vec<std::sync::Arc<dyn Tool>> {
    vec![
        std::sync::Arc::new(ReadTool),
        std::sync::Arc::new(EditTool),
        std::sync::Arc::new(WriteTool),
    ]
}

pub(crate) fn resolve_tool_path(
    workspace: &Path,
    cwd: &Path,
    candidate: &str,
) -> Result<PathBuf, ToolError> {
    resolve_in_workspace(workspace, cwd, candidate).map_err(path_err)
}

fn path_err(err: PathError) -> ToolError {
    ToolError::Message(err.to_string())
}

pub(crate) fn arg_string(args: &serde_json::Value, key: &str) -> Result<String, ToolError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ToolError::Message(format!("missing string argument `{key}`")))
}

pub(crate) fn ok_text(text: impl Into<String>) -> ToolResult {
    ToolResult::text(text)
}
