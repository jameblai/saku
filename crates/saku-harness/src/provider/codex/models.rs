//! Codex model allowlist and Effort rules (ADR 0011).

use crate::config::Effort;

pub const PROVIDER_ID: &str = "codex";

pub const ALLOWED_MODELS: &[&str] = &[
    "gpt-5.5",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
    "gpt-5.4-mini",
];

pub fn is_allowed_model(model: &str) -> bool {
    ALLOWED_MODELS.contains(&model)
}

pub fn default_effort_for_model(_model: &str) -> Effort {
    Effort::Medium
}

pub fn supported_efforts(model: &str) -> &'static [Effort] {
    if model.starts_with("gpt-5.6") {
        &[
            Effort::Minimal,
            Effort::Low,
            Effort::Medium,
            Effort::High,
            Effort::Xhigh,
            Effort::Max,
        ]
    } else {
        &[
            Effort::Minimal,
            Effort::Low,
            Effort::Medium,
            Effort::High,
            Effort::Xhigh,
        ]
    }
}

pub fn is_supported_effort(model: &str, effort: Effort) -> bool {
    supported_efforts(model).contains(&effort)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_only_on_56_family() {
        assert!(is_supported_effort("gpt-5.6-sol", Effort::Max));
        assert!(!is_supported_effort("gpt-5.5", Effort::Max));
    }
}
