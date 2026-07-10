//! Tool trait and registry.

pub mod file;

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::watch;

use crate::session::Session;
use crate::types::{ContentPart, ToolDefinition};

pub use file::{EditTool, ReadTool, WriteTool, file_tools};

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("unknown tool: {0}")]
    Unknown(String),
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: Vec<ContentPart>,
    pub is_error: bool,
    pub details: Option<Value>,
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
pub struct ToolContext<'a> {
    pub session: &'a Session,
    pub abort: watch::Receiver<bool>,
    pub progress: Option<Box<dyn Fn(String) + Send + Sync + 'a>>,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Value;
    async fn execute(
        &self,
        ctx: &ToolContext<'_>,
        args: Value,
    ) -> Result<ToolResult, ToolError>;

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
}
