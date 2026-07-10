//! Saku binary entrypoint.
//!
//! `saku` runs the Discord bot; `saku login` delegates to `saku-cli`.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use saku::cli::{Command, parse_args};
use saku_discord::run_bot;
use saku_harness::{Config, CredentialStore, Harness, create_codex_provider};

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
        Command::LoginCodex => match saku_cli::login_codex(None).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("saku login codex: {err}");
                ExitCode::FAILURE
            }
        },
        Command::Run { config } => run_bot_cmd(config).await,
    }
}

async fn run_bot_cmd(config: Option<PathBuf>) -> ExitCode {
    let config_path = config.unwrap_or_else(|| {
        dirs::home_dir()
            .expect("home")
            .join(".saku/config.toml")
    });

    let config = match Config::load(&config_path) {
        Ok(c) => c,
        Err(err) => {
            eprintln!("failed to load {}: {err}", config_path.display());
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
    saku_discord::bot::register_default_tools(&harness).await;

    // Keep Arc alive for clarity; harness is Clone.
    let _ = Arc::new(());

    if let Err(err) = run_bot(config, harness).await {
        eprintln!("discord bot error: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
