//! Saku binary entrypoint.
//!
//! `saku` runs the Discord bot; `saku setup` / `saku login` delegate to `saku-cli`.
//! Bare `saku` auto-recovers via Setup or Login when needed (ADR-0017).

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use saku::boot::{BootPlan, plan_boot};
use saku::cli::{Command, parse_args};
use saku_discord::run_bot;
use saku_harness::{CODEX_PROVIDER_ID, Config, CredentialStore, Harness, create_codex_provider};

#[tokio::main]
async fn main() -> ExitCode {
    let command = match parse_args(std::env::args_os().skip(1)) {
        Ok(c) => c,
        Err(err) => {
            let _ = err.print();
            return ExitCode::from(u8::try_from(err.exit_code()).unwrap_or(1));
        }
    };

    match command {
        Command::Setup => match saku_cli::setup(None).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("saku setup: {err}");
                ExitCode::FAILURE
            }
        },
        Command::LoginCodex => match saku_cli::login_codex(None).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("saku login codex: {err}");
                ExitCode::FAILURE
            }
        },
        Command::LoginExa => match saku_cli::login_exa(None).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("saku login exa: {err}");
                ExitCode::FAILURE
            }
        },
        Command::Update => match saku_cli::update().await {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("saku update: {err}");
                ExitCode::FAILURE
            }
        },
        Command::ServiceInstall => service_cmd(saku_cli::install),
        Command::ServiceUninstall => service_cmd(saku_cli::uninstall),
        Command::ServiceEnable => service_cmd(saku_cli::enable),
        Command::ServiceDisable => service_cmd(saku_cli::disable),
        Command::ServiceStatus => service_cmd(saku_cli::status),
        Command::Run { config } => run_bot_cmd(config).await,
    }
}

fn service_cmd(f: fn() -> Result<(), String>) -> ExitCode {
    match f() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("saku service: {err}");
            ExitCode::FAILURE
        }
    }
}

async fn run_bot_cmd(config: Option<PathBuf>) -> ExitCode {
    let config_path =
        config.unwrap_or_else(|| dirs::home_dir().expect("home").join(".saku/config.toml"));

    let config = match ensure_ready(&config_path).await {
        Ok(c) => c,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };

    let credentials = match CredentialStore::open(&config.data_dir) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("credential store: {err}");
            return ExitCode::FAILURE;
        }
    };
    let provider = create_codex_provider(credentials);
    let harness = match Harness::new(config.clone(), provider) {
        Ok(h) => h,
        Err(err) => {
            eprintln!("harness: {err}");
            return ExitCode::FAILURE;
        }
    };
    harness.register_default_tools().await;

    // Keep Arc alive for clarity; harness is Clone.
    let _ = Arc::new(());

    if let Err(err) = run_bot(config, harness).await {
        eprintln!("discord bot error: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// Load config, auto-running Setup or Login when bare `saku` needs recovery.
async fn ensure_ready(config_path: &Path) -> Result<Config, String> {
    let loaded = Config::load(config_path);
    let has_codex_credential = match &loaded {
        Ok(cfg) => match CredentialStore::open(&cfg.data_dir) {
            Ok(store) => store.read(CODEX_PROVIDER_ID).ok().flatten().is_some(),
            Err(_) => false,
        },
        Err(_) => false,
    };

    match plan_boot(loaded.as_ref(), has_codex_credential) {
        BootPlan::NeedsSetup => {
            eprintln!(
                "Config missing or invalid at {}; starting Setup…",
                config_path.display()
            );
            saku_cli::setup(Some(config_path.to_path_buf()))
                .await
                .map_err(|e| format!("saku setup: {e}"))?;
            Config::load(config_path)
                .map_err(|e| format!("failed to load {} after Setup: {e}", config_path.display()))
        }
        BootPlan::NeedsLogin => {
            eprintln!("Codex Credential missing; starting Login…");
            saku_cli::login_codex(Some(config_path.to_path_buf()))
                .await
                .map_err(|e| format!("saku login codex: {e}"))?;
            loaded.map_err(|e| format!("failed to load {}: {e}", config_path.display()))
        }
        BootPlan::Ready => {
            loaded.map_err(|e| format!("failed to load {}: {e}", config_path.display()))
        }
    }
}
