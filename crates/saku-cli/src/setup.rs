//! Interactive Setup: Discord config then Codex Login.

use std::io::{self, Write};
use std::path::PathBuf;

use saku_harness::Config;

/// Parse Authorised User snowflake ids from a comma- or whitespace-separated string.
///
/// Each id must be non-empty and digits-only.
pub fn parse_authorized_user_ids(input: &str) -> Result<Vec<String>, String> {
    let ids: Vec<String> = input
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    if ids.is_empty() {
        return Err("at least one Discord user id is required".into());
    }
    for id in &ids {
        if !id.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!(
                "invalid Discord user id `{id}` (expected digits only)"
            ));
        }
    }
    Ok(ids)
}

fn default_config_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "could not determine home directory".to_string())?;
    Ok(home.join(".saku/config.toml"))
}

/// Resolve the config.toml path for Setup.
pub fn resolve_config_path(config_path: Option<PathBuf>) -> Result<PathBuf, String> {
    match config_path {
        Some(p) => Ok(p),
        None => default_config_path(),
    }
}

fn print_discord_help() {
    println!(
        "Create a bot at https://discord.com/developers/applications, enable the intents below, then paste the token."
    );
    println!();
    println!("Required Gateway intents:");
    println!("  - GUILDS");
    println!("  - GUILD_MESSAGES");
    println!("  - MESSAGE_CONTENT (privileged — enable under Bot → Privileged Gateway Intents)");
    println!("  - GUILD_MESSAGE_REACTIONS");
    println!();
}

fn print_user_id_help() {
    println!(
        "Paste your Discord user id (enable Developer Mode, then Copy User ID from your profile)."
    );
}

fn prompt_line(label: &str) -> Result<String, String> {
    print!("{label}");
    let _ = io::stdout().flush();
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    Ok(line.trim().to_string())
}

fn prompt_token(prefill: Option<&str>) -> Result<String, String> {
    print_discord_help();
    loop {
        if prefill.is_some() {
            print!("Discord bot token [Enter to keep existing]: ");
        } else {
            print!("Discord bot token: ");
        }
        let _ = io::stdout().flush();
        let entered = rpassword::read_password().map_err(|e| e.to_string())?;
        let entered = entered.trim().to_string();
        if entered.is_empty() {
            if let Some(existing) = prefill {
                return Ok(existing.to_string());
            }
            eprintln!("discord bot token is required");
            continue;
        }
        return Ok(entered);
    }
}

fn prompt_authorized_user_ids(prefill: Option<&[String]>) -> Result<Vec<String>, String> {
    print_user_id_help();
    loop {
        let label = match prefill {
            Some(ids) if !ids.is_empty() => {
                format!(
                    "Authorised User id(s) [{}] (comma/space-separated, Enter to keep): ",
                    ids.join(", ")
                )
            }
            _ => "Authorised User id(s) (comma/space-separated): ".to_string(),
        };
        let line = prompt_line(&label)?;
        if line.is_empty() {
            if let Some(ids) = prefill
                && !ids.is_empty()
            {
                return Ok(ids.to_vec());
            }
            eprintln!("at least one Discord user id is required");
            continue;
        }
        match parse_authorized_user_ids(&line) {
            Ok(ids) => return Ok(ids),
            Err(err) => {
                eprintln!("{err}");
            }
        }
    }
}

/// Run interactive Setup: write Discord config, then Codex Login.
///
/// Explicit `saku setup` exits after success (does not start the bot).
/// If Login fails after a successful config write, the config is kept.
pub async fn setup(config_path: Option<PathBuf>) -> Result<(), String> {
    let path = resolve_config_path(config_path)?;
    let (pre_token, pre_ids) = Config::read_setup_prefill(&path);

    let token = prompt_token(pre_token.as_deref())?;
    let ids = prompt_authorized_user_ids(pre_ids.as_deref())?;

    Config::write_required(&path, &token, &ids).map_err(|e| e.to_string())?;
    println!("Wrote {}", path.display());

    match crate::login_codex(Some(path.clone())).await {
        Ok(()) => Ok(()),
        Err(err) => {
            eprintln!(
                "Codex Login failed (config kept at {}): {err}",
                path.display()
            );
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_id() {
        assert_eq!(
            parse_authorized_user_ids("123456789012345678").unwrap(),
            vec!["123456789012345678"]
        );
    }

    #[test]
    fn parses_comma_and_whitespace_separated() {
        assert_eq!(
            parse_authorized_user_ids("111, 222\t333\n444").unwrap(),
            vec!["111", "222", "333", "444"]
        );
    }

    #[test]
    fn rejects_empty() {
        assert!(parse_authorized_user_ids("").is_err());
        assert!(parse_authorized_user_ids("  ,  ").is_err());
    }

    #[test]
    fn rejects_non_digits() {
        assert!(parse_authorized_user_ids("abc").is_err());
        assert!(parse_authorized_user_ids("123,12a4").is_err());
    }

    #[test]
    fn resolve_config_path_uses_override() {
        let p = resolve_config_path(Some(PathBuf::from("/tmp/x.toml"))).unwrap();
        assert_eq!(p, PathBuf::from("/tmp/x.toml"));
    }
}
