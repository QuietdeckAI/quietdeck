//! herdr's TUI as the node's orchestrator: the agents and their tabs (`agent list`), its event stream
//! (`events.subscribe` on its socket), tabs opened and closed for the deck, and messages typed in with
//! `agent prompt`. A session's address is its herdr pane ID. Commands run through herdr's CLI (argv, never a shell).
//! The client contract this follows is in herdr's socket-api docs ("Client contract").

use std::path::{Path, PathBuf};

use crate::deliver::OFFLINE_PREFIX;
use async_trait::async_trait;
use jiff::Timestamp;
use quietdeck_types::directory::{SessionEnd, SessionInfo};
use quietdeck_types::node::Delivered;
use quietdeck_types::{AgentKind, AgentStatus};
use serde::Deserialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::{Event, EventStream, OpenTab, Opened, Orchestrator, StreamError};

/// herdr, run as `program` (`programs.herdr` in node.yaml), on the machine named `machine`.
pub struct Herdr {
    pub program: String,
    /// The machine's name, which herdr's `herdr://open` link takes to find it among herdr's saved machines.
    pub machine: String,
}

#[async_trait]
impl Orchestrator for Herdr {
    fn name(&self) -> &'static str {
        "herdr"
    }

    async fn sessions(&self) -> anyhow::Result<Vec<SessionInfo>> {
        let mut sessions = snapshot(&self.program).await?;
        for s in &mut sessions {
            s.open_url = s.tab.as_deref().map(|tab| open_url(&self.machine, tab));
            s.opens_in = Some("herdr".into());
        }
        Ok(sessions)
    }

    async fn subscribe(&self) -> Result<Box<dyn EventStream>, StreamError> {
        let path =
            socket_path(&self.program).await.map_err(|e| StreamError::Io(std::io::Error::other(e.to_string())))?;
        Ok(Box::new(Subscription::open(&path).await?))
    }

    async fn open(&self, tab: &OpenTab) -> Result<Opened, String> {
        create_tab(&self.program, &tab_create(tab)).await
    }

    async fn close(&self, session_id: &str) -> Result<(), String> {
        close_session(&self.program, session_id).await
    }

    async fn deliver(&self, address: &str, text: &str) -> Result<Delivered, String> {
        prompt(&self.program, address, text).await
    }

    fn own_messages(&self) -> Vec<String> {
        auto_continue_message().into_iter().collect()
    }
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    result: T,
}

#[derive(Debug, Deserialize)]
struct AgentList {
    agents: Vec<Agent>,
}

