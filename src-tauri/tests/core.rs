use codex_api_switcher::{
    AppConfig,
    accounts::{
        CodexActiveSource, CodexHomeTransaction, ManagedCodexAccount, ManagedCodexAccountSet,
        ObservedSystemCodexAccount, default_managed_homes_root_for, discover_account_in_home,
        resolve_codex_binary_from_path,
    },
    codex_live::{
        build_takeover_config, restore_original_config, switcher_backup_path, write_takeover_config,
    },
    history::ConversationHistoryStore,
    proxy::{chat_completions_url, responses_url},
    store::{ApiFormat, Provider},
    transform::{CodexChatReasoning, chat_completion_to_response, responses_to_chat_completions},
    usage::{
        CodexOAuthCredentials, parse_credentials, parse_usage_response, usage_home_for_account_id,
        usage_home_for_source, usage_request_needs_managed_store,
    },
};
use serde_json::json;

#[test]
fn takeover_config_points_codex_to_local_responses_proxy() {
    let config = build_takeover_config(15721, "DeepSeek", "deepseek-v4-flash");

    assert!(config.contains("base_url = \"http://127.0.0.1:15721/v1\""));
    assert!(config.contains("wire_api = \"responses\""));
    assert!(config.contains("experimental_bearer_token = \"PROXY_MANAGED\""));
    assert!(config.contains("model = \"deepseek-v4-flash\""));
    assert!(config.contains("name = \"DeepSeek · deepseek-v4-flash\""));
}

#[test]
fn empty_provider_config_stays_empty() {
    let mut config = AppConfig {
        providers: vec![],
        current_provider_id: None,
        proxy_port: 15721,
    };

    config.normalize();

    assert!(config.providers.is_empty());
    assert_eq!(config.current_provider_id, None);
}

#[test]
fn invalid_current_provider_normalizes_to_first_provider() {
    let provider = Provider::deepseek_preset();
    let mut config = AppConfig {
        providers: vec![provider],
        current_provider_id: Some("missing".to_string()),
        proxy_port: 15721,
    };

    config.normalize();

    assert_eq!(config.current_provider_id.as_deref(), Some("deepseek"));
}

#[test]
fn api_format_supports_direct_responses_providers() {
    let text = r#"{
        "id": "responses",
        "name": "Responses Provider",
        "baseUrl": "https://api.example.com",
        "apiKey": "key",
        "model": "gpt-like",
        "apiFormat": "open_ai_responses"
    }"#;

    let provider: Provider = serde_json::from_str(text).unwrap();

    assert_eq!(provider.api_format, ApiFormat::OpenAiResponses);
}

#[test]
fn provider_endpoint_builders_only_append_missing_api_path() {
    assert_eq!(
        chat_completions_url("https://api.example.com"),
        "https://api.example.com/v1/chat/completions"
    );
    assert_eq!(
        chat_completions_url("https://api.example.com/v1"),
        "https://api.example.com/v1/chat/completions"
    );
    assert_eq!(
        chat_completions_url("https://api.example.com/v1/chat/completions"),
        "https://api.example.com/v1/chat/completions"
    );
    assert_eq!(
        responses_url("https://api.example.com"),
        "https://api.example.com/v1/responses"
    );
    assert_eq!(
        responses_url("https://api.example.com/v1"),
        "https://api.example.com/v1/responses"
    );
    assert_eq!(
        responses_url("https://api.example.com/v1/responses"),
        "https://api.example.com/v1/responses"
    );
}

