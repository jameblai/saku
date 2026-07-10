//! Argv parsing for the `saku` binary.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Top-level command after parsing argv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Run the Discord bot.
    Run { config: Option<PathBuf> },
    /// Host CLI Login for a Provider.
    LoginCodex,
}

#[derive(Debug, Parser)]
#[command(
    name = "saku",
    about = "Saku Discord coding agent",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    /// Path to config.toml (bot only).
    #[arg(long)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<CliSubcommand>,
}

#[derive(Debug, Subcommand)]
enum CliSubcommand {
    /// Obtain and store a Provider Credential.
    Login {
        #[command(subcommand)]
        provider: LoginProvider,
    },
}

#[derive(Debug, Subcommand)]
enum LoginProvider {
    /// ChatGPT / Codex subscription (device-code OAuth).
    Codex,
}

/// Parse argv (excluding program name) into a [`Command`].
///
/// Returns `Err` with a clap [`Error`](clap::Error) on invalid usage (or `--help` / `--version`).
pub fn parse_args<I, S>(args: I) -> Result<Command, clap::Error>
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let cli = Cli::try_parse_from(
        std::iter::once(std::ffi::OsString::from("saku")).chain(args.into_iter().map(Into::into)),
    )?;

    Ok(match cli.command {
        None => Command::Run {
            config: cli.config,
        },
        Some(CliSubcommand::Login {
            provider: LoginProvider::Codex,
        }) => Command::LoginCodex,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_saku_runs_bot_with_default_config() {
        let cmd = parse_args([] as [&str; 0]).expect("parse");
        assert_eq!(cmd, Command::Run { config: None });
    }

    #[test]
    fn config_flag_selects_config_path() {
        let cmd = parse_args(["--config", "/tmp/custom.toml"]).expect("parse");
        assert_eq!(
            cmd,
            Command::Run {
                config: Some(PathBuf::from("/tmp/custom.toml")),
            }
        );
    }

    #[test]
    fn login_codex_subcommand() {
        let cmd = parse_args(["login", "codex"]).expect("parse");
        assert_eq!(cmd, Command::LoginCodex);
    }

    #[test]
    fn login_without_provider_is_error() {
        let err = parse_args(["login"]).unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn unknown_subcommand_is_error() {
        let err = parse_args(["setup"]).unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn config_conflicts_with_login_subcommand() {
        let err = parse_args(["--config", "/tmp/c.toml", "login", "codex"]).unwrap_err();
        assert!(!err.to_string().is_empty());
    }
}
