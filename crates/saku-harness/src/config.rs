//! Load and resolve `config.toml` for Saku.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

/// Default Command Prefix.
pub const DEFAULT_COMMAND_PREFIX: &str = "saku";

/// Default model for new Sessions.
pub const DEFAULT_MODEL: &str = "gpt-5.5";

/// Default Effort for new Sessions.
pub const DEFAULT_EFFORT: Effort = Effort::Medium;

/// Default Web Backend id.
pub const DEFAULT_WEB_BACKEND: &str = "exa";

/// Release line used by Install and Update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReleaseChannel {
    Stable,
    Nightly,
}

impl ReleaseChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Nightly => "nightly",
        }
    }
}

impl std::fmt::Display for ReleaseChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Reasoning / thinking level for a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }
}

impl std::fmt::Display for Effort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse config: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("missing required field `{0}`")]
    MissingField(&'static str),
    #[error("could not determine home directory")]
    NoHome,
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    discord_token: Option<String>,
    authorized_user_ids: Option<Vec<String>>,
    command_prefix: Option<String>,
    workspace: Option<String>,
    data_dir: Option<String>,
    default_model: Option<String>,
    default_effort: Option<Effort>,
    web_backend: Option<String>,
    release_channel: Option<ReleaseChannel>,
}

/// Resolved Saku configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub discord_token: String,
    pub authorized_user_ids: Vec<String>,
    pub command_prefix: String,
    pub workspace: PathBuf,
    pub data_dir: PathBuf,
    pub default_model: String,
    pub default_effort: Effort,
    pub web_backend: String,
    pub release_channel: ReleaseChannel,
}

impl Config {
    /// Load config from `path`, applying defaults and expanding `~`.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let text = fs::read_to_string(path)?;
        Self::parse(&text)
    }

    /// Parse TOML text into a resolved [`Config`].
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let raw: RawConfig = toml::from_str(text)?;
        let discord_token = raw
            .discord_token
            .filter(|s| !s.is_empty())
            .ok_or(ConfigError::MissingField("discord_token"))?;
        let authorized_user_ids = raw
            .authorized_user_ids
            .filter(|ids| !ids.is_empty())
            .ok_or(ConfigError::MissingField("authorized_user_ids"))?;

        let home = dirs::home_dir().ok_or(ConfigError::NoHome)?;
        let workspace = match raw.workspace {
            Some(p) => expand_path(&p, &home)?,
            None => home.clone(),
        };
        let data_dir = match raw.data_dir {
            Some(p) => expand_path(&p, &home)?,
            None => home.join(".saku"),
        };

        Ok(Self {
            discord_token,
            authorized_user_ids,
            command_prefix: raw
                .command_prefix
                .unwrap_or_else(|| DEFAULT_COMMAND_PREFIX.to_string()),
            workspace,
            data_dir,
            default_model: raw
                .default_model
                .unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            default_effort: raw.default_effort.unwrap_or(DEFAULT_EFFORT),
            web_backend: raw
                .web_backend
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| DEFAULT_WEB_BACKEND.to_string()),
            release_channel: raw.release_channel.unwrap_or(ReleaseChannel::Stable),
        })
    }

    /// Read **Release Channel** for Update without requiring a full bot config.
    ///
    /// Missing `release_channel` defaults to **stable**. A missing file is an error.
    pub fn load_release_channel(path: impl AsRef<Path>) -> Result<ReleaseChannel, ConfigError> {
        let text = fs::read_to_string(path.as_ref())?;
        let raw: RawConfig = toml::from_str(&text)?;
        Ok(raw.release_channel.unwrap_or(ReleaseChannel::Stable))
    }

    /// Best-effort prefill for Setup: token and Authorised User ids if readable.
    pub fn read_setup_prefill(path: impl AsRef<Path>) -> (Option<String>, Option<Vec<String>>) {
        let Ok(text) = fs::read_to_string(path) else {
            return (None, None);
        };
        let Ok(raw) = toml::from_str::<RawConfig>(&text) else {
            return (None, None);
        };
        let token = raw.discord_token.filter(|s| !s.is_empty());
        let ids = raw.authorized_user_ids.filter(|ids| !ids.is_empty());
        (token, ids)
    }

    /// Write required Setup fields to `path` (mode `0600`), preserving optional keys.
    pub fn write_required(
        path: impl AsRef<Path>,
        discord_token: &str,
        authorized_user_ids: &[String],
    ) -> Result<(), ConfigError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut table = if path.exists() {
            let text = fs::read_to_string(path)?;
            match text.parse::<toml::Table>() {
                Ok(t) => t,
                Err(_) => toml::Table::new(),
            }
        } else {
            toml::Table::new()
        };

        table.insert(
            "discord_token".into(),
            toml::Value::String(discord_token.to_string()),
        );
        table.insert(
            "authorized_user_ids".into(),
            toml::Value::Array(
                authorized_user_ids
                    .iter()
                    .map(|id| toml::Value::String(id.clone()))
                    .collect(),
            ),
        );

        let body = toml::to_string_pretty(&table).map_err(|e| {
            ConfigError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
        })?;

        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(body.as_bytes())?;
        file.sync_all()?;
        let mut perms = file.metadata()?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(path, perms)?;
        Ok(())
    }
}