#[test]
fn account_switch_replaces_auth_but_preserves_provider_config() {
    let temp = tempfile::tempdir().unwrap();
    let live_home = temp.path().join(".codex");
    let snapshot_home = temp.path().join("live-system-home");
    let managed_home = temp.path().join("managed-home");
    std::fs::create_dir_all(&live_home).unwrap();
    std::fs::create_dir_all(&managed_home).unwrap();
    std::fs::write(
        live_home.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"id_token":"live"}}"#,
    )
    .unwrap();
    std::fs::write(
        live_home.join("config.toml"),
        "model_provider = \"codex-api-switcher\"\n",
    )
    .unwrap();
    std::fs::write(
        managed_home.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"id_token":"managed"}}"#,
    )
    .unwrap();
    std::fs::write(
        managed_home.join("config.toml"),
        "model = \"official-managed\"\n",
    )
    .unwrap();

    let transaction = CodexHomeTransaction::new(live_home.clone(), snapshot_home);
    transaction
        .switch(
            &CodexActiveSource::LiveSystem,
            &CodexActiveSource::ManagedAccount {
                id: "managed-1".to_string(),
            },
            Some(&managed_home),
        )
        .unwrap();

    let auth = std::fs::read_to_string(live_home.join("auth.json")).unwrap();
    let config = std::fs::read_to_string(live_home.join("config.toml")).unwrap();
    assert!(auth.contains("managed"));
    assert_eq!(config, "model_provider = \"codex-api-switcher\"\n");
}

#[test]
fn account_switch_can_restore_live_system_auth_without_touching_config() {
    let temp = tempfile::tempdir().unwrap();
    let live_home = temp.path().join(".codex");
    let snapshot_home = temp.path().join("live-system-home");
    let managed_home = temp.path().join("managed-home");
    std::fs::create_dir_all(&live_home).unwrap();
    std::fs::create_dir_all(&managed_home).unwrap();
    std::fs::write(
        live_home.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"id_token":"live"}}"#,
    )
    .unwrap();
    std::fs::write(
        live_home.join("config.toml"),
        "model_provider = \"codex-api-switcher\"\n",
    )
    .unwrap();
    std::fs::write(
        managed_home.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"id_token":"managed"}}"#,
    )
    .unwrap();

    let transaction = CodexHomeTransaction::new(live_home.clone(), snapshot_home);
    transaction
        .switch(
            &CodexActiveSource::LiveSystem,
            &CodexActiveSource::ManagedAccount {
                id: "managed-1".to_string(),
            },
            Some(&managed_home),
        )
        .unwrap();
    transaction
        .switch(
            &CodexActiveSource::ManagedAccount {
                id: "managed-1".to_string(),
            },
            &CodexActiveSource::LiveSystem,
            None,
        )
        .unwrap();

    let auth = std::fs::read_to_string(live_home.join("auth.json")).unwrap();
    let config = std::fs::read_to_string(live_home.join("config.toml")).unwrap();
    assert!(auth.contains("live"));
    assert_eq!(config, "model_provider = \"codex-api-switcher\"\n");
}

#[test]
fn managed_account_set_deduplicates_same_identity() {
    let live = ObservedSystemCodexAccount {
        email: "User@Example.com".to_string(),
        workspace_label: None,
        workspace_account_id: Some("acct_1".to_string()),
        auth_fingerprint: Some("fp1".to_string()),
        codex_home_path: "/tmp/live".to_string(),
    };
    let first = ManagedCodexAccount::from_live_account(
        &live,
        default_managed_homes_root_for(tempfile::tempdir().unwrap().path())
            .join("one")
            .to_string_lossy()
            .into_owned(),
    );
    let second = ManagedCodexAccount::from_live_account(&live, "/tmp/another".to_string());

    let set = ManagedCodexAccountSet::new(vec![first, second]);

    assert_eq!(set.accounts.len(), 1);
    assert_eq!(set.accounts[0].email, "user@example.com");
}

#[test]
fn discover_account_in_home_reads_chatgpt_auth() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join(".codex");
    std::fs::create_dir_all(&home).unwrap();
    let token = test_id_token("user@example.com", "acct_1");
    std::fs::write(
        home.join("auth.json"),
        format!(r#"{{"auth_mode":"chatgpt","tokens":{{"id_token":"{token}"}}}}"#),
    )
    .unwrap();

    let account = discover_account_in_home(&home).unwrap();

    assert_eq!(account.email, "user@example.com");
    assert_eq!(account.workspace_account_id.as_deref(), Some("acct_1"));
}

