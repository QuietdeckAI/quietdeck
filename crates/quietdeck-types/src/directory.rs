//! The sessions directory: every node's sessions with herdr's metadata, summaries and work links, as the deck
//! stores and serves them.

use garde::Validate;
use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::session::{AgentKind, AgentStatus};

/// What a node knows about one of its sessions from herdr (and Claude Code's registry). Pushed in snapshots and on
/// every change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct SessionInfo {
    pub session_id: String,
    pub agent: AgentKind,
    pub workspace: Option<String>,
    pub tab: Option<String>,
    pub pane: Option<String>,
    pub cwd: Option<String>,
    pub status: AgentStatus,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub status_changed_at: Timestamp,
    /// The command that resumes it, to copy until herdr's create-and-resume is there.
    pub resume_command: Option<String>,
    /// Whether the agent is running now.
    pub running: bool,
}

/// A session as the deck serves it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct SessionRow {
    pub node: String,
    #[serde(flatten)]
    pub info: SessionInfo,
    pub project: Option<String>,
    pub sentence: Option<String>,
    pub paragraph: Option<String>,
    pub page: Option<String>,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub last_activity: Option<Timestamp>,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub pushed_at: Timestamp,
    /// The node is connected now. When it isn't, everything above is as last pushed.
    pub node_online: bool,
    pub links: Vec<SessionLink>,
}

impl SessionRow {
    pub fn address(&self) -> String {
        format!("{}@{}", self.info.tab.as_deref().unwrap_or(&self.info.session_id), self.node)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LinkKind {
    GithubPr,
    ShortcutStory,
    LinearIssue,
    Link,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LinkState {
    Open,
    Merged,
    Closed,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct SessionLink {
    pub url: String,
    pub kind: LinkKind,
    /// `owner/repo#1234`, `sc-6450`, `ENG-12`.
    pub r#ref: Option<String>,
    pub title: Option<String>,
    pub state: Option<LinkState>,
    /// `agent`, or the detector's name.
    pub source: String,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub added_at: Timestamp,
}

/// `quietdeck session link add <url>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct LinkAdd {
    #[garde(url, length(max = 2000))]
    pub url: String,
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 500)))]
    pub title: Option<String>,
    #[serde(default)]
    #[garde(skip)]
    pub state: Option<LinkState>,
    /// Defaults to the calling session.
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 300)))]
    pub session: Option<String>,
    /// Set by detectors; agents leave it empty.
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 100)))]
    pub source: Option<String>,
}

/// Where a "who has this ID?" hit came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum HitSource {
    Summary,
    Transcript,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct WhoHasHit {
    pub node: String,
    pub session_id: String,
    pub tab: Option<String>,
    pub source: HitSource,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub last_mentioned_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct WhoHas {
    pub query: String,
    pub hits: Vec<WhoHasHit>,
    /// Nodes whose transcripts weren't searched (offline, or didn't answer in time): their sessions were searched by
    /// summary only.
    pub offline_nodes: Vec<String>,
}

/// One transcript hit from a node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct TranscriptHit {
    pub session_id: String,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub last_mentioned_at: Option<Timestamp>,
}

/// A node as the deck serves it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct NodeRow {
    pub name: String,
    pub online: bool,
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub last_seen_at: Option<Timestamp>,
    pub version: Option<String>,
    pub runner: bool,
    pub runner_types: Vec<crate::action::ActionType>,
}
