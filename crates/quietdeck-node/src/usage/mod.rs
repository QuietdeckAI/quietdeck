//! The usage module: monitors this machine's Claude and Codex subscriptions, taken over from the account rotator's
//! coordinator, and serves the coordinator's API on its socket so the rotation plugin keeps working.
//!
//! This file is the boundary between the node's core (built by the quietdeck tab) and the module (ported by the
//! claude rotation tab). The records crossing it are in `quietdeck_types::usage`; the contract is
//! `docs/usage-interface.md`.
//!
//! The node:
//! - builds a [`UsageConfig`] from node.yaml's `usage:` section and a [`UsageEnv`];
//! - forwards every [`UsageEvent::Snapshot`] to the deck as a `UsagePush`, and serves the latest to the menu bar app;
//! - passes on [`UsageCommand`]s from the deck and the menu bar app through [`UsageHandle::command`].
//!
//! The module owns everything else: reading the sign-ins, polling, readings posted by sessions, refreshing idle
//! sign-ins with `claude -p /usage`, browser sign-ins and warnings, model weeks, Codex usage, the session-to-account
//! map, and the coordinator API on `coordinator.sock`. Tokens never leave it.
//!
//! One monitor runs per user, held by `coordinator.lock` in the rotator's directory, which the TypeScript coordinator
//! takes too. While another process holds it (a coordinator from before the node), the module follows that one over
//! its socket instead of reading the sign-ins itself, and takes over once it stops.

// The node's core doesn't call the handle yet.
#![allow(dead_code)]

pub mod codex;
pub mod core;
pub mod deps;
pub mod model;
pub mod server;
#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jiff::Timestamp;
use quietdeck_types::usage::{
    AccountUsage, ReadingSource, UsageCommand, UsageConfig, UsageHealth, UsageProvider, UsageWindow, UsageWindowKind,
};
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};

use self::core::{AccountState, Coordinator, Info, SharedState, Usage};
use self::deps::{Logger, RealDeps};
use self::model::{Health, Ms, parse_iso};

/// What GET /hello answers as `name`: the rotation plugin leaves a monitor of another name alone.
pub const NAME: &str = "quietdeck-node";
/// The coordinator API this module serves: the rotator release whose `docs/coordinator-api.md` it follows.
pub const API_VERSION: &str = "0.7.1";
/// Codex's `local_id`: Codex keeps one sign-in per machine.
pub const CODEX_ID: &str = "codex";

/// Machine-specific inputs that aren't user settings.
#[derive(Debug, Clone)]
pub struct UsageEnv {
    /// The rotator's directory, `~/.claude/account-rotator`: the socket `coordinator.sock` and its state.
    /// `CLAUDE_ACCOUNT_ROTATOR_DIR` overrides it, as the coordinator honours today.
    pub rotator_dir: PathBuf,
    /// The user's home directory (`~/.claude`, `~/.codex`).
    pub home: PathBuf,
    /// The `claude` binary, for refreshing idle sign-ins.
    pub claude_program: PathBuf,
    /// The module's own state, `<node state dir>/usage`.
    pub state_dir: PathBuf,
}

/// What the module tells the node.
#[derive(Debug, Clone, PartialEq)]
pub enum UsageEvent {
    /// Every subscription on this machine, whenever any of it changes (a reading, health, sessions).
    Snapshot(Vec<AccountUsage>),
}

#[derive(Debug, thiserror::Error)]
pub enum UsageError {
    #[error("no sign-in with ID {0} on this machine")]
    UnknownAccount(String),
    #[error("the usage module has stopped")]
    Stopped,
    #[error("{0}")]
    Other(String),
}

/// Who reads the Claude sign-ins: this module, or a coordinator that held the lock first.
#[derive(Clone)]
enum Claude {
    Own(Coordinator<RealDeps>),
    Following(PathBuf),
}

