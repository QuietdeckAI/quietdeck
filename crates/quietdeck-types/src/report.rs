//! The triage report: one living document the triage agent writes in Markdown and keeps up to date as mail arrives,
//! which the Inbox shows as the person's whole view of a round. Two tags put controls in it:
//!
//! - `<Default cards="c_1 c_2">forward both to the ads tab, then archive</Default>` shows "Default: …" with a
//!   Change this button. Approve All approves the cards of every default the person didn't change.
//! - `<Ask id="signin" options="Yes|No">Was this you?</Ask>` shows the question with a button per option, and with
//!   `other` an Other box too. The answer goes straight to the agent. With `cards="c_1"` it's about those cards,
//!   and the answer also shows in the thread of the default that names them.
//!
//! Everything else is Markdown; raw HTML shows as text.

use garde::Validate;
use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::action::SessionRef;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct Report {
    pub id: i64,
    /// Goes up with every edit; an edit names the version it was made from.
    pub version: i64,
    pub markdown: String,
    pub posted_by: SessionRef,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub created_at: Timestamp,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub updated_at: Timestamp,
    /// The person's answers and changes on this report, oldest first.
    pub answers: Vec<ReportAnswer>,
    /// Open cards (waiting on the person, handed off or failed) that no `<Default>` names: the Inbox points at the
    /// Cards screen for them, and `report set` lists them for the agent.
    pub cards_not_in_report: Vec<i64>,
}

/// `quietdeck report set`: the whole report, as the agent wants it now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct ReportSet {
    #[garde(length(min = 1, max = 200_000))]
    pub markdown: String,
    /// The version this edit starts from (`quietdeck report show` prints it), so an edit never overwrites one it
    /// didn't see. Leave it out only for the first report.
    #[serde(default)]
    #[garde(skip)]
    pub base: Option<i64>,
    /// Start a new report: the current one is finished (everything on it approved or handled).
    #[serde(default)]
    #[garde(skip)]
    pub new: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AnswerKind {
    /// An option of an `<Ask>`, or its Other text.
    Choice,
    /// What the person says about a `<Default>`: usually an adjustment or an addition, and the start or the next
    /// message of its thread.
    Change,
    /// The agent's reply in a default's thread (`quietdeck report reply`).
    Reply,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct ReportAnswer {
    pub id: i64,
    pub report_version: i64,
    /// The `<Ask>`'s id, or for a `<Default>` its id or `cards` value.
    pub control: String,
    pub kind: AnswerKind,
    /// The question or the default's text, so the agent reads the answer without the report.
    pub about: String,
    pub value: String,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub answered_at: Timestamp,
    /// The agent has it: delivered as a message, or taken with `quietdeck report answers`.
    pub picked_up: bool,
    /// For an answer to an `<Ask cards="…">`: the cards it's about. The Inbox shows it in the thread of the default
    /// that names them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cards: Option<String>,
}

/// An answer from the Inbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct ReportAnswerAdd {
    #[garde(skip)]
    pub report_id: i64,
    #[garde(skip)]
    pub report_version: i64,
    #[garde(length(min = 1, max = 500))]
    pub control: String,
    #[garde(skip)]
    pub kind: AnswerKind,
    #[garde(length(max = 2000))]
    pub about: String,
    #[garde(length(min = 1, max = 4000))]
    pub value: String,
    /// An `<Ask>`'s `cards`, when it names them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(length(max = 500))]
    #[ts(optional)]
    pub cards: Option<String>,
}

/// `quietdeck report reply`: the agent's answer in a default's thread, which the Inbox shows under it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct ReportReply {
    /// The default: the `[key]` its messages to you carry (`cards:c_12`, `archive-all`), or a card it names
    /// (`c_12`).
    #[garde(length(min = 1, max = 500))]
    pub to: String,
    #[garde(length(min = 1, max = 4000))]
    pub text: String,
}

/// The chat box under the report: plain words straight to the agent that writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct ReportChat {
    #[garde(length(min = 1, max = 8000))]
    pub text: String,
}

