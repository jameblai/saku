//! Session Compaction: summarize older turns when nearing context limits.

use crate::types::{ContentPart, Message, Role};

/// Rough token estimate (chars / 4), good enough for v1 thresholds.
pub fn estimate_tokens(messages: &[Message], system: &str) -> usize {
    let mut chars = system.chars().count();
    for msg in messages {
        for part in &msg.content {
            if let ContentPart::Text { text } = part {
                chars += text.chars().count();
            } else if let ContentPart::Image { bytes, .. } = part {
                // Images are expensive; rough fixed cost.
                chars += 1_000 + bytes.len() / 10;
            }
        }
        for call in &msg.tool_calls {
            chars += call.name.len() + call.arguments.to_string().len();
        }
    }
    chars / 4
}

/// Default soft limit before Compaction (code default).
pub const DEFAULT_COMPACTION_TOKEN_LIMIT: usize = 80_000;
/// Keep this many most-recent messages after Compaction.
pub const DEFAULT_KEEP_RECENT: usize = 12;

#[derive(Debug, Clone)]
pub struct CompactionResult {
    pub summary: String,
    pub kept_messages: Vec<Message>,
}

/// Compact older messages into a summary message + recent tail.
///
/// `summarize` is injected so tests can stub without a live Provider.
pub fn compact_messages(
    messages: &[Message],
    keep_recent: usize,
    summarize: impl FnOnce(&[Message]) -> String,
) -> Option<CompactionResult> {
    if messages.len() <= keep_recent {
        return None;
    }
    let split = messages.len().saturating_sub(keep_recent);
    let older = &messages[..split];
    let recent = messages[split..].to_vec();
    let summary = summarize(older);
    let summary_msg = Message {
        role: Role::User,
        content: vec![ContentPart::text(format!(
            "[compaction summary of earlier turns]\n{summary}"
        ))],
        tool_call_id: None,
        tool_calls: Vec::new(),
    };
    let mut kept = vec![summary_msg];
    kept.extend(recent);
    Some(CompactionResult {
        summary,
        kept_messages: kept,
    })
}

pub fn default_local_summarize(older: &[Message]) -> String {
    let mut lines = Vec::new();
    for msg in older.iter().take(40) {
        let role = match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        let text = msg
            .content
            .iter()
            .map(|c| match c {
                ContentPart::Text { text } => text.as_str(),
                _ => "[image]",
            })
            .collect::<Vec<_>>()
            .join(" ");
        let clipped: String = text.chars().take(200).collect();
        lines.push(format!("{role}: {clipped}"));
    }
    if older.len() > 40 {
        lines.push(format!("…and {} more messages", older.len() - 40));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_keeps_recent_and_summary() {
        let mut messages = Vec::new();
        for i in 0..20 {
            messages.push(Message::user_text(format!("msg {i}")));
        }
        let result = compact_messages(&messages, 5, |older| format!("summarized {}", older.len()))
            .expect("compacted");
        assert!(result.summary.contains("summarized 15"));
        assert_eq!(result.kept_messages.len(), 6); // summary + 5
        assert!(matches!(
            &result.kept_messages[0].content[0],
            ContentPart::Text { text } if text.contains("compaction summary")
        ));
    }
}
