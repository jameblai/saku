//! Saku binary entrypoint.
//!
//! `saku` runs the Discord bot; `saku login` delegates to `saku-cli`.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use saku_discord::run_bot;
use saku_harness::{Config, CredentialStore, create_codex_provider, Harness};

#[tokio::main]
async fn main() -> ExitCode {
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) == Some("login") {
        args.remove(0);
        return match args.as_slice() {
            [cmd] if cmd == "codex" => match saku_cli::login_codex(None).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("saku login codex: {err}");
                    ExitCode::FAILURE
                }
            },
            _ => {
                eprintln!("usage: saku login codex");
                ExitCode::FAILURE
            }
        };
    }

    let config_path = args
        .iter()
        .position(|a| a == "--config")
        .and_then(|i| args.get(i + 1).cloned())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
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
