//! The transcript viewer: a session's transcript read on its machine, a page at a time, in the deck's terms. The deck
//! relays each read from the node and stores nothing. Claude Code transcripts only for now.

use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Which part of a transcript to read. With neither `before` nor `after`, the end of it, back to the person's last
/// real message.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct TranscriptQuery {
    /// A subagent's transcript (its ID from [`TranscriptPage::subagents`]) instead of the session's own.
    #[serde(default)]
    pub subagent: Option<String>,
    /// Items that start before this byte offset ([`TranscriptPage::earlier`]).
    #[serde(default)]
    #[ts(type = "number | null")]
    pub before: Option<u64>,
    /// Items that start at or after this byte offset ([`TranscriptPage::end`]): what's new since the last read.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub after: Option<u64>,
}

/// A run of a transcript's items, oldest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct TranscriptPage {
    pub session_id: String,
    pub subagent: Option<String>,
    pub items: Vec<TranscriptItem>,
    /// Where these items start: ask with `before` set to this for earlier ones. None at the start of the transcript.
    #[ts(type = "number | null")]
    pub earlier: Option<u64>,
    /// Where they end: ask with `after` set to this for what's new.
    #[ts(type = "number")]
    pub end: u64,
    /// The session's subagents, in the order they started.
    pub subagents: Vec<Subagent>,
}

/// One thing in a transcript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct TranscriptItem {
    /// The byte offset of the record it came from, with `part` its place in that record: together, a stable key.
    #[ts(type = "number")]
    pub offset: u64,
    pub part: u32,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub at: Option<Timestamp>,
    #[serde(flatten)]
    pub body: ItemBody,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ItemBody {
    /// A message the person typed (or a slash command they ran).
    Person { text: String },
    /// A message from another session.
    Peer { from: Option<String>, text: String },
    /// What the agent wrote, Markdown.
    Agent { text: String },
    /// The agent's thinking, when the transcript keeps it.
    Thinking { text: String },
    /// A tool call: one line, with its input and result to expand.
    Tool {
        id: Option<String>,
        name: String,
        line: String,
        input: String,
        result: Option<String>,
        error: bool,
        /// The subagent this call started, when it started one.
        subagent: Option<String>,
    },
    /// A tool call's result whose call is in an earlier page: the viewer adds it to that call.
    ToolResult { id: String, result: String, error: bool },
    /// Something typed into the session that isn't the person: a background task finishing, a watcher waking it,
    /// the orchestrator's own auto-continue.
    WakeUp { text: String },
    /// The conversation was compacted here: what came before is summarised in `summary`.
    Compaction { summary: Option<String> },
    /// The person interrupted the agent.
    Interrupted,
}

/// A subagent the session started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct Subagent {
    pub id: String,
    pub description: Option<String>,
    pub agent_type: Option<String>,
    /// 1 for one the session started, 2 for one a subagent started, and so on.
    pub depth: u32,
    /// The tool call that started it.
    pub tool_use_id: Option<String>,
}
