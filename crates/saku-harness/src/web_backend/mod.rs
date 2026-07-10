//! Web Backend identities and Login helpers (not LLM Providers).

pub mod exa;

use crate::credentials::{Credential, CredentialStore};
use crate::status::WebBackendStatus;

pub use exa::{
    ExaClient, ExaError, ExaTeamInfo, WEB_BACKEND_ID as EXA_WEB_BACKEND_ID,
    login_api_key as login_exa_api_key,
};

/// Best-effort Web Backend status for `saku status`.
pub async fn fetch_web_backend_status(
    store: &CredentialStore,
    web_backend: &str,
) -> WebBackendStatus {
    let key = match store.read(web_backend) {
        Ok(Some(Credential::ApiKey { key })) => key,
        Ok(None) => return WebBackendStatus::NoCredential,
        Ok(Some(_)) => {
            return WebBackendStatus::Unavailable {
                reason: "Credential is not an API key".into(),
            };
        }
        Err(err) => {
            return WebBackendStatus::Unavailable {
                reason: err.to_string(),
            };
        }
    };
    if web_backend != exa::WEB_BACKEND_ID {
        return WebBackendStatus::Unavailable {
            reason: format!("status probe unsupported for `{web_backend}`"),
        };
    }
    match ExaClient::new(key).team_info().await {
        Ok(info) => WebBackendStatus::Connected {
            team_name: info.team_name,
        },
        Err(err) => WebBackendStatus::Unavailable {
            reason: short_exa_status_error(&err),
        },
    }
}

fn short_exa_status_error(err: &ExaError) -> String {
    match err {
        ExaError::Api { status, body } => {
            let body = body.trim();
            if body.is_empty() {
                format!("exa api error ({status})")
            } else {
                let truncated: String = body.chars().take(80).collect();
                if body.chars().count() > 80 {
                    format!("exa api error ({status}): {truncated}…")
                } else {
                    format!("exa api error ({status}): {truncated}")
                }
            }
        }
        other => other.to_string(),
    }
}
