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

/// A grant as the person writes it on the Rules screen or from a card ("Auto-approve matches in future").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct RuleInput {
    #[garde(skip)]
    pub mode: RuleMode,
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 2000)))]
    pub instruction: Option<String>,
    #[serde(default)]
    #[garde(dive)]
    pub criteria: Criteria,
    #[garde(dive)]
    pub action_template: ActionTemplate,
    /// The session the grant is tied to: when it closes, the grant is archived. Defaults to the template's target.
    #[serde(default)]
    #[garde(dive)]
    pub session_ref: Option<SessionRef>,
    /// The project whose policy governs every action. Without one, each action's project is found from its target
    /// session's directory or its account.
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 100)))]
    pub project: Option<String>,
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 200)))]
    pub topic: Option<String>,
    #[serde(default)]
    #[garde(length(max = 50), inner(length(min = 1, max = 253)))]
    pub known_sender_domains: Vec<String>,
    #[serde(default)]
    #[garde(skip)]
    pub created_from_card_id: Option<i64>,
}

impl RuleInput {
    /// Field rules, then what each mode needs.
    pub fn check(&self) -> Result<(), String> {
        if self.action_template.types.is_empty() {
            return Err("choose at least one action the rule allows".into());
        }
        match self.mode {
            RuleMode::Fuzzy => {
                if self.instruction.as_deref().is_none_or(|s| s.trim().is_empty()) {
                    return Err("a fuzzy rule needs its instruction: what counts as a match, in words".into());
                }
                if self.known_sender_domains.is_empty() {
                    return Err(
                        "a fuzzy rule needs at least one known sender domain: mail from any other is refused".into()
                    );
                }
            }
            RuleMode::Strict => {
                if self.criteria.account.is_none() {
                    return Err("a strict rule needs its account".into());
                }
                if self.criteria.from_address.is_none() && self.criteria.from_domain.is_none() {
                    return Err("a strict rule needs a sender: from_address or from_domain".into());
                }
            }
        }
        let targeted = [ActionType::Forward, ActionType::Resume, ActionType::NewSession];
        if self.action_template.target.is_none() && self.action_template.types.iter().any(|t| targeted.contains(t)) {
            return Err("a rule that forwards to, resumes or starts a session needs its target session".into());
        }
        self.validate().map_err(|e| e.to_string())
    }
}

/// Archive, restore, or restore tied to another session ("move").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct RuleStatusChange {
    #[garde(skip)]
    pub status: RuleStatus,
    /// With `active`: tie the grant to this session, and make it the target of its forwards and resumes.
    #[serde(default)]
    #[garde(dive)]
    pub move_to: Option<SessionRef>,
}

/// A project's autonomy policy, from deck.yaml.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct ProjectPolicy {
    pub name: String,
    pub dirs: Vec<String>,
    pub accounts: Vec<String>,
    pub automatic: Vec<ActionType>,
}

/// The Rules screen, and `quietdeck rules list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct RulesView {
    pub rules: Vec<Rule>,
    pub policies: Vec<ProjectPolicy>,
    /// Automatic resumes and new sessions allowed in any 24 hours.
    pub session_starts_per_day: u32,
}

/// On a card that forwards to, resumes or starts a session in a tab whose earlier session's grants were archived
/// when it closed: those grants can move to this card's target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct RuleOffer {
    pub card_id: i64,
    pub rule_ids: Vec<i64>,
    pub to: SessionRef,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rule(v: serde_json::Value) -> RuleInput {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn each_mode_needs_its_own_fields() {
        let to = json!({"node": "server", "tab": "esign cert"});
        let strict = rule(json!({"mode": "strict", "criteria": {"account": "work", "from_domain": "example.com"},
            "action_template": {"types": ["forward"], "target": to}}));
        strict.check().unwrap();
        let no_sender = rule(json!({"mode": "strict", "criteria": {"account": "work", "subject_contains": "x"},
            "action_template": {"types": ["archive"]}}));
        assert!(no_sender.check().unwrap_err().contains("sender"));
        let fuzzy = rule(json!({"mode": "fuzzy", "instruction": "CI failures for app",
            "action_template": {"types": ["archive"]}}));
        assert!(fuzzy.check().unwrap_err().contains("known sender domain"));
        let untargeted = rule(json!({"mode": "fuzzy", "instruction": "x", "known_sender_domains": ["a.com"],
            "action_template": {"types": ["forward"]}}));
        assert!(untargeted.check().unwrap_err().contains("target"));
    }
}
