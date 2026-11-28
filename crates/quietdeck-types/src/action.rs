//! Typed actions: what a card proposes, what the user approves, and exactly what the runner or the agent carries out.
//!
//! The deck validates every action when a card is posted (serde for the shape, garde for the field rules), and the
//! runner validates it again on its own machine before claiming it.

use garde::Validate;
use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A session by its tab, on a node. `session_id` pins one conversation when the tab has been reused.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
pub struct SessionRef {
    #[garde(length(min = 1, max = 100))]
    pub node: String,
    #[garde(length(min = 1, max = 200))]
    pub tab: String,
    #[garde(inner(length(min = 1, max = 100)))]
    pub session_id: Option<String>,
}

impl SessionRef {
    /// `tab@node`, the address `quietdeck msg send --to` takes.
    pub fn address(&self) -> String {
        format!("{}@{}", self.tab, self.node)
    }
}

/// A message in a mail thread: what a forward is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct MessageRef {
    #[garde(length(min = 1, max = 100))]
    pub account: String,
    #[garde(length(min = 1, max = 200))]
    pub thread_id: String,
    #[garde(inner(length(min = 1, max = 200)))]
    pub message_id: Option<String>,
}

/// A draft or a reply in a thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct Compose {
    #[garde(length(min = 1, max = 100))]
    pub account: String,
    #[garde(length(min = 1, max = 200))]
    pub thread_id: String,
    #[garde(length(min = 1, max = 300))]
    pub reply_to_message_id: String,
    /// The address it's sent from. Must be one the account can send as (checked when the card is posted, again
    /// before sending, and against the sent message's From header afterwards).
    #[garde(email)]
    pub from: String,
    #[garde(length(min = 1, max = 50), inner(email))]
    pub to: Vec<String>,
    #[serde(default)]
    #[garde(length(max = 50), inner(email))]
    pub cc: Vec<String>,
    #[garde(length(min = 1, max = 500))]
    pub subject: String,
    #[garde(length(min = 1, max = 100_000))]
    pub body: String,
    /// The session that asked for the draft, which gets what the user really sent (the sent-mail forward).
    #[serde(default)]
    #[garde(dive)]
    pub requested_by: Option<SessionRef>,
}

/// A tracker story: create one, or comment on one. Boards are names from deck config; the runner's and the skill's
/// own config map them to the tracker's IDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
#[ts(export)]
pub enum StoryOp {
    Create {
        #[garde(length(min = 1, max = 100))]
        board: String,
        #[garde(length(min = 1, max = 50))]
        story_type: String,
        #[garde(length(min = 1, max = 500))]
        title: String,
        #[garde(length(max = 100_000))]
        description: String,
        #[serde(default)]
        #[garde(length(max = 20), inner(length(min = 1, max = 100)))]
        labels: Vec<String>,
    },
    Comment {
        #[garde(length(min = 1, max = 100))]
        story_id: String,
        #[garde(length(min = 1, max = 100_000))]
        text: String,
    },
}

/// What to do with a task on the person's own task board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TaskOp {
    Create,
    Update,
    Done,
}

/// What to do with another session, approved from the Work tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SessionOp {
    /// Pass the person's words on (`text`).
    Message,
    /// Ask it to run its wrap-up: the end-of-session sweep for loose ends. `text` replaces the default request.
    WrapUp,
    /// Close it.
    Close,
    /// Compact its context (`/compact`, typed in by Fleetlayer while it's idle): for a long-running session that
    /// always has more to do (Nathan, 28 Nov 2026: "compact, not close").
    Compact,
}

impl SessionOp {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionOp::Message => "message",
            SessionOp::WrapUp => "wrap_up",
            SessionOp::Close => "close",
            SessionOp::Compact => "compact",
        }
    }
}

/// What a wrap-up action asks, when it doesn't say.
pub const WRAP_UP_TEXT: &str = "Approved on the deck's Work tab: run your wrap-up now (the `wrap-up-session` skill, or \
                                your own end-of-session sweep for loose ends) and leave its result in this session.";