#[test]
fn codex_binary_resolution_finds_nvm_install_when_desktop_path_is_minimal() {
    let temp = tempfile::tempdir().unwrap();
    let codex_path = temp
        .path()
        .join(".nvm")
        .join("versions")
        .join("node")
        .join("v22.22.3")
        .join("bin")
        .join("codex");
    std::fs::create_dir_all(codex_path.parent().unwrap()).unwrap();
    std::fs::write(&codex_path, "#!/bin/sh\n").unwrap();

    let resolved = resolve_codex_binary_from_path(Some("/usr/bin:/bin"), Some(temp.path()));

    assert_eq!(resolved.as_deref(), Some(codex_path.as_path()));
}

#[test]
fn usage_credentials_parse_tokens_from_auth_json() {
    let token = test_id_token("user@example.com", "acct_123");
    let credentials = parse_credentials(
        format!(
            r#"{{
                "last_refresh":"1717000000",
                "tokens":{{
                    "access_token":"access",
                    "refresh_token":"refresh",
                    "id_token":"{token}",
                    "account_id":"acct_123"
                }}
            }}"#
        )
        .as_bytes(),
    )
    .unwrap();

    assert_eq!(credentials.access_token, "access");
    assert_eq!(credentials.refresh_token, "refresh");
    assert_eq!(credentials.account_id.as_deref(), Some("acct_123"));
    assert_eq!(credentials.account_email.as_deref(), Some("user@example.com"));
}

#[test]
fn usage_response_maps_session_and_weekly_windows() {
    let credentials = CodexOAuthCredentials {
        access_token: "access".into(),
        refresh_token: "refresh".into(),
        id_token: Some(test_id_token("user@example.com", "acct_123")),
        account_id: Some("acct_123".into()),
        account_email: Some("user@example.com".into()),
        last_refresh: None,
    };

    let summary = parse_usage_response(
        br#"{
            "plan_type":"pro",
            "rate_limit":{
                "primary_window":{
                    "used_percent":61,
                    "reset_at":1717000100,
                    "limit_window_seconds":18000
                },
                "secondary_window":{
                    "used_percent":94,
                    "reset_at":1717600000,
                    "limit_window_seconds":604800
                }
            }
        }"#,
        &credentials,
    )
    .unwrap();

    assert_eq!(summary.plan.as_deref(), Some("pro"));
    assert_eq!(summary.account_email.as_deref(), Some("user@example.com"));
    assert_eq!(
        summary.session.as_ref().map(|window| window.used_percent),
        Some(61.0)
    );
    assert_eq!(
        summary.weekly.as_ref().map(|window| window.remaining_percent),
        Some(6.0)
    );
}

#[test]
fn usage_home_resolution_can_preview_accounts_without_switching() {
    let managed = ManagedCodexAccount {
        id: "managed-1".into(),
        email: "managed@example.com".into(),
        workspace_label: None,
        workspace_account_id: Some("acct_managed".into()),
        auth_fingerprint: None,
        managed_home_path: "/tmp/managed-preview".into(),
        created_at: 0,
        updated_at: 0,
        last_authenticated_at: Some(0),
    };

    let active_home = usage_home_for_source(
        &CodexActiveSource::ManagedAccount {
            id: managed.id.clone(),
        },
        std::slice::from_ref(&managed),
    )
    .unwrap();
    let preview_home = usage_home_for_account_id(
        Some(&managed.id),
        &CodexActiveSource::LiveSystem,
        std::slice::from_ref(&managed),
    )
    .unwrap();

    assert_eq!(active_home, std::path::Path::new("/tmp/managed-preview"));
    assert_eq!(preview_home, std::path::Path::new("/tmp/managed-preview"));
    assert!(!usage_request_needs_managed_store(
        Some("live"),
        &CodexActiveSource::ManagedAccount { id: managed.id }
    ));
    assert!(usage_request_needs_managed_store(
        Some("managed-1"),
        &CodexActiveSource::LiveSystem
    ));
}

fn test_id_token(email: &str, account_id: &str) -> String {
    use base64::Engine;
    let payload = json!({
        "email": email,
        "https://api.openai.com/auth": {
            "chatgpt_account_id": account_id
        }
    });
    format!(
        "header.{}.sig",
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap())
    )
}

