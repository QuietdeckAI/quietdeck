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
    /// Where the orchestrator runs it, in the orchestrator's own terms (herdr: a pane ID). Opaque to the deck.
    #[serde(alias = "pane")]
    pub address: Option<String>,
    pub cwd: Option<String>,
    pub status: AgentStatus,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub status_changed_at: Timestamp,
    /// The command that resumes it, to copy until herdr's create-and-resume is there.
    pub resume_command: Option<String>,
    /// Whether the agent is running now.
    pub running: bool,
    /// A link that brings the session forward in its orchestrator on the machine it runs on (herdr: `herdr://open`).
    #[serde(default)]
    pub open_url: Option<String>,
    /// The orchestrator's name, for "Open in …".
    #[serde(default)]
    pub opens_in: Option<String>,
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
    /// When herdr reported it closed. A session that isn't running and has no `closed_at` went away without a close
    /// event: herdr stopped, or the machine shut down.
    #[ts(type = "string | null")]
    #[schemars(with = "Option<String>")]
    pub closed_at: Option<Timestamp>,
    pub ended: Option<SessionEnd>,
}

/// How a session ended, from herdr's close events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(tag = "how", rename_all = "snake_case")]
#[ts(export)]
pub enum SessionEnd {
    /// Its tab closed: a person closed it, or the last process in it exited.
    TabClosed,
    /// The agent's process exited. `terminal_closed` is false when it went back to its terminal's shell and the tab
    /// stayed.
    AgentExited {
        final_status: Option<AgentStatus>,
        #[serde(alias = "pane_exited")]
        terminal_closed: bool,
    },
}

fn no_leading_dash(value: &str, _: &()) -> garde::Result {
    // Model and effort become arguments to the agent's command line: a value that starts with `-` would be read as
    // a flag, and the permission flags come only from node.yaml.
    if value.starts_with('-') || value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(garde::Error::new("can't start with - or hold spaces"));
    }
    Ok(())
}

fn printable(value: &str, _: &()) -> garde::Result {
    if value.chars().any(char::is_control) {
        return Err(garde::Error::new("can't hold control characters"));
    }
    Ok(())
}

/// Start a session on a machine (flow 3.15). The node checks it, then runs `herdr tab create --agent`, with the
/// agent's permission flags from its own node.yaml.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct SessionStart {
    /// The machine (node) it runs on.
    #[garde(length(min = 1, max = 100))]
    pub on: String,
    #[garde(length(min = 1, max = 100), custom(printable))]
    pub tab: String,
    /// The session's name. The tab name when absent.
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 100), custom(printable)))]
    pub name: Option<String>,
    #[garde(skip)]
    pub agent: AgentKind,
    /// The agent's own default when absent.
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 100), custom(no_leading_dash)))]
    pub model: Option<String>,
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 40), custom(no_leading_dash)))]
    pub effort: Option<String>,
    /// The working directory on that machine. `~/` is that machine's home.
    #[garde(length(min = 1, max = 1000), custom(printable))]
    pub cwd: String,
    /// Sent with `herdr agent prompt` once the agent is ready.
    #[serde(default)]
    #[garde(inner(length(min = 1, max = 20000)))]
    pub prompt: Option<String>,
}

/// Resume a session that isn't running, in a new tab on its machine, from what the deck knows about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct SessionResume {
    pub session_id: String,
    pub agent: AgentKind,
    /// Its last tab name; the new tab takes it.
    pub tab: Option<String>,
    pub cwd: String,
}

/// `quietdeck session resume <session>` and the registry's Resume and Close: a session by address (`tab@machine`,
/// a tab name, or a session ID).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct SessionTarget {
    #[garde(length(min = 1, max = 300))]
    pub session: String,
}

/// What the node did for a start or a resume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct SessionStarted {
    pub node: String,
    pub tab: String,
    /// The new session's address in the orchestrator.
    #[serde(alias = "pane")]
    pub address: String,
    /// Absent when the agent hadn't reported it yet (Codex before its first turn without `/status`).
    pub session_id: Option<String>,
    /// Set when the session started but its initial prompt couldn't be sent.
    pub prompt_error: Option<String>,
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
    /// Nodes that ran out of time: they searched their transcripts back to this time, newest first.
    pub partial: Vec<PartialSearch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct PartialSearch {
    pub node: String,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub searched_back_to: jiff::Timestamp,
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
    /// The action types the runner handed to its agent for lack of a handler, most often first: candidates for the
    /// next handler. Types only, no counts.
    #[serde(default)]
    pub no_handler: Vec<crate::action::ActionType>,
}

/// `quietdeck sessions show <session>`: one session, and with `--timeline` every summary point it has, oldest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct SessionDetail {
    pub session: SessionRow,
    pub timeline: Vec<crate::summary::SummaryRecord>,
}
