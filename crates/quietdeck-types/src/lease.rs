//! Leases: a session's claim on something for a while. Two kinds ship: a temporary Gmail filter, and a heavy-build
//! slot on a machine.

use garde::Validate;
use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::action::SessionRef;

/// The two kinds that ship. A kind is a deck plugin; more are added only when a real case turns up.
pub const KIND_GMAIL_FILTER: &str = "gmail-filter";
pub const KIND_HEAVY_BUILD: &str = "heavy-build";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LeaseState {
    Proposed,
    Queued,
    Approved,
    Active,
    Expiring,
    CleanupRequested,
    Released,
    Overdue,
    Rejected,
}

impl LeaseState {
    pub fn is_ended(self) -> bool {
        matches!(self, LeaseState::Released | LeaseState::Rejected)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CleanupBy {
    Runner,
    Holder,
    Fallback,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct Lease {
    pub id: i64,
    pub kind: String,
    pub resource: String,
    #[ts(type = "Record<string, unknown>")]
    pub params: serde_json::Value,
    pub holder: SessionRef,
    pub reason: String,
    pub state: LeaseState,
    pub ttl_seconds: i64,
    pub idle_minutes: i64,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub created_at: Timestamp,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub activated_at: Option<Timestamp>,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub last_renewed_at: Option<Timestamp>,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub expires_at: Option<Timestamp>,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub cleanup_requested_at: Option<Timestamp>,
    pub cleanup_by: Option<CleanupBy>,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub ended_at: Option<Timestamp>,
    pub end_reason: Option<String>,
    pub pattern_id: Option<i64>,
    /// What the kind's create step left behind, such as the Gmail filter's ID.
    #[ts(type = "Record<string, unknown> | null")]
    pub kind_state: Option<serde_json::Value>,
    /// The card that proposes it, while it's proposed.
    pub card_id: Option<i64>,
    /// Its place in the queue, for an exclusive kind.
    pub queue_position: Option<i64>,
}

/// `quietdeck lease propose|acquire <kind>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct LeaseRequest {
    #[garde(length(min = 1, max = 50))]
    pub kind: String,
    /// What's leased. For a gmail-filter, the account; for a heavy-build slot, the machine (default: the holder's).
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 500)))]
    pub resource: Option<String>,
    /// Kind-specific: a gmail-filter's `{query}`.
    #[serde(default)]
    #[ts(type = "Record<string, unknown>")]
    #[garde(skip)]
    pub params: serde_json::Value,
    #[garde(length(min = 1, max = 2000))]
    pub reason: String,
    /// A hard limit. Default 24 hours.
    #[serde(default)]
    #[garde(inner(range(min = 60, max = 2_592_000)))]
    pub ttl_seconds: Option<i64>,
    /// Ends once the holder has been idle or offline this long. Default 30 minutes.
    #[serde(default)]
    #[garde(inner(range(min = 1, max = 10_080)))]
    pub idle_minutes: Option<i64>,
}

/// The kind's create step is done: `quietdeck lease register <id>` with what it left behind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct LeaseRegister {
    #[ts(type = "Record<string, unknown>")]
    pub kind_state: serde_json::Value,
}

/// What a lease kind says about itself, for the CLI's help and the Settings screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct LeaseKindInfo {
    pub kind: String,
    /// Slots per resource with a queue, or none for a kind any number of sessions can hold at once.
    pub slots: Option<i64>,
    pub needs_approval: bool,
    pub describe: String,
}

/// What `lease propose` and `lease acquire` answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct LeaseAnswer {
    pub lease: Lease,
    /// What happened, in a sentence: approved at once, waiting on a card, queued.
    pub message: String,
}

/// A Gmail filter the person approved: the same filter on the same account needs no new proposal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct LeasePattern {
    pub id: i64,
    pub kind: String,
    /// For a gmail-filter: the account.
    pub resource: String,
    /// For a gmail-filter: the query.
    pub pattern: String,
    /// The card that approved it; none when an agent made it with proposals switched off.
    pub confirmed_by_card_id: Option<i64>,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub confirmed_at: Option<Timestamp>,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub revoked_at: Option<Timestamp>,
}

/// The leases list: what's held, queued, waiting or overdue, recent ones, and the approved filters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct LeasesView {
    pub leases: Vec<Lease>,
    pub patterns: Vec<LeasePattern>,
    pub kinds: Vec<LeaseKindInfo>,
}

/// The side effects a kind has: done by the runner, or without one by the holder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LeaseStep {
    /// Make what the lease stands for (create the filter, move matching inbox mail).
    Create,
    /// Undo it (delete the filter).
    Remove,
}
