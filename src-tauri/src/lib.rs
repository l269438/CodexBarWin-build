pub mod accounts;
pub mod codex_live;
pub mod history;
pub mod network_env;
pub mod official_auth;
pub mod official_backup;
pub mod proxy;
pub mod store;
pub mod transform;
pub mod usage;

use std::sync::Arc;

use accounts::{
    CodexActiveSource, CodexVisibleAccountProjection, FileActiveSourceStore,
    FileManagedCodexAccountStore, ManagedCodexAccountSet,
};
pub use network_env::NetworkEnvStatus;
pub use official_auth::ChatGptAuthStatus;
pub use official_backup::OriginalBackupStatus;
use serde::{Deserialize, Serialize};
pub use store::{AppConfig, Provider};
use tokio::sync::{Mutex, RwLock};

pub const DEFAULT_PROXY_PORT: u16 = 15721;

pub struct AppState {
    config: Arc<RwLock<AppConfig>>,
    proxy: Arc<Mutex<Option<proxy::ProxyHandle>>>,
    history: Arc<history::ConversationHistoryStore>,
    active_source: Arc<RwLock<CodexActiveSource>>,
}

impl AppState {
    pub fn load() -> Self {
        let config = store::load_config().unwrap_or_else(|_| AppConfig::default());
        let active_source = FileActiveSourceStore::default()
            .load()
            .unwrap_or(CodexActiveSource::LiveSystem);
        Self {
            config: Arc::new(RwLock::new(config)),
            proxy: Arc::new(Mutex::new(None)),
            history: Arc::new(history::ConversationHistoryStore::default()),
            active_source: Arc::new(RwLock::new(active_source)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyStatus {
    pub running: bool,
    pub port: u16,
    pub current_provider_id: Option<String>,
}

impl AppState {
    pub async fn get_app_config(&self) -> AppConfig {
        self.config.read().await.clone()
    }

    pub async fn save_provider(&self, provider: Provider) -> Result<AppConfig, String> {
        let mut config = self.config.write().await;
        config.upsert_provider(provider);
        store::save_config(&config).map_err(|e| e.to_string())?;
        Ok(config.clone())
    }

    pub async fn delete_provider(&self, id: String) -> Result<AppConfig, String> {
        let mut config = self.config.write().await;
        if config.providers.len() <= 1 {
            return Err("cannot delete the last provider".to_string());
        }
        config.providers.retain(|provider| provider.id != id);
        if config.current_provider_id.as_deref() == Some(&id) {
            config.current_provider_id =
                config.providers.first().map(|provider| provider.id.clone());
        }
        config.normalize();
        store::save_config(&config).map_err(|e| e.to_string())?;
        Ok(config.clone())
    }

    pub async fn switch_provider(&self, id: String) -> Result<AppConfig, String> {
        let mut config = self.config.write().await;
        if !config.providers.iter().any(|provider| provider.id == id) {
            return Err(format!("provider `{id}` does not exist"));
        }
        config.current_provider_id = Some(id);
        store::save_config(&config).map_err(|e| e.to_string())?;
        Ok(config.clone())
    }

    pub async fn write_codex_takeover(&self) -> Result<(), String> {
        let _ = official_auth::ensure_chatgpt_auth_mode(&codex_live::default_codex_dir());
        let config = self.config.read().await;
        let provider = config
            .current_provider()
            .ok_or_else(|| "no current provider selected".to_string())?;
        codex_live::write_takeover_config(
            &codex_live::default_codex_dir(),
            config.proxy_port,
            &provider.name,
            &provider.model,
        )
        .map_err(|e| e.to_string())
    }

    pub async fn start_proxy(&self) -> Result<ProxyStatus, String> {
        {
            let mut config = self.config.write().await;
            config.normalize();
            store::save_config(&config).map_err(|e| e.to_string())?;
        }

        self.write_codex_takeover().await?;

        let port = self.config.read().await.proxy_port;
        let mut guard = self.proxy.lock().await;
        if guard.is_none() {
            let handle = proxy::start_proxy(port, self.config.clone(), self.history.clone())
                .await
                .map_err(|e| e.to_string())?;
            *guard = Some(handle);
        }

        Ok(ProxyStatus {
            running: true,
            port,
            current_provider_id: self.config.read().await.current_provider_id.clone(),
        })
    }

    pub async fn stop_proxy(&self) -> Result<ProxyStatus, String> {
        let mut guard = self.proxy.lock().await;
        if let Some(handle) = guard.take() {
            handle.stop();
        }
        codex_live::restore_original_config(&codex_live::default_codex_dir())
            .map_err(|e| e.to_string())?;
        let config = self.config.read().await;
        Ok(ProxyStatus {
            running: false,
            port: config.proxy_port,
            current_provider_id: config.current_provider_id.clone(),
        })
    }

    pub async fn get_proxy_status(&self) -> ProxyStatus {
        let running = self.proxy.lock().await.is_some();
        let config = self.config.read().await;
        ProxyStatus {
            running,
            port: config.proxy_port,
            current_provider_id: config.current_provider_id.clone(),
        }
    }

    pub fn get_original_backup_status(&self) -> Result<OriginalBackupStatus, String> {
        official_backup::load_original_backup_status(
            &official_backup::default_original_backup_root(),
        )
        .map_err(|e| e.to_string())
    }

    pub fn get_chatgpt_auth_status(&self) -> Result<ChatGptAuthStatus, String> {
        official_auth::load_chatgpt_auth_status(&codex_live::default_codex_dir())
            .map_err(|e| e.to_string())
    }

    pub fn get_network_env_status(&self) -> Result<NetworkEnvStatus, String> {
        network_env::load_network_env_status(
            &codex_live::default_codex_dir(),
            &network_env::default_network_env_backup_root(),
        )
        .map_err(|e| e.to_string())
    }

    pub fn apply_network_proxy_env(&self, endpoint: String) -> Result<NetworkEnvStatus, String> {
        network_env::apply_network_proxy_env(
            &codex_live::default_codex_dir(),
            &network_env::default_network_env_backup_root(),
            &endpoint,
        )
        .map_err(|e| e.to_string())
    }

    pub fn restore_network_proxy_env(&self) -> Result<NetworkEnvStatus, String> {
        network_env::restore_network_proxy_env(
            &codex_live::default_codex_dir(),
            &network_env::default_network_env_backup_root(),
        )
        .map_err(|e| e.to_string())
    }

    pub fn repair_chatgpt_auth_mode(&self) -> Result<ChatGptAuthStatus, String> {
        official_auth::ensure_chatgpt_auth_mode(&codex_live::default_codex_dir())
            .map_err(|e| e.to_string())
    }

    pub fn create_original_backup(&self) -> Result<OriginalBackupStatus, String> {
        official_backup::create_original_backup(
            &codex_live::default_codex_dir(),
            &official_backup::default_original_backup_root(),
        )
        .map_err(|e| e.to_string())
    }

    pub async fn restore_original_backup(&self) -> Result<OriginalBackupStatus, String> {
        {
            let mut guard = self.proxy.lock().await;
            if let Some(handle) = guard.take() {
                handle.stop();
            }
        }
        let status = official_backup::restore_original_backup(
            &codex_live::default_codex_dir(),
            &official_backup::default_original_backup_root(),
        )
        .map_err(|e| e.to_string())?;
        let takeover_backup = codex_live::switcher_backup_path(&codex_live::default_codex_dir());
        if takeover_backup.exists() {
            std::fs::remove_file(takeover_backup).map_err(|e| e.to_string())?;
        }
        self.set_active_source(CodexActiveSource::LiveSystem)
            .await?;
        Ok(status)
    }

    pub async fn load_account_projection(&self) -> CodexVisibleAccountProjection {
        let (accounts, unreadable) = match FileManagedCodexAccountStore::default().load_accounts() {
            Ok(set) => (set.accounts, false),
            Err(_) => (Vec::new(), true),
        };
        let active = self.active_source.read().await.clone();
        let sanitized = accounts::sanitize_active_source(active, &accounts);
        self.persist_active_source_if_changed(sanitized.clone())
            .await;
        accounts::load_projection(sanitized, accounts, unreadable)
    }

    pub async fn load_account_usage(
        &self,
        account_id: Option<String>,
    ) -> Result<usage::CodexUsageSummary, String> {
        let active_source = self.active_source.read().await.clone();
        let stored_accounts =
            if usage::usage_request_needs_managed_store(account_id.as_deref(), &active_source) {
                FileManagedCodexAccountStore::default()
                    .load_accounts()?
                    .accounts
            } else {
                Vec::new()
            };
        let codex_home = usage::usage_home_for_account_id(
            account_id.as_deref(),
            &active_source,
            &stored_accounts,
        )?;
        usage::load_usage_for_home(&codex_home).await
    }

    pub async fn import_current_account(&self) -> Result<CodexVisibleAccountProjection, String> {
        let live = accounts::discover_live_account()
            .ok_or_else(|| "No live Codex account found in ~/.codex/auth.json".to_string())?;
        let store = FileManagedCodexAccountStore::default();
        let mut set = store.load_accounts()?;
        if let Some(existing) = set
            .accounts
            .iter_mut()
            .find(|account| account.matches_live_account(&live))
        {
            accounts::sync_auth_file(
                std::path::Path::new(&live.codex_home_path),
                std::path::Path::new(&existing.managed_home_path),
            )?;
            existing.refresh_from_live_account(&live);
        } else {
            let managed_home =
                accounts::default_managed_homes_root().join(uuid::Uuid::new_v4().to_string());
            accounts::sync_auth_file(std::path::Path::new(&live.codex_home_path), &managed_home)?;
            set.accounts
                .push(accounts::ManagedCodexAccount::from_live_account(
                    &live,
                    managed_home.to_string_lossy().into_owned(),
                ));
        }
        let set = ManagedCodexAccountSet::new(set.accounts);
        store.save_accounts(&set)?;
        Ok(self.load_account_projection().await)
    }

    pub async fn add_managed_account(&self) -> Result<CodexVisibleAccountProjection, String> {
        let managed_home =
            accounts::default_managed_homes_root().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&managed_home).map_err(|error| error.to_string())?;

        let login_result = accounts::ManagedCodexLoginRunner::run(
            &managed_home,
            std::time::Duration::from_secs(120),
        );
        let authenticated = accounts::discover_account_in_home(&managed_home);
        if login_result.is_err() && authenticated.is_none() {
            let _ = std::fs::remove_dir_all(&managed_home);
            return Err(login_result
                .err()
                .unwrap_or_else(|| "Codex login failed".to_string()));
        }
        let authenticated = authenticated.ok_or_else(|| {
            let _ = std::fs::remove_dir_all(&managed_home);
            "Codex login completed but no account could be read".to_string()
        })?;

        let store = FileManagedCodexAccountStore::default();
        let mut set = store.load_accounts()?;
        let target_id = if let Some(existing) = set
            .accounts
            .iter_mut()
            .find(|account| account.matches_live_account(&authenticated))
        {
            accounts::sync_auth_file(
                &managed_home,
                std::path::Path::new(&existing.managed_home_path),
            )?;
            existing.refresh_from_live_account(&authenticated);
            let id = existing.id.clone();
            let _ = std::fs::remove_dir_all(&managed_home);
            id
        } else {
            let account = accounts::ManagedCodexAccount::from_live_account(
                &authenticated,
                managed_home.to_string_lossy().into_owned(),
            );
            let id = account.id.clone();
            set.accounts.push(account);
            id
        };
        let set = ManagedCodexAccountSet::new(set.accounts);
        store.save_accounts(&set)?;
        self.switch_account(target_id).await?;
        Ok(self.load_account_projection().await)
    }

    pub async fn switch_account(&self, account_id: String) -> Result<(), String> {
        let store = FileManagedCodexAccountStore::default();
        let set = store.load_accounts()?;
        let current = self.active_source.read().await.clone();
        let target = if account_id == "live" {
            CodexActiveSource::LiveSystem
        } else {
            CodexActiveSource::ManagedAccount { id: account_id }
        };
        let target = accounts::sanitize_active_source(target, &set.accounts);
        let target_home = managed_home_for_source(&target, &set.accounts)?;
        accounts::CodexHomeTransaction::default().switch(
            &current,
            &target,
            target_home.as_deref().map(std::path::Path::new),
        )?;
        let _ = official_auth::ensure_chatgpt_auth_mode(&accounts::default_codex_home());
        self.set_active_source(target).await
    }

    pub async fn remove_managed_account(
        &self,
        account_id: String,
    ) -> Result<CodexVisibleAccountProjection, String> {
        let store = FileManagedCodexAccountStore::default();
        let mut set = store.load_accounts()?;
        let index = set
            .accounts
            .iter()
            .position(|account| account.id == account_id)
            .ok_or_else(|| "target managed account is missing".to_string())?;
        let removed = set.accounts[index].clone();

        if *self.active_source.read().await
            == (CodexActiveSource::ManagedAccount {
                id: account_id.clone(),
            })
        {
            self.switch_account("live".to_string()).await?;
        }

        set.accounts.remove(index);
        let set = ManagedCodexAccountSet::new(set.accounts);
        store.save_accounts(&set)?;
        if std::path::Path::new(&removed.managed_home_path).exists() {
            std::fs::remove_dir_all(&removed.managed_home_path)
                .map_err(|error| error.to_string())?;
        }
        Ok(self.load_account_projection().await)
    }

    pub async fn refresh_managed_account(
        &self,
        account_id: String,
    ) -> Result<CodexVisibleAccountProjection, String> {
        if *self.active_source.read().await != CodexActiveSource::LiveSystem {
            return Err(
                "Switch to the live Codex account before refreshing a managed account".to_string(),
            );
        }
        let live = accounts::discover_live_account()
            .ok_or_else(|| "No live Codex account found in ~/.codex/auth.json".to_string())?;
        let store = FileManagedCodexAccountStore::default();
        let mut set = store.load_accounts()?;
        let account = set
            .accounts
            .iter_mut()
            .find(|account| account.id == account_id)
            .ok_or_else(|| "target managed account is missing".to_string())?;
        accounts::sync_auth_file(
            std::path::Path::new(&live.codex_home_path),
            std::path::Path::new(&account.managed_home_path),
        )?;
        account.refresh_from_live_account(&live);
        let set = ManagedCodexAccountSet::new(set.accounts);
        store.save_accounts(&set)?;
        Ok(self.load_account_projection().await)
    }

    async fn persist_active_source_if_changed(&self, source: CodexActiveSource) {
        if *self.active_source.read().await == source {
            return;
        }
        let _ = self.set_active_source(source).await;
    }

    async fn set_active_source(&self, source: CodexActiveSource) -> Result<(), String> {
        FileActiveSourceStore::default().save(&source)?;
        *self.active_source.write().await = source;
        Ok(())
    }
}

fn managed_home_for_source(
    source: &CodexActiveSource,
    accounts: &[accounts::ManagedCodexAccount],
) -> Result<Option<String>, String> {
    match source {
        CodexActiveSource::LiveSystem => Ok(None),
        CodexActiveSource::ManagedAccount { id } => accounts
            .iter()
            .find(|account| &account.id == id)
            .map(|account| Some(account.managed_home_path.clone()))
            .ok_or_else(|| "target managed account is missing".to_string()),
    }
}
