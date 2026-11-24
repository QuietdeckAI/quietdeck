//! The node protocol (one WebSocket the node opens to the deck), the local socket protocol (the CLI, the MCP
//! headers helper and the menu bar app talking to their machine's node), and session tokens.
//!
//! Both directions carry requests with an `id` the answer echoes. Requests are never retried by the sender: a
//! delivery that fails fails at once, and a reconnect starts clean. A node remembers the IDs of recent deliveries
//! and refuses a repeat, so nothing is ever put in front of a session twice.

use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::action::{ActionType, SessionRef};
use crate::card::ActionReport;
use crate::directory::{SessionEnd, SessionInfo, SessionResume, SessionStart, TranscriptHit};
use crate::relay::{Delivery, DeliveryKind};
use crate::summary::{Catchup, SummaryEvent};
use crate::usage::{AccountUsage, UsageCommand};

/// Bumped when a frame changes incompatibly. The deck refuses a node with another version, with a message saying
/// which side to upgrade.
pub const PROTOCOL_VERSION: u32 = 1;

/// Node → deck.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum NodeFrame {
    /// The first frame after connecting.
    Hello {
        protocol: u32,
        version: String,
        runner: Option<RunnerInfo>,
    },
    /// Every session on the machine. Replaces what the deck has for this node.
    Sessions {
        sessions: Vec<SessionInfo>,
    },
    /// One session changed (herdr's status event, a new tab, a rename).
    Session {
        session: SessionInfo,
    },
    /// The tab closed or the agent's process exited (herdr's close events). A session that goes away without one
    /// (herdr stopped) comes as a `session` with `running: false` instead.
    SessionClosed {
        session_id: String,
        #[ts(type = "string")]
        at: Timestamp,
        #[serde(default)]
        ended: Option<SessionEnd>,
    },
    Summary {
        event: SummaryEvent,
    },
    Usage {
        accounts: Vec<AccountUsage>,
        #[ts(type = "string")]
        at: Timestamp,
    },
    Request {
        id: u64,
        request: NodeRequest,
    },
    Response {
        id: u64,
        reply: Reply,
    },
}

/// What a node with its runner on tells the deck.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct RunnerInfo {
    pub eligible: Vec<ActionType>,
    /// The session that gets wake-ups, hand-offs and error reports.
    pub agent_tab: String,
    pub accounts: Vec<String>,
}

/// Deck → node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum DeckFrame {
    /// The answer to `hello`: the name the deck knows this node by (from its token).
    Welcome {
        node: String,
    },
    /// The deck can't take this node (wrong protocol version). The node logs it and stops reconnecting.
    Refused {
        reason: String,
    },
    Request {
        id: u64,
        request: DeckRequest,
    },
    Response {
        id: u64,
        reply: Reply,
    },
}

/// What the node asks the deck.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "op", rename_all = "snake_case")]
#[ts(export)]
pub enum NodeRequest {
    /// A short-lived session token for a caller on this machine. `session` is None for a plain shell.
    MintToken { session: Option<TokenSession> },
    /// The runner: every released decision on its agent's cards that nobody has claimed or been handed yet.
    /// Answers `Vec<Released>`.
    RunnerTake,
    /// The runner's sorting of what it took: the decisions it carries out itself, and the ones it hands to the agent
    /// with a reason each. Answers `()`.
    RunnerSort { claims: Vec<i64>, handoffs: Vec<Handoff> },
    /// The runner, just before carrying an action out. Refused once Stop has been pressed. Answers the
    /// [`crate::card::ActionRecord`] as the deck has it now.
    ActionStart { action_id: i64 },
    /// The runner's results. Answers `Vec<ReportReceipt>`.
    ActionsReport { reports: Vec<ActionReport> },
    /// The runner: pass a forward (or another delivery) to a session by address, through the relay. Answers
    /// [`Delivered`].
    Relay { to: String, kind: DeliveryKind, body: String },
    /// The runner: an approved `new_session` on whichever machine it names (flow 3.15). Answers
    /// [`crate::directory::SessionStarted`].
    StartSession { spec: SessionStart },
    /// The runner: an approved `resume` of one session by its ID, on the machine it ran on. Answers
    /// [`crate::directory::SessionStarted`].
    ResumeSession { node: String, session_id: String, tab: String, cwd: String },
}

/// A decision the runner hands to the agent, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct Handoff {
    pub decision_id: i64,
    pub reason: String,
    /// Set when the reason is that the runner has no handler for this action type: the deck counts it, by type only.
    #[serde(default)]
    pub no_handler: Option<ActionType>,
}

/// Whether an address is one a mail account can send as, from the runner's send-as list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct SendAsCheck {
    pub allowed: bool,
    /// Every address the account can send as.
    pub addresses: Vec<String>,
}

/// The session a token speaks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct TokenSession {
    pub session_id: Option<String>,
    pub tab: Option<String>,
    #[serde(alias = "pane")]
    pub address: Option<String>,
}

