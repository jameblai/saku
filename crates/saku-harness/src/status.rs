//! Session / config / Codex Plan Usage status for the `status` Bot Command.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Effort;
use crate::types::TokenUsage;

/// Whether a Session currently has an active or queued Run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Idle,
    /// Active Run; `waiting` is how many Runs sit behind it in the Run Queue.
    Running {
        waiting: usize,
    },
    Queued {
        depth: usize,
    },
}

impl RunState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running { .. } => "running",
            Self::Queued { .. } => "queued",
        }
    }
}

/// One Codex rate-limit window (5h or weekly).
#[derive(Debug, Clone, PartialEq)]
pub struct PlanWindow {
    pub remaining_percent: f64,
    /// Unix seconds when this window resets, if known.
    pub reset_at: Option<i64>,
}

/// Live Codex account Plan Usage + Reset Credits.
#[derive(Debug, Clone, PartialEq)]
pub struct CodexAccountStatus {
    pub plan_type: Option<String>,
    pub primary: Option<PlanWindow>,
    pub weekly: Option<PlanWindow>,
    pub reset_credits_available: Option<u32>,
    /// Soonest Reset Credit expiry (unix seconds).
    pub reset_credit_expires_at: Option<i64>,
}

/// Web Backend line(s) for `saku status`.
#[derive(Debug, Clone, PartialEq)]
pub enum WebBackendStatus {
    NoCredential,
    /// An API-key Credential is stored for the configured Web Backend.
    CredentialConfigured,
    Unavailable {
        reason: String,
    },
}

/// Snapshot assembled for `saku status`.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusReport {
    pub model: String,
    pub effort: Effort,
    pub cwd: PathBuf,
    pub run_state: RunState,
    pub run_count: u64,
    pub usage: TokenUsage,
    pub estimated_cost_usd: f64,
    /// Context fill percent (0–100+), if computable.
    pub context_fill_percent: Option<f64>,
    pub context_tokens: Option<u64>,
    pub context_window: Option<u64>,
    pub default_model: String,
    pub default_effort: Effort,
    pub provider: String,
    pub account: Option<CodexAccountStatus>,
    pub account_error: Option<String>,
    /// Running Background Processes in this Session.
    pub background_running: usize,
    /// Exited (still listed) Background Processes in this Session.
    pub background_exited: usize,
    /// Tool names currently registered on the Harness (registration order).
    pub tool_names: Vec<String>,
    /// Configured Web Backend id (e.g. `exa`).
    pub web_backend: String,
    pub web_status: WebBackendStatus,
}