struct Shared {
    claude: Mutex<Option<Claude>>,
    /// The Claude side as last read, from our own monitor or the one we follow.
    claude_state: Mutex<Option<(SharedState, std::collections::HashMap<String, ReadingSource>)>>,
    codex: Mutex<Option<codex::CodexState>>,
    snapshot: Mutex<Vec<AccountUsage>>,
    /// Bumps when anything the snapshot reads changes.
    dirty: watch::Sender<u64>,
    /// Bumps to have Codex read now.
    codex_ask: watch::Sender<u64>,
    stopped: watch::Sender<bool>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A running usage module.
#[derive(Clone)]
pub struct UsageHandle {
    shared: Arc<Shared>,
}

impl UsageHandle {
    /// The latest snapshot, for the menu bar app and a deck that just reconnected.
    pub async fn snapshot(&self) -> Vec<AccountUsage> {
        lock(&self.shared.snapshot).clone()
    }

    /// Carries out a command from the deck or the menu bar app.
    pub async fn command(&self, command: UsageCommand) -> Result<(), UsageError> {
        if *self.shared.stopped.borrow() {
            return Err(UsageError::Stopped);
        }
        let claude = lock(&self.shared.claude).clone().ok_or(UsageError::Stopped)?;
        let is_known = |id: &str| {
            lock(&self.shared.claude_state).as_ref().is_some_and(|(s, _)| s.accounts.iter().any(|a| a.account.id == id))
        };
        match command {
            UsageCommand::SignIn { local_id: Some(id) } if id == CODEX_ID => {
                Err(UsageError::Other("Codex signs in with `codex login` in a terminal on this machine.".into()))
            }
            UsageCommand::SignIn { local_id } => {
                if let Some(id) = &local_id
                    && !is_known(id)
                {
                    return Err(UsageError::UnknownAccount(id.clone()));
                }
                match claude {
                    Claude::Own(c) => c.sign_in(local_id.as_deref()).map_err(|e| UsageError::Other(e.message)),
                    Claude::Following(sock) => {
                        let body = local_id.map_or(json!({}), |id| json!({ "accountId": id }));
                        follow_call(&sock, "POST", "/signin", Some(&body)).await
                    }
                }
            }
            UsageCommand::Check { local_id } => {
                let is_codex = local_id.as_deref() == Some(CODEX_ID);
                if local_id.is_none() || is_codex {
                    self.shared.codex_ask.send_modify(|v| *v += 1);
                }
                if is_codex {
                    return Ok(());
                }
                if let Some(id) = &local_id
                    && !is_known(id)
                {
                    return Err(UsageError::UnknownAccount(id.clone()));
                }
                match claude {
                    Claude::Own(c) => {
                        // Answered at once; the snapshot follows when the reading lands.
                        tokio::spawn(async move {
                            match local_id {
                                Some(id) => c.check(&id, Usage::Fresh).await,
                                None => c.check_all(Usage::Fresh).await,
                            }
                        });
                        Ok(())
                    }
                    Claude::Following(sock) => {
                        let path = local_id.map_or("/check".to_string(), |id| format!("/accounts/{id}/check"));
                        tokio::spawn(async move {
                            follow_call(&sock, "POST", &path, Some(&json!({ "fresh": true }))).await
                        });
                        Ok(())
                    }
                }
            }
        }
    }
}

async fn follow_call(sock: &std::path::Path, method: &str, path: &str, body: Option<&Value>) -> Result<(), UsageError> {
    match server::call(sock, method, path, body, Duration::from_secs(120)).await {
        Ok((200, _)) => Ok(()),
        Ok((_, body)) => Err(UsageError::Other(
            body.get("error").and_then(Value::as_str).unwrap_or("the coordinator refused").to_string(),
        )),
        Err(e) => Err(UsageError::Other(format!("the coordinator didn't answer: {e}"))),
    }
}

fn timestamp(ms: Ms) -> Option<Timestamp> {
    Timestamp::from_millisecond(ms).ok()
}

fn window(kind: UsageWindowKind, model: Option<&str>, w: &model::UsageWindow) -> UsageWindow {
    UsageWindow {
        kind,
        model: model.map(str::to_string),
        window_minutes: None,
        pct: w.pct,
        resets_at: w.resets_at.as_deref().and_then(parse_iso).and_then(timestamp),
    }
}

/// The deck's merge key for a Claude login: `<account uuid>/<org uuid>` from the profile endpoint. Until the
/// profile has been read (a login whose token has run out), the email stands in for the account UUID.
pub fn claude_key(a: &model::Account) -> Option<String> {
    let org = a.org_id.as_deref().unwrap_or("");
    match (&a.account_uuid, &a.email) {
        (Some(uuid), _) => Some(format!("{uuid}/{org}")),
        (None, Some(email)) => Some(format!("{email}/{org}")),
        (None, None) => None,
    }
}

fn claude_usage(a: &AccountState, sources: &std::collections::HashMap<String, ReadingSource>) -> AccountUsage {
    let mut windows = Vec::new();
    if let Some(r) = &a.reading {
        if let Some(w) = &r.five_hour {
            windows.push(window(UsageWindowKind::FiveHour, None, w));
        }
        if let Some(w) = &r.seven_day {
            windows.push(window(UsageWindowKind::Week, None, w));
        }
        for (name, w) in r.models.iter().flatten() {
            windows.push(window(UsageWindowKind::ModelWeek, Some(name), w));
        }
    }
    AccountUsage {
        provider: UsageProvider::Anthropic,
        // A slot nobody has signed in to yet has no subscription: keyed by machine, so it never merges.
        account_key: claude_key(&a.account).unwrap_or_else(|| format!("unsigned/{}", a.account.slot)),
        local_id: a.account.id.clone(),
        email: a.account.email.clone(),
        org_name: a.account.org_name.clone(),
        plan: a.account.plan.clone(),
        weight: a.weight,
        health: match &a.health {
            Health::Ok { .. } => UsageHealth::Ok,
            Health::LoginNeeded { why } => UsageHealth::SigninNeeded { why: why.clone() },
            Health::LoggingIn { since } => UsageHealth::SigningIn { since: timestamp(*since).unwrap_or_default() },
        },
        windows,
        read_at: a.reading.as_ref().and_then(|r| timestamp(r.at)),
        source: sources.get(&a.account.id).copied(),
        session_count: u32::try_from(a.session_count).unwrap_or(u32::MAX),
        retiring: a.is_retiring,
    }
}

fn codex_usage(s: &codex::CodexState) -> AccountUsage {
    let codex_window = |kind, w: &codex::CodexWindow| UsageWindow {
        kind,
        model: None,
        window_minutes: w.window_minutes,
        pct: w.pct,
        resets_at: w.resets_at.and_then(timestamp),
    };
    let r = s.reading.as_ref();
    let mut windows = Vec::new();
    if let Some(w) = r.and_then(|r| r.primary.as_ref()) {
        windows.push(codex_window(UsageWindowKind::CodexPrimary, w));
    }
    if let Some(w) = r.and_then(|r| r.secondary.as_ref()) {
        windows.push(codex_window(UsageWindowKind::CodexSecondary, w));
    }
    AccountUsage {
        provider: UsageProvider::Openai,
        account_key: s.account_id.clone(),
        local_id: CODEX_ID.into(),
        email: r.and_then(|r| r.email.clone()),
        org_name: None,
        plan: r.and_then(|r| r.plan.clone()),
        weight: 1,
        health: match &s.health {
            codex::CodexHealth::Ok => UsageHealth::Ok,
            codex::CodexHealth::SigninNeeded(why) => UsageHealth::SigninNeeded { why: why.clone() },
        },
        windows,
        read_at: r.and_then(|r| timestamp(r.at)),
        source: r.map(|_| if s.is_from_session { ReadingSource::Session } else { ReadingSource::Poll }),
        session_count: 0,
        retiring: false,
    }
}

/// Every subscription on the machine, from the Claude state and the Codex state as last read.
pub fn snapshot_of(
    claude: Option<&(SharedState, std::collections::HashMap<String, ReadingSource>)>,
    codex: Option<&codex::CodexState>,
) -> Vec<AccountUsage> {
    let mut out: Vec<AccountUsage> =
        claude.map(|(s, sources)| s.accounts.iter().map(|a| claude_usage(a, sources)).collect()).unwrap_or_default();
    out.extend(codex.map(codex_usage));
    out
}

fn persisted(file: &std::path::Path, logger: &Logger) -> Option<Value> {
    let text = std::fs::read_to_string(file).ok()?;
    match serde_json::from_str(&text) {
        Ok(v) => Some(v),
        Err(e) => {
            logger.log(&format!("could not read {}: {e}; starting afresh", file.display()));
            None
        }
    }
}

/// Runs our own monitor until the module stops.
async fn run_own(shared: Arc<Shared>, config: &UsageConfig, env: &UsageEnv, logger: Logger) {
    let dir = &env.rotator_dir;
    let deps = RealDeps::new(env.home.clone(), env.claude_program.clone(), server::state_path(dir), logger.clone());
    let info = Info {
        version: API_VERSION.into(),
        pid: std::process::id(),
        started_at: Timestamp::now().as_millisecond(),
        is_managed: true,
    };
    let coordinator = Coordinator::new(deps, info, persisted(&server::state_path(dir), &logger).as_ref());
    *lock(&shared.claude) = Some(Claude::Own(coordinator.clone()));

    // The Claude state, whenever it changes.
    let mut changes = coordinator.changes();
    let publisher = {
        let (shared, coordinator) = (shared.clone(), coordinator.clone());
        tokio::spawn(async move {
            loop {
                *lock(&shared.claude_state) = Some((coordinator.state(), coordinator.sources()));
                shared.dirty.send_modify(|v| *v += 1);
                if changes.changed().await.is_err() {
                    return;
                }
            }
        })
    };

    let mut stopped = shared.stopped.subscribe();
    let until_stopped = async move {
        let _ = stopped.wait_for(|s| *s).await;
    };
    if config.serve_coordinator_api {
        match server::bind(dir) {
            Ok(listener) => {
                logger.log(&format!(
                    "{NAME} serving the coordinator API {API_VERSION} on {} (pid {})",
                    server::socket_path(dir).display(),
                    std::process::id()
                ));
                coordinator.start().await;
                if let Err(e) = server::serve(listener, coordinator.clone(), until_stopped).await {
                    logger.log(&format!("the coordinator API stopped: {e}"));
                }
                let _ = std::fs::remove_file(server::socket_path(dir));
            }
            Err(e) => {
                logger.log(&format!("could not serve {}: {e}", server::socket_path(dir).display()));
                coordinator.start().await;
                until_stopped.await;
            }
        }
    } else {
        coordinator.start().await;
        until_stopped.await;
    }
    coordinator.stop();
    publisher.abort();
    coordinator.deps().flush();
    let _ = std::fs::remove_file(server::lock_path(dir));
}

/// Follows another monitor's state over its socket until it stops answering or the module stops.
async fn follow(shared: Arc<Shared>, sock: PathBuf) {
    *lock(&shared.claude) = Some(Claude::Following(sock.clone()));
    let mut version = 0u64;
    let mut stopped = shared.stopped.subscribe();
    loop {
        let path = format!("/state?since={version}&wait=55000");
        let answer = tokio::select! {
            a = server::call(&sock, "GET", &path, None, Duration::from_secs(70)) => a,
            _ = stopped.wait_for(|s| *s) => return,
        };
        let Ok((200, body)) = answer else { return };
        let Ok(state) = serde_json::from_value::<SharedState>(body) else {
            return;
        };
        version = state.version;
        *lock(&shared.claude_state) = Some((state, Default::default()));
        shared.dirty.send_modify(|v| *v += 1);
    }
}

/// Starts the module on the current tokio runtime, including the coordinator API on `rotator_dir/coordinator.sock`
/// when `serve_coordinator_api` is on. It runs until `events` closes.
pub fn spawn(config: UsageConfig, env: UsageEnv, events: mpsc::Sender<UsageEvent>) -> UsageHandle {
    let (dirty, _) = watch::channel(0);
    let (codex_ask, codex_asked) = watch::channel(0);
    let (stopped, _) = watch::channel(false);
    let shared = Arc::new(Shared {
        claude: Mutex::new(None),
        claude_state: Mutex::new(None),
        codex: Mutex::new(None),
        snapshot: Mutex::new(Vec::new()),
        dirty,
        codex_ask,
        stopped,
    });
    let handle = UsageHandle { shared: shared.clone() };
    if !config.enabled {
        shared.stopped.send_replace(true);
        return handle;
    }
    let _ = std::fs::create_dir_all(&env.rotator_dir);
    let logger = Logger::new(server::log_path(&env.rotator_dir));
    let poll_ms: Ms = Ms::from(config.poll_minutes.max(1)) * 60_000;

    // The Claude side: our own monitor while we hold the lock, else the one that does. Off (`claude: false`) where
    // Fleetlayer's claude-auth-manager owns the sign-ins and the coordinator API.
    if config.claude {
        let (shared, config, env, logger) = (shared.clone(), config.clone(), env.clone(), logger.clone());
        tokio::spawn(async move {
            let mut stopped = shared.stopped.subscribe();
            let mut followed: Option<i32> = None;
            loop {
                // Once a followed coordinator stops, its sessions ask for one again within 2 s and would start
                // the rotator's own: take the lock first.
                let wait = match server::take_lock(&server::lock_path(&env.rotator_dir)) {
                    Ok(None) => return run_own(shared, &config, &env, logger).await,
                    Ok(Some(pid)) => {
                        // Said once per coordinator: one still starting, whose socket isn't up, is asked again soon.
                        if followed != Some(pid) {
                            logger.log(&format!("{NAME}: a coordinator runs (pid {pid}); following it until it stops"));
                            followed = Some(pid);
                        }
                        follow(shared.clone(), server::socket_path(&env.rotator_dir)).await;
                        Duration::from_millis(250)
                    }
                    Err(e) => {
                        logger.log(&format!("{NAME}: {e}"));
                        Duration::from_secs(5)
                    }
                };
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = stopped.wait_for(|s| *s) => return,
                }
            }
        });
    }

