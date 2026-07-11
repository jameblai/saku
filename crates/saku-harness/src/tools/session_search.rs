//! `session_search` Tool: cross-Session full-text search over past conversation text.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::session::{DEFAULT_SEARCH_LIMIT, SearchHit, SessionSearchIndex};
use crate::tools::file::arg_string;
use crate::tools::{Tool, ToolContext, ToolError, ToolResult};

pub fn session_search_tools(index: Arc<SessionSearchIndex>) -> Vec<Arc<dyn Tool>> {
    vec![Arc::new(SessionSearchTool { index })]
}

pub struct SessionSearchTool {
    index: Arc<SessionSearchIndex>,
}

#[async_trait]
impl Tool for SessionSearchTool {
    fn name(&self) -> &str {
        "session_search"
    }

    fn description(&self) -> &str {
        "Full-text search across ALL past Sessions (Discord threads) for earlier decisions or \
         context. Indexes user + assistant text and compaction summaries only — not tool output. \
         Returns ranked snippets with thread id, session date, role, and an excerpt. Use this to \
         recall what was discussed in other threads; use `grep`/`find` for Workspace files."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Full-text query (terms AND-ed)" },
                "limit": { "type": "integer", "description": "Max hits (default 10)" }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _ctx: &ToolContext<'_>, args: Value) -> Result<ToolResult, ToolError> {
        let query = arg_string(&args, "query")?;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_SEARCH_LIMIT)
            .max(1);
        let index = Arc::clone(&self.index);
        let hits = tokio::task::spawn_blocking(move || index.search(&query, limit))
            .await
            .map_err(|e| ToolError::Message(e.to_string()))?
            .map_err(|e| ToolError::Message(e.to_string()))?;
        if hits.is_empty() {
            return Ok(ToolResult::text("(no matches)"));
        }
        Ok(ToolResult::text(format_hits(&hits)))
    }
}

fn format_hits(hits: &[SearchHit]) -> String {
    let mut out = String::new();
    for (i, hit) in hits.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!(
            "thread {} · {} · {}\n{}\n",
            hit.thread_id,
            format_date(&hit.session_date),
            hit.role,
            hit.excerpt,
        ));
    }
    out
}

/// Render a header `created_at` (unix seconds string) as `YYYY-MM-DD` (UTC).
/// Falls back to the raw value when it is not a parseable timestamp.
fn format_date(created_at: &str) -> String {
    match created_at.trim().parse::<i64>() {
        Ok(secs) if secs > 0 => ymd_utc(secs),
        _ => created_at.to_string(),
    }
}

/// Civil date (UTC) from unix seconds — Howard Hinnant's `civil_from_days`.
fn ymd_utc(unix_secs: i64) -> String {
    let days = unix_secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_date_renders_unix_seconds() {
        // 2021-01-01T00:00:00Z = 1609459200
        assert_eq!(format_date("1609459200"), "2021-01-01");
        // Non-numeric falls through unchanged.
        assert_eq!(format_date("not-a-date"), "not-a-date");
    }

    #[test]
    fn format_hits_includes_thread_role_and_excerpt() {
        let hits = vec![SearchHit {
            thread_id: "123".into(),
            session_date: "1609459200".into(),
            role: "user".into(),
            excerpt: "how does compaction work".into(),
        }];
        let text = format_hits(&hits);
        assert!(text.contains("thread 123"));
        assert!(text.contains("2021-01-01"));
        assert!(text.contains("user"));
        assert!(text.contains("how does compaction work"));
    }
}
