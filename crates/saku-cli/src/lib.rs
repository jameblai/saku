//! Host CLI: `login`, Setup, and config-related commands that do not need Discord.

mod setup;

pub use setup::{parse_authorized_user_ids, setup};

use std::io::{self, Write};
use std::path::PathBuf;

use saku_harness::{
    Config, CredentialStore, DeviceCodeInfo, LoginNotify, login_device_code,
};

struct StdioNotify;

impl LoginNotify for StdioNotify {
    fn device_code(&mut self, info: &DeviceCodeInfo) {
        println!("Open {} and enter code: {}", info.verification_uri, info.user_code);
        let _ = io::stdout().flush();
    }

    fn progress(&mut self, message: &str) {
        println!("{message}");
        let _ = io::stdout().flush();
    }
}

/// Run `saku login codex` using config Data Dir (or default ~/.saku).
pub async fn login_codex(config_path: Option<PathBuf>) -> Result<(), String> {
    let data_dir = resolve_data_dir(config_path)?;
    let store = CredentialStore::open(&data_dir).map_err(|e| e.to_string())?;
    let mut notify = StdioNotify;
    login_device_code(&store, &mut notify)
        .await
        .map_err(|e| e.to_string())?;
    println!("Codex credentials saved to {}", store.path().display());
    Ok(())
}

fn resolve_data_dir(config_path: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(path) = config_path {
        let cfg = Config::load(&path).map_err(|e| e.to_string())?;
        return Ok(cfg.data_dir);
    }
    let home = dirs::home_dir().ok_or_else(|| "could not determine home directory".to_string())?;
    let default_config = home.join(".saku/config.toml");
    if default_config.exists() {
        let cfg = Config::load(&default_config).map_err(|e| e.to_string())?;
        return Ok(cfg.data_dir);
    }
    Ok(home.join(".saku"))
}
