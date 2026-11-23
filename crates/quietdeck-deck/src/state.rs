use std::sync::Arc;

use crate::config::DeckConfig;
use crate::db::Db;
use crate::hub::Hub;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub config: Arc<DeckConfig>,
    /// Every Tailscale login that counts as the owner (lowercase). The first names the owner's user row.
    pub owner_logins: Vec<String>,
    pub dev_trust_local: bool,
    pub signing_key: Arc<Vec<u8>>,
    pub hub: Arc<Hub>,
    pub public_url: String,
    #[expect(dead_code, reason = "webhook endpoints' URLs use it (build step 11)")]
    pub hooks_public_url: Option<String>,
}

impl AppState {
    #[cfg(test)]
    #[expect(dead_code, reason = "the API tests use it (build step 2)")]
    pub async fn for_tests() -> AppState {
        let db = crate::db::memory().await;
        let signing_key = crate::auth::signing_key(&db).await.unwrap();
        AppState {
            db,
            config: Arc::new(DeckConfig::default()),
            owner_logins: vec!["owner@example.com".into(), "owner@work.example".into()],
            dev_trust_local: false,
            signing_key: Arc::new(signing_key),
            hub: Arc::new(Hub::default()),
            public_url: "https://deck.example.ts.net".into(),
            hooks_public_url: Some("https://hooks.example.com".into()),
        }
    }
}
