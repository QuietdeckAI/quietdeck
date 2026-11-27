//! Cards, the actions on them, and the user's decisions.
//!
//! One card per thread: posting a card for a thread that already has an open card replaces that card's content
//! with a new version.

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
    /// What the person needs to look at: a reply in their name, money, a decision, something only they can do.
    Actions,
    /// Routine mail Approve All just handles, one line each: an archive, with at most a label or a one-line
    /// unsubscribe reply ("Unsubscribe, thanks.") before it. Sales pitches and FYI mail go here.
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct SenderAuth {
    #[serde(default)]
    pub dmarc_pass: bool,
    #[serde(default)]
    pub dkim_pass: bool,
}

/// One fact on a card: an amount, a date, a reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct Fact {
    /// A few words: "Amount", "Due".
    #[garde(length(min = 1, max = 100))]
    pub label: String,
    /// "$1,200", "1 December".
    #[garde(length(min = 1, max = 1000))]
    pub value: String,
}

/// `quietdeck card add`: what an agent posts. The posting session comes from its token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct CardInput {
    /// An account label from deck config.
    #[garde(length(min = 1, max = 100))]
    pub account: String,
    /// The thread's ID as the mail service gives it: Gmail's thread ID, exactly (`19a4f0c2b7e1d3a5`), with no account
    /// or service prefix (the account is its own field). One card per account and thread; a forward without a
    /// `source` names the thread by it.
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
    /// Which of the person's addresses the newest message went to (an alias such as `hello@example.com` when that's
    /// where it arrived). The Inbox shows it with the sender; without it, the account's address stands in.
    #[serde(default)]
    #[garde(inner(length(min = 3, max = 320)))]
    #[ts(optional)]
    pub to: Option<String>,
    /// The email's own words for the Inbox's details pane, as plain text: the newest message verbatim, or a thread as
    /// one line per message (`Mary: Did we update this for Compass?`). Leave out quoted replies, signatures and
    /// footers. The pane shows the first lines. On an account whose rules keep customers' data out of other
    /// systems, leave out end users' personal data and document contents, as stories and forwards do.
    #[serde(default)]
    #[garde(inner(length(max = 4000)))]
    #[ts(optional)]
    pub excerpt: Option<String>,
    /// The `List-Id` header, for strict rules and the learning loop.
    #[serde(default)]
    #[garde(inner(length(max = 500)))]
    pub list_id: Option<String>,
    #[garde(length(max = 1000))]
    pub subject: String,
    /// What the email is, in a few words: the Inbox shows it on one line beside the proposed action. Lead with the kind
    /// of email and leave out who sent it and what their company does unless the person needs that to decide:
    /// "Sales pitch: card dunning", "Water bill, direct debit", "Customer asks how webhook retries work". Facts and
    /// drafts carry the rest.
    #[garde(length(min = 1, max = 5000))]
    pub summary: String,
    /// Key facts shown under the summary, each `{"label": "Amount", "value": "$1,200"}`.
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
    /// A card from its JSON, naming every missing and unknown field at the top level in one error, each missing one
    /// with what it's for. Problems inside an action come from serde, one at a time.
    pub fn from_json(text: &str) -> Result<CardInput, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("the card's JSON: {e}"))?;
        let Some(obj) = v.as_object() else { return Err("the card's JSON must be an object".into()) };
        let schema = Self::schema();
        let props = schema["properties"].as_object().expect("CardInput's schema has properties");
        let mut problems: Vec<String> = vec![];
        for name in schema["required"].as_array().into_iter().flatten().filter_map(|n| n.as_str()) {
            if !obj.contains_key(name) {
                let about = props[name]["description"]
                    .as_str()
                    .map(|d| format!(": {}", d.split_whitespace().collect::<Vec<_>>().join(" ")))
                    .unwrap_or_default();
                problems.push(format!("missing `{name}`{about}"));
            }
        }
        // Sorted: with serde_json's preserve_order (another crate in the build may turn it on), keys keep their order.
        let mut unknown: Vec<&str> = obj.keys().map(String::as_str).filter(|k| !props.contains_key(*k)).collect();
        unknown.sort_unstable();
        if !unknown.is_empty() {
            let mut known: Vec<&str> = props.keys().map(String::as_str).collect();
            known.sort_unstable();
            problems.push(format!(
                "unknown field{} {} (a card's fields: {})",
                if unknown.len() == 1 { "" } else { "s" },
                unknown.iter().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", "),
                known.join(", ")
            ));
        }
        if !problems.is_empty() {
            return Err(format!(
                "the card's JSON:\n- {}\n`quietdeck card add --help` has an example; `quietdeck card schema` prints \
                 every field.",
                problems.join("\n- ")
            ));
        }
        serde_path_to_error::deserialize(v).map_err(|e| {
            let path = e.path().to_string();
            let inner = e.into_inner();
            let field = path.split(['.', '[']).next().unwrap_or_default();
            let takes = props
                .get(field)
                .map(|p| Self::shape(&schema, p, 0))
                .filter(|s| s.len() <= 300)
                .map(|s| format!("\n`{field}` takes {s}."))
                .unwrap_or_default();
            format!(
                "the card's JSON: `{path}`: {inner}{takes}\n`quietdeck card add --help` has an example; `quietdeck card \
                 schema` prints every field."
            )
        })
    }

    /// A field's JSON shape in one line from its schema: `[{"label": string, "value": string}]`. Optional fields end in `?`.
    fn shape(root: &serde_json::Value, node: &serde_json::Value, depth: usize) -> String {
        use serde_json::Value;
        let node = match node.get("$ref").and_then(Value::as_str).and_then(|r| r.strip_prefix("#/$defs/")) {
            Some(name) => &root["$defs"][name],
            None => node,
        };
        if depth > 3 {
            return "…".into();
        }
        if let Some(values) = node.get("enum").and_then(Value::as_array) {
            return values.iter().map(Value::to_string).collect::<Vec<_>>().join(" | ");
        }
        if let Some(any) = node.get("anyOf").or_else(|| node.get("oneOf")).and_then(Value::as_array) {
            return any.iter().map(|n| Self::shape(root, n, depth + 1)).collect::<Vec<_>>().join(" | ");
        }
        let types: Vec<&str> = match &node["type"] {
            Value::String(t) => vec![t.as_str()],
            Value::Array(ts) => ts.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        let main = types.iter().find(|t| **t != "null").copied().unwrap_or("any");
        match main {
            "array" => format!("[{}]", Self::shape(root, &node["items"], depth + 1)),
            "object" => match node["properties"].as_object() {
                Some(props) => {
                    let required: Vec<&str> =
                        node["required"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                    let fields: Vec<String> = props
                        .iter()
                        .map(|(k, v)| {
                            let opt = if required.contains(&k.as_str()) { "" } else { "?" };
                            format!("\"{k}\"{opt}: {}", Self::shape(root, v, depth + 1))
                        })
                        .collect();
                    format!("{{{}}}", fields.join(", "))
                }
                None => "object".into(),
            },
            other => other.into(),
        }
    }

    /// The JSON Schema of a card, with every field's doc comment as its description.
    pub fn schema() -> serde_json::Value {
        schemars::schema_for!(CardInput).to_value()
    }

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
    /// The person's changes to this card's default in the Inbox, oldest first, in his own words: carry them out
    /// along with the actions.
    #[serde(default)]
    pub changes: Vec<String>,
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
    fn every_missing_and_unknown_field_is_named_at_once() {
        let e = CardInput::from_json(r#"{"account": "personal", "title": "x", "from": "a@b.c"}"#).unwrap_err();
        for f in [
            "missing `thread_key`",
            "missing `subject`",
            "missing `summary`",
            "missing `section`",
            "missing `reason`",
            "unknown fields `from`, `title`",
        ] {
            assert!(e.contains(f), "{f} in {e}");
        }
        assert!(e.contains("Gmail's thread ID"), "missing fields say what they are: {e}");
        assert!(!e.contains("missing `facts`"), "defaulted fields aren't required: {e}");
    }

    #[test]
    fn a_wrong_type_names_its_path_and_the_shape_the_field_takes() {
        let mut card = card();
        card["facts"] = json!(["Amount: $1,200"]);
        let e = CardInput::from_json(&card.to_string()).unwrap_err();
        assert!(e.contains("`facts[0]`"), "{e}");
        assert!(e.contains(r#"`facts` takes [{"label": string, "value": string}]"#), "{e}");
        card["facts"] = json!("Amount: $1,200");
        let e = CardInput::from_json(&card.to_string()).unwrap_err();
        assert!(e.contains("`facts`: invalid type: string"), "{e}");
    }

    #[test]
    fn unknown_fields_in_an_action_are_refused() {
        let mut v = card();
        v["actions"][0]["thread"] = json!("t1");
        assert!(CardInput::from_json(&v.to_string()).unwrap_err().contains("unknown field `thread`"));
        let mut v = card();
        v["actions"] = json!([{"type": "draft", "account": "personal", "thread_id": "t1", "reply_to_message_id": "m1",
            "from": "me@example.com", "to": ["you@example.com"], "subject": "Re: x", "body": "Thanks", "bcc": []}]);
        assert!(CardInput::from_json(&v.to_string()).unwrap_err().contains("unknown field `bcc`"));
        v["actions"][0].as_object_mut().unwrap().remove("bcc");
        CardInput::from_json(&v.to_string()).unwrap();
        v["actions"] = json!([{"type": "story", "op": "comment", "story_id": "sc-1", "text": "x"}]);
        CardInput::from_json(&v.to_string()).unwrap();
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
        let r: ActionReport =
            serde_json::from_value(json!({"action_id": 4, "result": "failed", "error": "401"})).unwrap();
        assert_eq!(r.result, ActionResult::Failed { error: "401".into() });
        let r: ActionReport = serde_json::from_value(json!({"action_id": 4, "result": "done"})).unwrap();
        assert_eq!(r.result, ActionResult::Done { outcome: None });
    }
}
