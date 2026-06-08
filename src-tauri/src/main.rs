#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use codex_api_switcher::{
    AppConfig, AppState, ChatGptAuthStatus, CodexSessionList, CodexSessionPreview,
    CopySessionResult, DeleteProjectSessionsResult, DeleteSessionResult, DeletedSessionEntry,
    NetworkEnvStatus, OriginalBackupStatus, ProxyStatus, RestoreDeletedSessionsResult,
    accounts::CodexVisibleAccountProjection, store::Provider, usage::CodexUsageSummary,
};
use std::process::Command;
use tauri::{
    AppHandle, Emitter, Manager, State, WindowEvent,
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

const MAIN_WINDOW_LABEL: &str = "main";
const USAGE_BUBBLE_WINDOW_LABEL: &str = "usage-bubble";
const TRAY_ID: &str = "codexpilot-main-tray";
const TRAY_TOOLTIP_DEFAULT: &str = "CodexPilot\n额度检测中";
const SHOW_USAGE_BUBBLE_EVENT: &str = "codexpilot://show-usage-bubble";
const TRAY_ICON_BYTES: &[u8] = include_bytes!("../icons/codexpilot-icon.png");

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
fn update_tray_usage_tooltip(
    app: AppHandle,
    account_email: Option<String>,
    session_remaining_percent: Option<f64>,
    weekly_remaining_percent: Option<f64>,
    proxy_running: bool,
) -> Result<(), String> {
    set_tray_tooltip(
        &app,
        &build_tray_usage_tooltip(
            account_email.as_deref(),
            session_remaining_percent,
            weekly_remaining_percent,
            proxy_running,
        ),
    )
}

#[tauri::command]
fn open_main_panel(app: AppHandle) {
    hide_usage_bubble_window(&app);
    show_main_window(&app);
}

#[tauri::command]
fn open_accounts_panel(app: AppHandle) {
    hide_usage_bubble_window(&app);
    show_main_window(&app);
    let _ = app.emit(SHOW_USAGE_BUBBLE_EVENT, ());
}

#[tauri::command]
fn get_original_backup_status(state: State<'_, AppState>) -> Result<OriginalBackupStatus, String> {
    state.get_original_backup_status()
}

#[tauri::command]
fn create_original_backup(state: State<'_, AppState>) -> Result<OriginalBackupStatus, String> {
    state.create_original_backup()
}

#[tauri::command]
fn get_chatgpt_auth_status(state: State<'_, AppState>) -> Result<ChatGptAuthStatus, String> {
    state.get_chatgpt_auth_status()
}

#[tauri::command]
fn get_network_env_status(state: State<'_, AppState>) -> Result<NetworkEnvStatus, String> {
    state.get_network_env_status()
}

#[tauri::command]
fn apply_network_proxy_env(
    state: State<'_, AppState>,
    endpoint: String,
) -> Result<NetworkEnvStatus, String> {
    state.apply_network_proxy_env(endpoint)
}

#[tauri::command]
fn restore_network_proxy_env(state: State<'_, AppState>) -> Result<NetworkEnvStatus, String> {
    state.restore_network_proxy_env()
}

#[tauri::command]
fn repair_chatgpt_auth_mode(state: State<'_, AppState>) -> Result<ChatGptAuthStatus, String> {
    state.repair_chatgpt_auth_mode()
}

#[tauri::command]
async fn restore_original_backup(
    state: State<'_, AppState>,
) -> Result<OriginalBackupStatus, String> {
    state.restore_original_backup().await
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
fn list_codex_sessions(state: State<'_, AppState>) -> Result<CodexSessionList, String> {
    state.list_codex_sessions()
}

#[tauri::command]
fn preview_codex_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<CodexSessionPreview, String> {
    state.preview_codex_session(session_id)
}

#[tauri::command]
fn delete_codex_session(
    state: State<'_, AppState>,
    session_id: String,
    confirmed: bool,
) -> Result<DeleteSessionResult, String> {
    state.delete_codex_session(session_id, confirmed)
}

#[tauri::command]
fn delete_codex_project_sessions(
    state: State<'_, AppState>,
    project_path: String,
    confirmed: bool,
) -> Result<DeleteProjectSessionsResult, String> {
    state.delete_codex_project_sessions(project_path, confirmed)
}

#[tauri::command]
fn list_deleted_codex_sessions(
    state: State<'_, AppState>,
) -> Result<Vec<DeletedSessionEntry>, String> {
    state.list_deleted_codex_sessions()
}

#[tauri::command]
fn restore_deleted_codex_sessions(
    state: State<'_, AppState>,
    deletion_id: String,
    confirmed: bool,
) -> Result<RestoreDeletedSessionsResult, String> {
    state.restore_deleted_codex_sessions(deletion_id, confirmed)
}

#[tauri::command]
fn copy_codex_session_to_account(
    state: State<'_, AppState>,
    session_id: String,
    target_account_id: String,
) -> Result<CopySessionResult, String> {
    state.copy_codex_session_to_account(session_id, target_account_id)
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

#[tauri::command]
fn open_codex_session(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    let preview = state.preview_codex_session(session_id)?;
    let path = codex_api_switcher::codex_live::default_codex_dir()
        .join("sessions")
        .join(preview.summary.relative_path);

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg("-R").arg(&path);
        command
    };

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("explorer");
        command.arg(format!("/select,{}", path.to_string_lossy()));
        command
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(path.parent().unwrap_or(&path));
        command
    };

    command.spawn().map_err(|error| error.to_string())?;
    Ok(())
}

fn build_tray_usage_tooltip(
    account_email: Option<&str>,
    session_remaining_percent: Option<f64>,
    weekly_remaining_percent: Option<f64>,
    proxy_running: bool,
) -> String {
    let account = account_email
        .filter(|email| !email.trim().is_empty())
        .unwrap_or("未检测到 Codex 账号");
    let session = format_tray_percent(session_remaining_percent);
    let weekly = format_tray_percent(weekly_remaining_percent);
    let proxy = if proxy_running {
        "三方代理运行中"
    } else {
        "官方配置"
    };

    format!("CodexPilot\n账号：{account}\n会话剩余：{session}\n每周剩余：{weekly}\n状态：{proxy}")
}

fn format_tray_percent(value: Option<f64>) -> String {
    value
        .filter(|number| number.is_finite())
        .map(|number| format!("{}%", number.round().clamp(0.0, 100.0)))
        .unwrap_or_else(|| "暂不可用".to_string())
}

fn set_tray_tooltip(app: &AppHandle, tooltip: &str) -> Result<(), String> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_tooltip(Some(tooltip))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn hide_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        let _ = window.hide();
    }
}

