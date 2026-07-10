//! Web Backend identities and Login helpers (not LLM Providers).

pub mod exa;

use crate::credentials::{Credential, CredentialStore};
use crate::status::WebBackendStatus;

pub use exa::{
    ExaClient, ExaError, WEB_BACKEND_ID as EXA_WEB_BACKEND_ID, login_api_key as login_exa_api_key,
};

/// Web Backend status for `saku status`: Credential presence only (no live probe).
pub fn fetch_web_backend_status(store: &CredentialStore, web_backend: &str) -> WebBackendStatus {
    match store.read(web_backend) {
        Ok(Some(Credential::ApiKey { .. })) => WebBackendStatus::CredentialConfigured,
        Ok(None) => WebBackendStatus::NoCredential,
        Ok(Some(_)) => WebBackendStatus::Unavailable {
            reason: "Credential is not an API key".into(),
        },
        Err(err) => WebBackendStatus::Unavailable {
            reason: err.to_string(),
        },
    }
}
