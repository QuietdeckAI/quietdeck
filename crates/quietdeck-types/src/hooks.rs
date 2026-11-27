//! Webhook endpoints: a secret URL a service posts to (Alertmanager, Argo CD notifications, Gatus). Each event goes
//! straight to the session that owns it, as outside data; the triage agent only hears a count for its report.

use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// How the deck reads what arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum HookKind {
    /// Any JSON (or text): a title is taken from the usual fields. Lenient, for Gatus, which doesn't escape its JSON.
    Generic,
    /// Alertmanager's `webhook_configs` format: one event per alert, firing and resolved.
    Alertmanager,
}

impl HookKind {
    pub fn as_str(self) -> &'static str {
        match self {
            HookKind::Generic => "generic",
            HookKind::Alertmanager => "alertmanager",
        }
    }
    pub fn parse(s: &str) -> Option<HookKind> {
        match s {
            "generic" => Some(HookKind::Generic),
            "alertmanager" => Some(HookKind::Alertmanager),
            _ => None,
        }
    }
}

/// `quietdeck hooks create`: an agent proposes an endpoint; it receives nothing until the person approves it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct HookCreate {
    /// What it's for, as the person reads it ("Ops alerts").
    pub label: String,
    pub kind: HookKind,
    /// The session its events go to, as `<tab>@<node>` ("alerts@server").
    pub to: String,
}

/// One endpoint, as Settings and `hooks list` show it. The secret is never shown again after creation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct HookEndpoint {
    pub id: String,
    pub label: String,
    pub kind: HookKind,
    /// `<tab>@<node>`.
    pub to: String,
    /// Who proposed it (`<tab>@<node>`), when an agent did.
    pub proposed_by: Option<String>,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub created_at: Timestamp,
    /// None until the person approves it: until then it receives nothing (404).
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub approved_at: Option<Timestamp>,
    pub event_count: i64,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub last_event_at: Option<Timestamp>,
    /// The latest time its owner couldn't be reached (the events went to the triage agent instead).
    #[serde(default)]
    pub last_failure: Option<HookFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct HookFailure {
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub at: Timestamp,
    pub reason: String,
}

/// What `hooks create` answers: the endpoint and its URL, shown this once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct HookCreated {
    pub endpoint: HookEndpoint,
    pub url: String,
}
