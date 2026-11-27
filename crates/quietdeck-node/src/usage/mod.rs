//! The usage module: monitors this machine's Codex subscription and pushes snapshots to the deck and the menu bar
//! app. The records crossing the boundary are in `quietdeck_types::usage`; the contract is `docs/usage-interface.md`.
//!
//! Claude sign-ins, their usage and the account rotator's coordinator API (`coordinator.sock`) are Fleetlayer's
//! claude-auth-manager plugin's: the node never reads a Claude sign-in, takes `coordinator.lock` or binds the socket.
//! node.yaml's old `usage.claude` and `usage.serve_coordinator_api` keys are still accepted and ignored.

pub mod codex;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jiff::Timestamp;
use quietdeck_types::usage::{
    AccountUsage, ReadingSource, UsageCommand, UsageConfig, UsageHealth, UsageProvider, UsageWindow, UsageWindowKind,
};
use tokio::sync::{mpsc, watch};

use self::codex::Ms;

/// Codex's `local_id`: Codex keeps one sign-in per machine.
pub const CODEX_ID: &str = "codex";

/// Machine-specific inputs that aren't user settings.
#[derive(Debug, Clone)]
pub struct UsageEnv {
    /// The user's home directory (`~/.codex`).
    pub home: PathBuf,
}

/// What the module tells the node.
#[derive(Debug, Clone, PartialEq)]
pub enum UsageEvent {
    /// Every subscription on this machine, whenever any of it changes.
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

struct Shared {
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
        match command {
            UsageCommand::SignIn { local_id: Some(id) } if id == CODEX_ID => {
                Err(UsageError::Other("Codex signs in with `codex login` in a terminal on this machine.".into()))
            }
            UsageCommand::SignIn { .. } => {
                Err(UsageError::Other("Claude sign-ins are managed by Fleetlayer's claude-auth-manager.".into()))
            }
            UsageCommand::Check { local_id: None } => {
                self.shared.codex_ask.send_modify(|v| *v += 1);
                Ok(())
            }
            UsageCommand::Check { local_id: Some(id) } if id == CODEX_ID => {
                self.shared.codex_ask.send_modify(|v| *v += 1);
                Ok(())
            }
            UsageCommand::Check { local_id: Some(id) } => Err(UsageError::UnknownAccount(id)),
        }
    }
}

fn timestamp(ms: Ms) -> Option<Timestamp> {
    Timestamp::from_millisecond(ms).ok()
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

/// Starts the module on the current tokio runtime. It runs until `events` closes.
pub fn spawn(config: UsageConfig, env: UsageEnv, events: mpsc::Sender<UsageEvent>) -> UsageHandle {
    let (dirty, _) = watch::channel(0);
    let (codex_ask, codex_asked) = watch::channel(0);
    let (stopped, _) = watch::channel(false);
    let shared =
        Arc::new(Shared { codex: Mutex::new(None), snapshot: Mutex::new(Vec::new()), dirty, codex_ask, stopped });
    let handle = UsageHandle { shared: shared.clone() };
    if !config.enabled {
        shared.stopped.send_replace(true);
        return handle;
    }
    let poll_ms: Ms = Ms::from(config.poll_minutes.max(1)) * 60_000;

    if config.codex {
        let shared2 = shared.clone();
        let home = env.home.clone();
        tokio::spawn(async move {
            let shared3 = shared2.clone();
            let mut stopped = shared2.stopped.subscribe();
            let watching = codex::watch(
                home,
                poll_ms,
                || Timestamp::now().as_millisecond(),
                |text| tracing::info!(target: "usage", "{text}"),
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
                let next: Vec<AccountUsage> = lock(&shared.codex).as_ref().map(codex_usage).into_iter().collect();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn claude_commands_are_refused_and_codex_checks_are_taken() {
        let (tx, _rx) = mpsc::channel(4);
        let config = UsageConfig { codex: false, ..UsageConfig::default() };
        let handle = spawn(config, UsageEnv { home: PathBuf::from("/nonexistent") }, tx);
        assert!(handle.command(UsageCommand::SignIn { local_id: None }).await.is_err());
        assert!(handle.command(UsageCommand::Check { local_id: Some("claude-1".into()) }).await.is_err());
        assert!(handle.command(UsageCommand::Check { local_id: Some(CODEX_ID.into()) }).await.is_ok());
        assert!(handle.snapshot().await.is_empty());
    }
}
