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
    /// Retired with the deck's leases (Fleetlayer's leases plugin approves filters now): kept within v1, and has no
    /// effect.
    #[garde(skip)]
    pub require_filter_proposal: bool,
    /// "Show the pending count on the tab icon": the number of cards in the Inbox on the deck's favicon and in its
    /// title. On unless switched off.
    #[serde(default = "yes")]
    #[garde(skip)]
    pub show_pending_count: bool,
    /// The triage report: the default web app's write-up of the Inbox. Off, the deck is cards only: the report's
    /// routes answer 404, and the Inbox, History, Approve All and the runner ignore any report kept from before. On
    /// unless switched off.
    #[serde(default = "yes")]
    #[garde(skip)]
    pub report: bool,
    /// Chat shortcuts: replies the Inbox's chat box offers in one tap ("Just archive and nothing else"). Each is sent
    /// as an ordinary message.
    #[serde(default = "default_chat_shortcuts")]
    #[garde(length(max = 30), inner(length(min = 1, max = 500)))]
    pub chat_shortcuts: Vec<String>,
}

fn yes() -> bool {
    true
}

/// The starting list: Nathan's two, then the replies he typed most in the Inbox up to 27 Nov 2026, one per kind of
/// reply (no rewordings of the same thing).
pub fn default_chat_shortcuts() -> Vec<String> {
    [
        "Just archive and nothing else",
        "Set up a rule to do this automatically",
        "Unsubscribe if possible, then archive",
        "Start a new session to investigate and propose what to do. Don't change anything without my OK",
        "Create the story AND start a new agent to fix it",
        "This doesn't belong in my tasks",
    ]
    .map(String::from)
    .to_vec()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            undo_seconds: 3,
            require_filter_proposal: true,
            show_pending_count: true,
            report: true,
            chat_shortcuts: default_chat_shortcuts(),
        }
    }
}

/// An account label from deck config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct AccountLabel {
    pub id: String,
    pub label: String,
    /// The mailbox's address (`you@example.com`): the web app's "View email" opens a card's thread in Gmail with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub address: Option<String>,
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
    /// The owner in a browser, signed in with a passkey: `device` is that passkey's name.
    Owner { device: String },
}

/// `GET /api/info`: what agents and the web app need to know about this deck.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct DeckInfo {
    pub version: String,
    /// The release running: the commit it was built from (scripts/build-images passes it), or None for a local build.
    pub commit: Option<String>,
    /// When that release was built.
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub built_at: Option<jiff::Timestamp>,
    /// When this deck started: with rolling deploys, when the release went live (or the deck last restarted).
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub started_at: jiff::Timestamp,
    pub caller: Caller,
    pub accounts: Vec<AccountLabel>,
    pub boards: Vec<String>,
    pub settings: Settings,
    /// Quietdeck solo: the deck on the person's own machine. The Inbox then polls `/api/solo/health` and warns when
    /// the runner or its agent is failing.
    #[serde(default)]
    pub solo: bool,
    /// Quietdeck solo: the actions its runner carries out once the person approves them. A card proposing another is
    /// refused when posted. Absent on other decks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub actions: Option<Vec<crate::ActionType>>,
}

/// `GET /api/auth/state`: what the sign-in screen shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct AuthState {
    pub signed_in: bool,
    /// No passkey is registered yet: the first one needs a setup code from the deck's container.
    pub setup_needed: bool,
    /// The deck signs its owner in through Fleetlayer's sign-in service too: the web app offers it, at
    /// `/auth/signin?next=<path>`.
    #[serde(default)]
    pub signin: bool,
}

/// Start registering a passkey. Signed in, it adds one for this device; with a setup code (from
/// `quietdeck-server setup-code`), it registers the first one, or recovers a deck whose passkeys are all lost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct RegisterStart {
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 40)))]
    pub setup_code: Option<String>,
    /// What to call this passkey in Settings ("MacBook", "iPhone").
    #[garde(length(min = 1, max = 60))]
    pub device_name: String,
}

/// A WebAuthn ceremony in progress: `options` goes to `navigator.credentials`, and the result comes back with
/// `ceremony`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct Ceremony {
    pub ceremony: String,
    /// WebAuthn options as JSON: pass them to `navigator.credentials.create()` (registering) or `.get()` (signing in).
    #[ts(type = "unknown")]
    pub options: serde_json::Value,
}

/// The browser's answer to a ceremony.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct CeremonyFinish {
    pub ceremony: String,
    /// The WebAuthn credential as JSON, from `navigator.credentials.create()` or `.get()`.
    #[ts(type = "unknown")]
    pub credential: serde_json::Value,
}

/// A registered passkey, for Settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct PasskeyInfo {
    pub id: i64,
    pub device_name: String,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub created_at: jiff::Timestamp,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub last_used_at: Option<jiff::Timestamp>,
    /// The passkey this browser signed in with.
    pub this_device: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct PasskeyRename {
    #[garde(length(min = 1, max = 60))]
    pub device_name: String,
}

/// One message on the event stream (`GET /api/v1/events`): what changed, so it refetches that. Never the data
/// itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Changed {
    Cards,
    Timeline,
    Sessions,
    Settings,
    Nodes,
    Rules,
    /// Pings, their feedback and thread watches.
    Pings,
    /// Devices that take pushes.
    Push,
    /// The Drafts and Sent tabs.
    Mail,
    /// The documents (tabs) and their settings.
    Documents,
    /// The person's marks on sessions: ignored, parked.
    SessionMarks,
    /// The stream fell behind: refetch everything.
    All,
}

/// `quietdeck round done`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct RoundDone {
    /// Whether a runner on the caller's machine was told. False when none runs for this session.
    pub runner_told: bool,
}

/// `quietdeck round status`: where the runner's mail watch is, for the agent it wakes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct RoundStatus {
    /// No runner on the caller's machine runs for this session, or it isn't connected: nothing else is set.
    pub runner: bool,
    /// A wake-up went and `round done` hasn't come yet, so no other goes.
    pub waiting: bool,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub woke_at: Option<jiff::Timestamp>,
    /// The last time the watch read the mailboxes' history.
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub checked_at: Option<jiff::Timestamp>,
    pub accounts: Vec<WatchedAccount>,
}

/// One mailbox the runner watches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct WatchedAccount {
    pub id: String,
    pub label: String,
    /// When the watch started on it. Mail from before then never woke anyone: it's the agent's own to sweep, or
    /// to leave. None when the watch started before this was recorded.
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub watching_since: Option<jiff::Timestamp>,
    /// Gmail's history ID the watch has read up to.
    pub history_id: Option<String>,
    /// Messages since the last wake-up, waiting for the next one. None: some, but how many isn't known.
    pub new_since_wakeup: Option<u64>,
}

/// Web push on this deck: the VAPID public key a browser subscribes with, or none when push is off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct PushConfig {
    pub public_key: Option<String>,
}

/// A browser's push subscription (`PushSubscription.toJSON()`, flattened), with a name for the device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct PushSubscribe {
    #[garde(length(min = 1, max = 2000), pattern("^https://"))]
    pub endpoint: String,
    #[garde(length(min = 1, max = 200))]
    pub p256dh: String,
    #[garde(length(min = 1, max = 100))]
    pub auth: String,
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 100)))]
    pub device: Option<String>,
}

/// One device that takes pushes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct PushDevice {
    pub endpoint: String,
    pub device: Option<String>,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub created_at: jiff::Timestamp,
    /// When a push last reached it.
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub last_ok_at: Option<jiff::Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct PushEndpoint {
    #[garde(length(min = 1, max = 2000))]
    pub endpoint: String,
}
