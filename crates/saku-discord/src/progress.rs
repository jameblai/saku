//! Progress Message formatting (ADR 0015).

use serde_json::Value;

pub fn tool_emoji(name: &str) -> &'static str {
    match name {
        "bash" => "💻",
        "read" => "📖",
        "edit" => "🔧",
        "write" => "✍️",
        "find" => "🔍",
        "grep" => "🔎",
        "ls" => "📁",
        "cd" => "📂",
        "bg_start" | "bg_list" | "bg_logs" | "bg_stop" => "🧵",
        _ => "🛠️",
    }
}

pub fn args_preview(args: &Value) -> String {
    let raw = match args {
        Value::Object(map) => {
            if let Some(cmd) = map.get("command").and_then(|v| v.as_str()) {
                cmd.to_string()
            } else if let Some(path) = map.get("path").and_then(|v| v.as_str()) {
                path.to_string()
            } else if let Some(pattern) = map.get("pattern").and_then(|v| v.as_str()) {
                pattern.to_string()
            } else {
                args.to_string()
            }
        }
        other => other.to_string(),
    };
    // Collapse → truncate → fence so newlines can't break the span and truncation
    // never chops the closing delimiter (ADR 0015).
    discord_inline_code(&truncate_chars(&collapse_whitespace(&raw), 80))
}

/// Collapse runs of whitespace (including newlines) to a single space.
fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Wrap `content` in a Discord inline code span. Fence length is one longer than
/// the longest backtick run inside; pad with spaces when content starts/ends with `` ` ``.
fn discord_inline_code(content: &str) -> String {
    let mut max_run = 0usize;
    let mut run = 0usize;
    for c in content.chars() {
        if c == '`' {
            run += 1;
            max_run = max_run.max(run);
        } else {
            run = 0;
        }
    }
    let fence = "`".repeat(max_run + 1);
    if content.starts_with('`') || content.ends_with('`') {
        format!("{fence} {content} {fence}")
    } else {
        format!("{fence}{content}{fence}")
    }
}

pub fn format_progress(lines: &[(String, String)]) -> String {
    let mut out = String::new();
    for (name, preview) in lines {
        out.push_str(&format!("{} {name}: {preview}\n", tool_emoji(name)));
    }
    trim_oldest_to_limit(&out, 1900)
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

/// Prefer dropping oldest tool lines so the newest Progress stays visible (ADR 0015).
fn trim_oldest_to_limit(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut lines: Vec<&str> = text.lines().collect();
    while lines.len() > 1 && lines.join("\n").chars().count() > max {
        lines.remove(0);
    }
    let joined = lines.join("\n");
    if joined.chars().count() > max {
        let chars: Vec<char> = joined.chars().collect();
        let start = chars.len().saturating_sub(max.saturating_sub(1));
        let mut t: String = chars[start..].iter().collect();
        t.insert(0, '…');
        t
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formats_progress_with_emoji() {
        let text = format_progress(&[
            ("bash".into(), "`ls`".into()),
            ("read".into(), "`a.rs`".into()),
        ]);
        assert!(!text.contains("Tools:"));
        assert!(text.starts_with("💻 bash:"));
        assert!(text.contains("📖 read:"));
    }

    #[test]
    fn args_preview_prefers_command_path_pattern() {
        assert_eq!(args_preview(&json!({"command": "pwd"})), "`pwd`");
        assert_eq!(args_preview(&json!({"path": "src"})), "`src`");
        assert_eq!(args_preview(&json!({"pattern": "foo"})), "`foo`");
    }

    #[test]
    fn args_preview_wraps_pipes_in_inline_code_without_backslash_escape() {
        let preview = args_preview(&json!({"command": "false || true"}));
        assert_eq!(preview, "`false || true`");
        assert_eq!(
            args_preview(&json!({"command": r#"pgrep -af "a|b""#})),
            "`pgrep -af \"a|b\"`"
        );
    }

    #[test]
    fn args_preview_collapses_whitespace_before_fencing() {
        let preview = args_preview(&json!({
            "command": "false || true\nsleep 1\necho hi"
        }));
        assert_eq!(preview, "`false || true sleep 1 echo hi`");
        assert!(!preview.contains('\n'));
    }

    #[test]
    fn args_preview_uses_longer_fence_when_content_has_backticks() {
        assert_eq!(
            args_preview(&json!({"command": "echo `whoami`"})),
            "`` echo `whoami` ``"
        );
        assert_eq!(
            args_preview(&json!({"command": "``already``"})),
            "``` ``already`` ```"
        );
    }
}
