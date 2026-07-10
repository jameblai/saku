//! Bot Command parsing (`saku …`).

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BotCommand {
    Help,
    Stop,
    Steer(String),
    Model {
        id: Option<String>,
        effort: Option<String>,
    },
    Effort {
        level: Option<String>,
    },
    Status,
}

pub fn parse_command(prefix: &str, content: &str) -> Option<BotCommand> {
    let content = content.trim();
    let rest = content.strip_prefix(prefix)?.trim_start();
    if rest.is_empty() {
        return None;
    }
    let mut parts = rest.splitn(3, char::is_whitespace);
    let cmd = parts.next()?.to_ascii_lowercase();
    match cmd.as_str() {
        "help" => Some(BotCommand::Help),
        "stop" => Some(BotCommand::Stop),
        "status" => Some(BotCommand::Status),
        "steer" => {
            let msg = rest.strip_prefix("steer").unwrap_or("").trim();
            if msg.is_empty() {
                None
            } else {
                Some(BotCommand::Steer(msg.to_string()))
            }
        }
        "model" => {
            let id = parts.next().map(str::to_string);
            let effort = parts.next().map(str::to_string);
            Some(BotCommand::Model { id, effort })
        }
        "effort" => {
            let level = parts.next().map(str::to_string);
            Some(BotCommand::Effort { level })
        }
        _ => None,
    }
}

pub fn help_text(prefix: &str) -> String {
    format!(
        "Bot Commands (prefix `{prefix}`):\n\
         - `{prefix} help`\n\
         - `{prefix} stop` — abort active Run and drain queue\n\
         - `{prefix} steer <message>` — redirect after current tool batch\n\
         - `{prefix} model` / `{prefix} model <id> [effort]`\n\
         - `{prefix} effort` / `{prefix} effort <level>`\n\
         - `{prefix} status` — Session, config, and Codex Plan Usage"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_help_and_model() {
        assert_eq!(parse_command("saku", "saku help"), Some(BotCommand::Help));
        assert_eq!(
            parse_command("saku", "saku model gpt-5.5 high"),
            Some(BotCommand::Model {
                id: Some("gpt-5.5".into()),
                effort: Some("high".into()),
            })
        );
    }

    #[test]
    fn parses_status() {
        assert_eq!(
            parse_command("saku", "saku status"),
            Some(BotCommand::Status)
        );
    }

    #[test]
    fn help_lists_status() {
        let text = help_text("saku");
        assert!(text.contains("`saku status`"));
    }
}