#[test]
fn writing_takeover_config_preserves_existing_auth_json() {
    let temp = tempfile::tempdir().unwrap();
    let codex_dir = temp.path().join(".codex");
    std::fs::create_dir_all(&codex_dir).unwrap();
    let auth_path = codex_dir.join("auth.json");
    std::fs::write(&auth_path, r#"{"tokens":{"id_token":"keep-me"}}"#).unwrap();

    write_takeover_config(&codex_dir, 15721, "DeepSeek", "deepseek-v4-flash").unwrap();

    let auth = std::fs::read_to_string(auth_path).unwrap();
    assert_eq!(auth, r#"{"tokens":{"id_token":"keep-me"}}"#);
    let config = std::fs::read_to_string(codex_dir.join("config.toml")).unwrap();
    assert!(config.contains("http://127.0.0.1:15721/v1"));
}

#[test]
fn writing_takeover_config_backs_up_existing_config_once() {
    let temp = tempfile::tempdir().unwrap();
    let codex_dir = temp.path().join(".codex");
    std::fs::create_dir_all(&codex_dir).unwrap();
    let config_path = codex_dir.join("config.toml");
    let backup_path = switcher_backup_path(&codex_dir);
    std::fs::write(&config_path, "model = \"gpt-5\"\n").unwrap();

    write_takeover_config(&codex_dir, 15721, "DeepSeek", "deepseek-v4-flash").unwrap();
    write_takeover_config(&codex_dir, 15721, "XIAOMI", "another-model").unwrap();

    let backup = std::fs::read_to_string(backup_path).unwrap();
    assert_eq!(backup, "model = \"gpt-5\"\n");
    let current = std::fs::read_to_string(config_path).unwrap();
    assert!(current.contains("model = \"another-model\""));
    assert!(current.contains("name = \"XIAOMI · another-model\""));
}

#[test]
fn restore_original_config_puts_back_backup_and_removes_backup_file() {
    let temp = tempfile::tempdir().unwrap();
    let codex_dir = temp.path().join(".codex");
    std::fs::create_dir_all(&codex_dir).unwrap();
    let config_path = codex_dir.join("config.toml");
    let backup_path = switcher_backup_path(&codex_dir);
    std::fs::write(&config_path, "model = \"gpt-5\"\n").unwrap();

    write_takeover_config(&codex_dir, 15721, "DeepSeek", "deepseek-v4-flash").unwrap();
    restore_original_config(&codex_dir).unwrap();

    let restored = std::fs::read_to_string(config_path).unwrap();
    assert_eq!(restored, "model = \"gpt-5\"\n");
    assert!(!backup_path.exists());
}

#[test]
fn responses_request_maps_to_deepseek_chat_request() {
    let input = json!({
        "model": "placeholder-client-model",
        "instructions": "You are Codex.",
        "input": [{
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "hello"}]
        }],
        "reasoning": {"effort": "high"},
        "max_output_tokens": 128,
        "stream": true,
        "tools": [{
            "type": "function",
            "name": "read_file",
            "description": "Read a file",
            "parameters": {"type": "object"}
        }]
    });

    let result = responses_to_chat_completions(
        input,
        Some("deepseek-v4-flash"),
        Some(&CodexChatReasoning::deepseek()),
    )
    .unwrap();

    assert_eq!(result["model"], "deepseek-v4-flash");
    assert_eq!(result["messages"][0]["role"], "system");
    assert_eq!(result["messages"][1]["role"], "user");
    assert_eq!(result["messages"][1]["content"], "hello");
    assert_eq!(result["max_tokens"], 128);
    assert_eq!(result["thinking"]["type"], "enabled");
    assert_eq!(result["reasoning_effort"], "high");
    assert_eq!(result["stream_options"]["include_usage"], true);
    assert_eq!(result["tools"][0]["function"]["name"], "read_file");
}

