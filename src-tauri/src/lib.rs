pub mod agent;
pub mod commands;
pub mod models;
pub mod pricing;
pub mod process_ext;
pub mod storage;
pub mod web_server;

use std::sync::atomic::{AtomicU16, AtomicU64};
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

pub type EffectiveWebPort = Arc<AtomicU16>;
pub type SharedTokenVersion = Arc<AtomicU64>;
pub type WsShutdownSender = Arc<broadcast::Sender<()>>;
pub type SharedLiveToken = Arc<tokio::sync::RwLock<String>>;
pub type WebServerCancel = Arc<tokio::sync::Mutex<CancellationToken>>;
pub type WebServerLock = Arc<tokio::sync::Mutex<()>>;
pub type WebServerHandle = Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>;

#[derive(Clone)]
pub struct WebServerGeneration(pub Arc<AtomicU64>);

#[derive(Clone)]
pub struct EffectiveWebBind(pub Arc<tokio::sync::RwLock<String>>);

#[derive(Clone)]
pub struct WebServerWarning(pub Arc<tokio::sync::RwLock<Option<String>>>);

pub async fn run_server() -> Result<(), String> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("opencovibe_server=info,warn"),
    )
    .format_timestamp_millis()
    .init();

    process_ext::setup_job_kill_on_close();
    storage::runs::reconcile_orphaned_runs();
    std::thread::spawn(agent::claude_stream::prime_path_cache);

    web_server::serve_from_env().await
}