/// Format a plain-text status reply (Discord-safe).
pub fn format_status(report: &StatusReport) -> String {
    let mut out = String::new();

    out.push_str("**Session**\n");
    out.push_str(&format!("Model: `{}`\n", report.model));
    out.push_str(&format!("Effort: `{}`\n", report.effort));
    out.push_str(&format!("Working Directory: `{}`\n", report.cwd.display()));
    match report.run_state {
        RunState::Running { waiting: 0 } => {
            out.push_str("Run state: running\n");
        }
        RunState::Running { waiting } => {
            out.push_str(&format!("Run state: running ({waiting} waiting)\n"));
        }
        RunState::Queued { depth } => {
            out.push_str(&format!("Run state: queued ({depth} waiting)\n"));
        }
        RunState::Idle => out.push_str("Run state: idle\n"),
    }
    out.push_str(&format!(
        "Background: {} running, {} exited\n",
        report.background_running, report.background_exited
    ));
    out.push_str(&format!("Runs: {}\n", report.run_count));
    out.push_str(&format_context_fill(report));
    out.push_str(&format!(
        "Tokens: input {} / output {} / cache read {} / cache write {}\n",
        report.usage.input, report.usage.output, report.usage.cache_read, report.usage.cache_write
    ));
    out.push_str(&format!("Est. cost: ${:.4}\n", report.estimated_cost_usd));

    out.push_str("\n**Tools**\n");
    if !report.tool_names.is_empty() {
        let names: Vec<String> = report.tool_names.iter().map(|n| format!("`{n}`")).collect();
        out.push_str(&names.join(" "));
        out.push('\n');
    }

    out.push_str("\n**Config**\n");
    out.push_str(&format!("Default model: `{}`\n", report.default_model));
    out.push_str(&format!("Default effort: `{}`\n", report.default_effort));
    out.push_str(&format!("Provider: `{}`\n", report.provider));

    out.push_str("\n**Codex account**\n");
    if let Some(err) = &report.account_error {
        out.push_str(&format!("Plan Usage unavailable: {err}\n"));
    } else if let Some(account) = &report.account {
        if let Some(plan) = &account.plan_type {
            out.push_str(&format!("Plan: {plan}\n"));
        }
        if let Some(w) = &account.primary {
            out.push_str(&format!(
                "Plan Usage 5h: {:.0}% left{}\n",
                w.remaining_percent,
                format_reset_suffix(w.reset_at)
            ));
        }
        if let Some(w) = &account.weekly {
            out.push_str(&format!(
                "Plan Usage weekly: {:.0}% left{}\n",
                w.remaining_percent,
                format_reset_suffix(w.reset_at)
            ));
        }
        match (
            account.reset_credits_available,
            account.reset_credit_expires_at,
        ) {
            (Some(n), Some(exp)) => {
                out.push_str(&format!(
                    "Reset Credits: {n} available{}\n",
                    format_expiry_suffix(exp)
                ));
            }
            (Some(n), None) => {
                out.push_str(&format!("Reset Credits: {n} available\n"));
            }
            _ => {}
        }
    } else {
        out.push_str("Plan Usage unavailable\n");
    }

    out.push_str("\n**Web Backend**\n");
    out.push_str(&format!("Backend: `{}`\n", report.web_backend));
    match &report.web_status {
        WebBackendStatus::NoCredential => out.push_str("no Credential\n"),
        WebBackendStatus::CredentialConfigured => out.push_str("Credential configured\n"),
        WebBackendStatus::Unavailable { reason } => {
            out.push_str(&format!("Web Backend unavailable: {reason}\n"));
        }
    }

    out
}

fn format_context_fill(report: &StatusReport) -> String {
    match (
        report.context_fill_percent,
        report.context_tokens,
        report.context_window,
    ) {
        (Some(pct), Some(tokens), Some(window)) => {
            format!("Context: {pct:.0}% ({tokens}/{window})\n")
        }
        (Some(pct), _, _) => format!("Context: {pct:.0}%\n"),
        _ => "Context: ?\n".into(),
    }
}

fn format_reset_suffix(reset_at: Option<i64>) -> String {
    match reset_at {
        Some(ts) => format!(" (resets {})", format_relative_time(ts)),
        None => String::new(),
    }
}

fn format_expiry_suffix(expires_at: i64) -> String {
    format!(" (next expires {})", format_relative_time(expires_at))
}

fn format_relative_time(unix_secs: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let delta = unix_secs - now;
    if delta.abs() < 60 {
        return "now".into();
    }
    let past = delta < 0;
    let secs = delta.abs();
    let label = if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        if m == 0 {
            format!("{h}h")
        } else {
            format!("{h}h {m}m")
        }
    } else {
        let d = secs / 86400;
        let h = (secs % 86400) / 3600;
        if h == 0 {
            format!("{d}d")
        } else {
            format!("{d}d {h}h")
        }
    };
    if past {
        format!("{label} ago")
    } else {
        format!("in {label}")
    }
}

/// Estimate USD cost from token counts and per-million rates.
pub fn estimate_cost_usd(usage: &TokenUsage, rates: &ModelRates) -> f64 {
    let m = 1_000_000.0;
    (usage.input as f64) * rates.input / m
        + (usage.output as f64) * rates.output / m
        + (usage.cache_read as f64) * rates.cache_read / m
        + (usage.cache_write as f64) * rates.cache_write / m
}

