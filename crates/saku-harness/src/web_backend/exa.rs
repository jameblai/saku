//! Exa Web Backend: API-key Credential Login + REST `/search` and `/contents`.

use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

use crate::credentials::{Credential, CredentialError, CredentialStore};

/// Credential / config id for the Exa Web Backend.
pub const WEB_BACKEND_ID: &str = "exa";

const DEFAULT_BASE_URL: &str = "https://api.exa.ai";

/// Persist an Exa API-key Credential, overwriting any existing entry.
pub fn login_api_key(store: &CredentialStore, key: &str) -> Result<(), CredentialError> {
    store.set(
        WEB_BACKEND_ID,
        Credential::ApiKey {
            key: key.to_string(),
        },
    )
}

#[derive(Debug, Error)]
pub enum ExaError {
    #[error("exa http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("exa api error ({status}): {body}")]
    Api { status: u16, body: String },
    #[error("exa: {0}")]
    Message(String),
}

/// HTTP client for Exa's official REST API.
#[derive(Clone)]
pub struct ExaClient {
    http: Client,
    base_url: String,
    api_key: String,
}

impl ExaClient {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            http: Client::new(),
            base_url: DEFAULT_BASE_URL.to_string(),
            api_key: api_key.into(),
        }
    }

    /// Override the API base URL (tests inject a fake HTTP backend).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// `POST /search` — title/url/snippet hits (highlights as snippets, not full bodies).
    pub async fn search(
        &self,
        query: &str,
        max_results: u32,
        depth: &str,
    ) -> Result<String, ExaError> {
        let body = json!({
            "query": query,
            "numResults": max_results,
            "type": depth,
            "contents": { "highlights": true }
        });
        let response = self
            .http
            .post(format!("{}/search", self.base_url.trim_end_matches('/')))
            .header("x-api-key", &self.api_key)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(ExaError::Api {
                status: status.as_u16(),
                body: text,
            });
        }
        let parsed: SearchResponse = serde_json::from_str(&text)
            .map_err(|e| ExaError::Message(format!("invalid search response: {e}")))?;
        Ok(format_search_hits(&parsed.results))
    }

    /// `POST /contents` — page text only for a single URL.
    pub async fn extract(&self, url: &str) -> Result<String, ExaError> {
        let body = json!({
            "urls": [url],
            "text": true
        });
        let response = self
            .http
            .post(format!("{}/contents", self.base_url.trim_end_matches('/')))
            .header("x-api-key", &self.api_key)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(ExaError::Api {
                status: status.as_u16(),
                body: text,
            });
        }
        let parsed: ContentsResponse = serde_json::from_str(&text)
            .map_err(|e| ExaError::Message(format!("invalid contents response: {e}")))?;
        let Some(result) = parsed.results.into_iter().next() else {
            return Err(ExaError::Message("no contents returned for url".into()));
        };
        Ok(result.text.unwrap_or_default())
    }
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    results: Vec<SearchHit>,
}

#[derive(Debug, Deserialize)]
struct SearchHit {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    highlights: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct ContentsResponse {
    results: Vec<ContentsHit>,
}

#[derive(Debug, Deserialize)]
struct ContentsHit {
    #[serde(default)]
    text: Option<String>,
}

fn format_search_hits(hits: &[SearchHit]) -> String {
    if hits.is_empty() {
        return "(no results)".to_string();
    }
    let mut out = String::new();
    for (i, hit) in hits.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let title = hit.title.as_deref().unwrap_or("(untitled)");
        let url = hit.url.as_deref().unwrap_or("");
        let snippet = hit
            .highlights
            .as_ref()
            .map(|h| h.join(" "))
            .unwrap_or_default();
        out.push_str(&format!(
            "{}. {title}\n   URL: {url}\n   Snippet: {snippet}",
            i + 1
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn login_api_key_persists_and_overwrites() {
        let tmp = TempDir::new().unwrap();
        let store = CredentialStore::open(tmp.path()).unwrap();

        login_api_key(&store, "key-one").expect("first login");
        assert_eq!(
            store.read(WEB_BACKEND_ID).unwrap(),
            Some(Credential::ApiKey {
                key: "key-one".into()
            })
        );

        login_api_key(&store, "key-two").expect("re-login");
        assert_eq!(
            store.read(WEB_BACKEND_ID).unwrap(),
            Some(Credential::ApiKey {
                key: "key-two".into()
            })
        );
    }
}
