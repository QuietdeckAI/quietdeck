//! Moving a session to another machine (the `move-session` skill). The old session writes a handoff, everything the
//! new session needs in its context, and an access manifest, everything it needs to reach. The deck keeps both, a
//! session started from the handoff (`quietdeck session start --handoff h_3`) continues the old one, and the old
//! session can then be closed. Its transcript stays on its machine and in the owner's archive.

use garde::Validate;
use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::action::SessionRef;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct SessionHandoff {
    pub id: i64,
    /// The session that wrote it: the one being moved.
    pub from: SessionRef,
    /// Markdown: what the session is for, where it's up to, open items, decisions and why, conventions and gotchas,
    /// and pointers.
    pub handoff: String,
    /// Markdown: repositories, CLIs, each credential and where it comes from, hosts and networks, MCP servers,
    /// watchers and timers.
    pub manifest: String,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub created_at: Timestamp,
    /// The session started from it, once there is one.
    pub continued_by: Option<SessionRef>,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub continued_at: Option<Timestamp>,
}

/// `quietdeck handoff add`: written by the session being moved, about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct SessionHandoffAdd {
    #[garde(length(min = 1, max = 100_000))]
    pub handoff: String,
    #[garde(length(min = 1, max = 50_000))]
    pub manifest: String,
}

/// One end of a move, as `sessions show` prints it: the session on the other end and the handoff between them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct MoveLink {
    pub handoff_id: i64,
    pub session: SessionRef,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub at: Timestamp,
}