/// Published API rates ($ / million tokens) for cost estimates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelRates {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample_report() -> StatusReport {
        StatusReport {
            model: "gpt-5.5".into(),
            effort: Effort::High,
            cwd: PathBuf::from("/home/james/ws"),
            run_state: RunState::Idle,
            run_count: 3,
            usage: TokenUsage {
                input: 1000,
                output: 200,
                cache_read: 500,
                cache_write: 0,
            },
            estimated_cost_usd: 0.0123,
            context_fill_percent: Some(12.0),
            context_tokens: Some(34_000),
            context_window: Some(272_000),
            default_model: "gpt-5.5".into(),
            default_effort: Effort::Medium,
            provider: "codex".into(),
            account: Some(CodexAccountStatus {
                plan_type: Some("pro".into()),
                primary: Some(PlanWindow {
                    remaining_percent: 96.0,
                    reset_at: Some(
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_secs() as i64
                            + 3600,
                    ),
                }),
                weekly: Some(PlanWindow {
                    remaining_percent: 80.0,
                    reset_at: Some(
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_secs() as i64
                            + 86400 * 2,
                    ),
                }),
                reset_credits_available: Some(2),
                reset_credit_expires_at: Some(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64
                        + 86400 * 12,
                ),
            }),
            account_error: None,
            background_running: 0,
            background_exited: 0,
            tool_names: vec![
                "read".into(),
                "edit".into(),
                "write".into(),
                "bash".into(),
                "web_search".into(),
            ],
            web_backend: "exa".into(),
            web_status: WebBackendStatus::NoCredential,
        }
    }

    #[test]
    fn format_status_includes_session_config_and_plan_sections() {
        let text = format_status(&sample_report());
        assert!(text.contains("**Session**"));
        assert!(text.contains("Model: `gpt-5.5`"));
        assert!(text.contains("Effort: `high`"));
        assert!(text.contains("Working Directory: `/home/james/ws`"));
        assert!(text.contains("Run state: idle"));
        assert!(text.contains("Background: 0 running, 0 exited"));
        assert!(text.contains("Runs: 3"));
        assert!(text.contains("Context: 12% (34000/272000)"));
        assert!(text.contains("Tokens: input 1000 / output 200 / cache read 500 / cache write 0"));
        assert!(text.contains("Est. cost: $0.0123"));
        assert!(text.contains("**Config**"));
        assert!(text.contains("Default model: `gpt-5.5`"));
        assert!(text.contains("Provider: `codex`"));
        assert!(text.contains("**Codex account**"));
        assert!(text.contains("Plan: pro"));
        assert!(text.contains("Plan Usage 5h: 96% left"));
        assert!(text.contains("Plan Usage weekly: 80% left"));
        assert!(text.contains("Reset Credits: 2 available"));
    }

    #[test]
    fn format_status_lists_registered_tool_names_after_session() {
        let text = format_status(&sample_report());
        let session_pos = text.find("**Session**").expect("Session");
        let tools_pos = text.find("**Tools**").expect("Tools");
        let config_pos = text.find("**Config**").expect("Config");
        assert!(session_pos < tools_pos && tools_pos < config_pos);
        assert!(text.contains("`read` `edit` `write` `bash` `web_search`"));
    }

    #[test]
    fn format_status_web_backend_after_codex_no_credential() {
        let text = format_status(&sample_report());
        let codex_pos = text.find("**Codex account**").expect("Codex");
        let web_pos = text.find("**Web Backend**").expect("Web Backend");
        assert!(codex_pos < web_pos);
        assert!(text.contains("Backend: `exa`"));
        assert!(text.contains("no Credential"));
    }

    #[test]
    fn format_status_web_backend_credential_configured() {
        let mut report = sample_report();
        report.web_status = WebBackendStatus::CredentialConfigured;
        let text = format_status(&report);
        assert!(text.contains("Credential configured\n"));
        assert!(!text.contains("no Credential"));
    }

    #[test]
    fn format_status_web_backend_unavailable_mirrors_codex() {
        let mut report = sample_report();
        report.web_status = WebBackendStatus::Unavailable {
            reason: "auth expired".into(),
        };
        let text = format_status(&report);
        assert!(text.contains("Web Backend unavailable: auth expired"));
    }

    #[test]
    fn format_status_shows_account_error() {
        let mut report = sample_report();
        report.account = None;
        report.account_error = Some("auth expired".into());
        let text = format_status(&report);
        assert!(text.contains("Plan Usage unavailable: auth expired"));
    }

    #[test]
    fn estimate_cost_uses_per_million_rates() {
        let usage = TokenUsage {
            input: 1_000_000,
            output: 0,
            cache_read: 0,
            cache_write: 0,
        };
        let rates = ModelRates {
            input: 5.0,
            output: 30.0,
            cache_read: 0.5,
            cache_write: 0.0,
        };
        assert!((estimate_cost_usd(&usage, &rates) - 5.0).abs() < 1e-9);
    }
}
