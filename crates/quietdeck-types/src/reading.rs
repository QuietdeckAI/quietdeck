//! While the person reads the Inbox, nothing new lands on it unasked (Nathan, 25 November): the page sends a
//! heartbeat on activity, and while it's fresh the runner holds new mail and messages instead of waking the triage
//! agent. What still gets through: the person's own requests, updates to a thread a card on screen already shows, and
//! whatever a "while I'm reading" rule lets through. Approve All also leaves out anything that changed in the last few
//! seconds before the click, since nobody can have read it.

use garde::Validate;
use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Seconds without activity after which the person counts as idle.
pub const IDLE_AFTER_SECONDS: i64 = 120;
/// Approve All leaves out a card whose version, or whose default on the report, is younger than this.
pub const SETTLE_SECONDS: i64 = 10;

/// `POST /api/presence`: the Inbox page's heartbeat. `active: false` when the tab is hidden or closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct PresenceBeat {
    /// One ID per browser tab, so one tab going idle doesn't end another's activity.
    #[garde(length(min = 1, max = 64))]
    pub device: String,
    #[garde(skip)]
    pub active: bool,
}

/// What the deck knows about the person reading, and what the runner holds meanwhile.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct Presence {
    pub active: bool,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub active_until: Option<Timestamp>,
    /// New items the runner holds until the person is idle, e.g. "2 DocSpring, 1 in Infrastructure Alerts
    /// (Matrix)". None when nothing waits.
    pub held: Option<String>,
    pub held_count: u64,
}

/// One new item the runner asks about while the person reads: what's known without reading a body.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct ItemFacts {
    /// `gmail` or `matrix`.
    pub source: String,
    /// The mail account (`personal`), or the room's name for Matrix.
    pub account: String,
    /// The Gmail thread ID, or the room's ID.
    #[serde(default)]
    pub thread_key: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    /// Gmail's snippet, or a Matrix message's text.
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
}

/// The deck's answer for one item: let it through now (with why, for the agent and the toast), or hold it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(tag = "verdict", rename_all = "snake_case")]
#[ts(export)]
pub enum HoldVerdict {
    Let { reason: String },
    Hold,
}

/// A "while I'm reading" rule: an item that matches every condition it sets gets through while the person reads.
/// Conditions left out match anything; a rule must set at least one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct ReadingRuleInput {
    /// What it's for, as the Rules screen shows it ("Critical alerts").
    #[garde(length(min = 1, max = 200))]
    pub name: String,
    /// `gmail` or `matrix`.
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 20)))]
    pub source: Option<String>,
    /// The mail account, or the Matrix room's name.
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 255)))]
    pub account: Option<String>,
    /// The sender contains this (case doesn't matter): `@alertmanager:`, `no-reply@accounts.google.com`.
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 320)))]
    pub from: Option<String>,
    /// An RE2 regular expression the subject and text are matched against: `(?i)\(critical\)`.
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 1000)))]
    pub text_regex: Option<String>,
    /// The item carries this Gmail label.
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 255)))]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct ReadingRule {
    pub id: i64,
    #[serde(flatten)]
    #[ts(flatten)]
    pub input: ReadingRuleInput,
    pub enabled: bool,
    pub match_count: i64,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub last_matched_at: Option<Timestamp>,
}

impl ReadingRuleInput {
    /// The conditions are sound: at least one is set, and the regex compiles.
    pub fn check(&self) -> Result<(), String> {
        if self.source.is_none()
            && self.account.is_none()
            && self.from.is_none()
            && self.text_regex.is_none()
            && self.label.is_none()
        {
            return Err("a rule needs at least one condition".into());
        }
        if let Some(s) = &self.source
            && s != "gmail"
            && s != "matrix"
        {
            return Err(format!("source {s:?}: gmail or matrix"));
        }
        if let Some(r) = &self.text_regex {
            regex::RegexBuilder::new(r)
                .size_limit(1 << 20)
                .build()
                .map_err(|e| format!("the regex doesn't compile: {e}"))?;
        }
        Ok(())
    }

    /// Whether an item matches every condition this rule sets.
    pub fn matches(&self, item: &ItemFacts) -> bool {
        let ci = |hay: &str, needle: &str| hay.to_lowercase().contains(&needle.to_lowercase());
        if let Some(s) = &self.source
            && s != &item.source
        {
            return false;
        }
        if let Some(a) = &self.account
            && !a.eq_ignore_ascii_case(&item.account)
        {
            return false;
        }
        if let Some(f) = &self.from
            && !item.from.as_deref().is_some_and(|from| ci(from, f))
        {
            return false;
        }
        if let Some(l) = &self.label
            && !item.labels.iter().any(|x| x.eq_ignore_ascii_case(l))
        {
            return false;
        }
        if let Some(r) = &self.text_regex {
            let Ok(re) = regex::RegexBuilder::new(r).size_limit(1 << 20).build() else { return false };
            let subject = item.subject.as_deref().unwrap_or_default();
            let text = item.text.as_deref().unwrap_or_default();
            if !re.is_match(subject) && !re.is_match(text) {
                return false;
            }
        }
        true
    }
}

/// One change to the report, for the Inbox's toast and Approve All's settle window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct ReportChange {
    pub id: i64,
    pub version: i64,
    /// What changed, in a line: the agent's `--change`, or what the deck worked out.
    pub summary: String,
    /// The cards whose defaults this change added or altered.
    pub cards: Vec<i64>,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert(text: &str) -> ItemFacts {
        ItemFacts {
            source: "matrix".into(),
            account: "Infrastructure Alerts".into(),
            from: Some("@alertmanager:example.com".into()),
            text: Some(text.into()),
            ..ItemFacts::default()
        }
    }

    #[test]
    fn a_rule_matches_only_when_every_condition_does() {
        let rule = ReadingRuleInput {
            name: "Critical alerts".into(),
            source: Some("matrix".into()),
            account: Some("infrastructure alerts".into()),
            text_regex: Some(r"(?i)\(critical\)".into()),
            ..ReadingRuleInput::default()
        };
        assert_eq!(rule.check(), Ok(()));
        assert!(rule.matches(&alert("FIRING\nDiskFull (critical)")));
        assert!(!rule.matches(&alert("FIRING\nDiskFull (warning)")));
        let mut mail = alert("(critical)");
        mail.source = "gmail".into();
        assert!(!rule.matches(&mail));
    }

    #[test]
    fn bad_rules_are_refused() {
        assert!(ReadingRuleInput { name: "x".into(), ..Default::default() }.check().is_err());
        let bad = ReadingRuleInput { name: "x".into(), text_regex: Some("(unclosed".into()), ..Default::default() };
        assert!(bad.check().unwrap_err().contains("doesn't compile"));
        // RE2 syntax: no backreferences.
        let backref = ReadingRuleInput { name: "x".into(), text_regex: Some(r"(a)\1".into()), ..Default::default() };
        assert!(backref.check().is_err());
    }
}
