use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

const ENV_FILE: &str = ".env";
const MANIFEST_FILE: &str = "manifest.json";
const BEGIN_MARKER: &str = "# BEGIN CODEXPILOT NETWORK PROXY";
const END_MARKER: &str = "# END CODEXPILOT NETWORK PROXY";
const DEFAULT_NO_PROXY: &str = "localhost,127.0.0.1,::1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkEnvStatus {
    pub exists: bool,
    pub configured: bool,
    pub managed: bool,
    pub backup_exists: bool,
    pub has_no_proxy: bool,
    pub proxy_endpoint: Option<String>,
    pub env_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BackupFileState {
    BackedUp,
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NetworkEnvBackupManifest {
    version: u32,
    created_at: i64,
    codex_home: String,
    env_file: BackupFileState,
}

pub fn default_network_env_backup_root() -> PathBuf {
    crate::accounts::data_root().join("network-env-backup")
}

pub fn load_network_env_status(
    codex_home: &Path,
    backup_root: &Path,
) -> anyhow::Result<NetworkEnvStatus> {
    let env_path = codex_home.join(ENV_FILE);
    let content = if env_path.exists() {
        Some(fs::read_to_string(&env_path)?)
    } else {
        None
    };
    Ok(status_from_content(
        &env_path,
        content.as_deref(),
        load_manifest(backup_root)?.is_some(),
    ))
}

pub fn apply_network_proxy_env(
    codex_home: &Path,
    backup_root: &Path,
    endpoint: &str,
) -> anyhow::Result<NetworkEnvStatus> {
    let endpoint = normalize_proxy_endpoint(endpoint)?;
    fs::create_dir_all(codex_home)?;
    create_backup_if_missing(codex_home, backup_root)?;

    let env_path = codex_home.join(ENV_FILE);
    let existing = if env_path.exists() {
        fs::read_to_string(&env_path)?
    } else {
        String::new()
    };
    let preserved = remove_managed_block(&existing);
    let next = append_managed_block(&preserved, &build_managed_block(&endpoint));
    fs::write(&env_path, next)?;

    load_network_env_status(codex_home, backup_root)
}

pub fn restore_network_proxy_env(
    codex_home: &Path,
    backup_root: &Path,
) -> anyhow::Result<NetworkEnvStatus> {
    let manifest = load_manifest(backup_root)?
        .ok_or_else(|| anyhow::anyhow!("network proxy environment backup has not been created"))?;
    fs::create_dir_all(codex_home)?;
    let env_path = codex_home.join(ENV_FILE);
    match manifest.env_file {
        BackupFileState::BackedUp => {
            fs::copy(backup_root.join("codex").join(ENV_FILE), &env_path)?;
        }
        BackupFileState::Missing if env_path.exists() => {
            fs::remove_file(&env_path)?;
        }
        BackupFileState::Missing => {}
    }
    load_network_env_status(codex_home, backup_root)
}

fn create_backup_if_missing(codex_home: &Path, backup_root: &Path) -> anyhow::Result<()> {
    if load_manifest(backup_root)?.is_some() {
        return Ok(());
    }

    fs::create_dir_all(backup_root.join("codex"))?;
    let env_path = codex_home.join(ENV_FILE);
    let env_file = if env_path.exists() {
        fs::copy(&env_path, backup_root.join("codex").join(ENV_FILE))?;
        BackupFileState::BackedUp
    } else {
        BackupFileState::Missing
    };
    let manifest = NetworkEnvBackupManifest {
        version: 1,
        created_at: current_unix_timestamp(),
        codex_home: codex_home.to_string_lossy().into_owned(),
        env_file,
    };
    fs::write(
        backup_root.join(MANIFEST_FILE),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(())
}

fn build_managed_block(endpoint: &str) -> String {
    format!(
        "{BEGIN_MARKER}\n\
HTTP_PROXY=http://{endpoint}\n\
HTTPS_PROXY=http://{endpoint}\n\
ALL_PROXY=socks5://{endpoint}\n\
NO_PROXY={DEFAULT_NO_PROXY}\n\
http_proxy=http://{endpoint}\n\
https_proxy=http://{endpoint}\n\
all_proxy=socks5://{endpoint}\n\
no_proxy={DEFAULT_NO_PROXY}\n\
{END_MARKER}\n"
    )
}

fn append_managed_block(preserved: &str, block: &str) -> String {
    let mut base = preserved.trim_end_matches('\n').to_string();
    if base.trim().is_empty() {
        return block.to_string();
    }
    base.push_str("\n\n");
    base.push_str(block);
    base
}

fn remove_managed_block(content: &str) -> String {
    let mut preserved = Vec::new();
    let mut in_managed_block = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == BEGIN_MARKER {
            in_managed_block = true;
            continue;
        }
        if trimmed == END_MARKER {
            in_managed_block = false;
            continue;
        }
        if !in_managed_block {
            preserved.push(line);
        }
    }
    let mut result = preserved.join("\n");
    if content.ends_with('\n') && !result.is_empty() {
        result.push('\n');
    }
    result
}

fn status_from_content(
    env_path: &Path,
    content: Option<&str>,
    backup_exists: bool,
) -> NetworkEnvStatus {
    let values = content.map(parse_env_values).unwrap_or_default();
    let managed = content
        .map(|text| text.contains(BEGIN_MARKER) && text.contains(END_MARKER))
        .unwrap_or(false);
    let http_proxy = first_env_value(
        &values,
        &["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"],
    );
    let all_proxy = first_env_value(&values, &["ALL_PROXY", "all_proxy"]);
    let no_proxy = first_env_value(&values, &["NO_PROXY", "no_proxy"]);
    let proxy_endpoint = http_proxy
        .or(all_proxy)
        .and_then(|value| normalize_proxy_endpoint(value).ok());
    let has_no_proxy = no_proxy
        .map(|value| {
            value.contains("localhost") && value.contains("127.0.0.1") && value.contains("::1")
        })
        .unwrap_or(false);
    NetworkEnvStatus {
        exists: content.is_some(),
        configured: managed && proxy_endpoint.is_some() && has_no_proxy,
        managed,
        backup_exists,
        has_no_proxy,
        proxy_endpoint,
        env_path: env_path.to_string_lossy().into_owned(),
    }
}

fn parse_env_values(content: &str) -> HashMap<String, String> {
    let mut values = HashMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        values.insert(
            key.trim().to_string(),
            value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string(),
        );
    }
    values
}

fn first_env_value<'a>(values: &'a HashMap<String, String>, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| values.get(*key).map(String::as_str))
}

fn normalize_proxy_endpoint(endpoint: &str) -> anyhow::Result<String> {
    let mut value = endpoint.trim();
    if value.is_empty() {
        return Err(anyhow::anyhow!("proxy endpoint is empty"));
    }
    let lower = value.to_ascii_lowercase();
    for prefix in ["http://", "https://", "socks5://"] {
        if lower.starts_with(prefix) {
            value = &value[prefix.len()..];
            break;
        }
    }
    value = value.split('/').next().unwrap_or(value).trim();
    if value.is_empty() || !value.contains(':') || value.contains(char::is_whitespace) {
        return Err(anyhow::anyhow!(
            "proxy endpoint must be host:port, for example 127.0.0.1:7890"
        ));
    }
    Ok(value.to_string())
}

fn load_manifest(backup_root: &Path) -> anyhow::Result<Option<NetworkEnvBackupManifest>> {
    let path = backup_root.join(MANIFEST_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read(path)?;
    Ok(Some(serde_json::from_slice(&raw)?))
}

fn current_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
