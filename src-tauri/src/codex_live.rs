use std::{
    fs,
    path::{Path, PathBuf},
};

use toml_edit::{DocumentMut, Item, Table, value};

const SWITCHER_PROVIDER_KEY: &str = "codex-api-switcher";
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
    let mut doc = DocumentMut::new();
    apply_takeover_config(&mut doc, port, provider_name, model);
    format!(
        "# Managed by Codex API Switcher. Existing config.toml is backed up before takeover.\n{}",
        doc
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
    let config_path = codex_dir.join("config.toml");
    let mut config = if config_path.exists() {
        fs::read_to_string(&config_path)?.parse::<DocumentMut>()?
    } else {
        DocumentMut::new()
    };
    apply_takeover_config(&mut config, port, provider_name, model);
    fs::write(config_path, config.to_string())?;
    Ok(())
}

fn apply_takeover_config(doc: &mut DocumentMut, port: u16, provider_name: &str, model: &str) {
    let proxy_base_url = format!("http://127.0.0.1:{port}/v1");
    let model_value = if model.trim().is_empty() {
        "deepseek-v4-flash"
    } else {
        model.trim()
    };
    let display_name = provider_display_name(provider_name, model_value);

    doc["model_provider"] = value(SWITCHER_PROVIDER_KEY);
    doc["model"] = value(model_value);
    doc["model_reasoning_effort"] = value("high");
    doc["disable_response_storage"] = value(true);

    if !doc.as_table().contains_key("model_providers") {
        doc["model_providers"] = Item::Table(Table::new());
    }
    if let Some(providers) = doc["model_providers"].as_table_like_mut() {
        providers.remove("OpenAI");
    }

    let mut provider = Table::new();
    provider["name"] = value(display_name);
    provider["base_url"] = value(proxy_base_url);
    provider["wire_api"] = value("responses");
    provider["requires_openai_auth"] = value(true);
    provider["experimental_bearer_token"] = value(PROXY_TOKEN_PLACEHOLDER);
    doc["model_providers"][SWITCHER_PROVIDER_KEY] = Item::Table(provider);
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

    if config_path.exists() {
        let current = fs::read_to_string(&config_path)?;
        if backup_path.exists() {
            if is_switcher_config(&current) {
                fs::copy(&backup_path, &config_path)?;
            }
            fs::remove_file(backup_path)?;
            return Ok(());
        }

        if is_switcher_config(&current) {
            fs::remove_file(config_path)?;
        }
    } else if backup_path.exists() {
        fs::copy(&backup_path, &config_path)?;
        fs::remove_file(backup_path)?;
    }

    Ok(())
}

fn backup_existing_config(codex_dir: &Path) -> anyhow::Result<()> {
    let config_path = codex_dir.join("config.toml");
    let backup_path = switcher_backup_path(codex_dir);

    if !config_path.exists() {
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
    let legacy = text.contains(r#"model_provider = "codex-api-switcher""#)
        && text.contains("[model_providers.codex-api-switcher]");
    let managed_openai = text.contains(r#"experimental_bearer_token = "PROXY_MANAGED""#)
        && text.contains("requires_openai_auth = true")
        && text.contains(r#"base_url = "http://127.0.0.1:"#);
    legacy || managed_openai
}
