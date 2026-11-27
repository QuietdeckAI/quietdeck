use std::sync::Arc;

use crate::config::DeckConfig;
use crate::db::Db;
use crate::hub::Hub;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The commit the release was built from, and when (Unix seconds): scripts/build-images sets both.
pub const COMMIT: Option<&str> = option_env!("QUIETDECK_COMMIT");
pub const BUILT_AT: Option<&str> = option_env!("QUIETDECK_BUILT_AT");
/// When this deck started (`serve` touches it first thing).
pub static STARTED_AT: std::sync::LazyLock<jiff::Timestamp> = std::sync::LazyLock::new(jiff::Timestamp::now);

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub config: Arc<DeckConfig>,
    pub dev_trust_local: bool,
    pub signing_key: Arc<Vec<u8>>,
    pub hub: Arc<Hub>,
    pub public_url: String,
    pub webauthn: Arc<webauthn_rs::prelude::Webauthn>,
    pub ceremonies: Arc<crate::passkeys::Ceremonies>,
    /// What changed, for the web app's event stream.
    pub events: tokio::sync::broadcast::Sender<crate::events::Changed>,
    /// Wakes the releaser when a batch is recorded.
    pub release: Arc<tokio::sync::Notify>,
    /// Web push, when the deck has VAPID keys.
    pub pusher: Option<Arc<crate::push::Pusher>>,
    pub hooks_public_url: Option<String>,
    /// Fleetlayer's control plane, when the deck has its address and a token.
    pub fleetlayer: Option<Arc<crate::fleetlayer::Client>>,
    /// True once the deck is stopping (SIGTERM): long-lived connections close.
    pub stopping: tokio::sync::watch::Receiver<bool>,
    /// Each node's last [`quietdeck_types::node::RunnerHealth`], and when it came. In memory only.
    pub runner_health: Arc<std::sync::Mutex<RunnerHealthByNode>>,
}

pub type RunnerHealthByNode = std::collections::HashMap<String, (quietdeck_types::node::RunnerHealth, jiff::Timestamp)>;

/// Resolves once the deck is stopping. A sender that's gone (nothing will ever say stop) never resolves.
pub async fn until_stopping(mut rx: tokio::sync::watch::Receiver<bool>) {
    if rx.wait_for(|s| *s).await.is_err() {
        std::future::pending::<()>().await;
    }
}

impl AppState {
    /// Tell the web app something changed. Nobody listening is fine.
    pub fn changed(&self, what: crate::events::Changed) {
        let _ = self.events.send(what);
    }

    #[cfg(test)]
    pub async fn for_tests() -> AppState {
        let db = crate::db::memory().await;
        let signing_key = crate::auth::signing_key(&db).await.unwrap();
        AppState {
            db,
            config: Arc::new(DeckConfig::default()),
            dev_trust_local: false,
            signing_key: Arc::new(signing_key),
            hub: Arc::new(Hub::default()),
            public_url: "https://deck.example.com".into(),
            webauthn: Arc::new(crate::passkeys::webauthn("https://deck.example.com").unwrap()),
            ceremonies: Arc::default(),
            events: tokio::sync::broadcast::channel(64).0,
            release: Arc::default(),
            pusher: None,
            hooks_public_url: Some("https://hooks.example.com".into()),
            fleetlayer: None,
            stopping: tokio::sync::watch::channel(false).1,
            runner_health: Arc::default(),
        }
    }
}
