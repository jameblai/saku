//! Load and resolve `config.toml` for Saku.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

/// Default Command Prefix.
pub const DEFAULT_COMMAND_PREFIX: &str = "saku";

/// Default model for new Sessions.
pub const DEFAULT_MODEL: &str = "gpt-5.5";

/// Default Effort for new Sessions.
pub const DEFAULT_EFFORT: Effort = Effort::Medium;

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
        })
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
"#;
        let cfg = Config::parse(text).expect("parse");
        assert_eq!(cfg.command_prefix, "bot");
        assert_eq!(cfg.workspace, home.join("projects"));
        assert_eq!(cfg.data_dir, home.join(".saku-custom"));
        assert_eq!(cfg.default_model, "gpt-5.4-mini");
        assert_eq!(cfg.default_effort, Effort::High);
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
}
