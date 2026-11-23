//! Cards, the actions on them, and the user's decisions.
//!
//! One card per thread: posting a card for a thread that already has an open card replaces that card's content
//! with a new version. Cards carry what the person needs to decide, never email bodies.

use garde::Validate;
use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::action::{Action, SessionRef};

/// Where a card sits in the Inbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CardSection {
    Actions,
    ArchiveList,
    Suggestion,
    Later,
    Sent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CardState {
    Open,
    Revising,
    Approved,
    Executing,
    HandedOff,
    Done,
    Failed,
    HandledElsewhere,
    Later,
    Automatic,
}

impl CardState {
    /// Finished cards move to History.
    pub fn is_finished(self) -> bool {
        matches!(self, CardState::Done | CardState::HandledElsewhere)
    }
}

/// The sender as the agent read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct Sender {
    #[serde(default)]
    #[garde(inner(length(max = 300)))]
    pub name: Option<String>,
    #[garde(length(min = 3, max = 320))]
    pub address: String,
}

impl Sender {
    pub fn domain(&self) -> Option<&str> {
        self.address.rsplit_once('@').map(|(_, d)| d)
    }
}

/// What the agent read in the message's authentication results. Strict sender rules need one of them to pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct SenderAuth {
    #[serde(default)]
    pub dmarc_pass: bool,
    #[serde(default)]
    pub dkim_pass: bool,
}

/// One fact on a card: an amount, a date, a reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct Fact {
    #[garde(length(min = 1, max = 100))]
    pub label: String,
    #[garde(length(min = 1, max = 1000))]
    pub value: String,
}

/// `quietdeck card add`: what an agent posts. The posting session comes from its token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct CardInput {
    /// An account label from deck config.
    #[garde(length(min = 1, max = 100))]
    pub account: String,
    /// The agent's reference for the thread (a Gmail thread ID). Opaque to the deck; one card per thread.
    #[garde(length(min = 1, max = 300))]
    pub thread_key: String,
    /// The newest message the card was prepared on, for freshness checks.
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 300)))]
    pub last_message_ref: Option<String>,
    /// When that message arrived. A thread left for later takes a new card only for a message newer than that.
    #[serde(default)]
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    #[garde(skip)]
    pub last_message_at: Option<Timestamp>,
    #[serde(default)]
    #[garde(length(max = 50), inner(length(min = 1, max = 300)))]
    pub refs: Vec<String>,
    #[serde(default)]
    #[garde(dive)]
    pub sender: Option<Sender>,
    #[serde(default)]
    #[garde(skip)]
    pub sender_auth: SenderAuth,
    /// The `List-Id` header, for strict rules and the learning loop.
    #[serde(default)]
    #[garde(inner(length(max = 500)))]
    pub list_id: Option<String>,
    #[garde(length(max = 1000))]
    pub subject: String,
    #[garde(length(min = 1, max = 5000))]
    pub summary: String,
    #[serde(default)]
    #[garde(length(max = 30), dive)]
    pub facts: Vec<Fact>,
    #[garde(skip)]
    pub section: CardSection,
    /// The agent wants to carry the card out itself, whatever the runner could do.
    #[serde(default)]
    #[garde(skip)]
    pub needs_agent: bool,
    /// Nothing touches the thread (a newsletter the person reads themselves).
    #[serde(default)]
    #[garde(skip)]
    pub keep_in_inbox: bool,
    #[garde(length(min = 1, max = 5000))]
    pub reason: String,
    /// The rules.md row the agent used.
    #[serde(default)]
    #[garde(inner(length(max = 500)))]
    pub rule_ref: Option<String>,
    /// Open in Gmail (or the draft).
    #[serde(default)]
    #[garde(inner(url))]
    pub link: Option<String>,
    #[serde(default)]
    #[garde(length(max = 20), dive)]
    pub actions: Vec<Action>,
    /// Ask for an automatic action under this grant: the deck runs `rules match`, and stores the card as automatic
    /// only when it allows.
    #[serde(default)]
    #[garde(skip)]
    pub auto_grant: Option<i64>,
}

