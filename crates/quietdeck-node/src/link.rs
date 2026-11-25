//! The node's one WebSocket to the deck: it connects out, says hello, pushes sessions, summaries and usage, and
//! answers the deck's requests. It reconnects with backoff, and starts clean each time.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use quietdeck_types::node::{CatchupReply, DeckFrame, DeckRequest, NodeFrame, NodeRequest, PROTOCOL_VERSION, Reply};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::Node;

/// "0.1.0 9ab8b00": the version and the release's commit (scripts/build-images sets it), as Settings' Machines list
/// shows it.
pub fn version() -> String {
    match option_env!("QUIETDECK_COMMIT").filter(|c| !c.is_empty()) {
        Some(commit) => format!("{} {commit}", env!("CARGO_PKG_VERSION")),
        None => env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// Summary events wait here while the deck is unreachable (a sleeping laptop, a deck restart), up to this many.
const OUTBOX_LIMIT: usize = 10_000;

#[derive(Default)]
pub struct Link {
    tx: Mutex<Option<mpsc::Sender<NodeFrame>>>,
    outbox: Mutex<std::collections::VecDeque<NodeFrame>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Reply>>>,
    next: AtomicU64,
    connected: AtomicBool,
}

impl Link {
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    /// Send a frame if connected. Pushes made while disconnected are dropped: the next connection sends a fresh
    /// snapshot.
    pub async fn push(&self, frame: NodeFrame) {
        if let Some(tx) = self.tx.lock().await.as_ref() {
            let _ = tx.send(frame).await;
        }
    }

    /// Send a frame that mustn't be lost (a summary): held while disconnected and sent after the next snapshot.
    pub async fn push_kept(&self, frame: NodeFrame) {
        let tx = self.tx.lock().await.clone();
        if let Some(tx) = tx
            && tx.send(frame.clone()).await.is_ok()
        {
            return;
        }
        let mut outbox = self.outbox.lock().await;
        if outbox.len() >= OUTBOX_LIMIT {
            outbox.pop_front();
            tracing::warn!("summary outbox full: dropping the oldest");
        }
        outbox.push_back(frame);
    }

    /// Ask the deck something and wait for its answer.
    pub async fn request(&self, request: NodeRequest, timeout: Duration) -> Result<Reply, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        let sent = match self.tx.lock().await.as_ref() {
            Some(t) => t.send(NodeFrame::Request { id, request }).await.is_ok(),
            None => false,
        };
        if !sent {
            self.pending.lock().await.remove(&id);
            return Err("the node isn't connected to the deck".into());
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(_)) => Err("the deck connection dropped".into()),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err("the deck didn't answer in time".into())
            }
        }
    }
}

fn ws_url(deck: &str) -> String {
    let base = deck.trim_end_matches('/');
    let base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    };
    format!("{base}/api/nodes/ws")
}

