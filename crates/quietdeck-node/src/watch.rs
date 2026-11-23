//! herdr's sessions, live: the node follows herdr's event stream and pushes each change to the deck and the
//! summaries module as it happens.
//!
//! - Status changes come straight from `pane.agent_status_changed`.
//! - Close events (`tab.closed`, `pane.agent_exited`) say how a session ended.
//! - `agent list` is read again after a lifecycle event that changes it (a new agent, its session ID arriving, a
//!   rename or a move), every 30 seconds as a check, and whenever the stream starts again: events from a gap are
//!   never replayed.
//! - A session that goes away without a close event (herdr stopped, the machine shut down) is pushed as not running,
//!   which isn't the same as closed. herdr sends no close events while it stops.
//! - A herdr from before its client contract can't subscribe to every pane: the node reads `agent list` every 3
//!   seconds instead, and tries the stream again every minute.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use quietdeck_types::AgentStatus;
use quietdeck_types::directory::{SessionEnd, SessionInfo};
use quietdeck_types::node::NodeFrame;
use quietdeck_types::summary::{SessionFacts, SessionSignal};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::Node;
use crate::herdr::{self, Event, StreamError, Subscription};

const CHECK_EVERY: Duration = Duration::from_secs(30);
const POLL_EVERY: Duration = Duration::from_secs(3);
/// How long after a lifecycle event `agent list` is read, so a burst of events reads it once.
const SETTLE: Duration = Duration::from_millis(200);
/// herdr unreachable this long means it has stopped, and its sessions with it. A live handoff (`herdr restart`)
/// takes far less.
const DOWN_AFTER: Duration = Duration::from_secs(10);
/// Sessions recently pushed as not running: a close event that arrives just after still says how they ended.
const GONE_LIMIT: usize = 64;

pub async fn run(node: Arc<Node>, signals: mpsc::Sender<SessionSignal>) {
    let herdr = node.config.programs.herdr.clone();
    let mut w = Watcher { node, signals, gone: VecDeque::new() };
    let mut backoff = Duration::from_secs(1);
    let mut down_since: Option<Instant> = None;
    let mut warned_old = false;
    loop {
        let opened = match herdr::socket_path(&herdr).await {
            Ok(path) => Subscription::open(&path).await,
            Err(e) => Err(StreamError::Io(std::io::Error::other(e.to_string()))),
        };
        match opened {
            Ok(mut sub) => {
                tracing::info!("following herdr's events");
                backoff = Duration::from_secs(1);
                down_since = None;
                w.reconcile().await;
                w.follow(&mut sub).await;
                // The stream ended: herdr stopped, restarted or handed off. Read what's running now.
                w.reconcile().await;
            }
            Err(StreamError::Refused(msg)) => {
                if !warned_old {
                    tracing::warn!(%msg, "this herdr can't report every pane's status: reading it every 3 seconds \
                        until herdr is updated");
                    warned_old = true;
                }
                let until = Instant::now() + Duration::from_secs(60);
                while Instant::now() < until {
                    w.reconcile().await;
                    tokio::time::sleep(POLL_EVERY).await;
                }
            }
            Err(StreamError::Io(e)) => {
                tracing::debug!(error = %e, "herdr's event stream");
                let since = *down_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= DOWN_AFTER {
                    // herdr isn't running: nothing it ran is either.
                    w.apply_snapshot(Vec::new()).await;
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(10));
            }
        }
    }
}

struct Watcher {
    node: Arc<Node>,
    signals: mpsc::Sender<SessionSignal>,
    gone: VecDeque<String>,
}

impl Watcher {
    async fn follow(&mut self, sub: &mut Subscription) {
        let mut check = tokio::time::interval_at(Instant::now() + CHECK_EVERY, CHECK_EVERY);
        let mut due: Option<Instant> = None;
        loop {
            let settle = async {
                match due {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                event = sub.next() => match event {
                    Ok(Some(e)) => {
                        if self.apply(e).await {
                            due.get_or_insert_with(|| Instant::now() + SETTLE);
                        }
                    }
                    Ok(None) => {
                        tracing::info!("herdr closed its event stream");
                        return;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "reading herdr's event stream");
                        return;
                    }
                },
                _ = settle => {
                    due = None;
                    self.reconcile().await;
                }
                _ = check.tick() => self.reconcile().await,
            }
        }
    }