fn hide_usage_bubble_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(USAGE_BUBBLE_WINDOW_LABEL) {
        let _ = window.hide();
    }
}

fn toggle_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        match window.is_visible() {
            Ok(true) => {
                let _ = window.hide();
            }
            _ => {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
    }
}

fn setup_system_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(
        app,
        "show-main-window",
        "显示 CodexPilot",
        true,
        None::<&str>,
    )?;
    let hide_item = MenuItem::with_id(app, "hide-main-window", "隐藏窗口", true, None::<&str>)?;
    let open_home_item = MenuItem::with_id(
        app,
        "open-codex-home",
        "打开 Codex 目录",
        true,
        None::<&str>,
    )?;
    let quit_item = MenuItem::with_id(app, "quit-codexpilot", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &hide_item, &open_home_item, &quit_item])?;
    let icon = Image::from_bytes(TRAY_ICON_BYTES)?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip(TRAY_TOOLTIP_DEFAULT)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show-main-window" => show_main_window(app),
            "hide-main-window" => hide_main_window(app),
            "open-codex-home" => {
                let _ = open_codex_home();
            }
            "quit-codexpilot" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::load())
        .setup(|app| {
            setup_system_tray(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == MAIN_WINDOW_LABEL
                    || window.label() == USAGE_BUBBLE_WINDOW_LABEL
                {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
            if window.label() == USAGE_BUBBLE_WINDOW_LABEL {
                if let WindowEvent::Focused(false) = event {
                    hide_usage_bubble_window(window.app_handle());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_app_config,
            save_provider,
            delete_provider,
            switch_provider,
            write_codex_takeover,
            start_proxy,
            stop_proxy,
            get_proxy_status,
            update_tray_usage_tooltip,
            open_main_panel,
            open_accounts_panel,
            get_original_backup_status,
            create_original_backup,
            get_chatgpt_auth_status,
            get_network_env_status,
            apply_network_proxy_env,
            restore_network_proxy_env,
            repair_chatgpt_auth_mode,
            restore_original_backup,
            load_account_projection,
            load_account_usage,
            import_current_account,
            add_managed_account,
            switch_account,
            remove_managed_account,
            refresh_managed_account,
            list_codex_sessions,
            preview_codex_session,
            delete_codex_session,
            delete_codex_project_sessions,
            list_deleted_codex_sessions,
            restore_deleted_codex_sessions,
            copy_codex_session_to_account,
            open_codex_home,
            open_codex_session,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Codex API Switcher");
}
