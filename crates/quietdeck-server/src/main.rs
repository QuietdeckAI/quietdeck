//! quietdeck-server: the deck. `serve` runs it; the other subcommands are for setting it up.

mod api;
mod auth;
mod config;
mod control;
mod db;
mod error;
mod events;
mod hooks;
mod hub;
mod mcp;
mod ops;
mod passkeys;
mod relay;
mod runner;
mod state;
mod store;
mod web;
mod ws;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use clap::{Parser, Subcommand};
use tower_http::trace::TraceLayer;

use crate::config::{DataEnv, DeckConfig, Env};
use crate::state::AppState;

#[derive(Parser)]
#[command(name = "quietdeck-server", version, about = "Quietdeck's server: the deck, its web app and API")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the deck: the app on QUIETDECK_LISTEN, the webhook receiver on QUIETDECK_HOOKS_LISTEN.
    Serve(Env),
    /// Print a new Web Push (VAPID) key pair for QUIETDECK_VAPID_PUBLIC_KEY and QUIETDECK_VAPID_PRIVATE_KEY.
    VapidKeys,
    /// Print a one-time code for `quietdeck node enroll`, without the web app (run it in the deck's container).
    EnrolmentCode(DataEnv),
    /// Print a one-time code for registering a passkey without being signed in: the first one, or a new one when
    /// every passkey is lost (run it in the deck's container).
    SetupCode(DataEnv),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .init();
    match Cli::parse().command {
        Command::Serve(env) => serve(env).await,
        Command::VapidKeys => {
            let (public, private) = vapid_keys();
            println!("QUIETDECK_VAPID_PUBLIC_KEY={public}");
            println!("QUIETDECK_VAPID_PRIVATE_KEY={private}");
            Ok(())
        }
        Command::SetupCode(data) => {
            let db = db::open(&data.db_path()).await?;
            let code = passkeys::create_setup_code(&db).await?;
            println!("{code}");
            eprintln!("Valid for 15 minutes, once: enter it on the deck's sign-in page to register a passkey.");
            Ok(())
        }
        Command::EnrolmentCode(data) => {
            let db = db::open(&data.db_path()).await?;
            let code = store::nodes::create_enrolment_code(&db).await?;
            println!("{}", code.code);
            eprintln!(
                "Valid for 15 minutes, once: quietdeck node enroll {} --deck <deck URL> --name <machine>",
                code.code
            );
            Ok(())
        }
    }
}

fn vapid_keys() -> (String, String) {
    loop {
        // A random 32-byte scalar is a valid P-256 key unless it's zero or past the group order (vanishingly rare).
        if let Ok(secret) = p256::SecretKey::from_slice(&auth::random_bytes::<32>()) {
            let public = p256::elliptic_curve::sec1::ToEncodedPoint::to_encoded_point(&secret.public_key(), false);
            return (B64.encode(public.as_bytes()), B64.encode(secret.to_bytes()));
        }
    }
}

async fn serve(env: Env) -> anyhow::Result<()> {
    let config = DeckConfig::load(&env.data.config_path())?;
    let webauthn = passkeys::webauthn(&env.public_url)?;
    std::fs::create_dir_all(&env.data.data_dir)
        .map_err(|e| anyhow::anyhow!("the data directory {}: {e}", env.data.data_dir.display()))?;

    // Health answers 503 until the database is open and migrated.
    let ready = Arc::new(AtomicBool::new(false));
    let app_listener = tokio::net::TcpListener::bind(&env.listen).await?;
    let hooks_listener = tokio::net::TcpListener::bind(&env.hooks_listen).await?;

    let db = db::open(&env.data.db_path()).await?;
    ensure_owner(&db).await?;
    let signing_key = auth::signing_key(&db).await?;
    if env.vapid_public_key.is_none() || env.vapid_private_key.is_none() {
        tracing::warn!("no VAPID keys: web push is off");
    }
    let state = AppState {
        db,
        config: Arc::new(config),
        dev_trust_local: env.dev_trust_local,
        signing_key: Arc::new(signing_key),
        hub: Arc::default(),
        public_url: env.public_url.clone(),
        webauthn: Arc::new(webauthn),
        ceremonies: Arc::default(),
        events: tokio::sync::broadcast::channel(256).0,
        release: Arc::default(),
        hooks_public_url: env.hooks_public_url.clone(),
    };
    if env.dev_trust_local {
        tracing::warn!("QUIETDECK_DEV_TRUST_LOCAL is on: every browser request counts as the signed-in owner");
    }
    tokio::spawn(ops::releaser(state.clone()));
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
    let app = Router::new()
        .route("/healthz", get(health))
        .merge(api::router())
        .merge(passkeys::router())
        .nest_service("/mcp", mcp::service(state.clone()))
        .fallback(web::serve)
        .with_state(state.clone())
        .layer(TraceLayer::new_for_http());
    let hooks = hooks::router(state.clone());

    tracing::info!(listen = %env.listen, hooks = %env.hooks_listen, "deck serving");
    let app_server = axum::serve(app_listener, app).with_graceful_shutdown(shutdown());
    let hooks_server = axum::serve(hooks_listener, hooks).with_graceful_shutdown(shutdown());
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
