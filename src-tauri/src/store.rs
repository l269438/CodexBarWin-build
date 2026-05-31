use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::DEFAULT_PROXY_PORT;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApiFormat {
    OpenAiChat,
    OpenAiResponses,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub api_format: ApiFormat,
}

impl Provider {
    pub fn deepseek_preset() -> Self {
        Self {
            id: "deepseek".to_string(),
            name: "DeepSeek".to_string(),
            base_url: "https://api.deepseek.com".to_string(),
            api_key: String::new(),
            model: "deepseek-v4-flash".to_string(),
            api_format: ApiFormat::OpenAiChat,
        }
    }

    pub fn with_generated_id(mut self) -> Self {
        if self.id.trim().is_empty() {
            self.id = Uuid::new_v4().to_string();
        }
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub providers: Vec<Provider>,
    pub current_provider_id: Option<String>,
    pub proxy_port: u16,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            current_provider_id: None,
            providers: Vec::new(),
            proxy_port: DEFAULT_PROXY_PORT,
        }
    }
}

impl AppConfig {
    pub fn normalize(&mut self) {
        if self.proxy_port == 0 {
            self.proxy_port = DEFAULT_PROXY_PORT;
        }
        let current_is_valid = self
            .current_provider_id
            .as_deref()
            .is_some_and(|id| self.providers.iter().any(|provider| provider.id == id));
        if !current_is_valid {
            self.current_provider_id = self.providers.first().map(|provider| provider.id.clone());
        }
    }

    pub fn current_provider(&self) -> Option<&Provider> {
        let id = self.current_provider_id.as_deref()?;
        self.providers.iter().find(|provider| provider.id == id)
    }

    pub fn upsert_provider(&mut self, provider: Provider) {
        let provider = provider.with_generated_id();
        if let Some(existing) = self
            .providers
            .iter_mut()
            .find(|existing| existing.id == provider.id)
        {
            *existing = provider;
        } else {
            if self.current_provider_id.is_none() {
                self.current_provider_id = Some(provider.id.clone());
            }
            self.providers.push(provider);
        }
    }
}

pub fn app_config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CODEX_API_SWITCHER_HOME") {
        return PathBuf::from(dir);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex-api-switcher")
}

pub fn config_path() -> PathBuf {
    app_config_dir().join("config.json")
}

pub fn load_config() -> anyhow::Result<AppConfig> {
    let path = config_path();
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    let text = fs::read_to_string(path)?;
    let mut config: AppConfig = serde_json::from_str(&text)?;
    config.normalize();
    Ok(config)
}

pub fn save_config(config: &AppConfig) -> anyhow::Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(config)?)?;
    Ok(())
}