#[derive(Debug, Deserialize)]
struct Agent {
    agent: Option<String>,
    agent_session: Option<AgentSession>,
    agent_status: Option<String>,
    cwd: Option<String>,
    pane_id: String,
    tab_id: Option<String>,
    workspace_id: Option<String>,
    state_changed_at_unix_ms: Option<i64>,
    /// Newer herdr puts the tab's label on the agent record.
    tab_label: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AgentSession {
    value: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TabList {
    tabs: Vec<Tab>,
}

#[derive(Debug, Deserialize)]
struct Tab {
    tab_id: String,
    label: Option<String>,
}

async fn run_json<T: serde::de::DeserializeOwned>(herdr: &str, args: &[&str]) -> anyhow::Result<T> {
    let out = tokio::process::Command::new(herdr).args(args).output().await?;
    if !out.status.success() {
        anyhow::bail!("herdr {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    let env: Envelope<T> =
        serde_json::from_slice(&out.stdout).map_err(|e| anyhow::anyhow!("reading herdr {}: {e}", args.join(" ")))?;
    Ok(env.result)
}

pub fn parse_status(s: Option<&str>) -> AgentStatus {
    match s {
        Some("idle") => AgentStatus::Idle,
        Some("done") => AgentStatus::Done,
        Some("working") => AgentStatus::Working,
        Some("blocked") => AgentStatus::Blocked,
        _ => AgentStatus::Unknown,
    }
}

/// Every agent session herdr knows on this machine. Agents without a session ID (still starting) are left out.
pub async fn snapshot(herdr: &str) -> anyhow::Result<Vec<SessionInfo>> {
    let agents: AgentList = run_json(herdr, &["agent", "list"]).await?;
    let tabs: TabList = run_json(herdr, &["tab", "list"]).await.unwrap_or(TabList { tabs: vec![] });
    Ok(join(agents, tabs))
}

fn agent_kind(s: Option<&str>) -> Option<AgentKind> {
    match s {
        Some("claude") => Some(AgentKind::Claude),
        Some("codex") => Some(AgentKind::Codex),
        _ => None,
    }
}

fn join(agents: AgentList, tabs: TabList) -> Vec<SessionInfo> {
    agents
        .agents
        .into_iter()
        .filter_map(|a| {
            let session_id = a.agent_session.and_then(|s| s.value)?;
            let agent = agent_kind(a.agent.as_deref())?;
            let tab = a.tab_label.or_else(|| {
                let id = a.tab_id.as_deref()?;
                tabs.tabs.iter().find(|t| t.tab_id == id).and_then(|t| t.label.clone())
            });
            Some(SessionInfo {
                session_id,
                agent,
                workspace: a.workspace_id,
                tab,
                address: Some(a.pane_id),
                cwd: a.cwd,
                status: parse_status(a.agent_status.as_deref()),
                status_changed_at: a
                    .state_changed_at_unix_ms
                    .and_then(|ms| Timestamp::from_millisecond(ms).ok())
                    .unwrap_or_else(Timestamp::now),
                resume_command: None,
                running: true,
                open_url: None,
                opens_in: None,
            })
        })
        .collect()
}

/// herdr's link that focuses a tab and brings its terminal forward, on this machine's herdr or a saved one.
fn open_url(machine: &str, tab: &str) -> String {
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("machine", machine)
        .append_pair("tab", tab)
        .finish();
    // A literal + is already %2B, so every + left stands for a space.
    format!("herdr://open?{}", query.replace('+', "%20"))
}

/// The auto-continue message herdr's config.toml sets, if any (`[session.auto_continue] message`).
fn auto_continue_message() -> Option<String> {
    let dir =
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| crate::config::home().join(".config"));
    let text = std::fs::read_to_string(dir.join("herdr/config.toml")).ok()?;
    auto_continue_from(&text)
}

fn auto_continue_from(toml_text: &str) -> Option<String> {
    let table: toml::Table = toml::from_str(toml_text).ok()?;
    table.get("session")?.get("auto_continue")?.get("message")?.as_str().map(str::to_string)
}

// ---------------------------------------------------------------- events

/// What the node subscribes to. Status changes for every pane, the two close events, and the lifecycle events after
/// which `agent list` reads differently (a new agent, its session ID arriving, a rename or a move).
const SUBSCRIPTIONS: &[&str] = &[
    "pane.agent_status_changed",
    "tab.closed",
    "pane.agent_exited",
    "pane.agent_detected",
    "pane.updated",
    "pane.moved",
    "tab.renamed",
    "tab.moved",
];

fn session_value(v: &Value) -> Option<String> {
    v.get("agent_session")?.get("value")?.as_str().map(str::to_string)
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

pub fn parse_event(line: &str) -> Event {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return Event::Other };
    let data = v.get("data").cloned().unwrap_or(Value::Null);
    match v.get("event").and_then(Value::as_str).unwrap_or_default() {
        "pane.agent_status_changed" => match str_field(&data, "pane_id") {
            Some(address) => {
                Event::Status { address, status: parse_status(data.get("agent_status").and_then(Value::as_str)) }
            }
            None => Event::Other,
        },
        "tab_closed" => Event::TabClosed {
            sessions: data
                .get("agents")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(session_value).collect())
                .unwrap_or_default(),
        },
        "pane_agent_exited" => Event::AgentExited {
            session: session_value(&data),
            end: SessionEnd::AgentExited {
                final_status: data.get("final_status").and_then(Value::as_str).map(|s| parse_status(Some(s))),
                terminal_closed: data.get("pane_exited").and_then(Value::as_bool).unwrap_or(false),
            },
        },
        "pane_updated" => {
            let pane = data.get("pane").cloned().unwrap_or(Value::Null);
            match str_field(&pane, "pane_id") {
                Some(id) => Event::Updated {
                    address: id,
                    session: session_value(&pane),
                    tab: str_field(&pane, "tab_label"),
                    cwd: str_field(&pane, "cwd"),
                },
                None => Event::Changed,
            }
        }
        "pane_agent_detected" | "pane_moved" | "tab_renamed" | "tab_moved" => Event::Changed,
        _ => Event::Other,
    }
}

/// herdr's API socket, as `herdr status server` reports it (it knows named sessions and dev builds).
async fn socket_path(herdr: &str) -> anyhow::Result<PathBuf> {
    let out = tokio::process::Command::new(herdr).args(["status", "server"]).output().await?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .find_map(|l| l.strip_prefix("socket:").map(|p| PathBuf::from(p.trim())))
        .filter(|_| text.lines().any(|l| l.trim() == "status: running"))
        .ok_or_else(|| anyhow::anyhow!("herdr isn't running"))
}

/// An open subscription: call [`Subscription::next`] for each event.
struct Subscription {
    lines: tokio::io::Lines<BufReader<tokio::net::UnixStream>>,
}

impl Subscription {
    async fn open(socket: &Path) -> Result<Subscription, StreamError> {
        let mut stream = tokio::net::UnixStream::connect(socket).await?;
        let subs: Vec<Value> = SUBSCRIPTIONS.iter().map(|t| serde_json::json!({"type": t})).collect();
        let req = serde_json::json!({"id": "quietdeck-node", "method": "events.subscribe",
            "params": {"subscriptions": subs}});
        stream.write_all(format!("{req}\n").as_bytes()).await?;
        let mut lines = BufReader::new(stream).lines();
        let first = lines.next_line().await?.ok_or_else(|| StreamError::Refused("herdr hung up".into()))?;
        let v: Value = serde_json::from_str(&first).unwrap_or(Value::Null);
        if v.pointer("/result/type").and_then(Value::as_str) == Some("subscription_started") {
            return Ok(Subscription { lines });
        }
        let msg = v.pointer("/error/message").and_then(Value::as_str).unwrap_or(&first).to_string();
        Err(StreamError::Refused(msg))
    }
}

#[async_trait]
impl EventStream for Subscription {
    /// None when herdr closed the stream (it stopped, restarted or handed off).
    async fn next(&mut self) -> Result<Option<Event>, StreamError> {
        Ok(self.lines.next_line().await?.map(|l| parse_event(&l)))
    }
}

// ---------------------------------------------------------------- starting, resuming and closing

/// `herdr tab create --agent …`: answers once the agent takes input, or with herdr's own message for a start-up
/// screen (the tab stays open for the person) or a failed start.
async fn create_tab(herdr: &str, args: &[String]) -> Result<Opened, String> {
    let run = tokio::process::Command::new(herdr).args(args).output();
    let out = tokio::time::timeout(std::time::Duration::from_secs(120), run)
        .await
        .map_err(|_| "herdr didn't answer within two minutes".to_string())?
        .map_err(|e| format!("running {herdr}: {e}"))?;
    if !out.status.success() {
        return Err(herdr_error(&out.stderr));
    }
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("reading herdr's answer: {e}"))?;
    let tab_id = v.pointer("/result/tab/tab_id").and_then(Value::as_str);
    let pane_id = v.pointer("/result/root_pane/pane_id").and_then(Value::as_str);
    match (tab_id, pane_id) {
        (Some(_), Some(p)) => {
            Ok(Opened { address: p.into(), session_id: v.pointer("/result/agent").and_then(session_value) })
        }
        _ => Err("herdr's answer named no tab or pane".into()),
    }
}

/// herdr's error as a sentence: its message, and for a failed start the agent's last line (`No conversation found
/// with session ID: …`).
fn herdr_error(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr).trim().to_string();
    let Ok(v) = serde_json::from_str::<Value>(&text) else { return text };
    let Some(err) = v.get("error") else { return text };
    let message = err.get("message").and_then(Value::as_str).unwrap_or(&text).to_string();
    let last = err
        .get("output")
        .and_then(Value::as_array)
        .and_then(|lines| lines.iter().rev().filter_map(Value::as_str).map(str::trim).find(|l| !l.is_empty()));
    match last {
        Some(l) if err.get("code").and_then(Value::as_str) == Some("agent_start_failed") => format!("{message}: {l}"),
        _ => message,
    }
}

