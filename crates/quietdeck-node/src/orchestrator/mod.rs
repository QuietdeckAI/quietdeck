//! The orchestrator: the program that runs this machine's agent sessions in terminals (herdr's TUI today). Everything
//! the node does with it goes through [`Orchestrator`], so supporting another orchestrator means one more
//! implementation, and nothing about it reaches the deck, the CLI or the data model. A session's place in an
//! orchestrator is its *address*, a string only that orchestrator understands. `docs/orchestrators.md` describes the
//! interface for people writing an adapter.

pub mod herdr;

use async_trait::async_trait;
use quietdeck_types::AgentKind;
use quietdeck_types::AgentStatus;
use quietdeck_types::directory::{SessionEnd, SessionInfo};
use quietdeck_types::node::Delivered;

/// What the node needs from an orchestrator.
#[async_trait]
pub trait Orchestrator: Send + Sync {
    /// Its name, for logs and messages ("herdr").
    fn name(&self) -> &'static str;

    /// Every agent session it runs on this machine, with `address` set. Sessions without an ID yet (an agent still
    /// starting) are left out. An error means it isn't running.
    async fn sessions(&self) -> anyhow::Result<Vec<SessionInfo>>;

    /// Its live events. [`StreamError::Refused`] means it can't stream, and the node reads [`Orchestrator::sessions`]
    /// every few seconds instead; [`StreamError::Io`] means it isn't reachable now.
    async fn subscribe(&self) -> Result<Box<dyn EventStream>, StreamError>;

    /// Open a new tab running an agent, without taking focus from the person. Answers once the agent takes input, or
    /// with a sentence saying why it couldn't start.
    async fn open(&self, tab: &OpenTab) -> Result<Opened, String>;

    /// Close the tab a running session is in.
    async fn close(&self, session_id: &str) -> Result<(), String>;

    /// Interrupt a running session, as Esc does: it stops its turn and waits for input.
    async fn interrupt(&self, session_id: &str) -> Result<(), String>;

    /// Type a message into a session as if the person had. An error starting with
    /// [`crate::deliver::OFFLINE_PREFIX`] means the session isn't there.
    async fn deliver(&self, address: &str, text: &str) -> Result<Delivered, String>;

    /// Messages the orchestrator types into sessions by itself (herdr's auto-continue), which summaries mustn't count
    /// as the person's.
    fn own_messages(&self) -> Vec<String>;
}

/// An open event stream.
#[async_trait]
pub trait EventStream: Send {
    /// The next event, or None when the orchestrator closed the stream (it stopped or restarted).
    async fn next(&mut self) -> Result<Option<Event>, StreamError>;
}

/// One event, in the node's terms.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A session's agent changed status.
    Status { address: String, status: AgentStatus },
    /// A tab closed, with the sessions that were running in it.
    TabClosed { sessions: Vec<String> },
    /// An agent's process exited.
    AgentExited { session: Option<String>, end: SessionEnd },
    /// A session's record changed. The node reads the sessions again when it shows something it doesn't know yet.
    Updated { address: String, session: Option<String>, tab: Option<String>, cwd: Option<String> },
    /// Something the sessions list shows differently now.
    Changed,
    /// An event the node doesn't use.
    Other,
}

/// Why a subscription failed or ended.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    /// It can't stream every session's events.
    #[error("the orchestrator refused the subscription: {0}")]
    Refused(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// A tab to open with an agent in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenTab {
    pub label: String,
    /// An absolute directory that exists.
    pub cwd: String,
    pub agent: AgentKind,
    /// The agent's own arguments (`--name x --resume <id>`), after its program.
    pub args: Vec<String>,
}

/// A tab the orchestrator opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    pub address: String,
    /// When the agent had reported it already.
    pub session_id: Option<String>,
}
