//! Background Process Tools (`bg_start`, `bg_list`, `bg_logs`, `bg_stop`).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::background::{DEFAULT_LOG_LINES, DEFAULT_SETTLE_SECS, MAX_LOG_LINES};
use crate::tools::file::arg_string;
use crate::tools::{Tool, ToolContext, ToolError, ToolResult};

pub fn background_tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(BgStartTool),
        Arc::new(BgListTool),
        Arc::new(BgLogsTool),
        Arc::new(BgStopTool),
    ]
}

pub struct BgStartTool;

#[async_trait]
impl Tool for BgStartTool {
    fn name(&self) -> &str {
        "bg_start"
    }

    fn description(&self) -> &str {
        "Start a Session-scoped Background Process that outlives the current Run \
         (e.g. an HTTP server). Starts in the Session Working Directory (frozen for \
         this process). Stdin is /dev/null (not a TTY). Optional settle seconds \
         (default 2) before returning the process-group-leader PID and a short \
         stdout/stderr snippet. Soft cap: 5 running."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to run in the background" },
                "settle": {
                    "type": "number",
                    "description": "Seconds to wait before returning PID + output snippet (default 2)"
                }
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, ctx: &ToolContext<'_>, args: Value) -> Result<ToolResult, ToolError> {
        let command = arg_string(&args, "command")?;
        let settle = args
            .get("settle")
            .and_then(|v| v.as_f64())
            .unwrap_or(DEFAULT_SETTLE_SECS);
        let cwd = &ctx.cwd;
        match ctx.session.background.start(command, cwd, settle).await {
            Ok(text) => Ok(ToolResult::text(text)),
            Err(msg) => Ok(ToolResult::error(msg)),
        }
    }
}

pub struct BgListTool;

#[async_trait]
impl Tool for BgListTool {
    fn name(&self) -> &str {
        "bg_list"
    }

    fn description(&self) -> &str {
        "List this Session's Background Processes (running and exited). Exited entries \
         keep their exit code and are not pruned."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(&self, ctx: &ToolContext<'_>, _args: Value) -> Result<ToolResult, ToolError> {
        Ok(ToolResult::text(ctx.session.bg_list_text().await))
    }
}

pub struct BgLogsTool;

#[async_trait]
impl Tool for BgLogsTool {
    fn name(&self) -> &str {
        "bg_logs"
    }

    fn description(&self) -> &str {
        "Tail stdout/stderr from a Background Process ring buffer. Optional lines \
         (default 100, hard-capped)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pid": { "type": "integer", "description": "Background Process PID" },
                "lines": {
                    "type": "integer",
                    "description": format!("Optional line count (default {DEFAULT_LOG_LINES}, max {MAX_LOG_LINES})")
                }
            },
            "required": ["pid"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, ctx: &ToolContext<'_>, args: Value) -> Result<ToolResult, ToolError> {
        let pid = args
            .get("pid")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| ToolError::Message("missing or invalid pid".into()))?
            as u32;
        let lines = args
            .get("lines")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        match ctx.session.bg_logs_text(pid, lines).await {
            Ok(text) => Ok(ToolResult::text(text)),
            Err(msg) => Ok(ToolResult::error(msg)),
        }
    }
}

pub struct BgStopTool;

#[async_trait]
impl Tool for BgStopTool {
    fn name(&self) -> &str {
        "bg_stop"
    }

    fn description(&self) -> &str {
        "Stop a Background Process by PID, or all running ones when all=true. \
         Signals the whole process group (SIGTERM, then SIGKILL after 5s). \
         Does not affect Run abort / saku stop."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pid": { "type": "integer", "description": "Background Process PID to stop" },
                "all": {
                    "type": "boolean",
                    "description": "When true, stop all running Background Processes"
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, ctx: &ToolContext<'_>, args: Value) -> Result<ToolResult, ToolError> {
        let all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
        let pid = args.get("pid").and_then(|v| v.as_u64()).map(|n| n as u32);
        let target = if all {
            None
        } else if let Some(pid) = pid {
            Some(pid)
        } else {
            return Ok(ToolResult::error("bg_stop requires pid or all=true"));
        };
        match ctx.session.bg_stop(target).await {
            Ok(text) => Ok(ToolResult::text(text)),
            Err(msg) => Ok(ToolResult::error(msg)),
        }
    }
}
