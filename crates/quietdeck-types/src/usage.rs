//! Account usage: what each node's usage module reads about the Claude and Codex subscriptions signed in on its
//! machine, and pushes to the deck. Readings only: no token, refresh token or sign-in secret ever leaves the machine.
//!
//! The deck merges the same subscription signed in on several machines into one row, matched on
//! (`provider`, `account_key`), and keeps the newest reading per window. The module's interface is in
//! `docs/usage-interface.md` and `crates/quietdeck-node/src/usage/mod.rs`.

use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum UsageProvider {
    /// Claude subscriptions (Claude Code sign-ins).
    Anthropic,
    /// ChatGPT subscriptions (Codex sign-ins).
    Openai,
}

/// Which limit a window is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum UsageWindowKind {
    /// Claude's 5-hour session window.
    FiveHour,
    /// Claude's weekly limit for the whole subscription.
    Week,
    /// A model family's own week (`model` names it, e.g. `Fable`).
    ModelWeek,
    /// Codex's primary rate-limit window (`window_minutes` says how long).
    CodexPrimary,
    /// Codex's secondary rate-limit window.
    CodexSecondary,
}

/// One limit as last read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct UsageWindow {
    pub kind: UsageWindowKind,
    /// For `model_week`: the model family, as the usage endpoint names it.
    pub model: Option<String>,
    /// The window's length, where the provider reports it (Codex does).
    pub window_minutes: Option<u32>,
    /// Percent used, 0 to 100, as last read. Count it as 0 once `resets_at` has passed.
    pub pct: f64,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub resets_at: Option<Timestamp>,
}

/// Whether the machine can use the sign-in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum UsageHealth {
    Ok,
    /// Needs a browser sign-in on this machine; `why` is a sentence for the warning.
    SigninNeeded { why: String },
    /// A browser sign-in is under way.
    SigningIn {
        #[ts(type = "string")]
        #[schemars(with = "String")]
        since: Timestamp,
    },
}

/// Where the newest reading came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ReadingSource {
    /// The node polled the provider's usage endpoint.
    Poll,
    /// A session reported the rate-limit headers of a response (Claude), or Codex wrote a snapshot into its log.
    Session,
}

/// One subscription as one machine sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct AccountUsage {
    pub provider: UsageProvider,
    /// The provider's stable ID for the billed subscription, the same on every machine: for Anthropic the account
    /// UUID and organisation UUID from the profile endpoint as `<account>/<org>`; for OpenAI the ChatGPT account ID.
    /// The deck's merge key, with `provider`.
    pub account_key: String,
    /// This machine's own ID for the sign-in (the coordinator's `"1"`, `"2"`; Codex's profile name). Commands from
    /// the deck use it.
    pub local_id: String,
    pub email: Option<String>,
    pub org_name: Option<String>,
    /// `max`, `pro`, `team`, `plus`...
    pub plan: Option<String>,
    /// Plan size for combined bars: 10 for Ultra, 5 for Max, 1 for Pro or unknown.
    pub weight: u32,
    pub health: UsageHealth,
    pub windows: Vec<UsageWindow>,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub read_at: Option<Timestamp>,
    pub source: Option<ReadingSource>,
    /// Live sessions on this sign-in on this machine.
    pub session_count: u32,
    /// Being removed from this machine.
    pub retiring: bool,
}

/// Every subscription on one machine, pushed whenever any of it changes. A full snapshot, so an account missing
/// from it is no longer signed in on that machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct UsagePush {
    pub node: String,
    pub accounts: Vec<AccountUsage>,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub at: Timestamp,
}

/// What the deck (the Usage screen's buttons) or the menu bar app asks a node's usage module to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum UsageCommand {
    /// Start the browser sign-in for one sign-in, or for every one that needs it.
    SignIn { local_id: Option<String> },
    /// Re-read now (the usage too, at most once per 60 s per account).
    Check { local_id: Option<String> },
}

/// `usage:` in node.yaml.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(default)]
#[ts(export)]
pub struct UsageConfig {
    pub enabled: bool,
    /// Poll each account's usage endpoint at most this often, and less while sessions report readings.
    pub poll_minutes: u32,
    /// Read Codex sign-ins too.
    pub codex: bool,
    /// Serve the account rotator's coordinator API on its socket, so the rotation plugin keeps working.
    pub serve_coordinator_api: bool,
}

impl Default for UsageConfig {
    fn default() -> Self {
        UsageConfig { enabled: true, poll_minutes: 5, codex: true, serve_coordinator_api: true }
    }
}
