//! Tool trait and registry.

pub mod background;
pub mod file;
pub mod memory;
pub mod search;
pub mod session_search;
pub mod shell;
pub mod subagent;
pub mod web;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::{Mutex, watch};

use crate::session::{ReadSnapshot, Session};
use crate::types::{ContentPart, ToolDefinition};

pub use background::{BgListTool, BgLogsTool, BgStartTool, BgStopTool, background_tools};
pub use file::{EditTool, ReadTool, WriteTool, file_tools};
pub use memory::{MemoryTool, memory_tools};
pub use search::{FindTool, GrepTool, LsTool, search_tools};
pub use session_search::{SessionSearchTool, session_search_tools};
pub use shell::{BashTool, CdTool, shell_tools};
pub use subagent::SubagentTool;
pub use web::{register_web_tools, web_tools, web_tools_from_store};

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("unknown tool: {0}")]
    Unknown(String),
    #[error("{0}")]
    Message(String),
    #[error("{0}")]
    Fatal(String),
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: Vec<ContentPart>,
    pub is_error: bool,
    pub details: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolBatchPolicy {
    Sequential,
    Concurrent { max: usize },
    Reject(String),
}

impl ToolResult {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![ContentPart::text(text)],
            is_error: false,
            details: None,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            content: vec![ContentPart::text(text)],
            is_error: true,
            details: None,
        }
    }
}

/// Shared context for tool execution.
///
/// Path fields (`workspace`, `cwd`, `data_dir`) are supplied by the Harness so Tools
/// do not read `Session` internals. `cwd` is a snapshot taken at tool start.
/// `session` is for Session methods only (e.g. Read Snapshots, `cd`, background).
pub struct ToolContext<'a> {
    pub session: &'a Session,
    pub workspace: &'a Path,
    pub cwd: PathBuf,
    pub data_dir: &'a Path,
    /// Exact System Prompt used for the parent Provider turn.
    pub system_prompt: &'a str,
    /// Discovered Skill base directories allowlisted for `read` outside the Workspace.
    /// Bash has no path jail, so skill scripts/assets under these dirs are already reachable.
    pub skill_roots: Vec<PathBuf>,
    /// Child-local snapshots. `None` uses the parent Session's persisted snapshots.
    pub local_read_snapshots: Option<Arc<Mutex<Vec<ReadSnapshot>>>>,
    pub abort: watch::Receiver<bool>,
    /// Optional progress callback shape; Harness currently passes `None`.
    pub progress: Option<Box<dyn Fn(String) + Send + Sync + 'a>>,
}

impl ToolContext<'_> {
    pub async fn record_read_snapshot(&self, path: &Path) -> Result<(), String> {
        let Some(snapshots) = &self.local_read_snapshots else {
            return self.session.record_read_snapshot(path).await;
        };
        let snapshot = ReadSnapshot::capture(path)?;
        let mut snapshots = snapshots.lock().await;
        if let Some(existing) = snapshots.iter_mut().find(|item| item.path == path) {
            *existing = snapshot;
        } else {
            snapshots.push(snapshot);
        }
        Ok(())
    }

    pub async fn assert_fresh_snapshot(&self, path: &Path) -> Result<(), String> {
        let Some(snapshots) = &self.local_read_snapshots else {
            return self.session.assert_fresh_snapshot(path).await;
        };
        let snapshots = snapshots.lock().await;
        let Some(snapshot) = snapshots.iter().find(|item| item.path == path) else {
            return Err(format!(
                "no Read Snapshot for {}; read the file before editing",
                path.display()
            ));
        };
        snapshot.assert_fresh()
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Value;
    async fn execute(&self, ctx: &ToolContext<'_>, args: Value) -> Result<ToolResult, ToolError>;

    fn batch_policy(&self, _arguments: &[Value]) -> ToolBatchPolicy {
        ToolBatchPolicy::Sequential
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().into(),
            description: self.description().into(),
            parameters: self.parameters_schema(),
        }
    }
}

#[derive(Default)]
pub struct ToolRegistry {
    tools: Vec<Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.push(tool);
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.iter().map(|t| t.definition()).collect()
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.name() == name).cloned()
    }

    pub fn selected(&self, names: &[&str]) -> Vec<Arc<dyn Tool>> {
        self.tools
            .iter()
            .filter(|tool| names.contains(&tool.name()))
            .cloned()
            .collect()
    }

    pub fn except(&self, excluded: &[&str]) -> Vec<Arc<dyn Tool>> {
        self.tools
            .iter()
            .filter(|tool| !excluded.contains(&tool.name()))
            .cloned()
            .collect()
    }
}
