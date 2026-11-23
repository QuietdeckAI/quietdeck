//! The summaries module: writes this machine's session summaries and catch-ups.
//!
//! This file is the boundary between the node's core (built by the quietdeck tab) and the module (ported from
//! claude-summarizer by the claude summarizer tab). The records crossing it are in `quietdeck_types::summary`; the
//! full contract is `docs/summaries-interface.md`.
//!
//! The node:
//! - builds a [`SummariesConfig`] from node.yaml's `summaries:` section and a [`SummariesEnv`];
//! - sends a [`SessionSignal::Status`] for every session in herdr's snapshot when it starts, then one per
//!   `pane.agent_status_changed` event, and a [`SessionSignal::Closed`] per close event;
//! - forwards every [`SummaryEvent`] the module sends to the deck, as a `SummaryPush` with its node name;
//! - calls [`SummariesHandle::catchup`] when the deck asks for a session's catch-up (the chat view's header).
//!
//! The module owns everything else: transcript discovery and offsets, the noise filter and the "worth a call"
//! threshold, the idle timer, the 30-minute gap, at most `max_concurrent` calls, the sweep, chunking, the cap, the
//! skips, choosing the account and running headless Claude Code, and its own state under `state_dir`.
//!
//! The bodies below are placeholders until the module lands; the signatures are the contract.

#![allow(dead_code)]

use std::path::PathBuf;

use quietdeck_types::summary::{Catchup, SessionSignal, SummariesConfig, SummaryEvent};
use tokio::sync::mpsc;

/// Machine-specific inputs that aren't user settings.
#[derive(Debug, Clone)]
pub struct SummariesEnv {
    /// The module's own state (offsets, last summary per session, failure counts, the catch-up cache). The node
    /// passes `<node state dir>/summaries`.
    pub state_dir: PathBuf,
    /// The user's home directory: transcripts are under `<home>/.claude/projects` and `<home>/.codex/sessions`.
    pub home: PathBuf,
    /// The `claude` binary for headless calls.
    pub claude_program: PathBuf,
    /// The account rotator's coordinator socket (`~/.claude/account-rotator/coordinator.sock`), for the "soonest
    /// reset" account. Once the node's usage module serves the same API on that socket, nothing here changes. None
    /// when no rotator is installed: calls use Claude Code's own login.
    pub coordinator_socket: Option<PathBuf>,
    /// The local time zone's IANA name, for hour, day and week periods.
    pub time_zone: String,
}

/// What the deck asked for: a catch-up for one session.
#[derive(Debug, Clone)]
pub struct CatchupRequest {
    pub session_id: String,
    /// The transcript, when the node has resolved it.
    pub transcript: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum SummariesError {
    #[error("no transcript found for session {0}")]
    NoTranscript(String),
    #[error("the summaries module has stopped")]
    Stopped,
    #[error("{0}")]
    Other(String),
}

/// A running summaries module.
#[derive(Clone)]
pub struct SummariesHandle {
    _private: (),
}

impl SummariesHandle {
    /// The session's catch-up. Answers at once from the cache, with `stale: true` and a regeneration started when
    /// the transcript has grown since; the regenerated one also arrives as [`SummaryEvent::Catchup`]. Writes one and
    /// waits for it only when nothing is cached.
    pub async fn catchup(&self, req: CatchupRequest) -> Result<Catchup, SummariesError> {
        Err(SummariesError::NoTranscript(req.session_id))
    }

    /// The one-off import of claude-summarizer's history (`index.json` and the Markdown updates under `dir`). Sends
    /// every record as [`SummaryEvent::Summary`] with `origin: claude_summarizer`, and returns how many it sent.
    pub async fn import_claude_summarizer(&self, dir: PathBuf) -> Result<usize, SummariesError> {
        let _ = dir;
        Ok(0)
    }
}

/// Starts the module on the current tokio runtime. It runs until `signals` closes.
pub fn spawn(
    config: SummariesConfig,
    env: SummariesEnv,
    signals: mpsc::Receiver<SessionSignal>,
    events: mpsc::Sender<SummaryEvent>,
) -> SummariesHandle {
    let _ = (config, env, signals, events);
    SummariesHandle { _private: () }
}
