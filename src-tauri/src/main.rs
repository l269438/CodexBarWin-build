use codex_api_switcher::{
    AppConfig, AppState, ProxyStatus, accounts::CodexVisibleAccountProjection, store::Provider,
    usage::CodexUsageSummary,
};
use std::process::Command;
use tauri::State;

#[tauri::command]
async fn get_app_config(state: State<'_, AppState>) -> Result<AppConfig, String> {
    Ok(state.get_app_config().await)
}

#[tauri::command]
async fn save_provider(
    state: State<'_, AppState>,
    provider: Provider,
) -> Result<AppConfig, String> {
    state.save_provider(provider).await
}

#[tauri::command]
async fn delete_provider(state: State<'_, AppState>, id: String) -> Result<AppConfig, String> {
    state.delete_provider(id).await
}

#[tauri::command]
async fn switch_provider(state: State<'_, AppState>, id: String) -> Result<AppConfig, String> {
    state.switch_provider(id).await
}

#[tauri::command]
async fn write_codex_takeover(state: State<'_, AppState>) -> Result<(), String> {
    state.write_codex_takeover().await
}

#[tauri::command]
async fn start_proxy(state: State<'_, AppState>) -> Result<ProxyStatus, String> {
    state.start_proxy().await
}

#[tauri::command]
async fn stop_proxy(state: State<'_, AppState>) -> Result<ProxyStatus, String> {
    state.stop_proxy().await
}

#[tauri::command]
async fn get_proxy_status(state: State<'_, AppState>) -> Result<ProxyStatus, String> {
    Ok(state.get_proxy_status().await)
}

#[tauri::command]
async fn load_account_projection(
    state: State<'_, AppState>,
) -> Result<CodexVisibleAccountProjection, String> {
    Ok(state.load_account_projection().await)
}

#[tauri::command]
async fn load_account_usage(
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<CodexUsageSummary, String> {
    state.load_account_usage(account_id).await
}

#[tauri::command]
async fn import_current_account(
    state: State<'_, AppState>,
) -> Result<CodexVisibleAccountProjection, String> {
    state.import_current_account().await
}

#[tauri::command]
async fn add_managed_account(
    state: State<'_, AppState>,
) -> Result<CodexVisibleAccountProjection, String> {
    state.add_managed_account().await
}

#[tauri::command]
async fn switch_account(state: State<'_, AppState>, account_id: String) -> Result<(), String> {
    state.switch_account(account_id).await
}

#[tauri::command]
async fn remove_managed_account(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<CodexVisibleAccountProjection, String> {
    state.remove_managed_account(account_id).await
}

#[tauri::command]
async fn refresh_managed_account(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<CodexVisibleAccountProjection, String> {
    state.refresh_managed_account(account_id).await
}

#[tauri::command]
fn open_codex_home() -> Result<(), String> {
    let path = dirs::home_dir()
        .ok_or_else(|| "home directory not found".to_string())?
        .join(".codex");
    std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(&path);
        command
    };

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("explorer");
        command.arg(&path);
        command
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(&path);
        command
    };

    command.spawn().map_err(|error| error.to_string())?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::load())
        .invoke_handler(tauri::generate_handler![
            get_app_config,
            save_provider,
            delete_provider,
            switch_provider,
            write_codex_takeover,
            start_proxy,
            stop_proxy,
            get_proxy_status,
            load_account_projection,
            load_account_usage,
            import_current_account,
            add_managed_account,
            switch_account,
            remove_managed_account,
            refresh_managed_account,
            open_codex_home,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Codex API Switcher");
}
