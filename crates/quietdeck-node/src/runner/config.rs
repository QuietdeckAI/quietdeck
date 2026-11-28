//! The runner's section of node.yaml. Which actions it takes on, and which tool carries each one out, is config, so a
//! different Executor setup or a CLI such as gog needs no code change. Addresses, account IDs and tool paths live in
//! the user's own node.yaml; the repository has only an example.

use std::collections::BTreeMap;
use std::time::Duration;

use quietdeck_types::ActionType;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerConfig {
    /// Off unless set: one process fewer on machines that don't run the triage agent.
    #[serde(default)]
    pub enabled: bool,
    /// The session that gets wake-ups, hand-offs and error reports, and whose cards the runner carries out.
    pub agent: AgentConfig,
    #[serde(default)]
    pub executor: Option<ExecutorConfig>,
    /// `gog`: the mail tools that aren't set run the user's own gog CLI (see [`super::gog`]). Set by hand, a tool
    /// still wins.
    #[serde(default)]
    pub mail: Option<MailBackend>,
    /// Where gog is, for `gog:` tools. Optional: `gog` on PATH.
    #[serde(default)]
    pub gog: Option<super::gog::GogConfig>,
    /// The mail accounts, by the account IDs the deck's config uses.
    #[serde(default)]
    pub accounts: BTreeMap<String, AccountConfig>,
    #[serde(default)]
    pub mail_watch: Option<MailWatchConfig>,
    /// A support inbox as a source (Intercom): conversations waiting on a reply wake the agent like new mail.
    #[serde(default)]
    pub intercom_watch: Option<IntercomWatchConfig>,
    /// Lists the addresses an account can send as (Gmail's `users.settings.sendAs.list` shape). Read hourly.
    #[serde(default)]
    pub send_as: Option<ToolSpec>,
    /// Reads a thread's message IDs and labels, never its content (Gmail's `users.threads.get` with
    /// `format: minimal`), for the freshness checks.
    #[serde(default)]
    pub thread: Option<ToolSpec>,
    /// Lists an account's labels (Gmail's `users.labels.list` shape), so a label that doesn't exist yet goes to the
    /// agent. Optional.
    #[serde(default)]
    pub labels: Option<ToolSpec>,
    /// Tracker boards by the names the deck's config uses, each with the IDs the story tool needs, as
    /// `${board.<name>}` (a Shortcut workflow state and team, a Linear team, a Trello list).
    #[serde(default)]
    pub boards: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    /// The tools behind gmail-filter leases: create and delete a filter, make the lease's label, and move mail
    /// already in the inbox. Without it, the holder creates and removes its own filters.
    #[serde(default)]
    pub gmail_filters: Option<GmailFiltersConfig>,
    #[serde(default = "default_eligible")]
    pub eligible: Vec<ActionType>,
    /// The tool that carries out each action type, by its name in `eligible`. Stories and tasks have a tool per op
    /// (`story.create`, `story.comment`, `task.create`, `task.update`, `task.done`), so an update never runs a create.
    /// Forwards, resumes, new sessions, leases and watches need no entry.
    #[serde(default)]
    pub tools: BTreeMap<String, ToolSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailBackend {
    Gog,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    /// The agent's tab, or its name when it's headless. Where Fleetlayer runs Quietdeck's plugin it can be
    /// `<tab>@<node>`, an agent on another node (the runner on a Unix user of its own).
    pub tab: String,
    /// Other agents the runner works for, each a tab or `<tab>@<node>`: it carries out their approved cards too
    /// (an overseer's session actions), and hands each card it can't back to the agent that posted it. Wake-ups and
    /// the mail watch stay `tab`'s.
    #[serde(default)]
    pub tabs: Vec<String>,
    /// Run the agent as one `claude -p` turn per message, resuming the same conversation, instead of pasting into a
    /// tab. See [`super::headless`].
    #[serde(default)]
    pub headless: Option<super::headless::HeadlessConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorConfig {
    /// Executor's MCP endpoint.
    pub url: String,
    /// The runner's own Executor token: a file only this user can read...
    #[serde(default)]
    pub token_file: Option<String>,
    /// ...or an environment variable.
    #[serde(default)]
    pub token_env: Option<String>,
    /// The MCP tool that runs one app tool by path. Executor calls it `call`.
    #[serde(default = "default_call_tool")]
    pub call_tool: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountConfig {
    pub address: String,
    /// How wake-ups and reports name it ("Personal"). Defaults to the ID with a capital letter.
    #[serde(default)]
    pub label: Option<String>,
    /// Anything else the tool inputs need, as `${account.<name>}` (an Executor account ID, a gog account name).
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailWatchConfig {
    #[serde(default = "default_every")]
    pub every: String,
    /// Reads the account's current history ID (Gmail's `users.getProfile` shape).
    pub profile: ToolSpec,
    /// Lists changes since a history ID (Gmail's `users.history.list` shape): message and thread IDs and labels.
    pub history: ToolSpec,
    /// Reads one new message's headers, labels and snippet (Gmail's `users.messages.get` with `format: metadata`
    /// and `metadataHeaders: [From, To, Subject]`), with `${message_id}`. Optional: with it, "while I'm reading"
    /// rules can match the sender and subject, and a message that's already gone (a follow-up nudge Gmail took back)
    /// doesn't wake the agent.
    #[serde(default)]
    pub message: Option<ToolSpec>,
    /// Lists an account's drafts (Gmail's `users.drafts.list` shape: `drafts[].id`, `drafts[].message.id`), every 5
    /// minutes, for the Drafts tab; each new draft's headers come from `message`. Optional.
    #[serde(default)]
    pub drafts: Option<ToolSpec>,
    /// Lists the IDs of every thread in the inbox (Gmail's `users.threads.list` shape: `threads[].id`, and a
    /// `nextPageToken` only when the list stopped part way), every `inbox_every`. A thread in the last complete list
    /// and missing from this one has left the inbox (archived, trashed or deleted), for a history tool that can't say
    /// so itself (gog's). Optional.
    #[serde(default)]
    pub inbox: Option<ToolSpec>,
    #[serde(default = "default_inbox_every")]
    pub inbox_every: String,
    /// Reads one new message's headers and text, with `${message_id}`, as `{from, to, cc, subject, date, body}` (gog's
    /// sanitised text: no HTML, no links). With it, each wake-up carries the new mail itself, capped, for an agent
    /// that has no mail tools of its own (Quietdeck solo). Optional.
    #[serde(default)]
    pub message_text: Option<ToolSpec>,
    /// Lists the inbox's newest messages (`{messages: [{id, threadId, labelIds}]}`), read once on the first check, so
    /// an agent with no mail tools starts with what's already there. Optional.
    #[serde(default)]
    pub recent: Option<ToolSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntercomWatchConfig {
    #[serde(default = "default_intercom_every")]
    pub every: String,
    /// The deck's account for the support inbox (`intercom`), which its cards and `close_conversation` actions name.
    /// It isn't one of `runner.accounts`, which are mailboxes.
    pub account: String,
    /// Lists the open conversations changed since `${updated_since}` (seconds since the epoch), the next page from
    /// `${starting_after}`: `{conversations: [{id, state, updated_at, waiting_since}], next}`. A conversation with no
    /// `waiting_since` (answered, or closed) doesn't wake the agent.
    pub conversations: ToolSpec,
}

/// One tool: an Executor app tool by path, a command that reads the input as JSON on stdin and prints JSON, or a
/// gog op (`gog: archive`, see [`super::gog::OPS`]) that runs the user's gog CLI with the input's fields. In
/// `input`, `${...}` names a value: `account.address`, `account.<var>`, `action.<field>`, `card.<field>` and, for
/// the mail watch, `history_id` and `page_token`. A string that is only a placeholder takes the value's own type,
/// and a missing value leaves the key out.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSpec {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub command: Option<Vec<String>>,
    #[serde(default)]
    pub gog: Option<String>,
    #[serde(default)]
    pub input: serde_json::Value,
}

/// In each input, `${lease.query}`, `${lease.label}` (`Quietdeck/Lease/<tab>`), `${lease.label_id}` (create),
/// `${lease.filter_id}` (delete) and `${thread_ids}` (move) name the lease's values, beside `${account.*}`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GmailFiltersConfig {
    /// Gmail's `users.settings.filters.create`: the query, add the label, remove INBOX and UNREAD. Returns `{id}`.
    pub create: ToolSpec,
    /// Gmail's `users.settings.filters.delete`.
    pub delete: ToolSpec,
    /// Gmail's `users.labels.create`, for the lease's label the first time. Returns `{id}`. (`runner.labels` finds
    /// it after that.)
    pub create_label: ToolSpec,
    /// Gmail's `users.threads.list` with `q: "${lease.query} in:inbox"`. Optional, with `move`.
    #[serde(default)]
    pub inbox: Option<ToolSpec>,
    /// Takes `${thread_ids}` out of the inbox, marked read, with the lease's label. Optional, with `inbox`.
    #[serde(default, rename = "move")]
    pub move_threads: Option<ToolSpec>,
}

impl ToolSpec {
    pub fn describe(&self) -> String {
        match (&self.path, &self.command, &self.gog) {
            (Some(p), _, _) => p.clone(),
            (None, Some(c), _) => c.first().cloned().unwrap_or_default(),
            (None, None, Some(op)) => format!("gog {op}"),
            (None, None, None) => "no tool".into(),
        }
    }

    /// The tool `runner.mail: gog` fills in for `op`.
    pub fn gog(op: &str) -> ToolSpec {
        ToolSpec { path: None, command: None, gog: Some(op.into()), input: super::gog::preset_input(op) }
    }
}

fn default_call_tool() -> String {
    "call".into()
}

fn default_every() -> String {
    "30s".into()
}

fn default_intercom_every() -> String {
    "2m".into()
}

fn default_inbox_every() -> String {
    "5m".into()
}

/// Every type the runner has a handler for, or will: watches arrive with thread watches and leases with the lease
/// kinds. Until then those go to the agent as having no handler.
pub fn default_eligible() -> Vec<ActionType> {
    use ActionType::*;
    vec![Archive, MarkRead, NotSpam, Label, Draft, Forward, Watch, Lease, Story, Task, Calendar, Resume, NewSession]
}

/// `30s`, `2m`, `1h`, or plain seconds.
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: u64 = num.parse().map_err(|_| format!("{s:?} isn't a duration like 30s, 2m or 1h"))?;
    let secs = match unit {
        "" | "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        _ => return Err(format!("{s:?} isn't a duration like 30s, 2m or 1h")),
    };
    if secs == 0 {
        return Err("a duration must be more than 0".into());
    }
    Ok(Duration::from_secs(secs))
}

/// The names `runner.tools` takes: one per action the runner carries out with a tool.
pub const TOOL_KEYS: [&str; 12] = [
    "archive",
    "mark_read",
    "not_spam",
    "close_conversation",
    "label",
    "draft",
    "calendar",
    "story.create",
    "story.comment",
    "task.create",
    "task.update",
    "task.done",
];

/// The `runner.tools` entry that carries out an action.
pub fn tool_key(a: &quietdeck_types::Action) -> String {
    use quietdeck_types::Action;
    use quietdeck_types::action::{StoryOp, TaskOp};
    match a {
        Action::Story(StoryOp::Create { .. }) => "story.create".into(),
        Action::Story(StoryOp::Comment { .. }) => "story.comment".into(),
        Action::Task { op: TaskOp::Create, .. } => "task.create".into(),
        Action::Task { op: TaskOp::Update, .. } => "task.update".into(),
        Action::Task { op: TaskOp::Done, .. } => "task.done".into(),
        other => other.action_type().as_str().into(),
    }
}

impl RunnerConfig {
    /// Fill in what `runner.mail` names: for gog, every mail tool node.yaml doesn't set itself, and the mail watch
    /// when there are accounts to watch. Run before [`RunnerConfig::check`].
    pub fn apply_presets(&mut self) {
        if self.mail != Some(MailBackend::Gog) {
            return;
        }
        let fill = |slot: &mut Option<ToolSpec>, op: &str| {
            if slot.is_none() {
                *slot = Some(ToolSpec::gog(op));
            }
        };
        fill(&mut self.thread, "thread");
        fill(&mut self.send_as, "send_as");
        fill(&mut self.labels, "labels");
        for op in ["archive", "mark_read", "not_spam", "label", "draft"] {
            self.tools.entry(op.into()).or_insert_with(|| ToolSpec::gog(op));
        }
        if self.mail_watch.is_none() && !self.accounts.is_empty() {
            self.mail_watch = Some(MailWatchConfig {
                every: default_every(),
                profile: ToolSpec::gog("profile"),
                history: ToolSpec::gog("history"),
                message: None,
                drafts: None,
                inbox: None,
                inbox_every: default_inbox_every(),
                message_text: None,
                recent: None,
            });
        }
        if let Some(m) = &mut self.mail_watch {
            fill(&mut m.message, "message");
            fill(&mut m.drafts, "drafts");
            fill(&mut m.inbox, "inbox");
            fill(&mut m.message_text, "message_text");
            fill(&mut m.recent, "recent");
        }
    }

    /// The rules config can't break: replies always go through the agent, every tool names exactly one way to run
    /// it, and Executor tools need an Executor.
    pub fn check(&self) -> Result<(), String> {
        if self.eligible.contains(&ActionType::Reply) {
            return Err("runner.eligible can't include reply: a reply sends mail straight away, so it always goes \
                through the triage agent, which rechecks the thread first"
                .into());
        }
        if self.agent.tab.trim().is_empty() {
            return Err("runner.agent.tab is empty".into());
        }
        if self.agent.tabs.iter().any(|t| t.trim().is_empty()) {
            return Err("runner.agent.tabs has an empty entry".into());
        }
        if let Some(h) = &self.agent.headless {
            h.check()?;
        }
        for key in self.tools.keys() {
            if !TOOL_KEYS.contains(&key.as_str()) {
                return Err(match key.as_str() {
                    "story" | "task" => format!(
                        "runner.tools.{key}: name the op ({}): each op has its own tool, so an update never runs a create",
                        TOOL_KEYS
                            .iter()
                            .filter(|k| k.starts_with(&format!("{key}.")))
                            .copied()
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    _ => format!("runner.tools.{key}: no such tool; the runner takes {}", TOOL_KEYS.join(", ")),
                });
            }
        }
        let mut specs: Vec<(String, &ToolSpec)> = self.tools.iter().map(|(t, s)| (format!("tools.{t}"), s)).collect();
        let watch_extras =
            self.mail_watch.as_ref().map(|m| [&m.message, &m.drafts, &m.inbox, &m.message_text, &m.recent]);
        for (name, s) in [
            "mail_watch.message",
            "mail_watch.drafts",
            "mail_watch.inbox",
            "mail_watch.message_text",
            "mail_watch.recent",
        ]
        .into_iter()
        .zip(watch_extras.into_iter().flatten())
        {
            if let Some(s) = s {
                specs.push((name.to_string(), s));
            }
        }
        if let Some(w) = &self.intercom_watch {
            parse_duration(&w.every).map_err(|e| format!("runner.intercom_watch.every: {e}"))?;
            if w.account.trim().is_empty() {
                return Err("runner.intercom_watch.account is empty".into());
            }
            if self.accounts.contains_key(&w.account) {
                return Err(format!(
                    "runner.intercom_watch.account {:?} is also in runner.accounts, which are mailboxes: give the \
                     support inbox an account of its own",
                    w.account
                ));
            }
            specs.push(("intercom_watch.conversations".into(), &w.conversations));
        }
        for (name, s) in [("send_as", &self.send_as), ("thread", &self.thread), ("labels", &self.labels)] {
            if let Some(s) = s {
                specs.push((name.to_string(), s));
            }
        }
        if let Some(m) = &self.mail_watch {
            parse_duration(&m.every).map_err(|e| format!("runner.mail_watch.every: {e}"))?;
            parse_duration(&m.inbox_every).map_err(|e| format!("runner.mail_watch.inbox_every: {e}"))?;
            specs.push(("mail_watch.profile".into(), &m.profile));
            specs.push(("mail_watch.history".into(), &m.history));
            if self.accounts.is_empty() {
                return Err("runner.mail_watch needs runner.accounts".into());
            }
        }
        for (name, s) in specs {
            let set = [s.path.is_some(), s.command.is_some(), s.gog.is_some()].iter().filter(|b| **b).count();
            if set > 1 {
                return Err(format!("runner.{name}: set one of path, command or gog"));
            }
            if set == 0 {
                return Err(format!("runner.{name}: set path (an Executor tool), command or gog"));
            }
            if let Some(c) = &s.command
                && c.is_empty()
            {
                return Err(format!("runner.{name}: command is empty"));
            }
            if s.path.is_some() && self.executor.is_none() {
                return Err(format!("runner.{name} names an Executor tool, but runner.executor isn't set"));
            }
            if let Some(op) = &s.gog
                && !super::gog::OPS.contains(&op.as_str())
            {
                return Err(format!("runner.{name}: gog has no op {op:?}; it takes {}", super::gog::OPS.join(", ")));
            }
        }
        if let Some(e) = &self.executor
            && e.token_file.is_some()
            && e.token_env.is_some()
        {
            return Err("runner.executor: set token_file or token_env, not both".into());
        }
        Ok(())
    }

    pub fn account_label(&self, id: &str) -> String {
        match self.accounts.get(id).and_then(|a| a.label.clone()) {
            Some(l) => l,
            None => {
                let mut c = id.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> &'static str {
        "agent: { tab: email triage }\nexecutor: { url: \"https://ex.example/mcp\", token_file: /tmp/t }\n"
    }

    #[test]
    fn reply_is_refused_at_start_up() {
        let c: RunnerConfig = yaml_serde::from_str(&format!("{}eligible: [archive, reply]\n", base())).unwrap();
        assert!(c.check().unwrap_err().contains("can't include reply"));
        let c: RunnerConfig = yaml_serde::from_str(base()).unwrap();
        c.check().unwrap();
        assert!(!c.eligible.contains(&ActionType::Reply));
    }

    #[test]
    fn tools_name_one_way_to_run() {
        let c: RunnerConfig =
            yaml_serde::from_str(&format!("{}tools:\n  archive: {{ path: tools.x, command: [gog] }}\n", base()))
                .unwrap();
        assert!(c.check().unwrap_err().contains("set one of path, command or gog"));
        let c: RunnerConfig =
            yaml_serde::from_str("agent: { tab: t }\ntools:\n  archive: { path: tools.google.gmail.modify }\n")
                .unwrap();
        assert!(c.check().unwrap_err().contains("runner.executor isn't set"));
    }

    #[test]
    fn stories_and_tasks_take_a_tool_per_op() {
        let tool = "{ path: tools.x }";
        let c: RunnerConfig = yaml_serde::from_str(&format!("{}tools:\n  story: {tool}\n", base())).unwrap();
        let e = c.check().unwrap_err();
        assert!(e.contains("story.create, story.comment"), "{e}");
        let c: RunnerConfig = yaml_serde::from_str(&format!("{}tools:\n  task: {tool}\n", base())).unwrap();
        assert!(c.check().unwrap_err().contains("task.create, task.update, task.done"));
        let c: RunnerConfig = yaml_serde::from_str(&format!("{}tools:\n  archiv: {tool}\n", base())).unwrap();
        assert!(c.check().unwrap_err().contains("no such tool"));
        let c: RunnerConfig =
            yaml_serde::from_str(&format!("{}tools:\n  story.create: {tool}\n  task.done: {tool}\n", base())).unwrap();
        c.check().unwrap();

        use quietdeck_types::Action;
        let action = |v: serde_json::Value| -> Action { serde_json::from_value(v).unwrap() };
        assert_eq!(
            tool_key(&action(serde_json::json!({"type": "story", "op": "comment", "story_id": "1", "text": "x"}))),
            "story.comment"
        );
        assert_eq!(
            tool_key(&action(serde_json::json!({"type": "task", "op": "update", "story_id": "1"}))),
            "task.update"
        );
        assert_eq!(
            tool_key(&action(serde_json::json!({"type": "archive", "account": "a", "thread_id": "t"}))),
            "archive"
        );
    }

    #[test]
    fn mail_gog_fills_every_mail_tool_and_keeps_what_is_set_by_hand() {
        let mut c: RunnerConfig = yaml_serde::from_str(
            "agent: { tab: t }\nmail: gog\naccounts:\n  personal: { address: me@example.com }\n\
             tools:\n  archive: { command: [my-archive] }\n",
        )
        .unwrap();
        c.apply_presets();
        c.check().unwrap();
        let w = c.mail_watch.as_ref().unwrap();
        assert_eq!(w.history.gog.as_deref(), Some("history"));
        assert_eq!(w.inbox.as_ref().unwrap().gog.as_deref(), Some("inbox"));
        assert_eq!(c.thread.as_ref().unwrap().gog.as_deref(), Some("thread"));
        assert_eq!(c.tools["archive"].command, Some(vec!["my-archive".to_string()]), "a tool set by hand stays");
        assert_eq!(c.tools["draft"].gog.as_deref(), Some("draft"));
        assert_eq!(c.tools["draft"].input["account"], "${account.address}", "the account comes from node.yaml");
        assert!(c.executor.is_none(), "no Executor needed");

        // Without `mail: gog` nothing is filled in, so Executor setups load exactly as before.
        let mut c: RunnerConfig = yaml_serde::from_str(base()).unwrap();
        c.apply_presets();
        assert!(c.mail_watch.is_none() && c.thread.is_none() && c.tools.is_empty());
    }

    #[test]
    fn gog_ops_are_checked() {
        let c: RunnerConfig = yaml_serde::from_str("agent: { tab: t }\ntools:\n  archive: { gog: archiv }\n").unwrap();
        assert!(c.check().unwrap_err().contains("gog has no op \"archiv\""));
        let c: RunnerConfig = yaml_serde::from_str("agent: { tab: t }\ntools:\n  archive: { gog: archive }\n").unwrap();
        c.check().unwrap();
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
        assert_eq!(parse_duration("45").unwrap(), Duration::from_secs(45));
        assert!(parse_duration("0s").is_err());
        assert!(parse_duration("soon").is_err());
    }

    #[test]
    fn account_labels_default_to_a_capitalised_id() {
        let c: RunnerConfig = yaml_serde::from_str(&format!(
            "{}accounts:\n  personal: {{ address: me@example.com }}\n  work: {{ address: w@example.com, label: Acme }}\n",
            base()
        ))
        .unwrap();
        assert_eq!(c.account_label("personal"), "Personal");
        assert_eq!(c.account_label("work"), "Acme");
    }
}