/// What the deck asks a node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "op", rename_all = "snake_case")]
#[ts(export)]
pub enum DeckRequest {
    /// Put this in front of a session now. Answers `delivered`, or an error when the session is offline.
    Deliver {
        delivery: Delivery,
    },
    /// "Who has this ID?" over the raw transcripts on this machine.
    SearchTranscripts {
        query: String,
        limit: u32,
    },
    /// The catch-up for the transcript viewer.
    Catchup {
        session_id: String,
    },
    /// Part of a session's transcript for the viewer. Answers [`crate::transcript::TranscriptPage`].
    Transcript {
        session_id: String,
        query: crate::transcript::TranscriptQuery,
    },
    Usage {
        command: UsageCommand,
    },
    /// Start a session in a new tab. Answers [`crate::directory::SessionStarted`].
    StartSession {
        spec: SessionStart,
    },
    /// Resume a session in a new tab. Answers [`crate::directory::SessionStarted`].
    ResumeSession {
        spec: SessionResume,
    },
    /// Close a running session's tab.
    CloseSession {
        session_id: String,
    },
    /// Interrupt a running session, as Esc does in Claude Code: it stops what it's doing and waits for input.
    InterruptSession {
        session_id: String,
    },
    /// To the runner: decisions on its agent's cards were released. It takes them with `runner_take`.
    DecisionsReleased,
    /// To the runner: carry out a lease's create or remove step. Answers the kind's state (`{filter_id, ...}`) for
    /// create, anything for remove.
    LeaseStep {
        lease: Box<crate::lease::Lease>,
        step: crate::lease::LeaseStep,
    },
    /// To the runner: its agent finished its round (`quietdeck round done`), so the next wake-up can go.
    RoundDone,
    /// To the runner: where its mail watch is. Answers [`crate::api::RoundStatus`].
    RoundStatus,
    /// To the runner: may this account send as `from`? Answers [`SendAsCheck`].
    SendAs {
        account: String,
        from: String,
    },
}

/// The answer to a request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "snake_case")]
#[ts(export)]
pub enum Reply {
    Ok {
        #[ts(type = "unknown")]
        value: serde_json::Value,
    },
    Err {
        error: String,
    },
}

impl Reply {
    pub fn ok<T: Serialize>(value: &T) -> Reply {
        match serde_json::to_value(value) {
            Ok(value) => Reply::Ok { value },
            Err(e) => Reply::Err { error: format!("serialising the answer: {e}") },
        }
    }

    pub fn into_result<T: serde::de::DeserializeOwned>(self) -> Result<T, String> {
        match self {
            Reply::Ok { value } => serde_json::from_value(value).map_err(|e| format!("reading the answer: {e}")),
            Reply::Err { error } => Err(error),
        }
    }
}

/// The token the deck minted, as the node caches it and hands it on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct MintedToken {
    pub token: String,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub expires_at: Timestamp,
    /// Who it speaks for, as the deck will name the caller.
    pub caller: SessionRef,
}

/// What the deck reads from a session token. The node never sees inside it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenClaims {
    pub node: String,
    pub session_id: Option<String>,
    pub tab: Option<String>,
    #[serde(alias = "pane")]
    pub address: Option<String>,
    /// Unix seconds.
    pub exp: i64,
}

/// Answers for deliveries and searches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Delivered {
    /// Handed to the session's own messaging socket.
    Socket,
    /// Sent through the orchestrator (herdr: `herdr agent prompt`).
    #[serde(alias = "herdr")]
    Orchestrator,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TranscriptSearch {
    pub hits: Vec<TranscriptHit>,
    /// Set when the search ran out of time: transcripts last written before this weren't searched.
    #[ts(type = "string | null")]
    #[serde(default)]
    pub searched_back_to: Option<jiff::Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CatchupReply {
    pub catchup: Catchup,
}

/// One line on the node's local socket, from the CLI, the MCP headers helper or the menu bar app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum LocalRequest {
    /// A session token for the caller. The node checks the claimed Claude Code process against the connecting
    /// process's ancestry before naming the session.
    Token {
        /// `CLAUDE_CODE_SESSION_ID`
        session_id: Option<String>,
        /// `CLAUDE_PID`
        claude_pid: Option<u32>,
        /// The session's address, from the environment variable its orchestrator sets
        /// ([`crate::SESSION_ADDRESS_ENV`]).
        #[serde(alias = "pane")]
        address: Option<String>,
    },
    /// The node's own state, for `quietdeck ping`-style checks and the menu bar app.
    Status,
    /// The latest usage readings.
    Usage,
    /// The one-off import of claude-summarizer's history into the running node's summaries module.
    ImportSummaries { dir: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LocalResponse {
    Token { token: String, deck_url: String, expires_at: Timestamp, caller: SessionRef },
    Status(NodeStatus),
    Usage { accounts: Vec<AccountUsage> },
    Imported { records: usize },
    Err { error: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NodeStatus {
    pub name: String,
    pub deck_url: String,
    pub connected: bool,
    pub version: String,
    pub runner: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_tagged() {
        let f = NodeFrame::Request { id: 3, request: NodeRequest::MintToken { session: None } };
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(v["type"], "request");
        assert_eq!(v["request"]["op"], "mint_token");
        let back: NodeFrame = serde_json::from_value(v).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn replies_carry_values_or_errors() {
        let r = Reply::ok(&Delivered::Socket);
        assert_eq!(r.clone().into_result::<Delivered>().unwrap(), Delivered::Socket);
        let e = Reply::Err { error: "api@laptop is offline".into() };
        assert_eq!(e.into_result::<Delivered>().unwrap_err(), "api@laptop is offline");
    }
}