    if config.codex {
        let (shared2, logger) = (shared.clone(), logger.clone());
        let home = env.home.clone();
        tokio::spawn(async move {
            let shared3 = shared2.clone();
            let mut stopped = shared2.stopped.subscribe();
            let watching = codex::watch(
                home,
                poll_ms,
                || Timestamp::now().as_millisecond(),
                move |text| logger.log(text),
                move |state| {
                    *lock(&shared3.codex) = state;
                    shared3.dirty.send_modify(|v| *v += 1);
                },
                codex_asked,
            );
            tokio::select! {
                _ = watching => {}
                _ = stopped.wait_for(|s| *s) => {}
            }
        });
    }

    // Snapshots to the node, each time what they show changes; the module stops once the node stops listening.
    {
        let shared = shared.clone();
        let mut dirty = shared.dirty.subscribe();
        tokio::spawn(async move {
            loop {
                let next = snapshot_of(lock(&shared.claude_state).as_ref(), lock(&shared.codex).as_ref());
                let is_new = {
                    let mut last = lock(&shared.snapshot);
                    let is_new = *last != next;
                    *last = next.clone();
                    is_new
                };
                if is_new && events.send(UsageEvent::Snapshot(next)).await.is_err() {
                    break;
                }
                tokio::select! {
                    r = dirty.changed() => if r.is_err() { break },
                    _ = events.closed() => break,
                }
                // A burst of changes (a check touches several fields) goes out as one snapshot.
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            shared.stopped.send_replace(true);
        });
    }
    handle
}
