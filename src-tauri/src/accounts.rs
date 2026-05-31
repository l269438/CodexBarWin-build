use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

const AUTH_FILE: &str = "auth.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CodexActiveSource {
    LiveSystem,
    ManagedAccount { id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedCodexAccount {
    pub id: String,
    pub email: String,
    pub workspace_label: Option<String>,
    pub workspace_account_id: Option<String>,
    pub auth_fingerprint: Option<String>,
    pub managed_home_path: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_authenticated_at: Option<i64>,
}

impl ManagedCodexAccount {
    pub fn from_live_account(
        account: &ObservedSystemCodexAccount,
        managed_home_path: String,
    ) -> Self {
        let now = current_unix_timestamp();
        Self {
            id: Uuid::new_v4().to_string(),
            email: normalize_email(&account.email),
            workspace_label: account.workspace_label.clone(),
            workspace_account_id: account.workspace_account_id.clone(),
            auth_fingerprint: account.auth_fingerprint.clone(),
            managed_home_path,
            created_at: now,
            updated_at: now,
            last_authenticated_at: Some(now),
        }
    }

    pub fn matches_live_account(&self, account: &ObservedSystemCodexAccount) -> bool {
        self.email == normalize_email(&account.email)
            && self.workspace_account_id == account.workspace_account_id
    }

    pub fn refresh_from_live_account(&mut self, account: &ObservedSystemCodexAccount) {
        let now = current_unix_timestamp();
        self.email = normalize_email(&account.email);
        self.workspace_label = account.workspace_label.clone();
        self.workspace_account_id = account.workspace_account_id.clone();
        self.auth_fingerprint = account.auth_fingerprint.clone();
        self.updated_at = now;
        self.last_authenticated_at = Some(now);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedSystemCodexAccount {
    pub email: String,
    pub workspace_label: Option<String>,
    pub workspace_account_id: Option<String>,
    pub auth_fingerprint: Option<String>,
    pub codex_home_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisibleCodexAccount {
    pub id: String,
    pub email: String,
    pub workspace_label: Option<String>,
    pub workspace_account_id: Option<String>,
    pub selection_source: CodexActiveSource,
    pub is_active: bool,
    pub is_live: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexVisibleAccountProjection {
    pub accounts: Vec<VisibleCodexAccount>,
    pub has_unreadable_store: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedCodexAccountSet {
    pub version: u32,
    pub accounts: Vec<ManagedCodexAccount>,
}

impl ManagedCodexAccountSet {
    pub fn new(accounts: Vec<ManagedCodexAccount>) -> Self {
        let mut sanitized = Vec::new();
        for account in accounts {
            let duplicate = sanitized.iter().any(|existing: &ManagedCodexAccount| {
                existing.email == account.email
                    && existing.workspace_account_id == account.workspace_account_id
            });
            if !duplicate {
                sanitized.push(account);
            }
        }
        Self {
            version: 1,
            accounts: sanitized,
        }
    }
}

pub struct FileManagedCodexAccountStore {
    path: PathBuf,
}

impl FileManagedCodexAccountStore {
    pub fn default() -> Self {
        Self {
            path: default_managed_accounts_path(),
        }
    }

    pub fn load_accounts(&self) -> Result<ManagedCodexAccountSet, String> {
        if !self.path.exists() {
            return Ok(ManagedCodexAccountSet::new(vec![]));
        }
        let raw = fs::read(&self.path).map_err(|error| error.to_string())?;
        let decoded: ManagedCodexAccountSet =
            serde_json::from_slice(&raw).map_err(|error| error.to_string())?;
        Ok(ManagedCodexAccountSet::new(decoded.accounts))
    }

    pub fn save_accounts(&self, set: &ManagedCodexAccountSet) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let payload = serde_json::to_vec_pretty(set).map_err(|error| error.to_string())?;
        fs::write(&self.path, payload).map_err(|error| error.to_string())
    }
}

pub struct FileActiveSourceStore {
    path: PathBuf,
}

impl FileActiveSourceStore {
    pub fn default() -> Self {
        Self {
            path: data_root().join("active-source.json"),
        }
    }

    pub fn load(&self) -> Result<CodexActiveSource, String> {
        if !self.path.exists() {
            return Ok(CodexActiveSource::LiveSystem);
        }
        let raw = fs::read(&self.path).map_err(|error| error.to_string())?;
        serde_json::from_slice(&raw).map_err(|error| error.to_string())
    }

    pub fn save(&self, source: &CodexActiveSource) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let payload = serde_json::to_vec_pretty(source).map_err(|error| error.to_string())?;
        fs::write(&self.path, payload).map_err(|error| error.to_string())
    }
}

pub struct CodexHomeTransaction {
    live_home: PathBuf,
    live_snapshot_home: PathBuf,
}

impl CodexHomeTransaction {
    pub fn new(live_home: PathBuf, live_snapshot_home: PathBuf) -> Self {
        Self {
            live_home,
            live_snapshot_home,
        }
    }

    pub fn default() -> Self {
        Self::new(default_codex_home(), live_snapshot_home())
    }

    pub fn switch(
        &self,
        current_source: &CodexActiveSource,
        target_source: &CodexActiveSource,
        target_managed_home: Option<&Path>,
    ) -> Result<(), String> {
        if current_source == target_source {
            return Ok(());
        }
        if matches!(current_source, CodexActiveSource::LiveSystem)
            && matches!(target_source, CodexActiveSource::ManagedAccount { .. })
        {
            self.sync_auth(&self.live_home, &self.live_snapshot_home)?;
        }

        let checkpoint = self.read_live_auth()?;
        let result = match target_source {
            CodexActiveSource::LiveSystem => self.restore_live_auth(),
            CodexActiveSource::ManagedAccount { .. } => {
                let managed_home = target_managed_home
                    .ok_or_else(|| "target managed home is missing".to_string())?;
                self.apply_managed_auth(managed_home)
            }
        };
        if let Err(error) = result {
            self.restore_checkpoint(checkpoint);
            return Err(error);
        }
        Ok(())
    }

    fn apply_managed_auth(&self, managed_home: &Path) -> Result<(), String> {
        if !managed_home.join(AUTH_FILE).exists() {
            return Err("target managed home is missing auth.json".to_string());
        }
        self.sync_auth(managed_home, &self.live_home)
    }

    fn restore_live_auth(&self) -> Result<(), String> {
        if !self.live_snapshot_home.join(AUTH_FILE).exists() {
            return Err("live system snapshot is missing auth.json".to_string());
        }
        self.sync_auth(&self.live_snapshot_home, &self.live_home)
    }

    fn sync_auth(&self, source_home: &Path, target_home: &Path) -> Result<(), String> {
        fs::create_dir_all(target_home).map_err(|error| error.to_string())?;
        let source = source_home.join(AUTH_FILE);
        let target = target_home.join(AUTH_FILE);
        if source.exists() {
            fs::copy(source, target).map_err(|error| error.to_string())?;
        } else if target.exists() {
            fs::remove_file(target).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn read_live_auth(&self) -> Result<Option<Vec<u8>>, String> {
        let path = self.live_home.join(AUTH_FILE);
        if path.exists() {
            fs::read(path).map(Some).map_err(|error| error.to_string())
        } else {
            Ok(None)
        }
    }

    fn restore_checkpoint(&self, checkpoint: Option<Vec<u8>>) {
        let path = self.live_home.join(AUTH_FILE);
        match checkpoint {
            Some(bytes) => {
                let _ = fs::write(path, bytes);
            }
            None if path.exists() => {
                let _ = fs::remove_file(path);
            }
            None => {}
        }
    }
}

pub struct ManagedCodexLoginRunner;

impl ManagedCodexLoginRunner {
    pub fn run(home_path: &Path, timeout: Duration) -> Result<String, String> {
        let mut child = spawn_codex_login(home_path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "Codex CLI not found. Install the Codex CLI and try again.".to_string()
            } else {
                error.to_string()
            }
        })?;
        let start = std::time::Instant::now();
        loop {
            match child.try_wait().map_err(|error| error.to_string())? {
                Some(status) if status.success() => return Ok("Codex login completed".to_string()),
                Some(status) => {
                    return Err(format!(
                        "Codex login exited with status {}",
                        status.code().unwrap_or(-1)
                    ));
                }
                None if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(200)),
                None => {
                    let _ = child.kill();
                    return Err(
                        "Codex login timed out. Finish browser sign-in, then retry.".to_string()
                    );
                }
            }
        }
    }
}

pub fn load_projection(
    active_source: CodexActiveSource,
    stored_accounts: Vec<ManagedCodexAccount>,
    has_unreadable_store: bool,
) -> CodexVisibleAccountProjection {
    let active_source = sanitize_active_source(active_source, &stored_accounts);
    let live_account = match &active_source {
        CodexActiveSource::LiveSystem => discover_live_account(),
        CodexActiveSource::ManagedAccount { .. } => discover_account_in_home(&live_snapshot_home()),
    };

    let mut accounts = Vec::new();
    if let Some(live_account) = live_account {
        accounts.push(VisibleCodexAccount {
            id: "live".to_string(),
            email: live_account.email,
            workspace_label: live_account.workspace_label,
            workspace_account_id: live_account.workspace_account_id,
            selection_source: CodexActiveSource::LiveSystem,
            is_active: active_source == CodexActiveSource::LiveSystem,
            is_live: true,
        });
    }
    for account in stored_accounts {
        accounts.push(VisibleCodexAccount {
            id: account.id.clone(),
            email: account.email,
            workspace_label: account.workspace_label,
            workspace_account_id: account.workspace_account_id,
            selection_source: CodexActiveSource::ManagedAccount {
                id: account.id.clone(),
            },
            is_active: active_source
                == CodexActiveSource::ManagedAccount {
                    id: account.id.clone(),
                },
            is_live: false,
        });
    }
    CodexVisibleAccountProjection {
        accounts,
        has_unreadable_store,
    }
}

pub fn sanitize_active_source(
    source: CodexActiveSource,
    accounts: &[ManagedCodexAccount],
) -> CodexActiveSource {
    match source {
        CodexActiveSource::ManagedAccount { ref id }
            if accounts.iter().any(|account| &account.id == id) =>
        {
            source
        }
        CodexActiveSource::ManagedAccount { .. } => CodexActiveSource::LiveSystem,
        CodexActiveSource::LiveSystem => CodexActiveSource::LiveSystem,
    }
}

pub fn discover_live_account() -> Option<ObservedSystemCodexAccount> {
    discover_account_in_home(&default_codex_home())
}

pub fn discover_account_in_home(codex_home: &Path) -> Option<ObservedSystemCodexAccount> {
    let raw = fs::read(codex_home.join(AUTH_FILE)).ok()?;
    let json: Value = serde_json::from_slice(&raw).ok()?;
    if json
        .get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .is_some_and(|key| !key.trim().is_empty())
    {
        return None;
    }
    let id_token = json
        .get("tokens")?
        .get("id_token")
        .or_else(|| json.get("tokens")?.get("idToken"))
        .and_then(Value::as_str)?;
    let payload = parse_id_token(id_token)?;
    Some(ObservedSystemCodexAccount {
        email: payload.email?,
        workspace_label: None,
        workspace_account_id: payload.account_id,
        auth_fingerprint: None,
        codex_home_path: codex_home.to_string_lossy().into_owned(),
    })
}

pub fn default_codex_home() -> PathBuf {
    if let Ok(dir) = std::env::var("CODEX_HOME") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
}

pub fn data_root() -> PathBuf {
    if let Ok(dir) = std::env::var("CODEX_API_SWITCHER_HOME") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex-api-switcher")
}

pub fn default_managed_accounts_path() -> PathBuf {
    data_root().join("managed-codex-accounts.json")
}

pub fn default_managed_homes_root() -> PathBuf {
    default_managed_homes_root_for(&data_root())
}

pub fn default_managed_homes_root_for(root: &Path) -> PathBuf {
    root.join("managed-codex-homes")
}

pub fn live_snapshot_home() -> PathBuf {
    data_root().join("live-system-home")
}

pub fn sync_auth_file(source_home: &Path, target_home: &Path) -> Result<(), String> {
    fs::create_dir_all(target_home).map_err(|error| error.to_string())?;
    let source = source_home.join(AUTH_FILE);
    if !source.exists() {
        return Err("source Codex home is missing auth.json".to_string());
    }
    fs::copy(source, target_home.join(AUTH_FILE)).map_err(|error| error.to_string())?;
    Ok(())
}

fn spawn_codex_login(home_path: &Path) -> Result<std::process::Child, std::io::Error> {
    Command::new("codex")
        .arg("login")
        .env("CODEX_HOME", home_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

#[derive(Debug, Clone)]
struct IdTokenPayload {
    email: Option<String>,
    account_id: Option<String>,
}

fn parse_id_token(id_token: &str) -> Option<IdTokenPayload> {
    let payload = id_token.split('.').nth(1)?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let json: Value = serde_json::from_slice(&decoded).ok()?;
    let email = json
        .get("email")
        .and_then(Value::as_str)
        .map(normalize_email)
        .filter(|value| !value.is_empty());
    let account_id = json
        .get("https://api.openai.com/auth")
        .and_then(|value| value.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    Some(IdTokenPayload { email, account_id })
}

fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

fn current_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