fn expand_path(raw: &str, home: &Path) -> Result<PathBuf, ConfigError> {
    if raw == "~" {
        return Ok(home.to_path_buf());
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return Ok(home.join(rest));
    }
    Ok(PathBuf::from(raw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::NamedTempFile;

    fn minimal_toml() -> String {
        r#"
discord_token = "test-token"
authorized_user_ids = ["111", "222"]
"#
        .to_string()
    }

    #[test]
    fn loads_required_fields_and_applies_defaults() {
        let home = dirs::home_dir().expect("home");
        let cfg = Config::parse(&minimal_toml()).expect("parse");
        assert_eq!(cfg.discord_token, "test-token");
        assert_eq!(cfg.authorized_user_ids, vec!["111", "222"]);
        assert_eq!(cfg.command_prefix, "saku");
        assert_eq!(cfg.workspace, home);
        assert_eq!(cfg.data_dir, home.join(".saku"));
        assert_eq!(cfg.default_model, "gpt-5.5");
        assert_eq!(cfg.default_effort, Effort::Medium);
        assert_eq!(cfg.web_backend, "exa");
        assert_eq!(cfg.release_channel, ReleaseChannel::Stable);
    }

    #[test]
    fn loads_optional_overrides_and_expands_tilde() {
        let home = dirs::home_dir().expect("home");
        let text = r#"
discord_token = "tok"
authorized_user_ids = ["1"]
command_prefix = "bot"
workspace = "~/projects"
data_dir = "~/.saku-custom"
default_model = "gpt-5.4-mini"
default_effort = "high"
web_backend = "other"
release_channel = "nightly"
"#;
        let cfg = Config::parse(text).expect("parse");
        assert_eq!(cfg.command_prefix, "bot");
        assert_eq!(cfg.workspace, home.join("projects"));
        assert_eq!(cfg.data_dir, home.join(".saku-custom"));
        assert_eq!(cfg.default_model, "gpt-5.4-mini");
        assert_eq!(cfg.default_effort, Effort::High);
        assert_eq!(cfg.web_backend, "other");
        assert_eq!(cfg.release_channel, ReleaseChannel::Nightly);
    }

    #[test]
    fn rejects_missing_discord_token() {
        let err = Config::parse("authorized_user_ids = [\"1\"]").unwrap_err();
        assert!(matches!(err, ConfigError::MissingField("discord_token")));
    }

    #[test]
    fn rejects_missing_authorized_user_ids() {
        let err = Config::parse("discord_token = \"tok\"").unwrap_err();
        assert!(matches!(
            err,
            ConfigError::MissingField("authorized_user_ids")
        ));
    }

    #[test]
    fn load_reads_from_file() {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "{}", minimal_toml()).unwrap();
        let cfg = Config::load(file.path()).expect("load");
        assert_eq!(cfg.discord_token, "test-token");
    }

    #[test]
    fn write_required_creates_loadable_config_with_mode_0600() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        Config::write_required(&path, "tok", &["111".into(), "222".into()]).expect("write");
        let cfg = Config::load(&path).expect("load");
        assert_eq!(cfg.discord_token, "tok");
        assert_eq!(cfg.authorized_user_ids, vec!["111", "222"]);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn write_required_preserves_optional_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
discord_token = "old"
authorized_user_ids = ["1"]
command_prefix = "bot"
workspace = "/tmp/ws"
release_channel = "nightly"
"#,
        )
        .unwrap();
        Config::write_required(&path, "new-tok", &["9".into()]).expect("write");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("command_prefix"));
        assert!(text.contains("workspace"));
        assert!(text.contains("release_channel = \"nightly\""));
        let cfg = Config::load(&path).expect("load");
        assert_eq!(cfg.discord_token, "new-tok");
        assert_eq!(cfg.authorized_user_ids, vec!["9"]);
        assert_eq!(cfg.command_prefix, "bot");
        assert_eq!(cfg.workspace, PathBuf::from("/tmp/ws"));
    }

    #[test]
    fn load_release_channel_defaults_to_stable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, r#"release_channel = "nightly""#).unwrap();
        assert_eq!(
            Config::load_release_channel(&path).expect("load"),
            ReleaseChannel::Nightly
        );
        std::fs::write(&path, "discord_token = \"tok\"\n").unwrap();
        assert_eq!(
            Config::load_release_channel(&path).expect("load"),
            ReleaseChannel::Stable
        );
    }

    #[test]
    fn read_setup_prefill_best_effort() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert_eq!(Config::read_setup_prefill(&path), (None, None));
        std::fs::write(
            &path,
            r#"
discord_token = "pre"
authorized_user_ids = ["42", "43"]
"#,
        )
        .unwrap();
        assert_eq!(
            Config::read_setup_prefill(&path),
            (Some("pre".into()), Some(vec!["42".into(), "43".into()]))
        );
    }
}
