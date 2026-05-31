use std::{
    fs,
    path::{Path, PathBuf},
};

const PROXY_TOKEN_PLACEHOLDER: &str = "PROXY_MANAGED";

pub fn default_codex_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CODEX_HOME") {
        return PathBuf::from(dir);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
}

pub fn build_takeover_config(port: u16, provider_name: &str, model: &str) -> String {
    let proxy_base_url = format!("http://127.0.0.1:{port}/v1");
    let model_value = if model.trim().is_empty() {
        "deepseek-v4-flash"
    } else {
        model.trim()
    };
    let display_name = provider_display_name(provider_name, model_value);
    let model = toml_string(model_value);
    let display_name = toml_string(&display_name);
    let proxy_base_url = toml_string(&proxy_base_url);
    let token = toml_string(PROXY_TOKEN_PLACEHOLDER);

    format!(
        r#"# Managed by Codex API Switcher. Existing config.toml is backed up before takeover.
model_provider = "codex-api-switcher"
model = {model}
model_reasoning_effort = "high"
disable_response_storage = true

[model_providers.codex-api-switcher]
name = {display_name}
base_url = {proxy_base_url}
wire_api = "responses"
requires_openai_auth = true
experimental_bearer_token = {token}
"#
    )
}

pub fn switcher_backup_path(codex_dir: &Path) -> PathBuf {
    codex_dir.join("config.toml.codex-api-switcher.bak")
}

pub fn write_takeover_config(
    codex_dir: &Path,
    port: u16,
    provider_name: &str,
    model: &str,
) -> anyhow::Result<()> {
    fs::create_dir_all(codex_dir)?;
    backup_existing_config(codex_dir)?;
    let config = build_takeover_config(port, provider_name, model);
    config.parse::<toml_edit::DocumentMut>()?;
    fs::write(codex_dir.join("config.toml"), config)?;
    Ok(())
}

fn provider_display_name(provider_name: &str, model: &str) -> String {
    let provider_name = provider_name.trim();
    if provider_name.is_empty() {
        model.to_string()
    } else {
        format!("{provider_name} · {model}")
    }
}

pub fn restore_original_config(codex_dir: &Path) -> anyhow::Result<()> {
    let config_path = codex_dir.join("config.toml");
    let backup_path = switcher_backup_path(codex_dir);

    if backup_path.exists() {
        fs::copy(&backup_path, &config_path)?;
        fs::remove_file(backup_path)?;
        return Ok(());
    }

    if config_path.exists() {
        let current = fs::read_to_string(&config_path)?;
        if is_switcher_config(&current) {
            fs::remove_file(config_path)?;
        }
    }

    Ok(())
}

fn backup_existing_config(codex_dir: &Path) -> anyhow::Result<()> {
    let config_path = codex_dir.join("config.toml");
    let backup_path = switcher_backup_path(codex_dir);

    if !config_path.exists() || backup_path.exists() {
        return Ok(());
    }

    let current = fs::read_to_string(&config_path)?;
    if is_switcher_config(&current) {
        return Ok(());
    }

    fs::copy(config_path, backup_path)?;
    Ok(())
}

fn is_switcher_config(text: &str) -> bool {
    text.contains(r#"model_provider = "codex-api-switcher""#)
        && text.contains("[model_providers.codex-api-switcher]")
}

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}
