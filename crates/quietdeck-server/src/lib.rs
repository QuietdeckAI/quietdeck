//! quietdeck-server as a library: the deck's modules and [`serve`], for the `quietdeck-server` binary and for
//! Quietdeck solo, which runs the deck and a node in one process.

pub mod api;
pub mod auth;
pub mod config;
pub mod control;
pub mod db;
pub mod documents;
pub mod error;
pub mod events;
pub mod fleetlayer;
pub mod handoffs;
pub mod health;
pub mod hooks;
pub mod hub;
pub mod leases;
pub mod mail;
pub mod mcp;
pub mod messages;
pub mod openapi;
pub mod ops;
pub mod overseer;
pub mod passkeys;
pub mod pings;
pub mod push;
pub mod reading;
pub mod relay;
pub mod reports;
pub mod rules;
pub mod runner;
pub mod solo;
pub mod state;
pub mod store;
pub mod web;
pub mod ws;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;
use tower_http::trace::TraceLayer;

use crate::config::{DeckConfig, Env};
use crate::state::AppState;

/// Run the deck until SIGTERM or Ctrl-C.
pub async fn serve(env: Env) -> anyhow::Result<()> {
    std::sync::LazyLock::force(&state::STARTED_AT);
    let config = DeckConfig::load(&env.data.config_path())?;
    let webauthn = passkeys::webauthn(&env.public_url)?;
    std::fs::create_dir_all(&env.data.data_dir)
        .map_err(|e| anyhow::anyhow!("the data directory {}: {e}", env.data.data_dir.display()))?;

    // Health answers 503 until the database is open and migrated.
    let ready = Arc::new(AtomicBool::new(false));
    // Solo: loopback only, checked before anything binds. No webhook receiver, since nothing outside can reach it.
    let local_only = match env.local_only {
        true => {
            let port = solo::loopback_port(&env.listen)?;
            Some(Arc::new(solo::LocalOnly::new(port, env.data.data_dir.join("solo"))?))
        }
        false => None,
    };
    let app_listener = tokio::net::TcpListener::bind(&env.listen).await?;
    let hooks_listener = match local_only {
        Some(_) => None,
        None => Some(tokio::net::TcpListener::bind(&env.hooks_listen).await?),
    };

    let db = db::open(&env.data.db_path()).await?;
    ensure_owner(&db).await?;
    let signing_key = auth::signing_key(&db).await?;
    let pusher = match (&env.vapid_public_key, &env.vapid_private_key) {
        (Some(public), Some(private)) => {
            Some(Arc::new(push::Pusher::new(public, private, env.vapid_subject.as_deref(), &env.public_url)?))
        }
        _ => {
            tracing::warn!("no VAPID keys: web push is off");
            None
        }
    };
    // Told once SIGTERM or Ctrl-C arrives: the servers stop taking connections, and the long-lived ones (nodes' links,
    // the web app's event streams) close at once, so a rolling deploy's new deck takes over in about a second instead
    // of after Kubernetes' grace period.
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        shutdown().await;
        tracing::info!("stopping: closing node links and event streams");
        let _ = stop_tx.send(true);
    });
    let state = AppState {
        db,
        config: Arc::new(config),
        // Solo's gate admits only the person's own browser (a session from a login link) and token holders, so the
        // app behind it treats a browser as the signed-in owner, as a development deck does.
        dev_trust_local: env.dev_trust_local || local_only.is_some(),
        signing_key: Arc::new(signing_key),
        hub: Arc::default(),
        public_url: env.public_url.clone(),
        webauthn: Arc::new(webauthn),
        ceremonies: Arc::default(),
        events: tokio::sync::broadcast::channel(256).0,
        release: Arc::default(),
        pusher,
        hooks_public_url: env.hooks_public_url.clone(),
        fleetlayer: match (&env.fleetlayer_url, &env.fleetlayer_token) {
            (Some(url), Some(token)) => Some(Arc::new(fleetlayer::Client::new(url, token)?)),
            (Some(_), None) | (None, Some(_)) => {
                tracing::warn!(
                    "QUIETDECK_FLEETLAYER_URL and QUIETDECK_FLEETLAYER_TOKEN go together: Fleetlayer is off"
                );
                None
            }
            (None, None) => None,
        },
        stopping: stop_rx.clone(),
        runner_health: Arc::default(),
        solo: local_only.is_some(),
    };
    if env.dev_trust_local {
        tracing::warn!("QUIETDECK_DEV_TRUST_LOCAL is on: every browser request counts as the signed-in owner");
    }
    tokio::spawn(ops::releaser(state.clone()));
    tokio::spawn(ops::stuck_watch(state.clone()));
    tokio::spawn(leases::ticker(state.clone()));
    tokio::spawn(reading::ticker(state.clone()));
    tokio::spawn(fleetlayer::ticker(state.clone()));
    tokio::spawn(overseer::ticker(state.clone()));
    match (&env.fleetlayer_url, &env.fleetlayer_send_token) {
        (Some(url), Some(token)) => {
            let _ = relay::FLEETLAYER.set(fleetlayer::Sender::new(url, token)?);
            tracing::info!("the deck's words to sessions go through Fleetlayer as service:quietdeck");
        }
        (None, Some(_)) => tracing::warn!(
            "QUIETDECK_FLEETLAYER_SEND_TOKEN needs QUIETDECK_FLEETLAYER_URL: not sending through Fleetlayer"
        ),
        _ => {}
    }
    ready.store(true, Ordering::SeqCst);

    let health = {
        let ready = ready.clone();
        move || {
            let ready = ready.clone();
            async move {
                if ready.load(Ordering::SeqCst) {
                    (StatusCode::OK, "ok")
                } else {
                    (StatusCode::SERVICE_UNAVAILABLE, "starting")
                }
            }
        }
    };
    let mut app = Router::new()
        .route("/healthz", get(health))
        .route("/healthz/runners", get(crate::health::runners))
        .route("/healthz/overseer", get(crate::overseer::health))
        .merge(openapi::routes(state.clone()).0)
        .nest_service("/mcp", mcp::service(state.clone()));
    if local_only.is_some() {
        app = app.merge(solo::router());
    }
    let mut app = match env.ui_dir.clone() {
        Some(dir) => {
            tracing::info!(dir = %dir.display(), "serving the frontend in QUIETDECK_UI_DIR, not the built-in web app");
            let dir = Arc::new(dir);
            app.fallback(move |uri: axum::http::Uri| web::serve_dir(dir.clone(), uri))
        }
        None => app.fallback(web::serve),
    }
    .with_state(state.clone());
    if let Some(lo) = local_only {
        app = app.layer(axum::middleware::from_fn_with_state(lo, solo::guard));
    }
    let app = app.layer(TraceLayer::new_for_http());

    let app_server = axum::serve(app_listener, app).with_graceful_shutdown(state::until_stopping(stop_rx.clone()));
    let Some(hooks_listener) = hooks_listener else {
        tracing::info!(listen = %env.listen, "solo deck serving, on this machine only");
        app_server.await?;
        return Ok(());
    };
    let hooks = hooks::router(state.clone());
    tracing::info!(listen = %env.listen, hooks = %env.hooks_listen, "deck serving");
    let hooks_server =
        axum::serve(hooks_listener, hooks).with_graceful_shutdown(state::until_stopping(stop_rx.clone()));
    tokio::try_join!(async { app_server.await }, async { hooks_server.await })?;
    Ok(())
}

/// The owner's user row. One owner per deck; the columns for teams wait for teams.
async fn ensure_owner(db: &db::Db) -> anyhow::Result<()> {
    sqlx::query("INSERT OR IGNORE INTO users (id, name, tailscale_login) VALUES (1, 'owner', 'owner')")
        .execute(db)
        .await?;
    Ok(())
}

async fn shutdown() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}
