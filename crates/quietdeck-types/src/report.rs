//! The triage report: one living document the triage agent writes in Markdown and keeps up to date as mail arrives,
//! which the Inbox shows as the person's whole view of a round. Two tags put controls in it:
//!
//! - `<Default cards="c_1 c_2">forward both to the ads tab, then archive</Default>` shows "Default: …" with a
//!   Change this button. Approve All approves the cards of every default the person didn't change.
//! - `<Ask id="signin" options="Yes|No">Was this you?</Ask>` shows the question with a button per option, and with
//!   `other` an Other box too. The answer goes straight to the agent. With `cards="c_1"` it's about those cards,
//!   and the answer also shows in the thread of the default that names them.
//! - `<Ask id="budget">How much can I spend on the venue?</Ask>`, with no `options`, is a question panel: the question
//!   with its own chat thread, which he answers in words. It has no approve check, and Approve All passes it by.
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
    /// The document it's on (`GET /documents`); 1 is the Inbox.
    #[serde(default = "inbox_id")]
    pub document_id: i64,
}

fn inbox_id() -> i64 {
    crate::document::INBOX_ID
}

/// A Work document's FYI part without its items: the lines under its `# FYI` (or `## FYI`) heading that are list items,
/// with their continuation lines, up to the next heading of that level or higher, or a `---` rule. The heading and
/// any other text stay. None when there's nothing to clear.
pub fn fyi_cleared(markdown: &str) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    let mut level: Option<usize> = None;
    let mut in_item = false;
    let mut removed = false;
    for line in markdown.split('\n') {
        let trimmed = line.trim_start();
        let hashes = trimmed.chars().take_while(|c| *c == '#').count();
        let heading = hashes > 0 && trimmed[hashes..].starts_with(' ');
        if heading {
            let title = trimmed[hashes..].trim().trim_end_matches(':');
            if title.eq_ignore_ascii_case("fyi") || title.to_ascii_lowercase().starts_with("fyi ") {
                level = Some(hashes);
                in_item = false;
                out.push(line);
                continue;
            }
            if level.is_some_and(|l| hashes <= l) {
                level = None;
            }
        }
        if level.is_some() && line.trim() == "---" {
            level = None;
        }
        if level.is_some() {
            let indent = line.len() - trimmed.len();
            let item = indent == 0
                && (trimmed.starts_with("- ") || trimmed.starts_with("* ") || numbered_item_width(trimmed).is_some());
            if item {
                in_item = true;
                removed = true;
                continue;
            }
            if in_item && indent > 0 && !trimmed.is_empty() {
                continue;
            }
            if !trimmed.is_empty() {
                in_item = false;
            }
        }
        out.push(line);
    }
    if !removed {
        return None;
    }
    // No run of blank lines where the items were.
    let mut joined = out.join("\n");
    while joined.contains("\n\n\n") {
        joined = joined.replace("\n\n\n", "\n\n");
    }
    Some(joined)
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
    /// What this edit changes, in a line, for the toast the person sees ("New: CI failure on web (c_67)").
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

/// `quietdeck chat reply`: the agent's answer to the person's chat in the Inbox, shown as a toast.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct ChatReply {
    pub text: String,
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

/// Each `<Ask>`'s key (its `id`, else its text) and whether it's a question panel (no `options`: answered in words, in
/// a thread of its own).
pub fn ask_keys(markdown: &str) -> Vec<(String, bool)> {
    let mut out = vec![];
    let mut rest = markdown;
    while let Some(at) = rest.find("<Ask") {
        rest = &rest[at + "<Ask".len()..];
        if !rest.starts_with(|c: char| c.is_whitespace() || c == '>') {
            continue;
        }
        let Some(end) = rest.find('>') else { break };
        let attrs = &rest[..end];
        let body = &rest[end + 1..];
        let text = body.find("</Ask>").map(|c| body[..c].trim()).unwrap_or_default();
        let key = attr(attrs, "id").map(str::to_string).unwrap_or_else(|| text.to_string());
        out.push((key, attr(attrs, "options").is_none_or(|o| o.trim().is_empty())));
        rest = &rest[end..];
    }
    out
}

