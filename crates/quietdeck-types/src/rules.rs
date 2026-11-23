//! Auto-approve rules (grants) and `rules match`.

use garde::Validate;
use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::action::{ActionType, SessionRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RuleMode {
    /// The agent judges matches from the instruction; the deck checks the grant, the sender's domain, the template
    /// and the policy.
    Fuzzy,
    /// The deck judges matches from the card's fields.
    Strict,
}

/// A strict rule's criteria. A sender criterion needs the card to report DMARC or DKIM passing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct Criteria {
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 100)))]
    pub account: Option<String>,
    #[serde(default)]
    #[garde(inner(length(min = 3, max = 320)))]
    pub from_address: Option<String>,
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 253)))]
    pub from_domain: Option<String>,
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 500)))]
    pub subject_contains: Option<String>,
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 500)))]
    pub list_id: Option<String>,
}

/// What a rule allows: action types, and for forwards, resumes and new sessions, the target.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct ActionTemplate {
    #[garde(length(min = 1, max = 13))]
    pub types: Vec<ActionType>,
    #[serde(default)]
    #[garde(dive)]
    pub target: Option<SessionRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RuleStatus {
    Active,
    Disabled,
    /// Archived when the session it was tied to closed.
    Archived,
}

/// A grant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct Rule {
    pub id: i64,
    pub mode: RuleMode,
    pub instruction: Option<String>,
    pub criteria: Criteria,
    pub action_template: ActionTemplate,
    pub session_ref: Option<SessionRef>,
    pub project: Option<String>,
    pub topic: Option<String>,
    /// A fuzzy match from any other domain is refused.
    pub known_sender_domains: Vec<String>,
    pub status: RuleStatus,
    pub archived_reason: Option<String>,
    pub match_count: i64,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub last_matched_at: Option<Timestamp>,
    pub created_from_card_id: Option<i64>,
    /// Shipped with the deck (the sent-mail grant). The person can turn it off but not delete it.
    pub builtin: bool,
}

/// `quietdeck rules match`: may this card be carried out without the person?
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct MatchRequest {
    #[garde(dive)]
    pub card: crate::card::CardInput,
    #[garde(skip)]
    pub grant_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct MatchResult {
    pub allow: bool,
    pub grant_id: i64,
    pub reason: String,
}
