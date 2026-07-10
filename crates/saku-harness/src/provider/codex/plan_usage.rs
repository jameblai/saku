//! Codex Plan Usage and Reset Credits (undocumented ChatGPT backend endpoints).

use serde::Deserialize;
use serde_json::Value;

use crate::credentials::CredentialStore;
use crate::provider::ProviderError;
use crate::status::{CodexAccountStatus, PlanWindow};

use super::login::{chatgpt_account_id, ensure_fresh_access};

const CODEX_BASE: &str = "https://chatgpt.com/backend-api";

#[derive(Debug, Deserialize)]
struct UsagePayload {
    plan_type: Option<String>,
    rate_limit: Option<RateLimitBlock>,
    rate_limit_reset_credits: Option<ResetCreditsSummary>,
}

#[derive(Debug, Deserialize)]
struct RateLimitBlock {
    primary_window: Option<WindowPayload>,
    secondary_window: Option<WindowPayload>,
}

#[derive(Debug, Deserialize)]
struct WindowPayload {
    used_percent: Option<f64>,
    reset_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ResetCreditsSummary {
    available_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ResetCreditsList {
    credits: Option<Vec<ResetCreditItem>>,
    available_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ResetCreditItem {
    status: Option<String>,
    expires_at: Option<String>,
    #[serde(default)]
    expires_at_unix: Option<i64>,
}

/// Fetch live Codex Plan Usage + Reset Credits using the Credential store.
pub async fn fetch_codex_account_status(
    store: &CredentialStore,
    client: &reqwest::Client,
) -> Result<CodexAccountStatus, ProviderError> {
    let access = ensure_fresh_access(store)
        .await
        .map_err(|e| ProviderError::Message(e.to_string()))?;
    let account_id = chatgpt_account_id(&access).ok_or_else(|| {
        ProviderError::Message("access token missing chatgpt_account_id claim".into())
    })?;

    let usage_url = format!("{CODEX_BASE}/wham/usage");
    let usage_resp = client
        .get(&usage_url)
        .header("Authorization", format!("Bearer {access}"))
        .header("chatgpt-account-id", &account_id)
        .send()
        .await
        .map_err(|e| ProviderError::Message(e.to_string()))?;
    if !usage_resp.status().is_success() {
        let status = usage_resp.status();
        let text = usage_resp.text().await.unwrap_or_default();
        return Err(ProviderError::Message(format!(
            "codex usage error ({status}): {text}"
        )));
    }
    let usage_json: Value = usage_resp
        .json()
        .await
        .map_err(|e| ProviderError::Message(e.to_string()))?;
    let mut account = parse_usage_payload(&usage_json)?;

    // Prefer detailed credit list for soonest expiry when available.
    let credits_url = format!("{CODEX_BASE}/wham/rate-limit-reset-credits");
    if let Ok(credits_resp) = client
        .get(&credits_url)
        .header("Authorization", format!("Bearer {access}"))
        .header("chatgpt-account-id", &account_id)
        .send()
        .await
        && credits_resp.status().is_success()
        && let Ok(credits_json) = credits_resp.json::<Value>().await
    {
        apply_reset_credits_list(&mut account, &credits_json);
    }

    Ok(account)
}

/// Parse `/wham/usage` JSON into account status (used_percent → remaining).
pub fn parse_usage_payload(value: &Value) -> Result<CodexAccountStatus, ProviderError> {
    let payload: UsagePayload = serde_json::from_value(value.clone())
        .map_err(|e| ProviderError::Message(format!("usage parse: {e}")))?;

    let primary = payload
        .rate_limit
        .as_ref()
        .and_then(|r| r.primary_window.as_ref())
        .and_then(window_from_payload);
    let weekly = payload
        .rate_limit
        .as_ref()
        .and_then(|r| r.secondary_window.as_ref())
        .and_then(window_from_payload);

    let reset_credits_available = payload
        .rate_limit_reset_credits
        .as_ref()
        .and_then(|c| c.available_count);

    Ok(CodexAccountStatus {
        plan_type: payload.plan_type,
        primary,
        weekly,
        reset_credits_available,
        reset_credit_expires_at: None,
    })
}

fn window_from_payload(w: &WindowPayload) -> Option<PlanWindow> {
    let used = w.used_percent?;
    Some(PlanWindow {
        remaining_percent: (100.0 - used).clamp(0.0, 100.0),
        reset_at: w.reset_at,
    })
}

fn apply_reset_credits_list(account: &mut CodexAccountStatus, value: &Value) {
    let Ok(list) = serde_json::from_value::<ResetCreditsList>(value.clone()) else {
        return;
    };
    if let Some(n) = list.available_count {
        account.reset_credits_available = Some(n);
    }
    let mut soonest: Option<i64> = None;
    for credit in list.credits.unwrap_or_default() {
        let available = credit
            .status
            .as_deref()
            .is_none_or(|s| s.eq_ignore_ascii_case("available"));
        if !available {
            continue;
        }
        let exp = credit
            .expires_at_unix
            .or_else(|| parse_rfc3339_secs(credit.expires_at.as_deref()));
        if let Some(ts) = exp {
            soonest = Some(match soonest {
                Some(cur) => cur.min(ts),
                None => ts,
            });
        }
    }
    if soonest.is_some() {
        account.reset_credit_expires_at = soonest;
    }
}

fn parse_rfc3339_secs(s: Option<&str>) -> Option<i64> {
    let s = s?;
    // Accept `2026-07-12T01:33:14Z` / with offset by using time crate if present;
    // fall back to a minimal Z-suffix parser to avoid new deps.
    if let Some(stripped) = s.strip_suffix('Z') {
        // YYYY-MM-DDTHH:MM:SS
        let (date, time) = stripped.split_once('T')?;
        let mut d = date.split('-');
        let y: i64 = d.next()?.parse().ok()?;
        let mo: i64 = d.next()?.parse().ok()?;
        let day: i64 = d.next()?.parse().ok()?;
        let mut t = time.split(':');
        let h: i64 = t.next()?.parse().ok()?;
        let mi: i64 = t.next()?.parse().ok()?;
        let se: i64 = t.next()?.parse::<f64>().ok()? as i64;
        return Some(days_from_civil(y, mo, day) * 86400 + h * 3600 + mi * 60 + se);
    }
    None
}

/// Civil date → days since Unix epoch (Howard Hinnant algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Parse usage object from a Codex `response.completed` SSE payload.
pub fn parse_response_usage(value: &Value) -> Option<crate::types::TokenUsage> {
    let usage = value
        .get("response")
        .and_then(|r| r.get("usage"))
        .or_else(|| value.get("usage"))?;
    let input_tokens = usage
        .get("input_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let output_tokens = usage
        .get("output_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let details = usage.get("input_tokens_details");
    let cached = details
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cache_write = details
        .and_then(|d| d.get("cache_write_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let input = input_tokens
        .saturating_sub(cached)
        .saturating_sub(cache_write);
    Some(crate::types::TokenUsage {
        input,
        output: output_tokens,
        cache_read: cached,
        cache_write,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_usage_remaining_from_used_percent() {
        let payload = json!({
            "plan_type": "pro",
            "rate_limit": {
                "primary_window": { "used_percent": 4.0, "reset_at": 1_700_000_000 },
                "secondary_window": { "used_percent": 20.0, "reset_at": 1_700_100_000 }
            },
            "rate_limit_reset_credits": { "available_count": 2 }
        });
        let account = parse_usage_payload(&payload).unwrap();
        assert_eq!(account.plan_type.as_deref(), Some("pro"));
        assert_eq!(account.primary.unwrap().remaining_percent, 96.0);
        assert_eq!(account.weekly.unwrap().remaining_percent, 80.0);
        assert_eq!(account.reset_credits_available, Some(2));
    }

    #[test]
    fn parses_response_usage_splitting_cache() {
        let value = json!({
            "type": "response.completed",
            "response": {
                "usage": {
                    "input_tokens": 1200,
                    "output_tokens": 80,
                    "input_tokens_details": {
                        "cached_tokens": 200,
                        "cache_write_tokens": 50
                    }
                }
            }
        });
        let usage = parse_response_usage(&value).unwrap();
        assert_eq!(usage.input, 950);
        assert_eq!(usage.output, 80);
        assert_eq!(usage.cache_read, 200);
        assert_eq!(usage.cache_write, 50);
    }

    #[test]
    fn reset_credits_list_picks_soonest_expiry() {
        let mut account = CodexAccountStatus {
            plan_type: None,
            primary: None,
            weekly: None,
            reset_credits_available: Some(1),
            reset_credit_expires_at: None,
        };
        let list = json!({
            "available_count": 2,
            "credits": [
                { "status": "available", "expires_at": "2026-08-01T00:00:00Z" },
                { "status": "available", "expires_at": "2026-07-20T00:00:00Z" },
                { "status": "redeemed", "expires_at": "2026-07-10T00:00:00Z" }
            ]
        });
        apply_reset_credits_list(&mut account, &list);
        assert_eq!(account.reset_credits_available, Some(2));
        assert_eq!(
            account.reset_credit_expires_at,
            Some(days_from_civil(2026, 7, 20) * 86400)
        );
    }
}
