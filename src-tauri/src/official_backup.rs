use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

const MANIFEST_FILE: &str = "manifest.json";
const AUTH_FILE: &str = "auth.json";
const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OriginalBackupStatus {
    pub exists: bool,
    pub created_at: Option<i64>,
    pub auth_json_backed_up: bool,
    pub config_toml_backed_up: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BackupFileState {
    BackedUp,
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OriginalBackupManifest {
    version: u32,
    created_at: i64,
    codex_home: String,
    auth_json: BackupFileState,
    config_toml: BackupFileState,
}

pub fn default_original_backup_root() -> PathBuf {
    crate::accounts::data_root().join("original-codex-backup")
}

pub fn load_original_backup_status(backup_root: &Path) -> anyhow::Result<OriginalBackupStatus> {
    let manifest = load_manifest(backup_root)?;
    Ok(status_from_manifest(manifest.as_ref()))
}

pub fn create_original_backup(
    codex_home: &Path,
    backup_root: &Path,
) -> anyhow::Result<OriginalBackupStatus> {
    if let Some(existing) = load_manifest(backup_root)? {
        return Ok(status_from_manifest(Some(&existing)));
    }

    fs::create_dir_all(backup_root.join("codex"))?;
    let auth_json = backup_file(codex_home, backup_root, AUTH_FILE)?;
    let config_toml = backup_file(codex_home, backup_root, CONFIG_FILE)?;
    let manifest = OriginalBackupManifest {
        version: 1,
        created_at: current_unix_timestamp(),
        codex_home: codex_home.to_string_lossy().into_owned(),
        auth_json,
        config_toml,
    };
    fs::write(
        backup_root.join(MANIFEST_FILE),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(status_from_manifest(Some(&manifest)))
}

pub fn restore_original_backup(
    codex_home: &Path,
    backup_root: &Path,
) -> anyhow::Result<OriginalBackupStatus> {
    let manifest = load_manifest(backup_root)?
        .ok_or_else(|| anyhow::anyhow!("original Codex backup has not been created"))?;
    fs::create_dir_all(codex_home)?;
    restore_file(codex_home, backup_root, AUTH_FILE, &manifest.auth_json)?;
    restore_file(codex_home, backup_root, CONFIG_FILE, &manifest.config_toml)?;
    Ok(status_from_manifest(Some(&manifest)))
}

fn backup_file(
    codex_home: &Path,
    backup_root: &Path,
    file_name: &str,
) -> anyhow::Result<BackupFileState> {
    let source = codex_home.join(file_name);
    if !source.exists() {
        return Ok(BackupFileState::Missing);
    }
    fs::copy(source, backup_root.join("codex").join(file_name))?;
    Ok(BackupFileState::BackedUp)
}

fn restore_file(
    codex_home: &Path,
    backup_root: &Path,
    file_name: &str,
    state: &BackupFileState,
) -> anyhow::Result<()> {
    let target = codex_home.join(file_name);
    match state {
        BackupFileState::BackedUp => {
            fs::copy(backup_root.join("codex").join(file_name), target)?;
        }
        BackupFileState::Missing if target.exists() => {
            fs::remove_file(target)?;
        }
        BackupFileState::Missing => {}
    }
    Ok(())
}

fn load_manifest(backup_root: &Path) -> anyhow::Result<Option<OriginalBackupManifest>> {
    let path = backup_root.join(MANIFEST_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read(path)?;
    Ok(Some(serde_json::from_slice(&raw)?))
}

fn status_from_manifest(manifest: Option<&OriginalBackupManifest>) -> OriginalBackupStatus {
    let Some(manifest) = manifest else {
        return OriginalBackupStatus {
            exists: false,
            created_at: None,
            auth_json_backed_up: false,
            config_toml_backed_up: false,
        };
    };
    OriginalBackupStatus {
        exists: true,
        created_at: Some(manifest.created_at),
        auth_json_backed_up: matches!(manifest.auth_json, BackupFileState::BackedUp),
        config_toml_backed_up: matches!(manifest.config_toml, BackupFileState::BackedUp),
    }
}

fn current_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
