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
    /// The harness's own menu it's waiting on (blocked), when its node reads one through Fleetlayer.
    #[serde(default)]
    pub prompt: Option<BlockedPrompt>,
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
    /// The handoff it starts from (`quietdeck handoff add`): the new session continues the one that wrote it, and
    /// without a `prompt` its first prompt tells it to read the handoff. The deck's alone: never sent to the node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(skip)]
    #[ts(optional)]
    pub handoff: Option<i64>,
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
    /// `tab@machine` of the session that continues this one, when it was moved: ask that one instead.
    #[serde(default)]
    pub moved_to: Option<String>,
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
    /// The session this one continues, when it was started from a handoff.
    pub continues: Option<crate::handoff::MoveLink>,
    /// The session that continues this one, when it was moved.
    pub moved_to: Option<crate::handoff::MoveLink>,
}

/// A blocked prompt: one of the harness's own menus (Claude Code's question form, a permission prompt, a plan to
/// approve) waiting in a session, as Fleetlayer reads it (`meta.prompt`). The Work tab shows it as a form; the answer
/// goes back through Fleetlayer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct BlockedPrompt {
    /// Fleetlayer's ID for this prompt: stable while it shows, new for every prompt. An answer names it.
    pub id: String,
    /// `question`, `permission` or `plan`.
    pub kind: String,
    /// When it appeared, as Fleetlayer gives it.
    #[serde(default)]
    pub at: Option<String>,
    /// A question form's questions, in order.
    #[serde(default)]
    pub questions: Vec<PromptQuestion>,
    /// A permission or plan prompt's choices.
    #[serde(default)]
    pub options: Vec<PromptOption>,
    /// A permission prompt's tool, by name, and a short form of its input.
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub tool_input: Option<String>,
    /// A plan prompt's plan.
    #[serde(default)]
    pub plan: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct PromptQuestion {
    #[serde(default)]
    pub header: Option<String>,
    pub question: String,
    #[serde(default, rename = "multiSelect", alias = "multi_select")]
    pub multi_select: bool,
    #[serde(default)]
    pub options: Vec<PromptOption>,
    /// It takes a free-text answer too.
    #[serde(default)]
    pub other: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct PromptOption {
    /// What to answer to pick it.
    pub n: u32,
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// One question's answer: an option, several (a multiple-choice question), or free text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(untagged)]
#[ts(export)]
pub enum PromptAnswer {
    Pick { pick: u32 },
    Picks { picks: Vec<u32> },
    Text { text: String },
}

/// The person's answer to a blocked prompt, from the Work tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, garde::Validate)]
#[ts(export)]
pub struct PromptAnswerSend {
    /// The session: `<session ID>@<node>`, as the deck names it.
    #[garde(length(min = 1, max = 300))]
    pub session: String,
    /// The prompt's ID, so an answer to a prompt that's gone is refused rather than typed into the next one.
    #[garde(length(min = 1, max = 300))]
    pub prompt: String,
    /// One per question, in order; one for a permission or plan prompt.
    #[garde(length(min = 1, max = 20))]
    pub answers: Vec<PromptAnswer>,
}

impl BlockedPrompt {
    /// Fleetlayer's `meta.prompt`, read leniently: a field it doesn't have is left empty, and a prompt without an ID
    /// or a kind isn't one.
    pub fn from_meta(v: &serde_json::Value) -> Option<BlockedPrompt> {
        let id = v.get("id")?.as_str()?.to_string();
        let kind = v.get("kind")?.as_str()?.to_string();
        let tool = v.get("tool");
        Some(BlockedPrompt {
            id,
            kind,
            at: v.get("at").and_then(|a| a.as_str().map(str::to_string).or_else(|| Some(a.to_string()))),
            questions: v.get("questions").and_then(|q| serde_json::from_value(q.clone()).ok()).unwrap_or_default(),
            options: v.get("options").and_then(|o| serde_json::from_value(o.clone()).ok()).unwrap_or_default(),
            tool: tool.and_then(|t| t.get("name")).and_then(|n| n.as_str()).map(str::to_string),
            tool_input: tool.and_then(|t| t.get("input")).map(|i| {
                let s = i.as_str().map(str::to_string).unwrap_or_else(|| i.to_string());
                if s.chars().count() > 600 { format!("{}…", s.chars().take(599).collect::<String>()) } else { s }
            }),
            plan: v.get("plan").and_then(|p| p.as_str()).map(str::to_string),
        })
    }
}

#[cfg(test)]
mod prompt_tests {
    use super::*;

    #[test]
    fn fleetlayers_prompt_reads_and_answers_serialise_as_it_takes_them() {
        let meta = serde_json::json!({"id": "toolu_1", "kind": "question", "at": "2026-11-28T00:10:00Z", "source": "hook",
            "questions": [{"header": "Provider", "question": "Which push provider?", "multiSelect": false,
                "options": [{"n": 1, "label": "APNs + FCM", "description": "the platforms' own"}, {"n": 2, "label": "OneSignal"}],
                "other": true}]});
        let p = BlockedPrompt::from_meta(&meta).unwrap();
        assert_eq!(p.questions[0].options[1].label, "OneSignal");
        assert!(p.questions[0].other && !p.questions[0].multi_select);
        let perm = serde_json::json!({"id": "p2", "kind": "permission", "tool": {"name": "Bash", "input": {"command": "rm -rf build"}},
            "options": [{"n": 1, "label": "Yes"}, {"n": 2, "label": "No"}]});
        let p = BlockedPrompt::from_meta(&perm).unwrap();
        assert_eq!((p.tool.as_deref(), p.options.len()), (Some("Bash"), 2));
        assert!(p.tool_input.unwrap().contains("rm -rf build"));
        assert!(BlockedPrompt::from_meta(&serde_json::json!({"kind": "plan"})).is_none());
        let answers = vec![
            PromptAnswer::Pick { pick: 2 },
            PromptAnswer::Picks { picks: vec![1, 3] },
            PromptAnswer::Text { text: "x".into() },
        ];
        assert_eq!(
            serde_json::to_value(&answers).unwrap(),
            serde_json::json!([{"pick": 2}, {"picks": [1, 3]}, {"text": "x"}])
        );
    }
}
