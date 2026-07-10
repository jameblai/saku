use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::time::{Duration, timeout};

use crate::path::resolve_in_workspace;
use crate::tools::file::arg_string;
use crate::tools::{Tool, ToolContext, ToolError, ToolResult};

pub fn shell_tools() -> Vec<Arc<dyn Tool>> {
    vec![Arc::new(BashTool), Arc::new(CdTool)]
}

pub struct CdTool;

#[async_trait]
impl Tool for CdTool {
    fn name(&self) -> &str {
        "cd"
    }

    fn description(&self) -> &str {
        "Change the Session Working Directory to a directory inside the Workspace."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Directory relative to current Working Directory or absolute under Workspace" }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, ctx: &ToolContext<'_>, args: Value) -> Result<ToolResult, ToolError> {
        let path_arg = arg_string(&args, "path")?;
        let state = ctx.session.snapshot().await;
        let memory = crate::memory::memory_path(&ctx.session.inner.data_dir);
        let resolved = resolve_in_workspace(
            &ctx.session.inner.workspace,
            &state.cwd,
            &path_arg,
            Some(&memory),
        )
        .map_err(|e| ToolError::Message(e.to_string()))?;

        if !resolved.is_dir() {
            return Ok(ToolResult::error(format!(
                "not a directory: {}",
                resolved.display()
            )));
        }

        {
            let mut state = ctx.session.state.lock().await;
            state.cwd = resolved.clone();
        }
        ctx.session
            .inner
            .store
            .append_cwd(&ctx.session.thread_id, &resolved)
            .map_err(|e| ToolError::Message(e.to_string()))?;

        Ok(ToolResult::text(format!("cwd: {}", resolved.display())))
    }
}

pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Run a bash command in the Session Working Directory. Optional timeout in seconds (no default)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" },
                "timeout": { "type": "number", "description": "Optional timeout in seconds" }
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, ctx: &ToolContext<'_>, args: Value) -> Result<ToolResult, ToolError> {
        let command = arg_string(&args, "command")?;
        let timeout_secs = args.get("timeout").and_then(|v| v.as_f64());
        let cwd = ctx.session.snapshot().await.cwd;

        let child = Command::new("bash")
            .arg("-lc")
            .arg(&command)
            .current_dir(&cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| ToolError::Message(e.to_string()))?;

        let wait = async {
            let output = child
                .wait_with_output()
                .await
                .map_err(|e| ToolError::Message(e.to_string()))?;
            Ok::<_, ToolError>(output)
        };

        // Poll abort while waiting.
        let abort = ctx.abort.clone();
        let run = async {
            tokio::select! {
                result = wait => result,
                _ = wait_for_abort(abort) => {
                    Err(ToolError::Message("bash aborted".into()))
                }
            }
        };

        let output = if let Some(secs) = timeout_secs {
            match timeout(Duration::from_secs_f64(secs.max(0.001)), run).await {
                Ok(result) => result?,
                Err(_) => {
                    return Ok(ToolResult::error(format!(
                        "bash timed out after {secs} seconds"
                    )));
                }
            }
        } else {
            run.await?
        };

        let mut text = String::new();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stdout.is_empty() {
            text.push_str(&stdout);
        }
        if !stderr.is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&stderr);
        }
        if text.is_empty() {
            text = format!("exit {}", output.status.code().unwrap_or(-1));
        } else if !output.status.success() {
            text.push_str(&format!("\nexit {}", output.status.code().unwrap_or(-1)));
        }

        if output.status.success() {
            Ok(ToolResult::text(text))
        } else {
            Ok(ToolResult::error(text))
        }
    }
}

async fn wait_for_abort(mut abort: tokio::sync::watch::Receiver<bool>) {
    loop {
        if *abort.borrow() {
            return;
        }
        if abort.changed().await.is_err() {
            return;
        }
    }
}
