//! GitHub Copilot integration: OAuth device flow, token exchange, model fetching
//!
//! Auth flow (mirrors OpenCode's github-copilot plugin):
//! 1. Device code flow → get `gho_*` OAuth token
//! 2. Exchange `gho_*` for short-lived Copilot API token
//! 3. Use Copilot API token for /models and /chat/completions

use anyhow::{anyhow, Result};
use serde_json::Value;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

const GITHUB_CLIENT_ID: &str = "Ov23li8tweQw6odWQebz";
const DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";
pub const DEFAULT_COPILOT_API_BASE: &str = "https://api.githubcopilot.com";

/// Result of GitHub device code initiation
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u64,
}

/// Result of Copilot token exchange
#[derive(Clone, Debug)]
pub struct CopilotTokenInfo {
    pub token: String,
    pub base_url: String,
    pub expires_at: Instant,
}

// ─── Step 1: GitHub OAuth Device Flow ───

/// Initiate GitHub device code flow
pub async fn request_device_code() -> Result<DeviceCodeResponse> {
    let client = reqwest::Client::new();
    let resp = client
        .post(DEVICE_CODE_URL)
        .header("Accept", "application/json")
        .form(&[("client_id", GITHUB_CLIENT_ID)])
        .timeout(Duration::from_secs(10))
        .send()
        .await?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Device code request failed: {}", body));
    }

    let body: Value = resp.json().await?;

    Ok(DeviceCodeResponse {
        device_code: body["device_code"]
            .as_str()
            .ok_or_else(|| anyhow!("Missing device_code"))?
            .to_string(),
        user_code: body["user_code"]
            .as_str()
            .ok_or_else(|| anyhow!("Missing user_code"))?
            .to_string(),
        verification_uri: body["verification_uri"]
            .as_str()
            .ok_or_else(|| anyhow!("Missing verification_uri"))?
            .to_string(),
        interval: body["interval"].as_u64().unwrap_or(5),
    })
}

