//! Web Tools: `web_search` and `web_extract` (Exa Web Backend).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::credentials::{Credential, CredentialStore};
use crate::harness::Harness;
use crate::tools::file::arg_string;
use crate::tools::{Tool, ToolContext, ToolError, ToolResult};
use crate::web_backend::exa::{ExaClient, WEB_BACKEND_ID};

const MAX_OUTPUT_CHARS: usize = 30_000;
const DEFAULT_MAX_RESULTS: u32 = 8;
const MAX_RESULTS_CLAMP: u32 = 100;

/// Register `web_search` / `web_extract` when the configured Web Backend has a Credential.
pub async fn register_web_tools(harness: &Harness) {
    for tool in web_tools_from_store(harness.credentials(), harness.web_backend()) {
        harness.register_tool(tool).await;
    }
}

/// Build web Tools for `web_backend` when an API-key Credential is present.
pub fn web_tools_from_store(store: &CredentialStore, web_backend: &str) -> Vec<Arc<dyn Tool>> {
    if web_backend != WEB_BACKEND_ID {
        return Vec::new();
    }
    let Ok(Some(Credential::ApiKey { key })) = store.read(web_backend) else {
        return Vec::new();
    };
    web_tools(Arc::new(ExaClient::new(key)))
}

pub fn web_tools(client: Arc<ExaClient>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(WebSearchTool {
            client: Arc::clone(&client),
        }),
        Arc::new(WebExtractTool { client }),
    ]
}

fn truncate_output(mut text: String) -> String {
    if text.chars().count() > MAX_OUTPUT_CHARS {
        text = text.chars().take(MAX_OUTPUT_CHARS).collect();
        text.push_str("\n...[truncated]");
    }
    text
}

struct WebSearchTool {
    client: Arc<ExaClient>,
}

struct WebExtractTool {
    client: Arc<ExaClient>,
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the public web via the configured Web Backend. Returns title/url/snippet hits, not full page bodies."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query" },
                "max_results": {
                    "type": "integer",
                    "description": "Max hits to return (default 8)"
                },
                "depth": {
                    "type": "string",
                    "enum": ["auto", "fast", "deep"],
                    "description": "Search depth mapped to Exa type (default auto)"
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _ctx: &ToolContext<'_>, args: Value) -> Result<ToolResult, ToolError> {
        let query = arg_string(&args, "query")?;
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32)
            .unwrap_or(DEFAULT_MAX_RESULTS)
            .clamp(1, MAX_RESULTS_CLAMP);
        let depth = args.get("depth").and_then(|v| v.as_str()).unwrap_or("auto");
        if !matches!(depth, "auto" | "fast" | "deep") {
            return Ok(ToolResult::error(format!(
                "invalid depth `{depth}`; expected auto, fast, or deep"
            )));
        }
        match self.client.search(&query, max_results, depth).await {
            Ok(text) => Ok(ToolResult::text(truncate_output(text))),
            Err(e) => Ok(ToolResult::error(e.to_string())),
        }
    }
}

#[async_trait]
impl Tool for WebExtractTool {
    fn name(&self) -> &str {
        "web_extract"
    }

    fn description(&self) -> &str {
        "Extract page text for a URL via the configured Web Backend (no highlights/summary modes)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "Page URL to extract" }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _ctx: &ToolContext<'_>, args: Value) -> Result<ToolResult, ToolError> {
        let url = arg_string(&args, "url")?;
        match self.client.extract(&url).await {
            Ok(text) => Ok(ToolResult::text(truncate_output(text))),
            Err(e) => Ok(ToolResult::error(e.to_string())),
        }
    }
}
