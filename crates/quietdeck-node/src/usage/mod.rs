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
//! The bodies below are placeholders until the module lands; the signatures are the contract.

#![allow(dead_code)]

use std::path::PathBuf;

use quietdeck_types::usage::{AccountUsage, UsageCommand, UsageConfig};
use tokio::sync::mpsc;

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

/// A running usage module.
#[derive(Clone)]
pub struct UsageHandle {
    _private: (),
}

impl UsageHandle {
    /// The latest snapshot, for the menu bar app and a deck that just reconnected.
    pub async fn snapshot(&self) -> Vec<AccountUsage> {
        Vec::new()
    }

    /// Carries out a command from the deck or the menu bar app.
    pub async fn command(&self, command: UsageCommand) -> Result<(), UsageError> {
        let _ = command;
        Err(UsageError::Stopped)
    }
}

/// Starts the module on the current tokio runtime, including the coordinator API on `rotator_dir/coordinator.sock`
/// when `serve_coordinator_api` is on. It runs until `events` closes.
pub fn spawn(config: UsageConfig, env: UsageEnv, events: mpsc::Sender<UsageEvent>) -> UsageHandle {
    let _ = (config, env, events);
    UsageHandle { _private: () }
}