/// One proposed action. The `type` tag is also the name `runner.eligible` uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema, Validate)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[ts(export)]
pub enum Action {
    Archive {
        #[garde(length(min = 1, max = 100))]
        account: String,
        #[garde(length(min = 1, max = 200))]
        thread_id: String,
    },
    MarkRead {
        #[garde(length(min = 1, max = 100))]
        account: String,
        #[garde(length(min = 1, max = 200))]
        thread_id: String,
    },
    /// Close a support conversation (Intercom): its "archive". `account` is the deck's account for the support inbox
    /// (`intercom`), `conversation_id` the conversation's ID there.
    CloseConversation {
        #[garde(length(min = 1, max = 100))]
        account: String,
        #[garde(length(min = 1, max = 200))]
        conversation_id: String,
    },
    /// Out of spam and back into the inbox: Gmail's "Not spam".
    NotSpam {
        #[garde(length(min = 1, max = 100))]
        account: String,
        #[garde(length(min = 1, max = 200))]
        thread_id: String,
    },
    Label {
        #[garde(length(min = 1, max = 100))]
        account: String,
        #[garde(length(min = 1, max = 200))]
        thread_id: String,
        #[serde(default)]
        #[garde(length(max = 20), inner(length(min = 1, max = 200)))]
        add: Vec<String>,
        #[serde(default)]
        #[garde(length(max = 20), inner(length(min = 1, max = 200)))]
        remove: Vec<String>,
    },
    /// Pass the email on to a session, with the fixed forward header.
    Forward {
        #[garde(dive)]
        to: SessionRef,
        #[garde(length(min = 1, max = 100_000))]
        text: String,
        /// The user's own comment, set on a sent row. Part of the approved payload, not an adjustment.
        #[serde(default)]
        #[garde(inner(length(max = 10_000)))]
        note: Option<String>,
        #[serde(default)]
        #[garde(dive)]
        source: Option<MessageRef>,
    },
    /// A Gmail draft in the thread, which the user edits and sends themselves. The normal reply path.
    Draft(#[garde(dive)] Compose),
    /// Sends straight away. Never runner-eligible.
    Reply(#[garde(dive)] Compose),
    Story(#[garde(dive)] StoryOp),
    Task {
        #[garde(skip)]
        op: TaskOp,
        #[serde(default)]
        #[garde(inner(length(min = 1, max = 500)))]
        title: Option<String>,
        #[serde(default)]
        #[garde(inner(length(max = 100_000)))]
        description: Option<String>,
        #[serde(default)]
        #[garde(inner(length(min = 1, max = 100)))]
        story_id: Option<String>,
        #[serde(default)]
        #[garde(inner(length(min = 1, max = 200)))]
        agent_tab: Option<String>,
    },
    Calendar {
        #[garde(length(min = 1, max = 100))]
        account: String,
        #[garde(length(min = 1, max = 300))]
        calendar: String,
        #[garde(length(min = 1, max = 500))]
        title: String,
        #[ts(type = "string")]
        #[schemars(with = "String")]
        #[garde(skip)]
        start: Timestamp,
        #[ts(type = "string")]
        #[schemars(with = "String")]
        #[garde(skip)]
        end: Timestamp,
        #[garde(length(min = 1, max = 100))]
        time_zone: String,
        #[serde(default)]
        #[garde(inner(length(max = 1000)))]
        location: Option<String>,
        #[serde(default)]
        #[garde(inner(length(max = 100_000)))]
        description: Option<String>,
    },
    Resume {
        #[garde(length(min = 1, max = 100))]
        node: String,
        #[garde(length(min = 1, max = 200))]
        tab: String,
        #[garde(length(min = 1, max = 100))]
        session_id: String,
        #[garde(length(min = 1, max = 4096))]
        cwd: String,
    },
    NewSession {
        #[garde(length(min = 1, max = 100))]
        node: String,
        #[garde(length(min = 1, max = 200))]
        tab: String,
        #[garde(length(min = 1, max = 4096))]
        cwd: String,
        #[garde(length(min = 1, max = 100_000))]
        prompt: String,
    },
    Lease {
        #[garde(length(min = 1, max = 50))]
        kind: String,
        #[garde(length(min = 1, max = 500))]
        resource: String,
        #[garde(skip)]
        #[ts(type = "Record<string, unknown>")]
        params: serde_json::Value,
        #[garde(dive)]
        holder: SessionRef,
        #[garde(length(min = 1, max = 2000))]
        reason: String,
    },
    Watch {
        #[garde(length(min = 1, max = 100))]
        account: String,
        #[garde(length(min = 1, max = 200))]
        thread_id: String,
        #[garde(skip)]
        on: bool,
    },
    /// Message, wrap up or close another session (the Work tab's session actions).
    Session {
        #[garde(skip)]
        op: SessionOp,
        #[garde(dive)]
        to: SessionRef,
        #[serde(default)]
        #[garde(inner(length(min = 1, max = 20_000)))]
        text: Option<String>,
    },
}

/// An action's type alone: the `type` tag, the names `runner.eligible` and project policies use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ActionType {
    Archive,
    MarkRead,
    NotSpam,
    CloseConversation,
    Label,
    Forward,
    Draft,
    Reply,
    Story,
    Task,
    Calendar,
    Resume,
    NewSession,
    Lease,
    Watch,
    Session,
}

impl ActionType {
    pub const ALL: [ActionType; 16] = [
        ActionType::Archive,
        ActionType::MarkRead,
        ActionType::NotSpam,
        ActionType::CloseConversation,
        ActionType::Label,
        ActionType::Forward,
        ActionType::Draft,
        ActionType::Reply,
        ActionType::Story,
        ActionType::Task,
        ActionType::Calendar,
        ActionType::Resume,
        ActionType::NewSession,
        ActionType::Lease,
        ActionType::Watch,
        ActionType::Session,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ActionType::Archive => "archive",
            ActionType::MarkRead => "mark_read",
            ActionType::NotSpam => "not_spam",
            ActionType::CloseConversation => "close_conversation",
            ActionType::Label => "label",
            ActionType::Forward => "forward",
            ActionType::Draft => "draft",
            ActionType::Reply => "reply",
            ActionType::Story => "story",
            ActionType::Task => "task",
            ActionType::Calendar => "calendar",
            ActionType::Resume => "resume",
            ActionType::NewSession => "new_session",
            ActionType::Lease => "lease",
            ActionType::Watch => "watch",
            ActionType::Session => "session",
        }
    }

    pub fn parse(s: &str) -> Option<ActionType> {
        ActionType::ALL.into_iter().find(|t| t.as_str() == s)
    }
}

impl std::fmt::Display for ActionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Action {
    pub fn action_type(&self) -> ActionType {
        match self {
            Action::Archive { .. } => ActionType::Archive,
            Action::MarkRead { .. } => ActionType::MarkRead,
            Action::NotSpam { .. } => ActionType::NotSpam,
            Action::CloseConversation { .. } => ActionType::CloseConversation,
            Action::Label { .. } => ActionType::Label,
            Action::Forward { .. } => ActionType::Forward,
            Action::Draft(_) => ActionType::Draft,
            Action::Reply(_) => ActionType::Reply,
            Action::Story(_) => ActionType::Story,
            Action::Task { .. } => ActionType::Task,
            Action::Calendar { .. } => ActionType::Calendar,
            Action::Resume { .. } => ActionType::Resume,
            Action::NewSession { .. } => ActionType::NewSession,
            Action::Lease { .. } => ActionType::Lease,
            Action::Watch { .. } => ActionType::Watch,
            Action::Session { .. } => ActionType::Session,
        }
    }