/// The cards a report's text names: `c_12` and `12` in a `<Default cards="…">` both mean card 12. Malformed entries
/// come back as errors, each quoted.
pub fn default_cards(markdown: &str) -> (Vec<i64>, Vec<String>) {
    let mut ids = vec![];
    let mut bad = vec![];
    let mut rest = markdown;
    while let Some(at) = rest.find("<Default") {
        rest = &rest[at + "<Default".len()..];
        let Some(end) = rest.find('>') else { break };
        let attrs = &rest[..end];
        if let Some(v) = attr(attrs, "cards") {
            for part in v.split(|c: char| c.is_whitespace() || c == ',').filter(|p| !p.is_empty()) {
                let digits = part.strip_prefix("c_").unwrap_or(part);
                match digits.parse::<i64>() {
                    Ok(n) => ids.push(n),
                    Err(_) => bad.push(format!("{part:?} isn't a card ID (c_12)")),
                }
            }
        } else {
            bad.push("a <Default> has no cards=\"…\"".into());
        }
        rest = &rest[end..];
    }
    ids.sort_unstable();
    ids.dedup();
    (ids, bad)
}

/// Each `<Default>`'s key, as the Inbox and the person's answers name it (its `id`, else `cards:` and its `cards`
/// value as written), and the cards it names.
pub fn default_keys(markdown: &str) -> Vec<(String, Vec<i64>)> {
    let mut out = vec![];
    let mut rest = markdown;
    while let Some(at) = rest.find("<Default") {
        rest = &rest[at + "<Default".len()..];
        let Some(end) = rest.find('>') else { break };
        let attrs = &rest[..end];
        let cards = attr(attrs, "cards").unwrap_or("");
        let key = attr(attrs, "id").map(str::to_string).unwrap_or_else(|| format!("cards:{cards}"));
        out.push((key, card_ids(cards)));
        rest = &rest[end..];
    }
    out
}

/// The card IDs in a `cards` value (`c_1 c_2`, `1,2`); anything else is skipped.
pub fn card_ids(cards: &str) -> Vec<i64> {
    cards
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter_map(|p| p.strip_prefix("c_").unwrap_or(p).parse().ok())
        .collect()
}

/// One attribute's value from a tag's attribute text: `name="value"`.
fn attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = attrs;
    while let Some(at) = rest.find(name) {
        let before_ok = at == 0 || rest[..at].ends_with(char::is_whitespace);
        let after = &rest[at + name.len()..];
        if before_ok && let Some(v) = after.strip_prefix("=\"") {
            return v.find('"').map(|end| &v[..end]);
        }
        rest = after;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_name_their_cards() {
        let md = "1. Ads\n   - <Default cards=\"c_1 c_2\">forward, then archive</Default>\n\
                  2. QuickBooks <Default id=\"qb\" cards=\"7,c_3\">task, then archive</Default>\n\
                  <Ask id=\"me\" options=\"Yes|No\">Was this you?</Ask>\n<Default cards=\"x9\">a</Default>";
        let (ids, bad) = default_cards(md);
        assert_eq!(ids, vec![1, 2, 3, 7]);
        assert_eq!(bad, vec!["\"x9\" isn't a card ID (c_12)".to_string()]);
        assert_eq!(default_cards("<Default>x</Default>").1, vec!["a <Default> has no cards=\"…\"".to_string()]);
    }
}

/// The Inbox document as it was when the person pressed Approve All, for History: the report with every answer on it,
/// the cards it named, and the changes and answers given since the report's previous snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct ReportSnapshot {
    pub id: i64,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub taken_at: Timestamp,
    pub report: Report,
    /// The cards the report's defaults named, and the ones Approve All approved, as they were.
    pub cards: Vec<crate::card::Card>,
    /// How many cards that Approve All approved.
    pub approved: i64,
    /// The person's changes and answers since the previous snapshot of this report, oldest first.
    pub round_answers: Vec<ReportAnswer>,
}

/// A snapshot as History lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct ReportSnapshotSummary {
    pub id: i64,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub taken_at: Timestamp,
    pub approved: i64,
    /// The changes and answers it records.
    pub answers: i64,
}