/// `herdr tab create` for a tab: herdr runs the agent itself, with the agent's arguments after `--`.
fn tab_create(tab: &OpenTab) -> Vec<String> {
    let kind = match tab.agent {
        AgentKind::Claude => "claude",
        AgentKind::Codex => "codex",
    };
    let mut a: Vec<String> =
        ["tab", "create", "--label", &tab.label, "--cwd", &tab.cwd, "--no-focus", "--agent", kind, "--"]
            .map(String::from)
            .into();
    a.extend(tab.args.iter().cloned());
    a
}

/// Close the tab a running session is in.
async fn close_session(herdr: &str, session_id: &str) -> Result<(), String> {
    let agents: AgentList = run_json(herdr, &["agent", "list"]).await.map_err(|e| e.to_string())?;
    let tab = agents
        .agents
        .iter()
        .find(|a| a.agent_session.as_ref().and_then(|s| s.value.as_deref()) == Some(session_id))
        .and_then(|a| a.tab_id.clone())
        .ok_or_else(|| format!("session {session_id} isn't running on this machine"))?;
    let out = tokio::process::Command::new(herdr)
        .args(["tab", "close", &tab])
        .output()
        .await
        .map_err(|e| format!("running {herdr}: {e}"))?;
    if out.status.success() { Ok(()) } else { Err(herdr_error(&out.stderr)) }
}

