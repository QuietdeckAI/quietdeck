//! The runner's two counterparts: the deck (over the node's connection) and its agent (on this machine).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use quietdeck_types::card::{ActionRecord, ActionReport, Released};
use quietdeck_types::decision::ReportReceipt;
use quietdeck_types::directory::{SessionStart, SessionStarted};
use quietdeck_types::node::{Delivered, Handoff, NodeRequest};
use quietdeck_types::relay::{Delivery, DeliveryKind, DeliveryTarget};

use crate::Node;

const ASK: Duration = Duration::from_secs(30);
/// Starting or resuming a session waits for herdr and the agent to come up.
const START: Duration = Duration::from_secs(160);

#[async_trait]
pub trait DeckApi: Send + Sync {
    async fn take(&self) -> Result<Vec<Released>, String>;
    async fn sort(&self, claims: Vec<i64>, handoffs: Vec<Handoff>) -> Result<Vec<i64>, String>;
    async fn start(&self, action_id: i64) -> Result<ActionRecord, String>;
    async fn report(&self, reports: Vec<ActionReport>) -> Result<Vec<ReportReceipt>, String>;
    async fn relay(&self, to: String, kind: DeliveryKind, body: String) -> Result<Delivered, String>;
    async fn start_session(&self, spec: SessionStart) -> Result<SessionStarted, String>;
    async fn resume_session(
        &self,
        node: String,
        session_id: String,
        tab: String,
        cwd: String,
    ) -> Result<SessionStarted, String>;
}

#[async_trait]
pub trait AgentInbox: Send + Sync {
    /// Put a message in front of the agent now, or fail: nothing is queued.
    async fn tell(&self, kind: DeliveryKind, body: String) -> Result<(), String>;
}

/// The deck, through this node's connection.
pub struct LinkDeck(pub Arc<Node>);

impl LinkDeck {
    async fn ask<T: serde::de::DeserializeOwned>(&self, req: NodeRequest, timeout: Duration) -> Result<T, String> {
        self.0.link.request(req, timeout).await?.into_result()
    }
}

#[async_trait]
impl DeckApi for LinkDeck {
    async fn take(&self) -> Result<Vec<Released>, String> {
        self.ask(NodeRequest::RunnerTake, ASK).await
    }
    async fn sort(&self, claims: Vec<i64>, handoffs: Vec<Handoff>) -> Result<Vec<i64>, String> {
        self.ask(NodeRequest::RunnerSort { claims, handoffs }, ASK).await
    }
    async fn start(&self, action_id: i64) -> Result<ActionRecord, String> {
        self.ask(NodeRequest::ActionStart { action_id }, ASK).await
    }
    async fn report(&self, reports: Vec<ActionReport>) -> Result<Vec<ReportReceipt>, String> {
        self.ask(NodeRequest::ActionsReport { reports }, ASK).await
    }
    async fn relay(&self, to: String, kind: DeliveryKind, body: String) -> Result<Delivered, String> {
        self.ask(NodeRequest::Relay { to, kind, body }, ASK).await
    }
    async fn start_session(&self, spec: SessionStart) -> Result<SessionStarted, String> {
        self.ask(NodeRequest::StartSession { spec }, START).await
    }
    async fn resume_session(
        &self,
        node: String,
        session_id: String,
        tab: String,
        cwd: String,
    ) -> Result<SessionStarted, String> {
        self.ask(NodeRequest::ResumeSession { node, session_id, tab, cwd }, START).await
    }
}

/// The agent, by its tab on this machine, through the node's own delivery (its messaging socket, or herdr).
pub struct LocalAgent {
    pub node: Arc<Node>,
    pub tab: String,
}

#[async_trait]
impl AgentInbox for LocalAgent {
    async fn tell(&self, kind: DeliveryKind, body: String) -> Result<(), String> {
        let session = {
            let sessions = self.node.sessions.lock().await;
            sessions.iter().find(|s| s.running && s.tab.as_deref() == Some(self.tab.as_str())).cloned()
        };
        let s = session.ok_or_else(|| format!("{} isn't running on this machine", self.tab))?;
        let delivery = Delivery {
            id: format!("r_{}", hex::encode(rand::random::<[u8; 12]>())),
            target: DeliveryTarget {
                tab: self.tab.clone(),
                session_id: Some(s.session_id.clone()),
                pane: s.pane.clone(),
                agent: s.agent,
            },
            kind,
            body,
        };
        self.node.deliverer.deliver(&delivery).await.map(|_| ())
    }
}
