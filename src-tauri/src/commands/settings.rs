use crate::models::{AgentSettings, UserSettings};
use crate::storage;
use std::sync::atomic::Ordering;

/// Shared logic for updating user settings with token rotation detection.
/// Used by both IPC (Tauri command) and WS (dispatch) paths.
pub async fn update_user_settings_with_rotation(
    patch: serde_json::Value,
    token_ver: &std::sync::atomic::AtomicU64,
    shutdown: &tokio::sync::broadcast::Sender<()>,
    live_token: &tokio::sync::RwLock<String>,
) -> Result<UserSettings, String> {
    let old = storage::settings::get_user_settings();
    let new_settings = storage::settings::update_user_settings(patch)?;
    if old.web_server_token != new_settings.web_server_token {
        match &new_settings.web_server_token {
            Some(new_tok) => *live_token.write().await = new_tok.clone(),
            None => *live_token.write().await = String::new(),
        }
        token_ver.fetch_add(1, Ordering::Relaxed);
        log::debug!("[web_server] token rotated, updating in-memory + disconnecting WS clients");
        let _ = shutdown.send(());
    }
    Ok(new_settings)
}

pub fn get_user_settings() -> UserSettings {
    log::debug!("[settings] get_user_settings");
    storage::settings::get_user_settings()
}

pub async fn update_user_settings(
    patch: serde_json::Value,
    token_ver: &crate::SharedTokenVersion,
    shutdown: &crate::WsShutdownSender,
    live_token: &crate::SharedLiveToken,
) -> Result<UserSettings, String> {
    log::debug!("[settings] update_user_settings");
    update_user_settings_with_rotation(patch, token_ver, shutdown, live_token).await
}

pub fn get_agent_settings(agent: String) -> AgentSettings {
    log::debug!("[settings] get_agent_settings: agent={}", agent);
    storage::settings::get_agent_settings(&agent)
}

pub fn update_agent_settings(
    agent: String,
    patch: serde_json::Value,
) -> Result<AgentSettings, String> {
    log::debug!("[settings] update_agent_settings: agent={}", agent);
    storage::settings::update_agent_settings(&agent, patch)
}
