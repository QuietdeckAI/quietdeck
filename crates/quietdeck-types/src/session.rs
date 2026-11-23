//! Sessions: one agent conversation (a Claude Code or Codex session) on one node, usually in a herdr pane.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Which harness runs the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AgentKind {
    Claude,
    Codex,
}

/// herdr's `agent_status`, as each node forwards it.
///
/// - `idle`: waiting for input, and the person has seen its last turn.
/// - `done`: idle, with a finished turn nobody has looked at yet (herdr's blue dot).
/// - `working`: running a turn.
/// - `blocked`: waiting on the person: a permission prompt, a question (herdr's red dot).
/// - `unknown`: herdr can't tell, or the agent isn't running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AgentStatus {
    Idle,
    Done,
    Working,
    Blocked,
    Unknown,
}

impl AgentStatus {
    /// Idle for summaries and the relay: `idle` or `done`. A `blocked` session is waiting on the person, never idle.
    pub fn is_idle(self) -> bool {
        matches!(self, AgentStatus::Idle | AgentStatus::Done)
    }
}

/// A session anywhere: the node it runs on and the harness's own session ID (Claude Code's `sessionId`, the
/// `agent_session.value` herdr reports). The deck's key for a session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct SessionKey {
    pub node: String,
    pub session_id: String,
}
