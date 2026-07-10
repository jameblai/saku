//! Codex model allowlist, Effort rules, context windows, and API rate tables (ADR 0011).

use crate::config::Effort;
use crate::status::ModelRates;

pub const PROVIDER_ID: &str = "codex";

pub const ALLOWED_MODELS: &[&str] = &[
    "gpt-5.5",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
    "gpt-5.4-mini",
];

/// Per-model metadata for status (context window + published API $/MTok).
#[derive(Debug, Clone, Copy)]
pub struct ModelInfo {
    pub context_window: u64,
    pub rates: ModelRates,
}

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

/// Look up context window and published API rates (pi Codex catalog).
pub fn model_info(model: &str) -> Option<ModelInfo> {
    Some(match model {
        "gpt-5.5" => ModelInfo {
            context_window: 272_000,
            rates: ModelRates {
                input: 5.0,
                output: 30.0,
                cache_read: 0.5,
                cache_write: 0.0,
            },
        },
        "gpt-5.6-sol" => ModelInfo {
            context_window: 372_000,
            rates: ModelRates {
                input: 5.0,
                output: 30.0,
                cache_read: 0.5,
                cache_write: 6.25,
            },
        },
        "gpt-5.6-terra" => ModelInfo {
            context_window: 372_000,
            rates: ModelRates {
                input: 2.5,
                output: 15.0,
                cache_read: 0.25,
                cache_write: 3.125,
            },
        },
        "gpt-5.6-luna" => ModelInfo {
            context_window: 372_000,
            rates: ModelRates {
                input: 1.0,
                output: 6.0,
                cache_read: 0.1,
                cache_write: 1.25,
            },
        },
        "gpt-5.4-mini" => ModelInfo {
            context_window: 272_000,
            rates: ModelRates {
                input: 0.75,
                output: 4.5,
                cache_read: 0.075,
                cache_write: 0.0,
            },
        },
        _ => return None,
    })
}

pub fn context_window_for(model: &str) -> Option<u64> {
    model_info(model).map(|m| m.context_window)
}

pub fn rates_for(model: &str) -> Option<ModelRates> {
    model_info(model).map(|m| m.rates)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_only_on_56_family() {
        assert!(is_supported_effort("gpt-5.6-sol", Effort::Max));
        assert!(!is_supported_effort("gpt-5.5", Effort::Max));
    }

    #[test]
    fn allowlisted_models_have_rates_and_context() {
        for model in ALLOWED_MODELS {
            let info = model_info(model).expect(model);
            assert!(info.context_window > 0);
            assert!(info.rates.input > 0.0);
        }
    }
}
