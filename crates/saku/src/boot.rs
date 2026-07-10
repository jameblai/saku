//! Boot recovery planning for bare `saku`.

use saku_harness::{Config, ConfigError};

/// What bare `saku` should do before starting the Discord bot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootPlan {
    /// Config cannot be loaded — run Setup, then continue.
    NeedsSetup,
    /// Config is OK but Codex Credential is missing — run Login only, then continue.
    NeedsLogin,
    /// Config and Credential are ready.
    Ready,
}

/// Decide recovery from a config load attempt and whether a Codex Credential exists.
pub fn plan_boot(config: Result<&Config, &ConfigError>, has_codex_credential: bool) -> BootPlan {
    match config {
        Err(_) => BootPlan::NeedsSetup,
        Ok(_) if !has_codex_credential => BootPlan::NeedsLogin,
        Ok(_) => BootPlan::Ready,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use saku_harness::Config;
    use std::io::ErrorKind;

    fn sample_config() -> Config {
        Config::parse(
            r#"
discord_token = "tok"
authorized_user_ids = ["1"]
"#,
        )
        .expect("config")
    }

    #[test]
    fn missing_config_needs_setup() {
        let err = ConfigError::Io(std::io::Error::new(
            ErrorKind::NotFound,
            "No such file or directory",
        ));
        assert_eq!(plan_boot(Err(&err), false), BootPlan::NeedsSetup);
        assert_eq!(plan_boot(Err(&err), true), BootPlan::NeedsSetup);
    }

    #[test]
    fn invalid_config_needs_setup() {
        let err = ConfigError::MissingField("discord_token");
        assert_eq!(plan_boot(Err(&err), false), BootPlan::NeedsSetup);
    }

    #[test]
    fn config_ok_without_credential_needs_login() {
        let cfg = sample_config();
        assert_eq!(plan_boot(Ok(&cfg), false), BootPlan::NeedsLogin);
    }

    #[test]
    fn config_and_credential_ready() {
        let cfg = sample_config();
        assert_eq!(plan_boot(Ok(&cfg), true), BootPlan::Ready);
    }
}
