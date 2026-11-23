//! herdr through its CLI (argv, never a shell): the agents on this machine, with their tabs.

use jiff::Timestamp;
use quietdeck_types::directory::SessionInfo;
use quietdeck_types::{AgentKind, AgentStatus};
use serde::Deserialize;

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
    let env: Envelope<T> = serde_json::from_slice(&out.stdout)
        .map_err(|e| anyhow::anyhow!("reading herdr {}: {e}", args.join(" ")))?;
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

fn join(agents: AgentList, tabs: TabList) -> Vec<SessionInfo> {
    agents
        .agents
        .into_iter()
        .filter_map(|a| {
            let session_id = a.agent_session.and_then(|s| s.value)?;
            let agent = match a.agent.as_deref() {
                Some("claude") => AgentKind::Claude,
                Some("codex") => AgentKind::Codex,
                _ => return None,
            };
            let tab = a.tab_label.or_else(|| {
                let id = a.tab_id.as_deref()?;
                tabs.tabs.iter().find(|t| t.tab_id == id).and_then(|t| t.label.clone())
            });
            Some(SessionInfo {
                session_id,
                agent,
                workspace: a.workspace_id,
                tab,
                pane: Some(a.pane_id),
                cwd: a.cwd,
                status: parse_status(a.agent_status.as_deref()),
                status_changed_at: a
                    .state_changed_at_unix_ms
                    .and_then(|ms| Timestamp::from_millisecond(ms).ok())
                    .unwrap_or_else(Timestamp::now),
                resume_command: None,
                running: true,
            })
        })
        .collect()
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
}
