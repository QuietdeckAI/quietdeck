//! Documents: the living Markdown pages an agent writes and the person reads, each under a top-level tab of the web
//! app. The Inbox document (today's mail, kind `inbox`) is the one every deck has; a `work` document is a page about
//! the person's agents (what needs them, what finished), written by an overseer agent. Each document has its own
//! report, answers, chat and Approve All. A top-level tab with one document shows no sub-tabs; with more, it shows
//! one per document, in `position` order.
//!
//! Session marks are the person's own records about sessions, kept for every document: an ignored session (the
//! overseer leaves it out unless it's blocked on a prompt) and a parked one (its entries go to the collapsed Parked
//! part, indefinitely or until a date).

use garde::Validate;
use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The ID of the Inbox document every deck has.
pub const INBOX_ID: i64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DocumentKind {
    /// Mail: cards about email threads, an archive part with Archive all, and a new report per round.
    Inbox,
    /// Sessions: entries about the person's agents, an FYI part that Approve All clears, and one report that's never
    /// replaced, only kept up to date and pruned.
    Work,
}

impl DocumentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DocumentKind::Inbox => "inbox",
            DocumentKind::Work => "work",
        }
    }

    pub fn parse(s: &str) -> Option<DocumentKind> {
        match s {
            "inbox" => Some(DocumentKind::Inbox),
            "work" => Some(DocumentKind::Work),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct Document {
    pub id: i64,
    /// Its name in URLs and `?doc=`: `inbox`, `work`, `docspring`.
    pub slug: String,
    /// What the person calls it, shown as its sub-tab: free text.
    pub name: String,
    /// The top-level tab it sits under: free text ("Inbox", "Work").
    pub tab: String,
    pub kind: DocumentKind,
    /// The agent that writes it: an agent's report commands go to the document it writes. None: any agent may (the
    /// Inbox, as before documents).
    pub writer_node: Option<String>,
    pub writer_tab: Option<String>,
    /// Its order among the documents, lowest first; top-level tabs follow their first document.
    pub position: i64,
}

/// A new document (Settings, or `POST /documents`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct DocumentAdd {
    #[garde(length(min = 1, max = 40), pattern(r"^[a-z0-9][a-z0-9-]*$"))]
    pub slug: String,
    #[garde(length(min = 1, max = 80))]
    pub name: String,
    #[garde(length(min = 1, max = 40))]
    pub tab: String,
    #[garde(skip)]
    pub kind: DocumentKind,
    /// The agent that writes it (both or neither).
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 100)))]
    pub writer_node: Option<String>,
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 200)))]
    pub writer_tab: Option<String>,
    #[serde(default)]
    #[garde(skip)]
    pub position: Option<i64>,
}

/// A change to a document: only the fields given change. `clear_writer` makes it writable by any agent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct DocumentEdit {
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 80)))]
    #[ts(optional)]
    pub name: Option<String>,
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 40)))]
    #[ts(optional)]
    pub tab: Option<String>,
    /// A new writer (both or neither).
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 100)))]
    #[ts(optional)]
    pub writer_node: Option<String>,
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 200)))]
    #[ts(optional)]
    pub writer_tab: Option<String>,
    /// Any agent may write it from now on.
    #[serde(default)]
    #[garde(skip)]
    #[ts(optional)]
    pub clear_writer: Option<bool>,
    #[serde(default)]
    #[garde(skip)]
    #[ts(optional)]
    pub position: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MarkKind {
    /// The overseer leaves the session out of Work, unless it's blocked on a prompt.
    Ignored,
    /// The person put the session's work aside: its entries show in the collapsed Parked part, until `until` or until
    /// unparked.
    Parked,
}

impl MarkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MarkKind::Ignored => "ignored",
            MarkKind::Parked => "parked",
        }
    }

    pub fn parse(s: &str) -> Option<MarkKind> {
        match s {
            "ignored" => Some(MarkKind::Ignored),
            "parked" => Some(MarkKind::Parked),
            _ => None,
        }
    }
}

/// The person's mark on a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct SessionMark {
    pub node: String,
    pub tab: String,
    pub mark: MarkKind,
    /// A parked session comes back on its own at this time (snoozed); None: until unparked.
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub until: Option<Timestamp>,
    /// What it's about, in the person's words or the entry's headline, for the Parked list.
    pub note: Option<String>,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub at: Timestamp,
}

/// Set a mark (it replaces the same kind of mark on that session).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct SessionMarkSet {
    #[garde(length(min = 1, max = 100))]
    pub node: String,
    #[garde(length(min = 1, max = 200))]
    pub tab: String,
    #[garde(skip)]
    pub mark: MarkKind,
    #[serde(default)]
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    #[garde(skip)]
    pub until: Option<Timestamp>,
    #[serde(default)]
    #[garde(inner(length(max = 500)))]
    pub note: Option<String>,
}

/// Take a mark off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct SessionMarkRemove {
    #[garde(length(min = 1, max = 100))]
    pub node: String,
    #[garde(length(min = 1, max = 200))]
    pub tab: String,
    #[garde(skip)]
    pub mark: MarkKind,
}