    /// Apply one event. True when `agent list` should be read again.
    async fn apply(&mut self, event: Event) -> bool {
        match event {
            Event::Status { pane, status } => {
                let (known, changed) = {
                    let mut sessions = self.node.sessions.lock().await;
                    match sessions.iter_mut().find(|s| s.pane.as_deref() == Some(pane.as_str())) {
                        Some(s) if s.status != status => {
                            s.status = status;
                            s.status_changed_at = jiff::Timestamp::now();
                            (true, Some(s.clone()))
                        }
                        Some(_) => (true, None),
                        None => (false, None),
                    }
                };
                if let Some(s) = changed {
                    self.pushed(&s).await;
                }
                // A pane the node has no session for: an agent that has just started (herdr doesn't always send
                // `pane_updated` when its session ID arrives), or a plain shell, which costs one `agent list`.
                !known
            }
            Event::TabClosed { sessions } => {
                for id in sessions {
                    self.closed(&id, SessionEnd::TabClosed).await;
                }
                false
            }
            Event::AgentExited { session: Some(id), end } => {
                self.closed(&id, end).await;
                false
            }
            Event::AgentExited { session: None, .. } | Event::Other => false,
            Event::PaneUpdated { pane, session: Some(id), tab, cwd } => {
                let sessions = self.node.sessions.lock().await;
                !sessions
                    .iter()
                    .any(|s| s.session_id == id && s.pane.as_deref() == Some(&pane) && s.tab == tab && s.cwd == cwd)
            }
            Event::PaneUpdated { session: None, .. } => false,
            Event::Changed => true,
        }
    }

    async fn closed(&mut self, session_id: &str, end: SessionEnd) {
        let known = {
            let mut sessions = self.node.sessions.lock().await;
            let before = sessions.len();
            sessions.retain(|s| s.session_id != session_id);
            sessions.len() != before
        };
        let recently_gone = self.gone.iter().any(|g| g == session_id);
        if !known && !recently_gone {
            return; // never pushed: the deck has no row for it
        }
        self.gone.retain(|g| g != session_id);
        let at = jiff::Timestamp::now();
        self.node
            .link
            .push(NodeFrame::SessionClosed { session_id: session_id.to_string(), at, ended: Some(end) })
            .await;
        if known {
            let _ = self.signals.send(SessionSignal::Closed { session_id: session_id.to_string(), at }).await;
        }
    }

    async fn reconcile(&mut self) {
        match herdr::snapshot(&self.node.config.programs.herdr).await {
            Ok(now) => self.apply_snapshot(now).await,
            Err(e) => tracing::debug!(error = %e, "herdr agent list"),
        }
    }

    async fn apply_snapshot(&mut self, now: Vec<SessionInfo>) {
        let before = std::mem::replace(&mut *self.node.sessions.lock().await, now.clone());
        for s in &now {
            let old = before.iter().find(|b| b.session_id == s.session_id);
            if old.is_none_or(|o| o.status != s.status || o.tab != s.tab || o.cwd != s.cwd || o.pane != s.pane) {
                self.gone.retain(|g| g != &s.session_id);
                self.pushed(s).await;
            }
        }
        for b in before.iter().filter(|b| !now.iter().any(|s| s.session_id == b.session_id)) {
            let at = jiff::Timestamp::now();
            let gone = SessionInfo { running: false, status: AgentStatus::Unknown, status_changed_at: at, ..b.clone() };
            self.node.link.push(NodeFrame::Session { session: gone }).await;
            let _ = self.signals.send(SessionSignal::Closed { session_id: b.session_id.clone(), at }).await;
            self.gone.push_back(b.session_id.clone());
            if self.gone.len() > GONE_LIMIT {
                self.gone.pop_front();
            }
        }
    }

    /// A running session changed: tell the deck and the summaries module.
    async fn pushed(&self, s: &SessionInfo) {
        self.node.link.push(NodeFrame::Session { session: s.clone() }).await;
        let facts = SessionFacts {
            session_id: s.session_id.clone(),
            agent: s.agent,
            tab: s.tab.clone(),
            cwd: s.cwd.clone(),
            transcript: None,
        };
        let _ = self
            .signals
            .send(SessionSignal::Status { session: facts, status: s.status, at: s.status_changed_at })
            .await;
    }
}
