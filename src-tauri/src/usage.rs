use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::accounts::{self, CodexActiveSource, ManagedCodexAccount};

const DEFAULT_USAGE_BASE_URL: &str = "https://chatgpt.com/backend-api";
const DEFAULT_USAGE_ENDPOINT: &str = "/wham/usage";
const REFRESH_TOKEN_ENDPOINT: &str = "https://auth.openai.com/oauth/token";
const REFRESH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexOAuthCredentials {
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: Option<String>,
    pub account_id: Option<String>,
    pub account_email: Option<String>,
    pub last_refresh: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexUsageSummary {
    pub session: Option<CodexUsageWindow>,
    pub weekly: Option<CodexUsageWindow>,
    pub plan: Option<String>,
    pub account_email: Option<String>,
    pub fetched_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexUsageWindow {
    pub used_percent: f64,
    pub remaining_percent: f64,
    pub reset_at: i64,
    pub limit_window_seconds: i64,
}

#[derive(Debug, Deserialize)]
struct CodexUsageResponse {
    #[serde(default)]
    plan_type: Option<String>,
    #[serde(default)]
    rate_limit: Option<CodexRateLimit>,
}

#[derive(Debug, Deserialize)]
struct CodexRateLimit {
    #[serde(default)]
    primary_window: Option<CodexRateWindow>,
    #[serde(default)]
    secondary_window: Option<CodexRateWindow>,
}

#[derive(Debug, Deserialize)]
struct CodexRateWindow {
    used_percent: f64,
    reset_at: i64,
    limit_window_seconds: i64,
}

#[derive(Debug, Deserialize)]
struct RefreshTokenResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

pub async fn load_usage_for_home(codex_home: &Path) -> Result<CodexUsageSummary, String> {
    let auth_path = codex_home.join("auth.json");
    let mut credentials = load_credentials(&auth_path)?;
    let client = reqwest::Client::builder()
        .user_agent("CodexPilot")
        .build()
        .map_err(|error| error.to_string())?;

    if credentials
        .last_refresh
        .as_deref()
        .is_some_and(needs_refresh)
    {
        credentials = refresh_credentials(&client, credentials, &auth_path).await?;
    }

    match fetch_usage(&client, &credentials).await {
        Ok(summary) => Ok(summary),
        Err(message) if message == "unauthorized" && !credentials.refresh_token.is_empty() => {
            let refreshed = refresh_credentials(&client, credentials, &auth_path).await?;
            fetch_usage(&client, &refreshed).await
        }
        Err(message) => Err(message),
    }
}

pub fn load_credentials(auth_path: &Path) -> Result<CodexOAuthCredentials, String> {
    let raw = fs::read(auth_path).map_err(|error| error.to_string())?;
    parse_credentials(&raw)
}

pub fn parse_credentials(raw: &[u8]) -> Result<CodexOAuthCredentials, String> {
    let json: Value = serde_json::from_slice(raw).map_err(|error| error.to_string())?;

    if let Some(api_key) = json.get("OPENAI_API_KEY").and_then(Value::as_str) {
        let trimmed = api_key.trim();
        if !trimmed.is_empty() {
            return Ok(CodexOAuthCredentials {
                access_token: trimmed.to_string(),
                refresh_token: String::new(),
                id_token: None,
                account_id: None,
                account_email: None,
                last_refresh: None,
            });
        }
    }

    let tokens = json
        .get("tokens")
        .and_then(Value::as_object)
        .ok_or_else(|| "Codex auth.json exists but contains no tokens".to_string())?;
    let access_token = tokens
        .get("access_token")
        .or_else(|| tokens.get("accessToken"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Codex auth.json exists but contains no access token".to_string())?;
    let refresh_token = tokens
        .get("refresh_token")
        .or_else(|| tokens.get("refreshToken"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let id_token = tokens
        .get("id_token")
        .or_else(|| tokens.get("idToken"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let account_id = tokens
        .get("account_id")
        .or_else(|| tokens.get("accountId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let account_email = id_token.as_deref().and_then(email_from_id_token);
    let last_refresh = json
        .get("last_refresh")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    Ok(CodexOAuthCredentials {
        access_token: access_token.to_string(),
        refresh_token: refresh_token.to_string(),
        id_token,
        account_id,
        account_email,
        last_refresh,
    })
}

pub fn parse_usage_response(
    raw: &[u8],
    credentials: &CodexOAuthCredentials,
) -> Result<CodexUsageSummary, String> {
    let response: CodexUsageResponse =
        serde_json::from_slice(raw).map_err(|error| error.to_string())?;
    Ok(map_usage_summary(response, credentials))
}

pub fn usage_home_for_source(
    active_source: &CodexActiveSource,
    stored_accounts: &[ManagedCodexAccount],
) -> Result<PathBuf, String> {
    match active_source {
        CodexActiveSource::LiveSystem => Ok(accounts::default_codex_home()),
        CodexActiveSource::ManagedAccount { id } => stored_accounts
            .iter()
            .find(|account| account.id == *id)
            .map(|account| PathBuf::from(&account.managed_home_path))
            .ok_or_else(|| "target managed account is missing".to_string()),
    }
}

pub fn usage_home_for_account_id(
    account_id: Option<&str>,
    active_source: &CodexActiveSource,
    stored_accounts: &[ManagedCodexAccount],
) -> Result<PathBuf, String> {
    match account_id {
        None => usage_home_for_source(active_source, stored_accounts),
        Some("live") => {
            if matches!(active_source, CodexActiveSource::ManagedAccount { .. }) {
                Ok(accounts::live_snapshot_home())
            } else {
                Ok(accounts::default_codex_home())
            }
        }
        Some(account_id) => stored_accounts
            .iter()
            .find(|account| account.id == account_id)
            .map(|account| PathBuf::from(&account.managed_home_path))
            .ok_or_else(|| "target managed account is missing".to_string()),
    }
}

pub fn usage_request_needs_managed_store(
    account_id: Option<&str>,
    active_source: &CodexActiveSource,
) -> bool {
    match account_id {
        Some("live") => false,
        Some(_) => true,
        None => matches!(active_source, CodexActiveSource::ManagedAccount { .. }),
    }
}

fn map_usage_summary(
    response: CodexUsageResponse,
    credentials: &CodexOAuthCredentials,
) -> CodexUsageSummary {
    CodexUsageSummary {
        session: response
            .rate_limit
            .as_ref()
            .and_then(|limits| limits.primary_window.as_ref())
            .map(map_window),
        weekly: response
            .rate_limit
            .as_ref()
            .and_then(|limits| limits.secondary_window.as_ref())
            .map(map_window),
        plan: response.plan_type,
        account_email: credentials.account_email.clone(),
        fetched_at: current_unix_timestamp(),
    }
}

fn map_window(window: &CodexRateWindow) -> CodexUsageWindow {
    let used_percent = window.used_percent.clamp(0.0, 100.0);
    CodexUsageWindow {
        used_percent,
        remaining_percent: (100.0 - used_percent).clamp(0.0, 100.0),
        reset_at: window.reset_at,
        limit_window_seconds: window.limit_window_seconds,
    }
}

async fn fetch_usage(
    client: &reqwest::Client,
    credentials: &CodexOAuthCredentials,
) -> Result<CodexUsageSummary, String> {
    let mut request = client
        .get(resolve_usage_url())
        .bearer_auth(&credentials.access_token)
        .header(reqwest::header::ACCEPT, "application/json");

    if let Some(account_id) = credentials.account_id.as_deref() {
        request = request.header("ChatGPT-Account-Id", account_id);
    }

    let response = request.send().await.map_err(|error| error.to_string())?;
    let status = response.status();
    let body = response.bytes().await.map_err(|error| error.to_string())?;

    if status.as_u16() == 401 {
        return Err("unauthorized".into());
    }
    if !status.is_success() {
        return Err(format!(
            "Codex usage API error {}: {}",
            status.as_u16(),
            String::from_utf8_lossy(&body)
        ));
    }

    parse_usage_response(&body, credentials)
}

async fn refresh_credentials(
    client: &reqwest::Client,
    credentials: CodexOAuthCredentials,
    auth_path: &Path,
) -> Result<CodexOAuthCredentials, String> {
    if credentials.refresh_token.is_empty() {
        return Ok(credentials);
    }

    let response = client
        .post(REFRESH_TOKEN_ENDPOINT)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&serde_json::json!({
            "client_id": REFRESH_CLIENT_ID,
            "grant_type": "refresh_token",
            "refresh_token": credentials.refresh_token,
            "scope": "openid profile email"
        }))
        .send()
        .await
        .map_err(|error| error.to_string())?;

    let status = response.status();
    let body = response.bytes().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        return Err(format!(
            "Codex token refresh failed with status {}",
            status.as_u16()
        ));
    }

    let payload: RefreshTokenResponse =
        serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    let id_token = payload.id_token.or(credentials.id_token);
    let refreshed = CodexOAuthCredentials {
        access_token: payload
            .access_token
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(credentials.access_token),
        refresh_token: payload
            .refresh_token
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(credentials.refresh_token),
        account_email: id_token.as_deref().and_then(email_from_id_token),
        id_token,
        account_id: credentials.account_id,
        last_refresh: Some(current_unix_timestamp().to_string()),
    };
    save_credentials(auth_path, &refreshed)?;
    Ok(refreshed)
}

fn save_credentials(auth_path: &Path, credentials: &CodexOAuthCredentials) -> Result<(), String> {
    let mut json = if auth_path.exists() {
        serde_json::from_slice::<Value>(&fs::read(auth_path).map_err(|error| error.to_string())?)
            .unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let object = json
        .as_object_mut()
        .ok_or_else(|| "Codex auth.json root must be a JSON object".to_string())?;
    object.insert(
        "tokens".into(),
        serde_json::json!({
            "access_token": credentials.access_token,
            "refresh_token": credentials.refresh_token,
            "id_token": credentials.id_token,
            "account_id": credentials.account_id
        }),
    );
    object.insert(
        "last_refresh".into(),
        Value::String(
            credentials
                .last_refresh
                .clone()
                .unwrap_or_else(|| current_unix_timestamp().to_string()),
        ),
    );

    if let Some(parent) = auth_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(
        auth_path,
        serde_json::to_vec_pretty(&json).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

fn resolve_usage_url() -> String {
    if let Ok(base_url) = std::env::var("CODEXPILOT_CODEX_USAGE_BASE_URL") {
        let trimmed = base_url.trim().trim_end_matches('/');
        if !trimmed.is_empty() {
            return format!("{trimmed}{DEFAULT_USAGE_ENDPOINT}");
        }
    }
    format!("{DEFAULT_USAGE_BASE_URL}{DEFAULT_USAGE_ENDPOINT}")
}

fn needs_refresh(last_refresh: &str) -> bool {
    last_refresh.parse::<u64>().map_or(false, |value| {
        let now = current_unix_timestamp() as u64;
        now.saturating_sub(value) > 8 * 24 * 60 * 60
    })
}

fn current_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn email_from_id_token(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let json: Value = serde_json::from_slice(&decoded).ok()?;
    json.get("email")
        .and_then(Value::as_str)
        .map(|email| email.trim().to_ascii_lowercase())
        .filter(|email| !email.is_empty())
}