    /// The mail account the action touches, when it touches one.
    pub fn account(&self) -> Option<&str> {
        match self {
            Action::Archive { account, .. }
            | Action::MarkRead { account, .. }
            | Action::NotSpam { account, .. }
            | Action::CloseConversation { account, .. }
            | Action::Label { account, .. }
            | Action::Calendar { account, .. }
            | Action::Watch { account, .. } => Some(account),
            Action::Draft(c) | Action::Reply(c) => Some(&c.account),
            Action::Forward { source, .. } => source.as_ref().map(|s| s.account.as_str()),
            _ => None,
        }
    }

    /// Rules beyond field lengths: a calendar event ends after it starts, a label change changes something, a task
    /// update names what it updates.
    pub fn check(&self) -> Result<(), String> {
        self.validate().map_err(|e| e.to_string())?;
        match self {
            Action::Calendar { start, end, .. } if end <= start => Err("calendar: end must be after start".into()),
            Action::Label { add, remove, .. } if add.is_empty() && remove.is_empty() => {
                Err("label: add or remove at least one label".into())
            }
            Action::Task { op: TaskOp::Create, title: None, .. } => Err("task: create needs a title".into()),
            Action::Task { op: TaskOp::Update | TaskOp::Done, story_id: None, .. } => {
                Err("task: update and done need the task's story_id".into())
            }
            Action::Session { op: SessionOp::Message, text: None, .. } => {
                Err("session: a message needs its text".into())
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn archive_round_trips_with_its_type_tag() {
        let a: Action =
            serde_json::from_value(json!({"type": "archive", "account": "personal", "thread_id": "t1"})).unwrap();
        assert_eq!(a.action_type(), ActionType::Archive);
        assert_eq!(serde_json::to_value(&a).unwrap()["type"], "archive");
        a.check().unwrap();
    }

    #[test]
    fn draft_needs_valid_addresses() {
        let mut v = json!({"type": "draft", "account": "personal", "thread_id": "t", "reply_to_message_id": "m",
            "from": "me@example.com", "to": ["you@example.com"], "subject": "Re: hi", "body": "Thanks"});
        let a: Action = serde_json::from_value(v.clone()).unwrap();
        a.check().unwrap();
        v["from"] = json!("not an address");
        let a: Action = serde_json::from_value(v.clone()).unwrap();
        assert!(a.check().is_err());
        v["from"] = json!("me@example.com");
        v["to"] = json!([]);
        let a: Action = serde_json::from_value(v).unwrap();
        assert!(a.check().is_err(), "a draft goes to someone");
    }

    #[test]
    fn unknown_types_and_fields_of_the_wrong_shape_are_refused() {
        assert!(serde_json::from_value::<Action>(json!({"type": "unsubscribe", "account": "a"})).is_err());
        assert!(serde_json::from_value::<Action>(json!({"type": "archive", "account": 3, "thread_id": "t"})).is_err());
    }

    #[test]
    fn calendar_must_end_after_it_starts() {
        let a: Action =
            serde_json::from_value(json!({"type": "calendar", "account": "personal", "calendar": "primary",
            "title": "Call", "start": "2026-11-24T10:00:00Z", "end": "2026-11-24T09:00:00Z", "time_zone": "UTC"}))
            .unwrap();
        assert!(a.check().unwrap_err().contains("end must be after start"));
    }

    #[test]
    fn story_ops_are_tagged() {
        let a: Action =
            serde_json::from_value(json!({"type": "story", "op": "comment", "story_id": "123", "text": "x"})).unwrap();
        a.check().unwrap();
        let a: Action = serde_json::from_value(json!({"type": "story", "op": "create", "board": "engineering",
            "story_type": "bug", "title": "", "description": ""}))
        .unwrap();
        assert!(a.check().is_err(), "a story needs a title");
    }

    #[test]
    fn every_action_type_name_parses_back() {
        for t in ActionType::ALL {
            assert_eq!(ActionType::parse(t.as_str()), Some(t));
            assert_eq!(serde_json::to_value(t).unwrap(), json!(t.as_str()));
        }
    }
}
