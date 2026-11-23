//! REST API shapes that aren't a record of their own: errors, enrolment, settings, the caller.

use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::action::SessionRef;

/// Every error the REST API returns, with an HTTP status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct ApiError {
    pub error: String,
}

/// `quietdeck node enroll <code>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct EnrolRequest {
    #[garde(length(min = 8, max = 100))]
    pub code: String,
    /// The machine's name on the deck: letters, digits, dots, dashes and underscores.
    #[garde(length(min = 1, max = 64), pattern(r"^[A-Za-z0-9][A-Za-z0-9._-]*$"))]
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct Enrolled {
    pub node: String,
    /// Shown once. The node keeps it in a file only its user can read.
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct EnrolmentCode {
    pub code: String,
    /// Unix seconds.
    pub expires_at: i64,
}

/// The deck's settings (the Settings screen).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct Settings {
    /// Approve All's undo window in seconds: 0 (off) or 1 to 60.
    #[garde(range(min = 0, max = 60))]
    pub undo_seconds: u32,
    /// "Require a proposal for every new filter". A hard rule in code while on.
    #[garde(skip)]
    pub require_filter_proposal: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { undo_seconds: 3, require_filter_proposal: true }
    }
}

/// An account label from deck config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct AccountLabel {
    pub id: String,
    pub label: String,
}

/// Who the deck thinks is calling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum Caller {
    /// An agent (or a shell) on a node, with a session token.
    Session { session: SessionRef },
    /// A node itself (its runner), with the node token.
    Node { node: String },
    /// The owner in a browser.
    Owner { login: String },
}

/// `GET /api/info`: what agents and the web app need to know about this deck.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct DeckInfo {
    pub version: String,
    pub caller: Caller,
    pub accounts: Vec<AccountLabel>,
    pub boards: Vec<String>,
    pub settings: Settings,
}
