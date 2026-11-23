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
    /// The mail accounts, by the account IDs the deck's config uses.
    #[serde(default)]
    pub accounts: BTreeMap<String, AccountConfig>,
    #[serde(default)]
    pub mail_watch: Option<MailWatchConfig>,
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
    /// The tool that carries out each action type. Forwards, resumes and new sessions go through the deck and the orchestrator
    /// and need no entry.
    #[serde(default)]
    pub tools: BTreeMap<ActionType, ToolSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    pub tab: String,
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
    /// Lists changes since a history ID (Gmail's `users.history.list` shape), message IDs and labels only.
    pub history: ToolSpec,
}

/// One tool: an Executor app tool by path, or a command that reads the input as JSON on stdin and prints JSON. In
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
        match (&self.path, &self.command) {
            (Some(p), _) => p.clone(),
            (None, Some(c)) => c.first().cloned().unwrap_or_default(),
            (None, None) => "no tool".into(),
        }
    }
}

fn default_call_tool() -> String {
    "call".into()
}

fn default_every() -> String {
    "30s".into()
}

/// Every type the runner has a handler for, or will: watches arrive with thread watches and leases with the lease
/// kinds. Until then those go to the agent as having no handler.
pub fn default_eligible() -> Vec<ActionType> {
    use ActionType::*;
    vec![Archive, MarkRead, Label, Draft, Forward, Watch, Lease, Story, Task, Calendar, Resume, NewSession]
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

impl RunnerConfig {
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
        let mut specs: Vec<(String, &ToolSpec)> = self.tools.iter().map(|(t, s)| (format!("tools.{t}"), s)).collect();
        for (name, s) in [("send_as", &self.send_as), ("thread", &self.thread), ("labels", &self.labels)] {
            if let Some(s) = s {
                specs.push((name.to_string(), s));
            }
        }
        if let Some(m) = &self.mail_watch {
            parse_duration(&m.every).map_err(|e| format!("runner.mail_watch.every: {e}"))?;
            specs.push(("mail_watch.profile".into(), &m.profile));
            specs.push(("mail_watch.history".into(), &m.history));
            if self.accounts.is_empty() {
                return Err("runner.mail_watch needs runner.accounts".into());
            }
        }
        for (name, s) in specs {
            match (&s.path, &s.command) {
                (Some(_), Some(_)) => return Err(format!("runner.{name}: set path or command, not both")),
                (None, None) => return Err(format!("runner.{name}: set path (an Executor tool) or command")),
                (None, Some(c)) if c.is_empty() => return Err(format!("runner.{name}: command is empty")),
                (Some(_), None) if self.executor.is_none() => {
                    return Err(format!("runner.{name} names an Executor tool, but runner.executor isn't set"));
                }
                _ => {}
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
        assert!(c.check().unwrap_err().contains("not both"));
        let c: RunnerConfig =
            yaml_serde::from_str("agent: { tab: t }\ntools:\n  archive: { path: tools.google.gmail.modify }\n")
                .unwrap();
        assert!(c.check().unwrap_err().contains("runner.executor isn't set"));
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
