//! The runner's two counterparts: the deck (over the node's connection) and its agent (on this machine).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use quietdeck_types::card::{ActionRecord, ActionReport, Released};
use quietdeck_types::decision::ReportReceipt;
use quietdeck_types::directory::{SessionStart, SessionStarted};
use quietdeck_types::node::{Delivered, Handoff, NodeRequest};
use quietdeck_types::reading::{HoldVerdict, ItemFacts};
use quietdeck_types::relay::{Delivery, DeliveryKind, DeliveryTarget};

use crate::Node;

const ASK: Duration = Duration::from_secs(30);
/// Starting or resuming a session waits for the orchestrator and the agent to come up.
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
    /// While the person reads the Inbox: let each new item through now, or hold it? A deck that can't say lets
    /// everything through, as before reading mattered.
    async fn hold_check(&self, items: Vec<ItemFacts>) -> Result<Vec<HoldVerdict>, String> {
        Ok(items.iter().map(|_| HoldVerdict::Let { reason: String::new() }).collect())
    }
    /// What the runner holds now, for the Inbox's "waiting" line.
    async fn held(&self, _summary: Option<String>, _count: u64) -> Result<(), String> {
        Ok(())
    }
    /// Messages seen sent, for the Sent tab.
    async fn mail_sent(&self, _items: Vec<quietdeck_types::mail::MailItem>) -> Result<(), String> {
        Ok(())
    }
    /// Every draft on one account now, for the Drafts tab.
    async fn mail_drafts(&self, _account: String, _drafts: Vec<quietdeck_types::mail::MailItem>) -> Result<(), String> {
        Ok(())
    }
    /// Threads that left the inbox and threads that came back; the deck answers with the cards it resolved.
    async fn threads_left(
        &self,
        _left: Vec<quietdeck_types::mail::LeftThread>,
        _back: Vec<quietdeck_types::mail::LeftThread>,
    ) -> Result<Vec<quietdeck_types::mail::LeftCard>, String> {
        Ok(vec![])
    }
    /// The runner's health, after each mail check. A deck that doesn't take it is never sent it.
    async fn health(&self, _health: quietdeck_types::node::RunnerHealth) {}
}

#[async_trait]
pub trait AgentInbox: Send + Sync {
    /// Put a message in front of the agent now, or fail: nothing is queued.
    async fn tell(&self, kind: DeliveryKind, body: String) -> Result<(), String>;
    /// How the agent's last turns went, when the runner runs them itself (headless): (last good turn, last failure
    /// since then with its error). None for an agent in a tab.
    fn turns(&self) -> Option<AgentTurns> {
        None
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgentTurns {
    pub ok_at: Option<jiff::Timestamp>,
    pub failed: Option<(jiff::Timestamp, String)>,
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
        // Through Fleetlayer where it runs Quietdeck's plugin; the deck's relay otherwise (solo, older machines).
        if let Some(sent) = crate::fleetlayer::send(&crate::fleetlayer::Outgoing::of(&to, &kind, &body, false)).await {
            return sent.map_err(|e| e.to_string());
        }
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
    async fn hold_check(&self, items: Vec<ItemFacts>) -> Result<Vec<HoldVerdict>, String> {
        self.ask(NodeRequest::HoldCheck { items }, ASK).await
    }
    async fn held(&self, summary: Option<String>, count: u64) -> Result<(), String> {
        self.ask(NodeRequest::RunnerHeld { summary, count }, ASK).await
    }
    async fn mail_sent(&self, items: Vec<quietdeck_types::mail::MailItem>) -> Result<(), String> {
        self.ask(NodeRequest::MailSent { items }, ASK).await
    }
    async fn mail_drafts(&self, account: String, drafts: Vec<quietdeck_types::mail::MailItem>) -> Result<(), String> {
        self.ask(NodeRequest::MailDrafts { account, drafts }, ASK).await
    }
    async fn threads_left(
        &self,
        left: Vec<quietdeck_types::mail::LeftThread>,
        back: Vec<quietdeck_types::mail::LeftThread>,
    ) -> Result<Vec<quietdeck_types::mail::LeftCard>, String> {
        self.ask(NodeRequest::ThreadsLeftInbox { left, back }, ASK).await
    }
    async fn health(&self, health: quietdeck_types::node::RunnerHealth) {
        if self.0.link.deck_supports(quietdeck_types::node::FEATURE_RUNNER_HEALTH) {
            self.0.link.push(quietdeck_types::node::NodeFrame::RunnerHealth { health }).await;
        }
    }
}

/// The agent, by its tab on this machine: through Fleetlayer where it runs Quietdeck's plugin, else the node's own
/// delivery (its messaging socket, or the orchestrator).
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
        // Through Fleetlayer where it runs Quietdeck's plugin: to the tab on this machine, or to `<tab>@<node>` when
        // the agent runs on another node (a runner on its own Unix user, say).
        let local = !self.tab.contains('@');
        if let Some(sent) =
            crate::fleetlayer::send(&crate::fleetlayer::Outgoing::of(&self.tab, &kind, &body, local)).await
        {
            sent.map_err(|e| e.to_string())?;
            if let Some(s) = session {
                self.node.mark_working(&s.session_id).await;
            }
            return Ok(());
        }
        let s = session.ok_or_else(|| format!("{} isn't running on this machine", self.tab))?;
        let delivery = Delivery {
            id: format!("r_{}", hex::encode(rand::random::<[u8; 12]>())),
            target: DeliveryTarget {
                tab: self.tab.clone(),
                session_id: Some(s.session_id.clone()),
                address: s.address.clone(),
                agent: s.agent,
            },
            kind,
            body,
        };
        let id = delivery.target.session_id.clone();
        self.node.deliverer.deliver(&delivery).await?;
        if let Some(id) = id {
            self.node.mark_working(&id).await;
        }
        Ok(())
    }
}