/// A `<Default>` written flush left right after a numbered item (with or without a blank line between) belongs to
/// that item: it's indented to the item's text, with no blank line, so the Inbox draws it inside the item. Written the
/// other way it renders as a group's Default, flush left with a gap above (the triage agent did that on 27 Nov 2026,
/// and every item's panel moved). Bulleted lists are left alone: a flush-left Default after one is a group's.
/// Everything else is unchanged, and running it twice changes nothing more.
pub fn item_defaults_indented(markdown: &str) -> String {
    let lines: Vec<&str> = markdown.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    // The open numbered item's text indent ("1. " is 3), while its lines continue.
    let mut item: Option<usize> = None;
    for line in lines {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if indent == 0
            && trimmed.starts_with("<Default")
            && let Some(width) = item
        {
            while out.last().is_some_and(|l| l.trim().is_empty()) {
                out.pop();
            }
            out.push(format!("{}{line}", " ".repeat(width)));
            continue;
        }
        if indent == 0 && !trimmed.is_empty() {
            item = numbered_item_width(trimmed);
        } else if indent > 0 && trimmed.starts_with("<Default") {
            // The item has its own Default already: a flush one after it is a group's.
            item = None;
        }
        out.push(line.to_string());
    }
    out.join("\n")
}

/// `"12. text"` → Some(4), the indent of the item's text; anything else → None.
fn numbered_item_width(line: &str) -> Option<usize> {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits > 3 {
        return None;
    }
    let rest = &line[digits..];
    rest.strip_prefix(". ").map(|_| digits + 2)
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
    fn a_flush_default_after_a_numbered_item_moves_inside_it() {
        let written = "# Inbox\n\n1. **Sentry:** a new error.\n\n<Default id=\"s\" cards=\"c_1\">Forward</Default>\n\n\
                       10. **Renovate:** two PRs.\n<Default cards=\"c_2\">Archive</Default>\n";
        let fixed = item_defaults_indented(written);
        assert_eq!(
            fixed,
            "# Inbox\n\n1. **Sentry:** a new error.\n   <Default id=\"s\" cards=\"c_1\">Forward</Default>\n\n\
             10. **Renovate:** two PRs.\n    <Default cards=\"c_2\">Archive</Default>\n"
        );
        assert_eq!(item_defaults_indented(&fixed), fixed, "running it again changes nothing");
    }

    #[test]
    fn group_defaults_and_ones_already_inside_items_are_left_alone() {
        let right = "1. **One:** text.\n   <Default cards=\"c_1\">Archive</Default>\n2. **Two:** text.\n   more text.\n   \
                     <Default cards=\"c_2\">Archive</Default>\n\n## Archiving\n\n- a\n- b\n\n\
                     <Default id=\"archive-all\" cards=\"c_3 c_4\">Archive all</Default>\n";
        assert_eq!(item_defaults_indented(right), right);
        // After a heading or a paragraph that closed the numbered list, a flush Default stays a group's.
        let after = "1. **One:** text.\n   <Default cards=\"c_1\">Archive</Default>\n\nSome words.\n\n\
                     <Default cards=\"c_5\">Keep in inbox</Default>\n";
        assert_eq!(item_defaults_indented(after), after);
        // A numbered item with its own Default, then a group's after the list.
        let group = "1. **One:** text.\n   <Default cards=\"c_1\">Archive</Default>\n\n\
                     <Default id=\"keep\" cards=\"c_6 c_7\">Keep in inbox</Default>\n";
        assert_eq!(item_defaults_indented(group), group);
    }

    #[test]
    fn the_fyi_part_is_cleared_and_everything_else_stays() {
        let md = "# Needs you\n\n1. **enterprise:** PR #12 waits for review.\n\n## FYI\n\n- `ci` finished: green.\n\
                  - `docs` shipped the guide\n  with screenshots.\n\nKept text.\n\n# Later\n\n- stays\n";
        assert_eq!(
            fyi_cleared(md).unwrap(),
            "# Needs you\n\n1. **enterprise:** PR #12 waits for review.\n\n## FYI\n\nKept text.\n\n# Later\n\n- stays\n"
        );
        assert_eq!(fyi_cleared("# Needs you\n\n- a\n"), None);
        assert_eq!(fyi_cleared("# FYI\n\nNothing new.\n"), None);
    }

    #[test]
    fn asks_are_keyed_and_question_panels_have_no_options() {
        let md = "<Ask id=\"me\" options=\"Yes|No\">Was this you?</Ask>\n<Ask id=\"budget\">How much?</Ask>\n\
                  <Ask>What next?</Ask>\n<Asking>not a tag</Asking>";
        assert_eq!(
            ask_keys(md),
            vec![("me".to_string(), false), ("budget".to_string(), true), ("What next?".to_string(), true)]
        );
    }

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