impl CardInput {
    /// Field rules, then every action's own rules.
    pub fn check(&self) -> Result<(), String> {
        self.validate().map_err(|e| e.to_string())?;
        for (i, a) in self.actions.iter().enumerate() {
            a.check().map_err(|e| format!("actions[{i}]: {e}"))?;
        }
        if self.section == CardSection::Sent && !self.actions.iter().all(|a| matches!(a, Action::Forward { .. })) {
            return Err("a sent row carries only forwards".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ActionStatus {
    Pending,
    Released,
    Started,
    Done,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RunBy {
    Runner,
    Agent,
}

/// An action as stored on a card version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct ActionRecord {
    pub id: i64,
    pub card_id: i64,
    pub card_version: i64,
    pub seq: i64,
    pub action: Action,
    pub status: ActionStatus,
    pub run_by: Option<RunBy>,
    /// What was sent and what came back.
    #[ts(type = "unknown")]
    pub outcome: Option<serde_json::Value>,
    pub error: Option<String>,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub started_at: Option<Timestamp>,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub finished_at: Option<Timestamp>,
}

/// A card as the deck serves it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct Card {
    pub id: i64,
    pub version: i64,
    pub posted_by: SessionRef,
    pub state: CardState,
    pub handoff_reason: Option<String>,
    pub grant_id: Option<i64>,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub prepared_at: Timestamp,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub rechecked_at: Option<Timestamp>,
    pub recheck_note: Option<String>,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub updated_at: Timestamp,
    #[serde(flatten)]
    pub input: CardInput,
    /// The current version's actions, in order.
    pub action_records: Vec<ActionRecord>,
}

/// `quietdeck card add`'s answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct CardAdded {
    pub card: Card,
    /// True when it replaced the content of the thread's open card.
    pub replaced: bool,
    /// With `auto_grant`: what `rules match` said. A denied card is stored as a normal card.
    pub auto: Option<crate::rules::MatchResult>,
}

/// `quietdeck card resolve`: the agent's recheck found the card needs nothing more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[serde(tag = "how", rename_all = "snake_case")]
#[ts(export)]
pub enum CardResolution {
    /// The thread is no longer in the inbox: the card moves to History.
    ArchivedInGmail,
    /// Someone handled it (the person replied, a session mentioned it): the card moves to the archive list with
    /// the note, which the person reads.
    HandledElsewhere {
        #[garde(length(min = 1, max = 1000))]
        note: String,
    },
    /// The agent rechecked it and it still stands.
    Rechecked {
        #[garde(inner(length(max = 1000)))]
        note: Option<String>,
    },
}

/// The kinds of decision the person makes on a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DecisionKind {
    Approve,
    Edit,
    Adjust,
    MoveDown,
    MoveUp,
    Later,
    Stop,
}

/// A decision on one card version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct Decision {
    pub id: i64,
    pub card_id: i64,
    pub card_version: i64,
    pub kind: DecisionKind,
    /// The person's edits: the actions as approved, when they changed any.
    pub fields: Option<Vec<Action>>,
    /// The person's adjustment or comment. Free text makes the decision the agent's.
    pub text: Option<String>,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub released_at: Option<Timestamp>,
    pub claimed_by: Option<RunBy>,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub picked_up_at: Option<Timestamp>,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub at: Timestamp,
}

/// A released decision with the card it approves, as the runner and `quietdeck decisions take` get it. `actions` is
/// exactly what was approved, with the person's edits applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct Released {
    pub decision: Decision,
    pub card: Card,
    pub actions: Vec<ActionRecord>,
    /// Why the runner handed it over, when it did.
    pub handoff_reason: Option<String>,
}

/// One action's result, for `quietdeck action done|fail` and `quietdeck actions report`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[serde(tag = "result", rename_all = "snake_case")]
#[ts(export)]
pub enum ActionResult {
    Done {
        /// What was sent and what came back.
        #[serde(default)]
        #[ts(type = "unknown")]
        #[garde(skip)]
        outcome: Option<serde_json::Value>,
    },
    Failed {
        #[garde(length(min = 1, max = 5000))]
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct ActionReport {
    #[garde(skip)]
    pub action_id: i64,
    #[serde(flatten)]
    #[garde(dive)]
    pub result: ActionResult,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn card() -> serde_json::Value {
        json!({"account": "personal", "thread_key": "t1", "subject": "Invoice", "summary": "Your invoice is ready",
               "section": "actions", "reason": "Invoices are archived after a look",
               "actions": [{"type": "archive", "account": "personal", "thread_id": "t1"}]})
    }

    #[test]
    fn a_minimal_card_passes() {
        let c: CardInput = serde_json::from_value(card()).unwrap();
        c.check().unwrap();
    }

    #[test]
    fn a_card_with_a_bad_action_names_it() {
        let mut v = card();
        v["actions"][0]["thread_id"] = json!("");
        let c: CardInput = serde_json::from_value(v).unwrap();
        assert!(c.check().unwrap_err().contains("actions[0]"));
    }

    #[test]
    fn a_sent_row_carries_only_forwards() {
        let mut v = card();
        v["section"] = json!("sent");
        let c: CardInput = serde_json::from_value(v).unwrap();
        assert!(c.check().is_err());
    }

    #[test]
    fn a_link_must_be_a_url() {
        let mut v = card();
        v["link"] = json!("not a url");
        let c: CardInput = serde_json::from_value(v).unwrap();
        assert!(c.check().is_err());
    }

    #[test]
    fn reports_flatten_their_result() {
        let r: ActionReport = serde_json::from_value(json!({"action_id": 4, "result": "failed", "error": "401"})).unwrap();
        assert_eq!(r.result, ActionResult::Failed { error: "401".into() });
        let r: ActionReport = serde_json::from_value(json!({"action_id": 4, "result": "done"})).unwrap();
        assert_eq!(r.result, ActionResult::Done { outcome: None });
    }
}
