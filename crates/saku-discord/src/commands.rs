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
    /// `saku bg` — list Background Processes.
    BgList,
    /// `saku bg logs <pid>`
    BgLogs {
        pid: u32,
    },
    /// `saku bg stop <pid>` or `saku bg stop all`
    BgStop {
        /// `None` means stop all running.
        pid: Option<u32>,
    },
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
        "bg" => parse_bg(rest),
        _ => None,
    }
}

fn parse_bg(rest: &str) -> Option<BotCommand> {
    // rest starts with "bg" (any case); strip it.
    let after = {
        let trimmed = rest.trim_start();
        let (head, tail) = trimmed
            .split_once(char::is_whitespace)
            .unwrap_or((trimmed, ""));
        if !head.eq_ignore_ascii_case("bg") {
            return None;
        }
        tail.trim_start()
    };
    if after.is_empty() {
        return Some(BotCommand::BgList);
    }
    let mut parts = after.split_whitespace();
    let sub = parts.next()?.to_ascii_lowercase();
    match sub.as_str() {
        "logs" => {
            let pid = parts.next()?.parse().ok()?;
            if parts.next().is_some() {
                return None;
            }
            Some(BotCommand::BgLogs { pid })
        }
        "stop" => {
            let target = parts.next()?;
            if parts.next().is_some() {
                return None;
            }
            if target.eq_ignore_ascii_case("all") {
                Some(BotCommand::BgStop { pid: None })
            } else {
                let pid = target.parse().ok()?;
                Some(BotCommand::BgStop { pid: Some(pid) })
            }
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
         - `{prefix} status` — Session, Tools, config, Codex Plan Usage, Web Backend\n\
         - `{prefix} bg` — list Background Processes\n\
         - `{prefix} bg logs <pid>` — tail Background Process logs\n\
         - `{prefix} bg stop <pid>` — stop one Background Process\n\
         - `{prefix} bg stop all` — stop all running Background Processes"
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
        assert!(text.contains("Tools"));
        assert!(text.contains("Web Backend"));
    }

    #[test]
    fn parses_bg_family() {
        assert_eq!(parse_command("saku", "saku bg"), Some(BotCommand::BgList));
        assert_eq!(
            parse_command("saku", "saku bg logs 12345"),
            Some(BotCommand::BgLogs { pid: 12345 })
        );
        assert_eq!(
            parse_command("saku", "saku bg stop 99"),
            Some(BotCommand::BgStop { pid: Some(99) })
        );
        assert_eq!(
            parse_command("saku", "saku bg stop all"),
            Some(BotCommand::BgStop { pid: None })
        );
    }

    #[test]
    fn rejects_bg_start_via_bot_command() {
        assert_eq!(parse_command("saku", "saku bg start sleep 1"), None);
        assert_eq!(parse_command("saku", "saku bg start"), None);
    }

    #[test]
    fn help_lists_bg_family() {
        let text = help_text("saku");
        assert!(text.contains("`saku bg`"));
        assert!(text.contains("`saku bg logs <pid>`"));
        assert!(text.contains("`saku bg stop <pid>`"));
        assert!(text.contains("`saku bg stop all`"));
    }
}
