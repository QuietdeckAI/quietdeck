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
    /// The latest changes, newest last (at most 20): the Inbox toasts new ones.
    #[serde(default)]
    pub changes: Vec<crate::reading::ReportChange>,
    /// The agent said nothing on it waits for the person any more (`quietdeck report done`): the Inbox shows "All
    /// done." until the agent edits it or starts the next one.
    #[serde(default)]
    pub finished: bool,
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
    /// What this edit changes, in a line, for the toast the person sees ("New: CI failure on homeserver (c_67)").
    /// Left out, the deck works one out from the defaults that changed.
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 300)))]
    pub change: Option<String>,
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
        // The line it's on, so the error points at it.
        let line = markdown[..markdown.len() - rest.len() + at].matches('\n').count() + 1;
        rest = &rest[at + "<Default".len()..];
        let Some(end) = rest.find('>') else { break };
        let attrs = &rest[..end];
        if let Some(v) = attr(attrs, "cards") {
            for part in v.split(|c: char| c.is_whitespace() || c == ',').filter(|p| !p.is_empty()) {
                let digits = part.strip_prefix("c_").unwrap_or(part);
                match digits.parse::<i64>() {
                    Ok(n) => ids.push(n),
                    Err(_) => bad.push(format!("line {line}: {part:?} isn't a card ID (c_12)")),
                }
            }
        } else {
            bad.push(format!("line {line}: a <Default> has no cards=\"…\""));
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

/// Each `<Default>`: its key (as [`default_keys`] gives it), the cards it names, its text, and what it's about: the
/// list item or line it belongs to ("New sign-in to your work account"), empty for a default with its own `id` (a
/// group's "Archive all").
pub fn default_blocks(markdown: &str) -> Vec<(String, Vec<i64>, String, String)> {
    let mut out = vec![];
    let mut rest = markdown;
    while let Some(at) = rest.find("<Default") {
        let before = &markdown[..markdown.len() - rest.len() + at];
        rest = &rest[at + "<Default".len()..];
        let Some(end) = rest.find('>') else { break };
        let attrs = &rest[..end];
        let cards = attr(attrs, "cards").unwrap_or("");
        let key = attr(attrs, "id").map(str::to_string).unwrap_or_else(|| format!("cards:{cards}"));
        let about = if attr(attrs, "id").is_some() { String::new() } else { about_line(before) };
        rest = &rest[end + 1..];
        let text = match rest.find("</Default>") {
            Some(close) => rest[..close].trim().to_string(),
            None => String::new(),
        };
        out.push((key, card_ids(cards), text, about));
    }
    out
}

/// What a default is about: the text before it on its line, else the nearest line above, without list markers or
/// emphasis, and only its bold part when it has one ("**Acme: invoice overdue.** They ask..." gives the title).
fn about_line(before: &str) -> String {
    let same = before.rsplit('\n').next().unwrap_or_default();
    let line = if same.trim().is_empty() {
        before.lines().rev().map(str::trim).find(|l| !l.is_empty() && !l.contains("<Default")).unwrap_or_default()
    } else {
        same.trim()
    };
    let line = line.trim_start_matches(|c: char| c.is_ascii_digit());
    let line = line.strip_prefix(". ").or_else(|| line.strip_prefix(") ")).unwrap_or(line);
    let line = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")).unwrap_or(line).trim_start();
    let line = match line.strip_prefix("**").and_then(|r| r.split_once("**")) {
        Some((bold, _)) => bold,
        None => line,
    };
    line.replace("**", "").replace('`', "").trim().trim_end_matches(['.', ':']).to_string()
}

/// What changed between two versions of a report's defaults: the cards whose default is new or reads differently,
/// and a line saying so ("New: Archive (c_3); changed: Forward to ads (c_1, c_2)"). None when no default changed.
pub fn defaults_changed(old: &str, new: &str) -> Option<(Vec<i64>, String)> {
    let before = default_blocks(old);
    let mut cards = vec![];
    let mut added = vec![];
    let mut altered = vec![];
    for (_, ids, text, about) in default_blocks(new) {
        let was = before.iter().find(|(_, b_ids, _, _)| b_ids.iter().any(|i| ids.contains(i)));
        let what = if about.is_empty() { short(&text) } else { format!("{} → {}", short(&about), short(&text)) };
        let label = format!("{what} ({})", ids.iter().map(|i| format!("c_{i}")).collect::<Vec<_>>().join(", "));
        match was {
            None => added.push(label),
            Some((_, b_ids, b_text, _)) if b_text != &text || b_ids != &ids => altered.push(label),
            Some(_) => continue,
        }
        cards.extend(ids);
    }
    if cards.is_empty() {
        return None;
    }
    let mut parts = vec![];
    if !added.is_empty() {
        parts.push(format!("New: {}", added.join("; ")));
    }
    if !altered.is_empty() {
        parts.push(format!("{}: {}", if added.is_empty() { "Changed" } else { "changed" }, altered.join("; ")));
    }
    cards.sort_unstable();
    cards.dedup();
    Some((cards, parts.join("; ")))
}

fn short(text: &str) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= 60 { one } else { format!("{}…", one.chars().take(59).collect::<String>()) }
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
        assert_eq!(bad, vec!["line 5: \"x9\" isn't a card ID (c_12)".to_string()]);
        assert_eq!(default_cards("<Default>x</Default>").1, vec!["line 1: a <Default> has no cards=\"…\"".to_string()]);
        assert_eq!(default_cards("# A\n\n<Default>x</Default>").1[0], "line 3: a <Default> has no cards=\"…\"");
    }

    #[test]
    fn changed_defaults_are_named() {
        let old = "<Default cards=\"c_1 c_2\">Forward to ads</Default>\n<Default cards=\"c_3\">Archive</Default>";
        assert_eq!(defaults_changed(old, old), None);
        assert_eq!(defaults_changed(old, &format!("# New heading\n{old}")), None, "text outside defaults");
        let new = "<Default cards=\"c_1 c_2\">Forward to ads, then archive</Default>\n\
                   <Default cards=\"c_3\">Archive</Default>\n<Default cards=\"c_4\">Reply</Default>";
        let (cards, line) = defaults_changed(old, new).unwrap();
        assert_eq!(cards, vec![1, 2, 4]);
        assert_eq!(line, "New: Reply (c_4); changed: Forward to ads, then archive (c_1, c_2)");
        let titled = format!(
            "{old}\n2. **Acme: invoice overdue.** They ask by Friday.\n   <Default cards=\"c_9\">Pay it</Default>"
        );
        assert_eq!(defaults_changed(old, &titled).unwrap().1, "New: Acme: invoice overdue → Pay it (c_9)");
        let joined =
            "<Default cards=\"c_1 c_2 c_5\">Forward to ads</Default>\n<Default cards=\"c_3\">Archive</Default>";
        assert_eq!(defaults_changed(old, joined).unwrap().0, vec![1, 2, 5]);
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
