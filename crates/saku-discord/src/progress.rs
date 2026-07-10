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
        _ => "🛠️",
    }
}

pub fn args_preview(args: &Value) -> String {
    let raw = match args {
        Value::Object(map) => {
            if let Some(cmd) = map.get("command").and_then(|v| v.as_str()) {
                format!("\"{cmd}\"")
            } else if let Some(path) = map.get("path").and_then(|v| v.as_str()) {
                format!("\"{path}\"")
            } else if let Some(pattern) = map.get("pattern").and_then(|v| v.as_str()) {
                format!("\"{pattern}\"")
            } else {
                args.to_string()
            }
        }
        other => other.to_string(),
    };
    truncate_chars(&raw, 80)
}

pub fn format_progress(lines: &[(String, String)]) -> String {
    let mut out = String::from("Tools:\n");
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
        lines.remove(1);
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
            ("bash".into(), "\"ls\"".into()),
            ("read".into(), "\"a.rs\"".into()),
        ]);
        assert!(text.contains("💻 bash:"));
        assert!(text.contains("📖 read:"));
    }

    #[test]
    fn args_preview_prefers_command_path_pattern() {
        assert_eq!(args_preview(&json!({"command": "pwd"})), "\"pwd\"");
        assert_eq!(args_preview(&json!({"path": "src"})), "\"src\"");
    }
}