/// Connect, serve, reconnect: forever, unless the deck refuses this node's protocol.
pub async fn run(node: Arc<Node>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        match connect_once(&node).await {
            Ok(Outcome::Refused(reason)) => {
                tracing::error!(%reason, "the deck refused this node; not reconnecting");
                return;
            }
            Ok(Outcome::Closed) => {
                tracing::info!("deck connection closed");
                backoff = Duration::from_secs(1);
            }
            Err(e) => tracing::warn!(error = %e, "deck connection failed"),
        }
        node.link.connected.store(false, Ordering::SeqCst);
        *node.link.tx.lock().await = None;
        node.link.pending.lock().await.clear();
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

enum Outcome {
    Closed,
    Refused(String),
}

async fn connect_once(node: &Arc<Node>) -> anyhow::Result<Outcome> {
    let token = node.config.node_token()?;
    let mut req = ws_url(&node.config.deck_url()).into_client_request()?;
    req.headers_mut().insert("Authorization", format!("Bearer {token}").parse()?);
    let (ws, _) = tokio::time::timeout(Duration::from_secs(15), tokio_tungstenite::connect_async(req))
        .await
        .map_err(|_| anyhow::anyhow!("timed out connecting to the deck"))??;
    let (mut sink, mut stream) = ws.split();

    let runner = node.runner.get().map(|r| r.info());
    let hello = NodeFrame::Hello { protocol: PROTOCOL_VERSION, version: version(), runner };
    sink.send(Message::Text(serde_json::to_string(&hello)?.into())).await?;
    match tokio::time::timeout(Duration::from_secs(10), stream.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => match serde_json::from_str::<DeckFrame>(&t)? {
            DeckFrame::Welcome { node: name } => {
                tracing::info!(node = %name, "connected to the deck");
                // Decisions released while this node was away are waiting.
                if let Some(r) = node.runner.get() {
                    r.signal(crate::runner::Signal::Released);
                }
            }
            DeckFrame::Refused { reason } => return Ok(Outcome::Refused(reason)),
            other => anyhow::bail!("expected welcome, got {other:?}"),
        },
        _ => anyhow::bail!("the deck didn't welcome this node"),
    }

    let (tx, mut rx) = mpsc::channel::<NodeFrame>(256);
    *node.link.tx.lock().await = Some(tx.clone());
    node.link.connected.store(true, Ordering::SeqCst);

    // A fresh snapshot each time: the deck may have missed pushes while we were away.
    let sessions = node.sessions.lock().await.clone();
    let _ = tx.send(NodeFrame::Sessions { sessions }).await;
    if let Some(usage) = &node.usage {
        let accounts = usage.snapshot().await;
        if !accounts.is_empty() {
            let _ = tx.send(NodeFrame::Usage { accounts, at: jiff::Timestamp::now() }).await;
        }
    }

    let held: Vec<NodeFrame> = node.link.outbox.lock().await.drain(..).collect();
    if !held.is_empty() {
        tracing::info!(count = held.len(), "sending summaries held while disconnected");
    }
    for frame in held {
        let _ = tx.send(frame).await;
    }

    loop {
        tokio::select! {
            out = rx.recv() => {
                let Some(frame) = out else { return Ok(Outcome::Closed) };
                sink.send(Message::Text(serde_json::to_string(&frame)?.into())).await?;
            }
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Text(t))) => match serde_json::from_str::<DeckFrame>(&t) {
                    Ok(DeckFrame::Request { id, request }) => {
                        let node = node.clone();
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            let reply = answer(&node, request).await;
                            let _ = tx.send(NodeFrame::Response { id, reply }).await;
                        });
                    }
                    Ok(DeckFrame::Response { id, reply }) => {
                        if let Some(w) = node.link.pending.lock().await.remove(&id) {
                            let _ = w.send(reply);
                        }
                    }
                    Ok(DeckFrame::Refused { reason }) => return Ok(Outcome::Refused(reason)),
                    Ok(DeckFrame::Welcome { .. }) => {}
                    Err(e) => tracing::warn!(error = %e, "unreadable frame from the deck"),
                },
                Some(Ok(Message::Ping(p))) => sink.send(Message::Pong(p)).await?,
                Some(Ok(Message::Close(_))) | None => return Ok(Outcome::Closed),
                Some(Err(e)) => return Err(e.into()),
                Some(Ok(_)) => {}
            }
        }
    }
}

