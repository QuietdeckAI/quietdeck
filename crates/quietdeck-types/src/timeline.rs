//! The automatic-actions timeline, History, pings, thread watches and webhook endpoints.

use garde::Validate;
use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::action::SessionRef;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TimelineKind {
    AutoAction,
    SentForwarded,
    /// The four lease kinds are retired with the deck's leases (they're Fleetlayer's now): kept for entries stored
    /// before.
    LeaseStarted,
    LeaseAbsorbed,
    LeaseExpired,
    LeaseCleanupOverdue,
    FilterWithoutProposal,
    SessionStarted,
    SessionResumed,
    SessionClosed,
    WebhookDelivered,
    GrantArchived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum EntrySource {
    Runner,
    Agent,
    Deck,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct TimelineEntry {
    pub id: i64,
    pub account: Option<String>,
    pub kind: TimelineKind,
    pub source: EntrySource,
    pub refs: Vec<String>,
    pub text: String,
    pub count: i64,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub created_at: Timestamp,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub read_at: Option<Timestamp>,
}

/// `quietdeck timeline add`: agents post automatic actions they carried out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct TimelineAdd {
    #[garde(skip)]
    pub kind: TimelineKind,
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 100)))]
    pub account: Option<String>,
    #[serde(default)]
    #[garde(length(max = 50), inner(length(min = 1, max = 300)))]
    pub refs: Vec<String>,
    #[garde(length(min = 1, max = 2000))]
    pub text: String,
}

/// `quietdeck history --since`: what happened since the calling session last looked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct History {
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub since: Timestamp,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub until: Timestamp,
    pub entries: Vec<TimelineEntry>,
    /// Cards whose state or actions changed in that time.
    pub cards: Vec<crate::card::Card>,
    /// The person's comments on this session's timeline entries and History rows, not picked up before. Empty on
    /// the History screen.
    #[serde(default)]
    pub comments: Vec<Comment>,
    /// The Inbox document as it was at each Approve All in that time. Only on the History screen.
    #[serde(default)]
    pub snapshots: Vec<crate::report::ReportSnapshotSummary>,
}

/// What a comment is on: a timeline entry, or a card in History.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
#[ts(export)]
pub enum CommentTarget {
    Timeline(i64),
    Card(i64),
}

/// The person comments on a timeline entry or a History row. It reaches the agent like an adjustment, and anything
/// it proposes comes back as a card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct CommentAdd {
    #[garde(skip)]
    pub target: CommentTarget,
    #[garde(length(min = 1, max = 4000))]
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct Comment {
    pub id: i64,
    pub target: CommentTarget,
    /// What it's on, in a line: the entry's text, or the card's subject.
    pub on: String,
    pub text: String,
    /// The session it went to.
    pub to: Option<SessionRef>,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub created_at: Timestamp,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub picked_up_at: Option<Timestamp>,
}

/// `quietdeck ping`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct PingRequest {
    #[garde(length(min = 1, max = 200))]
    pub title: String,
    #[garde(length(min = 1, max = 2000))]
    pub body: String,
    /// A card or a thread.
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 2000)))]
    pub link: Option<String>,
    /// The agent's one line on why this can't wait a few hours.
    #[garde(length(min = 1, max = 500))]
    pub reason: String,
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 100)))]
    pub account: Option<String>,
    /// Repeats for the same thread collapse into one notification.
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 300)))]
    pub thread_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct Ping {
    pub id: i64,
    pub sent_by: SessionRef,
    pub title: String,
    pub body: String,
    pub link: Option<String>,
    pub reason: String,
    pub account: Option<String>,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub sent_at: Timestamp,
    /// How many devices took it.
    pub result: Option<String>,
    /// "Don't ping for this again", with the person's words (empty when they left no note).
    pub feedback: Option<String>,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub feedback_at: Option<Timestamp>,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub feedback_picked_up_at: Option<Timestamp>,
}

/// The Rules screen's pings: the last seven days' against the weekly budget, which suppresses nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct PingsView {
    pub weekly_budget: u32,
    /// Pings sent in the last seven days.
    pub last_week: u32,
    /// The newest first, up to 100.
    pub pings: Vec<Ping>,
}

/// "Don't ping for this again", with an optional note for the agent that sent it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct PingFeedback {
    #[serde(default)]
    #[garde(inner(length(max = 2000)))]
    pub note: Option<String>,
}

/// Watch a thread from the card's menu: replies on it may ping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct WatchAdd {
    #[garde(length(min = 1, max = 100))]
    pub account: String,
    #[garde(length(min = 1, max = 300))]
    pub thread_key: String,
    #[serde(default)]
    #[garde(inner(length(max = 500)))]
    pub subject: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct ThreadWatch {
    pub id: i64,
    pub account: String,
    pub thread_key: String,
    pub subject: Option<String>,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub created_at: Timestamp,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct WebhookFailure {
    pub id: i64,
    pub endpoint_id: String,
    pub source: crate::hooks::HookKind,
    pub monitor: Option<String>,
    /// firing or resolved
    pub state: Option<String>,
    pub title: Option<String>,
    pub target: String,
    pub reason: String,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub at: Timestamp,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub seen_at: Option<Timestamp>,
}
