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
    LeaseStarted,
    LeaseAbsorbed,
    LeaseExpired,
    LeaseCleanupOverdue,
    FilterWithoutProposal,
    SessionStarted,
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

/// `quietdeck timeline add`: agents post automatic actions they carried out and mail a lease absorbed.
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
    #[serde(default)]
    #[garde(skip)]
    pub lease_id: Option<i64>,
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
    /// "Don't ping for this again", with the person's words.
    pub feedback: Option<String>,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub feedback_picked_up_at: Option<Timestamp>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum HookSource {
    Gatus,
    Cronitor,
    Ohdear,
    Generic,
}

impl HookSource {
    pub fn label(self) -> &'static str {
        match self {
            HookSource::Gatus => "Gatus",
            HookSource::Cronitor => "Cronitor",
            HookSource::Ohdear => "Oh Dear",
            HookSource::Generic => "Webhook",
        }
    }
}

/// `quietdeck hook create <source> --to <tab>[@<node>]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct HookCreate {
    #[garde(skip)]
    pub source: HookSource,
    #[garde(length(min = 1, max = 300))]
    pub to: String,
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 100)))]
    pub account: Option<String>,
    #[garde(length(min = 1, max = 100))]
    pub label: String,
    #[serde(default)]
    #[garde(skip)]
    pub fail_to_sender: bool,
    /// Oh Dear's signing secret, for its HMAC check. Stored on the deck: it's the endpoint's own secret, not a
    /// service credential.
    #[serde(default)]
    #[garde(inner(length(min = 8, max = 500)))]
    pub signing_secret: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct HookEndpoint {
    pub id: String,
    pub source: HookSource,
    pub target: SessionRef,
    pub account: Option<String>,
    pub label: String,
    pub fail_to_sender: bool,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub created_at: Timestamp,
}

/// `hook create`'s answer. The URL holds the secret and is shown once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct HookCreated {
    pub endpoint: HookEndpoint,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct WebhookFailure {
    pub id: i64,
    pub endpoint_id: String,
    pub source: HookSource,
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
