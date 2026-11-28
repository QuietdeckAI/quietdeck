//! Fleetlayer's access requests on the deck: what an agent asked to do that Fleetlayer's policy says the person must
//! approve (start a session elsewhere, use a tool, open an admin window), and the admin windows that are open. The
//! deck reads and decides them with an `approve` token, which can do nothing else; opening a window also needs the
//! person's passkey for that one request, which Fleetlayer's control plane checks.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One pending request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct FleetApproval {
    pub id: String,
    /// Who asks (`ops@server-admin`).
    pub from: String,
    /// What it asks to do (`start`, `use_tool`, `activate`).
    pub verb: String,
    /// On what (`node:laptop`, `tool:browser@laptop`, `window:admin`).
    pub to: String,
    /// Milliseconds since the epoch.
    #[ts(type = "number")]
    pub created_at: i64,
    /// When the request lapses, if it does.
    #[ts(type = "number | null")]
    pub expires_at: Option<i64>,
    /// The request in plain words, built by Fleetlayer from access control's own names only ("The cookies plugin on
    /// laptop wants to send its x set to worker"): shown first. Fleetlayer v0.4.2 and later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub line: Option<String>,
    /// What else Fleetlayer says about it (the call's arguments, the reason): the asker's own text, outside data.
    pub detail: Option<String>,
    /// Approving allows one use only.
    pub once: bool,
    /// It asks to open a role's window: approving needs the person's passkey first.
    pub window: bool,
}

/// An open admin window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct FleetWindow {
    pub id: String,
    pub role: String,
    pub requested_by: Option<String>,
    #[ts(type = "number | null")]
    pub opened_at: Option<i64>,
    /// When it closes by itself, in milliseconds since the epoch.
    #[ts(type = "number | null")]
    pub ends_at: Option<i64>,
    /// What went wrong closing an earlier one, if a node reported a problem.
    pub problem: Option<String>,
}

/// `GET /fleetlayer/approvals`: the pending requests and the open windows.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct FleetApprovals {
    /// The deck has an approve token. Without one, there's nothing to show.
    pub configured: bool,
    pub approvals: Vec<FleetApproval>,
    pub windows: Vec<FleetWindow>,
    /// Why Fleetlayer couldn't be read, when it couldn't.
    pub error: Option<String>,
}

/// `POST /fleetlayer/approvals/{id}`: approve or refuse.
#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct FleetDecide {
    pub approve: bool,
    /// How long an approval holds (`10m`, `1h`); the request's own when absent. For a window, how long it's open.
    #[serde(default, rename = "for", skip_serializing_if = "Option::is_none")]
    #[ts(optional, rename = "for")]
    pub hold: Option<String>,
}

/// What deciding did. A window that needs the passkey first isn't decided: `stepup_url` is the sign-in service's
/// page to confirm it on, which comes back to the deck, and the deck then approves it again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct FleetDecided {
    pub decided: bool,
    pub stepup_url: Option<String>,
}