/// Poll for access token after user authorizes in browser
pub async fn poll_for_access_token(device_code: &str, interval: u64) -> Result<String> {
    let client = reqwest::Client::new();
    let poll_interval = Duration::from_secs(interval.max(5));

    loop {
        tokio::time::sleep(poll_interval).await;

        let resp = client
            .post(ACCESS_TOKEN_URL)
            .header("Accept", "application/json")
            .form(&[
                ("client_id", GITHUB_CLIENT_ID),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .timeout(Duration::from_secs(10))
            .send()
            .await?;

        let body: Value = resp.json().await?;

        if let Some(token) = body["access_token"].as_str() {
            return Ok(token.to_string());
        }

        match body["error"].as_str() {
            Some("authorization_pending") => {
                debug!("Waiting for user authorization...");
                continue;
            }
            Some("slow_down") => {
                debug!("Slowing down polling");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
            Some("expired_token") => {
                return Err(anyhow!("Device code expired. Please try again."));
            }
            Some("access_denied") => {
                return Err(anyhow!("Authorization denied by user."));
            }
            Some(err) => {
                return Err(anyhow!("OAuth error: {}", err));
            }
            None => continue,
        }
    }
}

// ─── Step 2: Copilot Token Exchange ───

/// Exchange GitHub OAuth token (gho_*) for Copilot API token
pub async fn exchange_copilot_token(github_token: &str) -> Result<CopilotTokenInfo> {
    let client = reqwest::Client::new();
    let resp = client
        .get(COPILOT_TOKEN_URL)
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {}", github_token))
        .header("User-Agent", "GithubCopilot/1.0")
        .timeout(Duration::from_secs(10))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Copilot token exchange returned HTTP {}: {}", status, body));
    }

    let body: Value = resp.json().await?;

    let token = body["token"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing 'token' in Copilot response"))?
        .to_string();

    let expires_at_unix = body["expires_at"].as_i64().unwrap_or(0);
    let expires_at = if expires_at_unix > 0 {
        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let secs_remaining = (expires_at_unix - now_unix).max(0) as u64;
        Instant::now() + Duration::from_secs(secs_remaining)
    } else {
        // Default: 30 minutes
        Instant::now() + Duration::from_secs(30 * 60)
    };

    let base_url = derive_copilot_base_url(&token)
        .unwrap_or_else(|| DEFAULT_COPILOT_API_BASE.to_string());

    info!("Copilot token exchanged, base_url: {}", base_url);

    Ok(CopilotTokenInfo {
        token,
        base_url,
        expires_at,
    })
}

/// Extract proxy-ep from Copilot token and convert to API base URL
fn derive_copilot_base_url(token: &str) -> Option<String> {
    let proxy_ep = token
        .split(';')
        .filter_map(|part| part.trim().strip_prefix("proxy-ep="))
        .next()?;

    let proxy_ep = proxy_ep.trim();
    if proxy_ep.is_empty() {
        return None;
    }

    let host = proxy_ep
        .trim_start_matches("https://")
        .trim_start_matches("http://");

    let host = if host.starts_with("proxy.") {
        host.replacen("proxy.", "api.", 1)
    } else {
        host.to_string()
    };

    if host.is_empty() {
        return None;
    }

    Some(format!("https://{}", host))
}

// ─── Step 3: Model Fetching ───

/// Fetch available models from Copilot API
pub async fn fetch_models(copilot_token: &str, base_url: &str) -> Result<Vec<String>> {
    let client = reqwest::Client::new();
    let url = format!("{}/models", base_url.trim_end_matches('/'));

    let resp = client
        .get(&url)
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {}", copilot_token))
        .header("Copilot-Integration-Id", "vscode-chat")
        .header("User-Agent", "GithubCopilot/1.0")
        .timeout(Duration::from_secs(10))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Copilot /models returned HTTP {}: {}", status, body));
    }

    let body: Value = resp.json().await?;

    // OpenAI format: { "data": [{ "id": "model-name", ... }, ...] }
    let models = body["data"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item["id"].as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(models)
}

// ─── Utility: Read OpenCode auth.json ───

/// Read GitHub Copilot OAuth token from OpenCode's auth.json
pub fn read_github_token_from_opencode() -> Option<String> {
    let auth_path = if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        std::path::PathBuf::from(xdg).join("opencode/auth.json")
    } else if let Some(home) = dirs::home_dir() {
        home.join(".local/share/opencode/auth.json")
    } else {
        return None;
    };

    let content = std::fs::read_to_string(&auth_path).ok()?;
    let auth: Value = serde_json::from_str(&content).ok()?;

    // Try "github-copilot" key
    if let Some(entry) = auth.get("github-copilot") {
        if let Some(access) = entry.get("access").and_then(|v| v.as_str()) {
            if !access.is_empty() {
                return Some(access.to_string());
            }
        }
        if let Some(key) = entry.get("key").and_then(|v| v.as_str()) {
            if !key.is_empty() {
                return Some(key.to_string());
            }
        }
    }

    None
}

/// Full setup: read or login → exchange → fetch models → return everything
pub async fn full_setup(github_token: &str) -> Result<(CopilotTokenInfo, Vec<String>)> {
    // Step 1: Exchange token
    let token_info = match exchange_copilot_token(github_token).await {
        Ok(info) => info,
        Err(e) => {
            warn!("Token exchange failed ({}), using gho token directly", e);
            CopilotTokenInfo {
                token: github_token.to_string(),
                base_url: DEFAULT_COPILOT_API_BASE.to_string(),
                expires_at: Instant::now() + Duration::from_secs(60 * 60),
            }
        }
    };

    // Step 2: Fetch models
    let models = fetch_models(&token_info.token, &token_info.base_url).await?;

    info!("GitHub Copilot setup complete: {} models, base_url: {}", models.len(), token_info.base_url);

    Ok((token_info, models))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_copilot_base_url() {
        let token = "tid=abc;exp=123;proxy-ep=proxy.individual.githubcopilot.com;st=dotcom";
        assert_eq!(
            derive_copilot_base_url(token),
            Some("https://api.individual.githubcopilot.com".to_string())
        );
    }

    #[test]
    fn test_derive_copilot_base_url_no_proxy() {
        let token = "tid=abc;exp=123;st=dotcom";
        assert_eq!(derive_copilot_base_url(token), None);
    }

    #[test]
    fn test_derive_copilot_base_url_with_https() {
        let token = "proxy-ep=https://proxy.example.com";
        assert_eq!(
            derive_copilot_base_url(token),
            Some("https://api.example.com".to_string())
        );
    }

    #[test]
    fn test_derive_copilot_base_url_non_proxy_host() {
        let token = "proxy-ep=api.githubcopilot.com";
        assert_eq!(
            derive_copilot_base_url(token),
            Some("https://api.githubcopilot.com".to_string())
        );
    }
}
