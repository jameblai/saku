//! Codex device-code OAuth Login (pi-compatible endpoints).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rand::Rng;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::credentials::{Credential, CredentialStore};

use super::models::PROVIDER_ID;

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTH_BASE: &str = "https://auth.openai.com";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const DEVICE_USER_CODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
const DEVICE_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
const DEVICE_VERIFICATION_URI: &str = "https://auth.openai.com/codex/device";
const DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
const DEVICE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Error)]
pub enum LoginError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Credentials(#[from] crate::credentials::CredentialError),
}

#[derive(Debug, Clone)]
pub struct DeviceCodeInfo {
    pub user_code: String,
    pub verification_uri: String,
    pub interval_seconds: u64,
}

/// Callbacks while Login runs (CLI prints these).
pub trait LoginNotify: Send {
    fn device_code(&mut self, info: &DeviceCodeInfo);
    fn progress(&mut self, message: &str);
}

#[derive(Deserialize)]
struct UserCodeResponse {
    device_auth_id: String,
    user_code: String,
    #[serde(default)]
    interval: serde_json::Value,
}

#[derive(Deserialize)]
struct DeviceTokenResponse {
    authorization_code: String,
    code_verifier: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
}

/// Run device-code Login and store the oauth Credential under provider id `codex`.
pub async fn login_device_code(
    store: &CredentialStore,
    notify: &mut dyn LoginNotify,
) -> Result<Credential, LoginError> {
    let client = reqwest::Client::new();
    let start: UserCodeResponse = client
        .post(DEVICE_USER_CODE_URL)
        .json(&serde_json::json!({ "client_id": CLIENT_ID }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let interval = match &start.interval {
        serde_json::Value::Number(n) => n.as_u64().unwrap_or(5),
        serde_json::Value::String(s) => s.trim().parse().unwrap_or(5),
        _ => 5,
    };

    notify.device_code(&DeviceCodeInfo {
        user_code: start.user_code.clone(),
        verification_uri: DEVICE_VERIFICATION_URI.into(),
        interval_seconds: interval,
    });

    let deadline = tokio::time::Instant::now() + DEVICE_TIMEOUT;
    let mut sleep_for = Duration::from_secs(interval.max(1));

    let device_token = loop {
        if tokio::time::Instant::now() > deadline {
            return Err(LoginError::Message("device code login timed out".into()));
        }
        tokio::time::sleep(sleep_for).await;
        notify.progress("polling for authorization…");

        let response = client
            .post(DEVICE_TOKEN_URL)
            .json(&serde_json::json!({
                "device_auth_id": start.device_auth_id,
                "user_code": start.user_code,
            }))
            .send()
            .await?;

        if response.status().is_success() {
            let body: DeviceTokenResponse = response.json().await?;
            break body;
        }

        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if status.as_u16() == 403 || status.as_u16() == 404 {
            continue;
        }
        if text.contains("deviceauth_authorization_pending") {
            continue;
        }
        if text.contains("slow_down") {
            sleep_for = sleep_for.saturating_mul(2);
            continue;
        }
        return Err(LoginError::Message(format!(
            "device auth failed ({status}): {text}"
        )));
    };

    notify.progress("exchanging authorization code…");
    let token = exchange_code(
        &client,
        &device_token.authorization_code,
        &device_token.code_verifier,
    )
    .await?;

    let credential = Credential::OAuth {
        access: token.access_token,
        refresh: token.refresh_token,
        expires: now_ms() + token.expires_in * 1000,
    };
    store.set(PROVIDER_ID, credential.clone())?;
    Ok(credential)
}

async fn exchange_code(
    client: &reqwest::Client,
    code: &str,
    verifier: &str,
) -> Result<TokenResponse, LoginError> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "authorization_code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("code", code)
        .append_pair("code_verifier", verifier)
        .append_pair("redirect_uri", DEVICE_REDIRECT_URI)
        .finish();
    let response = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await?
        .error_for_status()?;
    Ok(response.json().await?)
}

/// Refresh an expired oauth Credential; write is serialized via CredentialStore::modify.
pub async fn refresh_oauth_tokens(store: &CredentialStore) -> Result<Credential, LoginError> {
    let current = store
        .read(PROVIDER_ID)?
        .ok_or_else(|| LoginError::Message("no codex credential; run saku login codex".into()))?;
    let Credential::OAuth { refresh, .. } = current else {
        return Err(LoginError::Message("codex credential is not oauth".into()));
    };

    let client = reqwest::Client::new();
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "refresh_token")
        .append_pair("refresh_token", &refresh)
        .append_pair("client_id", CLIENT_ID)
        .finish();
    let response = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await?
        .error_for_status()?;
    let token: TokenResponse = response.json().await?;
    let credential = Credential::OAuth {
        access: token.access_token,
        refresh: token.refresh_token,
        expires: now_ms() + token.expires_in * 1000,
    };

    // Serialize write under lock so concurrent refresh cannot race.
    store.modify(PROVIDER_ID, |_| Ok(Some(credential.clone())))?;
    Ok(credential)
}

pub async fn ensure_fresh_access(store: &CredentialStore) -> Result<String, LoginError> {
    let current = store
        .read(PROVIDER_ID)?
        .ok_or_else(|| LoginError::Message("no codex credential; run saku login codex".into()))?;
    match current {
        Credential::OAuth {
            access, expires, ..
        } => {
            let skew_ms = 60_000;
            if now_ms() + skew_ms < expires {
                return Ok(access);
            }
            let refreshed = refresh_oauth_tokens(store).await?;
            match refreshed {
                Credential::OAuth { access, .. } => Ok(access),
                _ => Err(LoginError::Message("unexpected credential type".into())),
            }
        }
        Credential::ApiKey { key } => Ok(key),
    }
}

pub fn chatgpt_account_id(access_token: &str) -> Option<String> {
    let payload = access_token.split('.').nth(1)?;
    let decoded = base64_url_decode(payload).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    json.get("https://api.openai.com/auth")
        .and_then(|v| v.get("chatgpt_account_id"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn base64_url_decode(input: &str) -> Result<Vec<u8>, ()> {
    let mut s = input.replace('-', "+").replace('_', "/");
    while s.len() % 4 != 0 {
        s.push('=');
    }
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, s).map_err(|_| ())
}

/// PKCE helpers kept for potential browser login later.
#[allow(dead_code)]
fn generate_pkce() -> (String, String) {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let verifier = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes);
    let challenge = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        Sha256::digest(verifier.as_bytes()),
    );
    (verifier, challenge)
}

#[allow(dead_code)]
fn _auth_base() -> &'static str {
    AUTH_BASE
}
