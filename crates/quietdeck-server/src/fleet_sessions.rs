//! The Sessions screen from Fleetlayer's registry (`GET /v1/fleet`), where the deck has Fleetlayer's address and a
//! read token: every machine in the fleet, including ones with no Quietdeck node, with Fleetlayer's status for each
//! session. The deck's own table fills in what Fleetlayer doesn't keep: sessions that ended more than an hour ago
//! (history for `--all` and `--since`) and machines without Fleetlayer.

use std::collections::{HashMap, HashSet};

use jiff::Timestamp;
use quietdeck_types::directory::{SessionEnd, SessionInfo, SessionRow};
use quietdeck_types::{AgentKind, AgentStatus};
use serde::Deserialize;

use crate::state::AppState;
use crate::store::sessions::{self, ListFilter};

/// `GET /v1/fleet`. Unknown fields are ignored: Fleetlayer only adds fields.
#[derive(Debug, Deserialize)]
pub struct Fleet {
    #[serde(default)]
    nodes: Vec<FleetNode>,
    #[serde(default)]
    sessions: Vec<FleetSession>,
}

#[derive(Debug, Deserialize)]
struct FleetNode {
    name: String,
    #[serde(default)]
    online: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FleetSession {
    node: String,
    session_id: String,
    cwd: Option<String>,
    updated_at: Option<Timestamp>,
    #[serde(default)]
    meta: Meta,
    ended: Option<Ended>,
    summary: Option<Summary>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Meta {
    harness: Option<String>,
    address: Option<String>,
    #[serde(default)]
    named: bool,
    workspace: Option<String>,
    tab: Option<String>,
    status: Option<String>,
    status_changed_at: Option<Timestamp>,
    running: Option<bool>,
    open_url: Option<String>,
    opens_in: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Ended {
    at: Timestamp,
    how: How,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum How {
    TabClosed,
    #[serde(rename_all = "camelCase")]
    AgentExited {
        final_status: Option<String>,
        #[serde(default)]
        terminal_closed: bool,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    sentence: Option<String>,
    paragraph: Option<String>,
    page: Option<String>,
    /// Unix milliseconds.
    last_activity: Option<i64>,
}

fn status(s: Option<&str>) -> AgentStatus {
    match s {
        Some("idle") => AgentStatus::Idle,
        Some("done") => AgentStatus::Done,
        Some("working") => AgentStatus::Working,
        Some("blocked") => AgentStatus::Blocked,
        _ => AgentStatus::Unknown,
    }
}

/// A Fleetlayer session as the deck serves it, or None for one it can't show (still starting, without its real ID
/// yet; an agent program it doesn't know).
fn row(state: &AppState, online: &HashSet<String>, s: FleetSession) -> Option<SessionRow> {
    if !s.meta.named {
        return None;
    }
    let agent = match s.meta.harness.as_deref() {
        Some("claude") => AgentKind::Claude,
        Some("codex") => AgentKind::Codex,
        _ => return None,
    };
    let running = s.ended.is_none() && s.meta.running.unwrap_or(true);
    let ended = s.ended.as_ref().map(|e| match &e.how {
        How::TabClosed => SessionEnd::TabClosed,
        How::AgentExited { final_status, terminal_closed } => SessionEnd::AgentExited {
            final_status: final_status.as_deref().map(|f| status(Some(f))),
            terminal_closed: *terminal_closed,
        },
    });
    let summary = s.summary.as_ref();
    let project = s.cwd.as_deref().and_then(|c| sessions::project_for_cwd(&state.config, c));
    Some(SessionRow {
        node_online: online.contains(&s.node),
        node: s.node,
        info: SessionInfo {
            session_id: s.session_id,
            agent,
            workspace: s.meta.workspace,
            tab: s.meta.tab,
            address: s.meta.address,
            cwd: s.cwd,
            status: status(s.meta.status.as_deref()),
            status_changed_at: s.meta.status_changed_at.or(s.updated_at).unwrap_or_else(Timestamp::now),
            resume_command: None,
            running,
            open_url: s.meta.open_url,
            opens_in: s.meta.opens_in,
        },
        project,
        sentence: summary.and_then(|m| m.sentence.clone()).filter(|t| !t.is_empty()),
        paragraph: summary.and_then(|m| m.paragraph.clone()).filter(|t| !t.is_empty()),
        page: summary.and_then(|m| m.page.clone()).filter(|t| !t.is_empty()),
        last_activity: summary.and_then(|m| m.last_activity).and_then(|ms| Timestamp::from_millisecond(ms).ok()),
        pushed_at: s.updated_at.unwrap_or_else(Timestamp::now),
        links: vec![],
        closed_at: s.ended.as_ref().map(|e| e.at),
        ended,
        prompt: None,
    })
}

/// The sessions `f` asks for: Fleetlayer's, then the deck's own for anything Fleetlayer doesn't list.
pub async fn list(state: &AppState, fleet: Fleet, f: &ListFilter) -> anyhow::Result<Vec<SessionRow>> {
    let online: HashSet<String> = fleet.nodes.iter().filter(|n| n.online).map(|n| n.name.clone()).collect();
    let since = f.since.as_deref().map(sessions::parse_since).transpose().map_err(anyhow::Error::msg)?;
    let wanted = |r: &SessionRow| {
        f.node.as_ref().is_none_or(|n| *n == r.node)
            && match since {
                Some(ms) => r.last_activity.unwrap_or(r.info.status_changed_at).as_millisecond() >= ms,
                None => f.all || r.info.running,
            }
    };
    let mut rows: Vec<SessionRow> =
        fleet.sessions.into_iter().filter_map(|s| row(state, &online, s)).filter(|r| wanted(r)).collect();
    let listed: HashSet<(String, String)> = rows.iter().map(|r| (r.node.clone(), r.info.session_id.clone())).collect();
    // A machine Fleetlayer knows is shown as Fleetlayer sees it: the deck's rows for it add only history (ended
    // sessions Fleetlayer has dropped), never a running session Fleetlayer doesn't list.
    let fleet_nodes: HashSet<&str> = fleet.nodes.iter().map(|n| n.name.as_str()).collect();
    let ours = sessions::list(&state.db, &state.hub, f).await?;
    let mut known: HashMap<(String, String), SessionRow> = HashMap::new();
    for r in ours {
        let key = (r.node.clone(), r.info.session_id.clone());
        if listed.contains(&key) {
            known.insert(key, r);
        } else if !fleet_nodes.contains(r.node.as_str()) {
            rows.push(r);
        } else if !r.info.running {
            // History on a machine Fleetlayer knows: whether that machine is up now is Fleetlayer's to say.
            let mut r = r;
            r.node_online = online.contains(&r.node);
            rows.push(r);
        }
    }
    // What only the deck knows about a session Fleetlayer lists: its links (moved from, moved to), its resume
    // command, the menu it's blocked on, and its summary when Fleetlayer has none yet.
    for r in &mut rows {
        if let Some(mine) = known.remove(&(r.node.clone(), r.info.session_id.clone())) {
            r.links = mine.links;
            r.info.resume_command = mine.info.resume_command;
            // The menu a blocked session waits on comes from its node, through the deck.
            if r.info.status == AgentStatus::Blocked {
                r.prompt = mine.prompt;
            }
            if r.sentence.is_none() {
                (r.sentence, r.paragraph, r.page) = (mine.sentence, mine.paragraph, mine.page);
            }
            r.last_activity = r.last_activity.max(mine.last_activity);
            r.project = r.project.take().or(mine.project);
        } else if r.links.is_empty() {
            r.links = sessions::links_for(&state.db, &r.node, &r.info.session_id).await?;
        }
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.last_activity.unwrap_or(r.info.status_changed_at)));
    let limit = f.limit.unwrap_or(if since.is_some() { 5000 } else { 200 }).clamp(1, 5000) as usize;
    rows.truncate(limit);
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DeckConfig;

    fn fleet() -> Fleet {
        serde_json::from_value(serde_json::json!({
            "nodes": [
                {"name": "agents", "machine": "agents", "online": true},
                {"name": "agents-talira", "machine": "agents", "online": true},
                {"name": "laptop", "machine": "laptop", "online": false}
            ],
            "sessions": [
                {"node": "agents", "sessionId": "s-api", "cwd": "/home/sam/code/api", "updatedAt": "2026-11-27T07:00:00Z",
                 "meta": {"harness": "claude", "address": "w1:p1", "named": true, "tab": "api", "status": "working",
                          "statusChangedAt": "2026-11-27T07:00:00Z", "running": true, "opensIn": "herdr", "new": 1},
                 "ended": null,
                 "summary": {"sentence": "Adding rate limits to the API.", "paragraph": "p", "page": "pg",
                             "lastActivity": 1_795_770_000_000_i64}},
                {"node": "agents-talira", "sessionId": "s-infra", "cwd": "/home/talira/infra",
                 "meta": {"harness": "codex", "named": true, "tab": "infra", "status": "idle", "running": true},
                 "ended": null},
                {"node": "agents", "sessionId": "s-old", "meta": {"harness": "claude", "named": true, "tab": "old",
                 "running": false}, "ended": {"at": "2026-11-27T06:30:00Z",
                 "how": {"kind": "agent_exited", "finalStatus": "done", "terminalClosed": true}}},
                {"node": "agents", "sessionId": "@w1:p9", "meta": {"harness": "claude", "named": false,
                 "tab": "starting", "running": true}, "ended": null}
            ]
        }))
        .unwrap()
    }

    fn info(id: &str, tab: &str, running: bool) -> SessionInfo {
        SessionInfo {
            session_id: id.into(),
            agent: AgentKind::Claude,
            workspace: Some("w1".into()),
            tab: Some(tab.into()),
            address: Some("w1:p5".into()),
            cwd: Some("/home/sam/code/app".into()),
            status: AgentStatus::Idle,
            status_changed_at: Timestamp::now(),
            resume_command: Some("cd /home/sam/code/api && claude --resume s-api".into()),
            running,
            open_url: None,
            opens_in: None,
        }
    }

    #[tokio::test]
    async fn fleetlayers_sessions_with_the_decks_history_and_machines_it_doesnt_know() {
        let state = AppState::for_tests().await;
        let c = DeckConfig::default();
        // The deck's own rows: the same api session (its resume command only the deck has), a running session on
        // agents that Fleetlayer no longer lists (stale), an old ended one, and a machine without Fleetlayer.
        sessions::upsert(&state.db, &c, "agents", &info("s-api", "api", true)).await.unwrap();
        sessions::upsert(&state.db, &c, "agents", &info("s-stale", "stale", true)).await.unwrap();
        sessions::upsert(&state.db, &c, "agents", &info("s-last-week", "last week", false)).await.unwrap();
        sessions::upsert(&state.db, &c, "solo-laptop", &info("s-solo", "triage", true)).await.unwrap();

        let running = list(&state, fleet(), &ListFilter::default()).await.unwrap();
        let ids: Vec<(&str, &str)> = running.iter().map(|r| (r.node.as_str(), r.info.session_id.as_str())).collect();
        assert!(ids.contains(&("agents", "s-api")), "{ids:?}");
        assert!(ids.contains(&("agents-talira", "s-infra")), "a machine with no Quietdeck node: {ids:?}");
        assert!(ids.contains(&("solo-laptop", "s-solo")), "a machine without Fleetlayer: {ids:?}");
        assert!(!ids.contains(&("agents", "s-stale")), "Fleetlayer's list is the truth on its machines: {ids:?}");
        assert!(!ids.iter().any(|(_, id)| id.starts_with('@')), "no session without its real ID: {ids:?}");
        assert!(!ids.contains(&("agents", "s-old")), "ended sessions only with all: {ids:?}");

        let api = running.iter().find(|r| r.info.session_id == "s-api").unwrap();
        assert_eq!(api.info.status, AgentStatus::Working);
        assert_eq!(api.sentence.as_deref(), Some("Adding rate limits to the API."));
        assert!(api.node_online);
        assert!(api.info.resume_command.is_some(), "what only the deck knows is kept");
        let infra = running.iter().find(|r| r.info.session_id == "s-infra").unwrap();
        assert_eq!(infra.info.agent, AgentKind::Codex);

        let all = list(&state, fleet(), &ListFilter { all: true, ..Default::default() }).await.unwrap();
        let old = all.iter().find(|r| r.info.session_id == "s-old").unwrap();
        assert!(!old.info.running);
        assert!(matches!(old.ended, Some(SessionEnd::AgentExited { terminal_closed: true, .. })));
        let last_week = all.iter().find(|r| r.info.session_id == "s-last-week").expect("history Fleetlayer dropped");
        assert!(last_week.node_online, "its machine is up, as Fleetlayer says");

        let one = list(&state, fleet(), &ListFilter { node: Some("agents-talira".into()), ..Default::default() })
            .await
            .unwrap();
        assert_eq!(one.len(), 1);
    }
}