/// `herdr agent prompt`: types the text into the pane and submits it, once the agent takes input.
async fn prompt(herdr: &str, pane: &str, text: &str) -> Result<Delivered, String> {
    let out = tokio::process::Command::new(herdr)
        .args(["agent", "prompt", pane, text])
        .output()
        .await
        .map_err(|e| format!("running {herdr}: {e}"))?;
    if out.status.success() {
        return Ok(Delivered::Orchestrator);
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let err = if err.is_empty() { String::from_utf8_lossy(&out.stdout).trim().to_string() } else { err };
    if err.contains("not found") || err.contains("no such pane") || err.contains("unknown pane") {
        return Err(format!("{OFFLINE_PREFIX} {err}"));
    }
    Err(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_join_their_tab_labels() {
        let agents: Envelope<AgentList> = serde_json::from_value(serde_json::json!({"result": {"agents": [
            {"agent": "claude", "agent_session": {"value": "s1"}, "agent_status": "blocked", "cwd": "/w",
             "pane_id": "w1:p1", "tab_id": "w1:t1", "workspace_id": "w1", "state_changed_at_unix_ms": 1795402660205i64},
            {"agent": "claude", "agent_status": "idle", "pane_id": "w1:p2", "tab_id": "w1:t2"},
            {"agent": "shell", "agent_session": {"value": "x"}, "pane_id": "w1:p3"}
        ]}}))
        .unwrap();
        let tabs: Envelope<TabList> =
            serde_json::from_value(serde_json::json!({"result": {"tabs": [{"tab_id": "w1:t1", "label": "billing"}]}}))
                .unwrap();
        let s = join(agents.result, tabs.result);
        assert_eq!(s.len(), 1, "no session ID, or not an agent: left out");
        assert_eq!(s[0].tab.as_deref(), Some("billing"));
        assert_eq!(s[0].status, AgentStatus::Blocked);
    }

    #[test]
    fn the_open_link_names_the_machine_and_tab() {
        assert_eq!(open_url("mac", "esign cert+1"), "herdr://open?machine=mac&tab=esign%20cert%2B1");
    }

    // Lines as a herdr server sent them (patched branch, 23 November 2026).
    #[test]
    fn herdr_events_parse() {
        let status = r#"{"data":{"agent":"claude","agent_status":"working","pane_id":"w1:p3","tab_id":"w1:t3","tab_label":"qd probe","workspace_id":"w1"},"event":"pane.agent_status_changed"}"#;
        assert_eq!(parse_event(status), Event::Status { address: "w1:p3".into(), status: AgentStatus::Working });

        let closed = r#"{"data":{"agents":[{"agent":"claude","agent_session":{"agent":"claude","kind":"id","source":"herdr:claude","value":"4f20528f"},"name":"qd-probe","pane_id":"w1:p3"}],"label":"qd probe","tab_id":"w1:t3","type":"tab_closed","workspace_id":"w1"},"event":"tab_closed"}"#;
        assert_eq!(parse_event(closed), Event::TabClosed { sessions: vec!["4f20528f".into()] });

        let exited = r#"{"data":{"agent":"claude","final_status":"done","name":"qd-probe","pane_exited":false,"pane_id":"w1:p2","tab_id":"w1:t2","tab_label":"qd probe","type":"pane_agent_exited","workspace_id":"w1"},"event":"pane_agent_exited"}"#;
        assert_eq!(
            parse_event(exited),
            Event::AgentExited {
                session: None,
                end: SessionEnd::AgentExited { final_status: Some(AgentStatus::Done), terminal_closed: false }
            }
        );

        let updated = r#"{"data":{"pane":{"agent":"claude","agent_session":{"agent":"claude","kind":"id","source":"herdr:claude","value":"4f20528f"},"agent_status":"idle","cwd":"/Users/x/code/q","pane_id":"w1:p3","tab_id":"w1:t3","tab_label":"qd probe","workspace_id":"w1"},"type":"pane_updated"},"event":"pane_updated"}"#;
        assert_eq!(
            parse_event(updated),
            Event::Updated {
                address: "w1:p3".into(),
                session: Some("4f20528f".into()),
                tab: Some("qd probe".into()),
                cwd: Some("/Users/x/code/q".into())
            }
        );
        assert_eq!(parse_event(r#"{"event":"tab_renamed","data":{"type":"tab_renamed"}}"#), Event::Changed);
        assert_eq!(parse_event(r#"{"event":"some_new_kind","data":{}}"#), Event::Other);
        assert_eq!(parse_event("not json"), Event::Other);
    }

    #[test]
    fn a_tab_runs_the_agent_with_its_own_arguments_after_the_separator() {
        let tab = OpenTab {
            label: "esign cert".into(),
            cwd: "/w".into(),
            agent: AgentKind::Claude,
            args: vec!["--name".into(), "esign cert".into()],
        };
        assert_eq!(
            tab_create(&tab),
            [
                "tab",
                "create",
                "--label",
                "esign cert",
                "--cwd",
                "/w",
                "--no-focus",
                "--agent",
                "claude",
                "--",
                "--name",
                "esign cert"
            ]
        );
    }

    #[test]
    fn the_auto_continue_message_comes_from_herdrs_config() {
        let text =
            "[ui]\nconfirm_close = true\n\n[session.auto_continue]\nenabled = true\nmessage = \"Go on ({reason})\"\n";
        assert_eq!(auto_continue_from(text).as_deref(), Some("Go on ({reason})"));
        assert_eq!(auto_continue_from("[session.auto_continue]\nenabled = true\n"), None);
        assert_eq!(auto_continue_from("not = = toml"), None);
    }

    #[test]
    fn herdr_errors_read_as_sentences() {
        let screen = br#"{"error":{"code":"agent_startup_screen","message":"qd probe needs you in herdr: the agent stopped at the folder trust prompt while starting. The tab is left open.","pane_id":"w1:p2","screen":"trust","tab_closed":false,"tab_id":"w1:t2"},"id":"cli:tab:create"}"#;
        assert!(herdr_error(screen).starts_with("qd probe needs you in herdr: the agent stopped at the folder trust"));
        let failed = br#"{"error":{"code":"agent_start_failed","message":"agent process exited before becoming interactive","output":["claude --resume 0000","No conversation found with session ID: 0000",""],"tab_closed":true},"id":"cli:tab:create"}"#;
        assert_eq!(
            herdr_error(failed),
            "agent process exited before becoming interactive: No conversation found with session ID: 0000"
        );
        assert_eq!(herdr_error(b"plain words"), "plain words");
    }
}