async fn answer(node: &Node, request: DeckRequest) -> Reply {
    match request {
        DeckRequest::Deliver { delivery } => match node.deliverer.deliver(&delivery).await {
            Ok(how) => {
                if let Some(id) = &delivery.target.session_id {
                    node.mark_working(id).await;
                }
                Reply::ok(&how)
            }
            Err(error) => Reply::Err { error },
        },
        DeckRequest::SearchTranscripts { query, limit } => {
            Reply::ok(&crate::transcripts::search(node.home.clone(), query, limit.min(500) as usize).await)
        }
        DeckRequest::Catchup { session_id } => match &node.summaries {
            Some(s) => match s.catchup(crate::summaries::CatchupRequest { session_id, transcript: None }).await {
                Ok(catchup) => Reply::ok(&CatchupReply { catchup }),
                Err(e) => Reply::Err { error: e.to_string() },
            },
            None => Reply::Err { error: "summaries are off on this node".into() },
        },
        DeckRequest::Transcript { session_id, query } => {
            match crate::viewer::page(node.home.clone(), session_id, query).await {
                Ok(page) => Reply::ok(&page),
                Err(error) => Reply::Err { error },
            }
        }
        DeckRequest::StartSession { spec } => match crate::control::start(node, spec).await {
            Ok(started) => Reply::ok(&started),
            Err(error) => Reply::Err { error },
        },
        DeckRequest::ResumeSession { spec } => match crate::control::resume(node, spec).await {
            Ok(started) => Reply::ok(&started),
            Err(error) => Reply::Err { error },
        },
        DeckRequest::CloseSession { session_id } => match crate::control::close(node, &session_id).await {
            Ok(()) => Reply::ok(&serde_json::json!({})),
            Err(error) => Reply::Err { error },
        },
        DeckRequest::InterruptSession { session_id } => match crate::control::interrupt(node, &session_id).await {
            Ok(()) => Reply::ok(&serde_json::json!({})),
            Err(error) => Reply::Err { error },
        },
        DeckRequest::DecisionsReleased => match node.runner.get() {
            Some(r) => {
                r.signal(crate::runner::Signal::Released);
                Reply::ok(&())
            }
            None => Reply::Err { error: "this node's runner is off".into() },
        },
        DeckRequest::LeaseStep { lease, step } => match node.runner.get() {
            Some(r) => match r.runner.lease_step(&lease, step).await {
                Ok(state) => Reply::ok(&state),
                Err(error) => Reply::Err { error },
            },
            None => Reply::Err { error: "this node's runner is off".into() },
        },
        DeckRequest::RoundDone => match node.runner.get() {
            Some(r) => {
                r.signal(crate::runner::Signal::RoundDone);
                Reply::ok(&())
            }
            None => Reply::Err { error: "this node's runner is off".into() },
        },
        DeckRequest::MessagesArrived { channels } => match node.runner.get() {
            Some(r) => {
                // Answered at once: a wake-up can take a while to reach the agent.
                let runner = r.runner.clone();
                tokio::spawn(async move { runner.messages_arrived(&channels).await });
                Reply::ok(&())
            }
            None => Reply::Err { error: "this node's runner is off".into() },
        },
        DeckRequest::Presence { active } => match node.runner.get() {
            Some(r) => {
                let runner = r.runner.clone();
                tokio::spawn(async move { runner.set_reading(active).await });
                Reply::ok(&())
            }
            None => Reply::Err { error: "this node's runner is off".into() },
        },
        DeckRequest::ReleaseHeld => match node.runner.get() {
            Some(r) => {
                let runner = r.runner.clone();
                tokio::spawn(async move { runner.release_held().await });
                Reply::ok(&())
            }
            None => Reply::Err { error: "this node's runner is off".into() },
        },
        DeckRequest::RoundStatus => match node.runner.get() {
            Some(r) => Reply::ok(&r.runner.round_status().await),
            None => Reply::Err { error: "this node's runner is off".into() },
        },
        DeckRequest::SendAs { account, from } => match node.runner.get() {
            Some(r) => match r.runner.send_as_check(&account, &from).await {
                Ok(c) => Reply::ok(&c),
                Err(error) => Reply::Err { error },
            },
            None => Reply::Err { error: "this node's runner is off".into() },
        },
        DeckRequest::Usage { command } => match &node.usage {
            Some(u) => match u.command(command).await {
                Ok(()) => Reply::ok(&serde_json::json!({})),
                Err(e) => Reply::Err { error: e.to_string() },
            },
            None => Reply::Err { error: "usage monitoring is off on this node".into() },
        },
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn websocket_urls() {
        assert_eq!(super::ws_url("https://deck.example.ts.net/"), "wss://deck.example.ts.net/api/nodes/ws");
        assert_eq!(super::ws_url("http://127.0.0.1:8080"), "ws://127.0.0.1:8080/api/nodes/ws");
    }
}
