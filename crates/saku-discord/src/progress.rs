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
    truncate(&raw, 80)
}

pub fn format_progress(lines: &[(String, String)]) -> String {
    let mut out = String::from("Tools:\n");
    for (name, preview) in lines {
        out.push_str(&format!("{} {name}: {preview}\n", tool_emoji(name)));
    }
    truncate(&out, 1900)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}
