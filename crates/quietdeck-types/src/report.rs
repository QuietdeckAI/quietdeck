//! The triage report: one living document the triage agent writes in Markdown and keeps up to date as mail arrives,
//! which the Inbox shows as the person's whole view of a round. Two tags put controls in it:
//!
//! - `<Default cards="c_1 c_2">forward both to the ads tab, then archive</Default>` shows "Default: …" with a
//!   Change this button. Approve All approves the cards of every default the person didn't change.
//! - `<Ask id="signin" options="Yes|No">Was this you?</Ask>` shows the question with a button per option, and with
//!   `other` an Other box too. The answer goes straight to the agent.
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
    /// What to do instead of a `<Default>`.
    Change,
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