#[test]
fn chat_response_maps_back_to_responses_shape_with_reasoning() {
    let input = json!({
        "id": "chatcmpl_123",
        "created": 123,
        "model": "deepseek-v4-flash",
        "choices": [{
            "finish_reason": "stop",
            "message": {
                "role": "assistant",
                "reasoning_content": "Need answer.",
                "content": "Done"
            }
        }],
        "usage": {"prompt_tokens": 4, "completion_tokens": 2, "total_tokens": 6}
    });

    let result = chat_completion_to_response(input).unwrap();

    assert_eq!(result["object"], "response");
    assert_eq!(result["status"], "completed");
    assert_eq!(result["output"][0]["type"], "reasoning");
    assert_eq!(result["output"][0]["summary"][0]["text"], "Need answer.");
    assert_eq!(result["output"][1]["type"], "message");
    assert_eq!(result["output"][1]["content"][0]["text"], "Done");
    assert_eq!(result["usage"]["input_tokens"], 4);
    assert_eq!(result["usage"]["output_tokens"], 2);
}

#[tokio::test]
async fn previous_response_id_restores_cached_chat_transcript() {
    let history = ConversationHistoryStore::default();
    let first_request = vec![
        json!({"role": "system", "content": "You are Codex."}),
        json!({"role": "user", "content": "hello"}),
    ];
    let first_response = json!({
        "id": "chatcmpl_1",
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "hi"
            }
        }]
    });

    history
        .record_chat_response(first_request, &first_response)
        .await
        .unwrap();

    let responses_request = json!({
        "previous_response_id": "resp_chatcmpl_1",
        "instructions": "You are Codex.",
        "input": [{
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "continue"}]
        }]
    });
    let mut chat_request =
        responses_to_chat_completions(responses_request.clone(), Some("deepseek-v4-flash"), None)
            .unwrap();

    assert_eq!(
        history
            .enrich_chat_request(&responses_request, &mut chat_request)
            .await,
        2
    );
    assert_eq!(
        chat_request["messages"],
        json!([
            {"role": "system", "content": "You are Codex."},
            {"role": "user", "content": "hello"},
            {"role": "assistant", "content": "hi"},
            {"role": "user", "content": "continue"}
        ])
    );
}

#[tokio::test]
async fn previous_response_id_restores_tool_call_before_tool_output() {
    let history = ConversationHistoryStore::default();
    let first_request = vec![json!({"role": "user", "content": "read file"})];
    let first_response = json!({
        "id": "chatcmpl_tool",
        "choices": [{
            "message": {
                "role": "assistant",
                "content": null,
                "reasoning_content": "Need file contents.",
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {"name": "read_file", "arguments": "{\"path\":\"README.md\"}"}
                }]
            }
        }]
    });

    history
        .record_chat_response(first_request, &first_response)
        .await
        .unwrap();

    let responses_request = json!({
        "previous_response_id": "resp_chatcmpl_tool",
        "input": [{
            "type": "function_call_output",
            "call_id": "call_1",
            "output": "file text"
        }]
    });
    let mut chat_request =
        responses_to_chat_completions(responses_request.clone(), Some("deepseek-v4-flash"), None)
            .unwrap();

    history
        .enrich_chat_request(&responses_request, &mut chat_request)
        .await;

    assert_eq!(chat_request["messages"][0]["role"], "user");
    assert_eq!(chat_request["messages"][1]["role"], "assistant");
    assert_eq!(chat_request["messages"][1]["tool_calls"][0]["id"], "call_1");
    assert_eq!(chat_request["messages"][2]["role"], "tool");
    assert_eq!(chat_request["messages"][2]["tool_call_id"], "call_1");
}

#[tokio::test]
async fn streamed_response_id_can_be_used_by_next_request() {
    let history = ConversationHistoryStore::default();
    history
        .record_stream_response(
            "resp_stream_1",
            vec![json!({"role": "user", "content": "hello"})],
            "streamed hi",
            "short reasoning",
        )
        .await
        .unwrap();

    let responses_request = json!({
        "previous_response_id": "resp_stream_1",
        "input": [{
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "continue"}]
        }]
    });
    let mut chat_request =
        responses_to_chat_completions(responses_request.clone(), Some("deepseek-v4-flash"), None)
            .unwrap();

    history
        .enrich_chat_request(&responses_request, &mut chat_request)
        .await;

    assert_eq!(
        chat_request["messages"],
        json!([
            {"role": "user", "content": "hello"},
            {"role": "assistant", "content": "streamed hi"},
            {"role": "user", "content": "continue"}
        ])
    );
}
