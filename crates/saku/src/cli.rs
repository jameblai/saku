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
    /// Host CLI Login for the Exa Web Backend.
    LoginExa,
    /// Interactive Setup (config + Codex Login).
    Setup,
    /// Replace the installed binary from the configured Release Channel.
    Update,
    /// Install the Host Service unit file and enable linger.
    ServiceInstall,
    /// Remove the Host Service unit (linger stays on).
    ServiceUninstall,
    /// Enable and start the Host Service.
    ServiceEnable,
    /// Disable and stop the Host Service.
    ServiceDisable,
    /// Report Host Service status.
    ServiceStatus,
}

#[derive(Debug, Parser)]
#[command(
    name = "saku",
    about = "Saku Discord coding agent",
    version = env!("SAKU_VERSION"),
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
    /// Interactive Setup: Discord config then Codex Login.
    Setup,
    /// Update Saku from the configured Release Channel.
    Update,
    /// Manage the user-level Host Service (systemd unit).
    Service {
        #[command(subcommand)]
        action: Option<ServiceAction>,
    },
    /// Obtain and store a Provider or Web Backend Credential.
    Login {
        #[command(subcommand)]
        id: LoginId,
    },
}

#[derive(Debug, Subcommand)]
enum ServiceAction {
    /// Write the unit file and enable linger.
    Install,
    /// Stop, disable, and remove the unit file.
    Uninstall,
    /// Enable and start the unit now.
    Enable,
    /// Disable and stop the unit.
    Disable,
}

#[derive(Debug, Subcommand)]
enum LoginId {
    /// ChatGPT / Codex subscription (device-code OAuth).
    Codex,
    /// Exa Web Backend (API key).
    Exa,
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
        None => Command::Run { config: cli.config },
        Some(CliSubcommand::Setup) => Command::Setup,
        Some(CliSubcommand::Update) => Command::Update,
        Some(CliSubcommand::Service { action }) => match action {
            None => Command::ServiceStatus,
            Some(ServiceAction::Install) => Command::ServiceInstall,
            Some(ServiceAction::Uninstall) => Command::ServiceUninstall,
            Some(ServiceAction::Enable) => Command::ServiceEnable,
            Some(ServiceAction::Disable) => Command::ServiceDisable,
        },
        Some(CliSubcommand::Login { id: LoginId::Codex }) => Command::LoginCodex,
        Some(CliSubcommand::Login { id: LoginId::Exa }) => Command::LoginExa,
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
    fn login_exa_subcommand() {
        let cmd = parse_args(["login", "exa"]).expect("parse");
        assert_eq!(cmd, Command::LoginExa);
    }

    #[test]
    fn setup_subcommand() {
        let cmd = parse_args(["setup"]).expect("parse");
        assert_eq!(cmd, Command::Setup);
    }

    #[test]
    fn update_subcommand() {
        let cmd = parse_args(["update"]).expect("parse");
        assert_eq!(cmd, Command::Update);
    }

    #[test]
    fn service_status_without_subcommand() {
        let cmd = parse_args(["service"]).expect("parse");
        assert_eq!(cmd, Command::ServiceStatus);
    }

    #[test]
    fn service_install_subcommand() {
        let cmd = parse_args(["service", "install"]).expect("parse");
        assert_eq!(cmd, Command::ServiceInstall);
    }

    #[test]
    fn service_uninstall_subcommand() {
        let cmd = parse_args(["service", "uninstall"]).expect("parse");
        assert_eq!(cmd, Command::ServiceUninstall);
    }

    #[test]
    fn service_enable_subcommand() {
        let cmd = parse_args(["service", "enable"]).expect("parse");
        assert_eq!(cmd, Command::ServiceEnable);
    }

    #[test]
    fn service_disable_subcommand() {
        let cmd = parse_args(["service", "disable"]).expect("parse");
        assert_eq!(cmd, Command::ServiceDisable);
    }

    #[test]
    fn service_rejects_unknown_subcommand() {
        assert!(parse_args(["service", "start"]).is_err());
    }

    #[test]
    fn update_rejects_channel_flag() {
        assert!(parse_args(["update", "--channel", "nightly"]).is_err());
    }

    #[test]
    fn login_without_id_is_error() {
        let err = parse_args(["login"]).unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn unknown_subcommand_is_error() {
        let err = parse_args(["not-a-command"]).unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn config_conflicts_with_login_subcommand() {
        let err = parse_args(["--config", "/tmp/c.toml", "login", "codex"]).unwrap_err();
        assert!(!err.to_string().is_empty());
    }
}
