use std::sync::Arc;

use crate::config::DeckConfig;
use crate::db::Db;
use crate::hub::Hub;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

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
    #[expect(dead_code, reason = "webhook endpoints' URLs use it (build step 11)")]
    pub hooks_public_url: Option<String>,
    /// True once the deck is stopping (SIGTERM): long-lived connections close.
    pub stopping: tokio::sync::watch::Receiver<bool>,
}

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
            stopping: tokio::sync::watch::channel(false).1,
        }
    }
}
