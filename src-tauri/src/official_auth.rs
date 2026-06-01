use std::{fs, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const AUTH_FILE: &str = "auth.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatGptAuthStatus {
    pub has_auth_file: bool,
    pub auth_mode_chatgpt: bool,
    pub api_key_disabled: bool,
    pub has_tokens: bool,
    pub compatible: bool,
    pub can_repair: bool,
}

pub fn load_chatgpt_auth_status(codex_home: &Path) -> anyhow::Result<ChatGptAuthStatus> {
    let Some(auth) = load_auth_object(codex_home)? else {
        return Ok(status_from_object(None));
    };
    Ok(status_from_object(Some(&auth)))
}

pub fn ensure_chatgpt_auth_mode(codex_home: &Path) -> anyhow::Result<ChatGptAuthStatus> {
    let Some(mut auth) = load_auth_object(codex_home)? else {
        return Ok(status_from_object(None));
    };

    let status = status_from_object(Some(&auth));
    if status.can_repair {
        auth.insert(
            "auth_mode".to_string(),
            Value::String("chatgpt".to_string()),
        );
        auth.insert("OPENAI_API_KEY".to_string(), Value::Null);
        fs::write(
            codex_home.join(AUTH_FILE),
            serde_json::to_vec_pretty(&Value::Object(auth))?,
        )?;
        return load_chatgpt_auth_status(codex_home);
    }

    Ok(status)
}

fn load_auth_object(codex_home: &Path) -> anyhow::Result<Option<Map<String, Value>>> {
    let path = codex_home.join(AUTH_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let Ok(raw) = fs::read(path) else {
        return Ok(Some(Map::new()));
    };
    let Ok(value) = serde_json::from_slice::<Value>(&raw) else {
        return Ok(Some(Map::new()));
    };
    Ok(Some(value.as_object().cloned().unwrap_or_default()))
}

fn status_from_object(auth: Option<&Map<String, Value>>) -> ChatGptAuthStatus {
    let Some(auth) = auth else {
        return ChatGptAuthStatus {
            has_auth_file: false,
            auth_mode_chatgpt: false,
            api_key_disabled: false,
            has_tokens: false,
            compatible: false,
            can_repair: false,
        };
    };

    let auth_mode_chatgpt = auth
        .get("auth_mode")
        .and_then(Value::as_str)
        .is_some_and(|mode| mode == "chatgpt");
    let api_key_disabled = auth.get("OPENAI_API_KEY").is_none_or(Value::is_null);
    let has_tokens = auth
        .get("tokens")
        .and_then(Value::as_object)
        .is_some_and(|tokens| !tokens.is_empty());
    let compatible = auth_mode_chatgpt && api_key_disabled && has_tokens;
    let can_repair = has_tokens && !compatible;

    ChatGptAuthStatus {
        has_auth_file: true,
        auth_mode_chatgpt,
        api_key_disabled,
        has_tokens,
        compatible,
        can_repair,
    }
}
