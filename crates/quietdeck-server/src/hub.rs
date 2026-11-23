//! Connected nodes and requests to them.
//!
//! A request goes to the node's live connection and waits for its answer. Nothing is queued for a node that isn't
//! connected, and nothing is retried: the caller gets "offline" at once.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use quietdeck_types::node::{DeckFrame, DeckRequest, Reply};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::db::now_ms;

/// A node counts as online for display until it has been disconnected this long (a sleeping laptop, a restart).
pub const OFFLINE_AFTER_MS: i64 = 60_000;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HubError {
    #[error("node {0} is offline")]
    Offline(String),
    #[error("node {0} didn't answer in time")]
    Timeout(String),
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Reply>>>>;

struct Conn {
    conn_id: u64,
    tx: mpsc::Sender<DeckFrame>,
    pending: Pending,
}

#[derive(Default)]
pub struct Hub {
    conns: Mutex<HashMap<String, Conn>>,
    disconnected_at: Mutex<HashMap<String, i64>>,
    next_id: AtomicU64,
}

/// What a connection's reader needs to answer requests and to unregister itself.
pub struct Registration {
    pub conn_id: u64,
    pub rx: mpsc::Receiver<DeckFrame>,
    pub tx: mpsc::Sender<DeckFrame>,
    pending: Pending,
}

impl Registration {
    /// A node answered one of the deck's requests.
    pub async fn resolve(&self, id: u64, reply: Reply) {
        if let Some(waiter) = self.pending.lock().await.remove(&id) {
            let _ = waiter.send(reply);
        }
    }
}

impl Hub {
    pub fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// A node connected. A newer connection replaces an older one: the older one's requests fail as offline.
    pub async fn register(&self, node: &str) -> Registration {
        let (tx, rx) = mpsc::channel(256);
        let conn_id = self.next_id();
        let pending: Pending = Arc::default();
        let old = self
            .conns
            .lock()
            .await
            .insert(node.to_string(), Conn { conn_id, tx: tx.clone(), pending: pending.clone() });
        if let Some(old) = old {
            old.pending.lock().await.clear();
        }
        self.disconnected_at.lock().await.remove(node);
        Registration { conn_id, rx, tx, pending }
    }

    pub async fn unregister(&self, node: &str, conn_id: u64) {
        let mut conns = self.conns.lock().await;
        if conns.get(node).is_some_and(|c| c.conn_id == conn_id) {
            if let Some(c) = conns.remove(node) {
                c.pending.lock().await.clear();
            }
            self.disconnected_at.lock().await.insert(node.to_string(), now_ms());
        }
    }

    /// Connected right now: the relay's test.
    pub async fn is_connected(&self, node: &str) -> bool {
        self.conns.lock().await.contains_key(node)
    }

    /// Online for display: connected, or disconnected less than a minute ago.
    pub async fn is_online(&self, node: &str) -> bool {
        if self.is_connected(node).await {
            return true;
        }
        self.disconnected_at.lock().await.get(node).is_some_and(|t| now_ms() - t < OFFLINE_AFTER_MS)
    }

    pub async fn connected_nodes(&self) -> Vec<String> {
        let mut v: Vec<String> = self.conns.lock().await.keys().cloned().collect();
        v.sort();
        v
    }

    /// Ask a node something and wait for the answer.
    pub async fn request(&self, node: &str, request: DeckRequest, timeout: Duration) -> Result<Reply, HubError> {
        let (id, rx) = {
            let conns = self.conns.lock().await;
            let conn = conns.get(node).ok_or_else(|| HubError::Offline(node.to_string()))?;
            let id = self.next_id();
            let (tx, rx) = oneshot::channel();
            conn.pending.lock().await.insert(id, tx);
            if conn.tx.try_send(DeckFrame::Request { id, request }).is_err() {
                conn.pending.lock().await.remove(&id);
                return Err(HubError::Offline(node.to_string()));
            }
            (id, rx)
        };
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(reply)) => Ok(reply),
            // The connection went away while we waited.
            Ok(Err(_)) => Err(HubError::Offline(node.to_string())),
            Err(_) => {
                if let Some(conn) = self.conns.lock().await.get(node) {
                    conn.pending.lock().await.remove(&id);
                }
                Err(HubError::Timeout(node.to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quietdeck_types::node::DeckRequest;

    #[tokio::test]
    async fn a_request_to_a_missing_node_fails_at_once() {
        let hub = Hub::default();
        let r = hub
            .request("laptop", DeckRequest::SearchTranscripts { query: "x".into(), limit: 1 }, Duration::from_secs(5))
            .await;
        assert_eq!(r.unwrap_err(), HubError::Offline("laptop".into()));
    }

    #[tokio::test]
    async fn a_request_gets_its_answer() {
        let hub = Arc::new(Hub::default());
        let mut reg = hub.register("laptop").await;
        let h = hub.clone();
        let asker = tokio::spawn(async move {
            h.request("laptop", DeckRequest::SearchTranscripts { query: "x".into(), limit: 1 }, Duration::from_secs(5))
                .await
        });
        let DeckFrame::Request { id, .. } = reg.rx.recv().await.unwrap() else { panic!() };
        reg.resolve(id, Reply::Err { error: "nope".into() }).await;
        assert_eq!(asker.await.unwrap().unwrap(), Reply::Err { error: "nope".into() });
    }

    #[tokio::test]
    async fn waiters_fail_as_offline_when_the_node_disconnects() {
        let hub = Arc::new(Hub::default());
        let mut reg = hub.register("laptop").await;
        let h = hub.clone();
        let asker = tokio::spawn(async move {
            h.request("laptop", DeckRequest::SearchTranscripts { query: "x".into(), limit: 1 }, Duration::from_secs(5))
                .await
        });
        let _ = reg.rx.recv().await.unwrap();
        hub.unregister("laptop", reg.conn_id).await;
        assert_eq!(asker.await.unwrap().unwrap_err(), HubError::Offline("laptop".into()));
        assert!(!hub.is_connected("laptop").await);
        assert!(hub.is_online("laptop").await, "still shown online for a minute");
    }
}
