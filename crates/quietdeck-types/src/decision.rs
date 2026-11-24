//! Approve All, the undo window, Stop, and what the Inbox shows. The decision records themselves are in
//! [`crate::card`].

use garde::Validate;
use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::action::Action;
use crate::card::{Card, DecisionKind};
use crate::timeline::TimelineEntry;

/// The person's decision on one card version, as the web app sends it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct CardDecision {
    #[garde(skip)]
    pub card_id: i64,
    /// The version on screen. A card that has a newer version by the time the decision is released is skipped.
    #[garde(skip)]
    pub version: i64,
    #[garde(skip)]
    pub kind: DecisionKind,
    /// The actions as the person edited them, replacing the proposed ones. Absent means as proposed.
    #[serde(default)]
    #[garde(dive)]
    pub fields: Option<Vec<Action>>,
    /// An adjustment or comment: free text makes it the agent's decision.
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 4000)))]
    pub text: Option<String>,
}

impl CardDecision {
    /// The field rules, plus each edited action's own checks ([`Action::check`]) and at most 20 of them.
    pub fn check(&self) -> Result<(), String> {
        self.validate().map_err(|e| e.to_string())?;
        if let Some(fields) = &self.fields {
            if fields.len() > 20 {
                return Err("a card takes at most 20 actions".into());
            }
            for a in fields {
                a.check()?;
            }
        }
        Ok(())
    }
}

/// Approve All: every card on screen with its decision (unmentioned means yes, so the app sends approve for each
/// card the person didn't change), and the timeline entries on screen, which move to History.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct ApproveAll {
    #[garde(length(max = 500), dive)]
    pub cards: Vec<CardDecision>,
    #[serde(default)]
    #[garde(length(max = 1000))]
    pub timeline: Vec<i64>,
}

/// A card Approve All left alone, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct Skipped {
    pub card_id: i64,
    pub reason: String,
}

/// What Approve All did: nothing is released until `release_at`, and Undo with `batch` cancels it until then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct Approving {
    pub batch: String,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub release_at: Timestamp,
    /// The cards with a decision in this batch.
    pub cards: Vec<i64>,
    pub skipped: Vec<Skipped>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct UndoBatch {
    pub batch: String,
}

/// What Stop stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct StopResult {
    /// Actions that hadn't started and now won't.
    pub actions: i64,
    /// Decisions still in their undo window, cancelled.
    pub decisions: i64,
}

/// One card's own decision outside Approve All: Leave for later, Move up, Move down, or an adjustment typed into its
/// box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct Decide {
    #[garde(skip)]
    pub version: i64,
    #[garde(skip)]
    pub kind: DecisionKind,
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 4000)))]
    pub text: Option<String>,
}

/// One action's report from `quietdeck actions report`, as the deck took it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct ReportReceipt {
    pub action_id: i64,
    /// Why the deck refused this one; absent when it was recorded.
    pub error: Option<String>,
}

/// The Inbox, for one account or all of them. Nothing here is a count: `has_later` only says whether to show the
/// Later link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Inbox {
    pub actions: Vec<Card>,
    pub suggestions: Vec<Card>,
    pub archive_list: Vec<Card>,
    pub sent: Vec<Card>,
    /// Unread automatic actions.
    pub timeline: Vec<TimelineEntry>,
    pub has_later: bool,
    /// The batch still in its undo window, if any.
    pub approving: Option<Approving>,
    /// Whether any approved action is still waiting or running, for Stop.
    pub running: bool,
    /// When an agent last posted a card or took decisions. The Inbox shows it only when it's over an hour ago.
    #[ts(type = "string | null")]
    pub last_agent_at: Option<Timestamp>,
    /// Cards whose target's tab had grants archived when its earlier session closed.
    #[serde(default)]
    pub rule_offers: Vec<crate::rules::RuleOffer>,
    /// The triage agent's current report, which the Inbox shows in place of the cards.
    #[serde(default)]
    pub report: Option<crate::report::Report>,
    /// Whether the person is reading the Inbox, and what the runner holds meanwhile.
    #[serde(default)]
    pub presence: crate::reading::Presence,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn approve_all_checks_each_card() {
        let ok: ApproveAll = serde_json::from_value(json!({
            "cards": [{"card_id": 1, "version": 2, "kind": "approve"},
                      {"card_id": 2, "version": 1, "kind": "adjust", "text": "forward it to api instead"}],
            "timeline": [4, 5]
        }))
        .unwrap();
        ok.validate().unwrap();
        let empty_text: ApproveAll =
            serde_json::from_value(json!({"cards": [{"card_id": 1, "version": 1, "kind": "adjust", "text": ""}]}))
                .unwrap();
        assert!(empty_text.validate().is_err());
        let bad_edit: ApproveAll = serde_json::from_value(json!({"cards": [{"card_id": 1, "version": 1,
            "kind": "edit", "fields": [{"type": "archive", "account": "", "thread_id": "t"}]}]}))
        .unwrap();
        assert!(bad_edit.validate().is_err(), "edited actions get the same field rules as posted ones");
    }
}
