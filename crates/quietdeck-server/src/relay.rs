//! The message relay: pass a delivery to the node that runs the target session, or fail at once. Nothing about a
//! message is stored; only per-node counts.

use std::time::Duration;

use quietdeck_types::directory::SessionRow;
use quietdeck_types::node::{DeckRequest, Delivered};
use quietdeck_types::relay::{Delivery, DeliveryError, DeliveryKind, DeliveryTarget};

use crate::db::{Db, now_ms};
use crate::hub::{Hub, HubError};
use crate::store::sessions;

/// How long a node gets to hand a delivery over. Delivery itself is quick (a socket write or one herdr call).
/// Delivering to a session is a socket write or a herdr prompt, done in well under a second: the CLI waits 5 seconds
/// for `msg send` in all, so the deck answers within 4.
pub const DELIVERY_TIMEOUT: Duration = Duration::from_secs(4);

/// The one running session an address names, or why there isn't one.
pub async fn find_target(db: &Db, hub: &Hub, address: &str) -> Result<SessionRow, DeliveryError> {
    let fail = |e: anyhow::Error| DeliveryError::Failed { address: address.to_string(), error: e.to_string() };
    let rows = sessions::resolve_address(db, hub, address).await.map_err(fail)?;
    let mut nodes: Vec<&str> = rows.iter().map(|r| r.node.as_str()).collect();
    nodes.dedup();
    match rows.len() {
        // None running: one the node knew that has stopped is offline; a name nobody has is a wrong name.
        0 => {
            if !sessions::resolve_ended(db, hub, address).await.map_err(fail)?.is_empty() {
                return Err(DeliveryError::SessionOffline { address: address.to_string() });
            }
            let (tab, node) = match address.rsplit_once('@') {
                Some((t, n)) if !t.is_empty() && !n.is_empty() => (t.trim(), Some(n.trim())),
                _ => (address.trim(), None),
            };
            let suggestions = sessions::similar_tabs(db, tab, node).await.map_err(fail)?;
            Err(DeliveryError::SessionNotFound { tab: tab.to_string(), node: node.map(str::to_string), suggestions })
        }
        _ if nodes.len() > 1 => {
            Err(DeliveryError::Ambiguous { candidates: rows.iter().map(SessionRow::address).collect() })
        }
        // Several sessions in one node's tabs with the same name: the most recently active one (rows are sorted).
        _ => {
            let row = rows.into_iter().next().expect("checked non-empty");
            if !hub.is_connected(&row.node).await {
                return Err(DeliveryError::NodeOffline { address: row.address() });
            }
            Ok(row)
        }
    }
}

pub fn target_of(row: &SessionRow) -> DeliveryTarget {
    DeliveryTarget {
        tab: row.info.tab.clone().unwrap_or_default(),
        session_id: Some(row.info.session_id.clone()),
        address: row.info.address.clone(),
        agent: row.info.agent,
    }
}

/// Deliver to a session found by address.
pub async fn send(
    db: &Db,
    hub: &Hub,
    address: &str,
    kind: DeliveryKind,
    body: String,
) -> Result<Delivered, DeliveryError> {
    let row = find_target(db, hub, address).await?;
    deliver_to(db, hub, &row, kind, body).await
}

/// Fleetlayer's control plane, when the deck has a send token for it (set once at start).
pub static FLEETLAYER: std::sync::OnceLock<crate::fleetlayer::Sender> = std::sync::OnceLock::new();

pub async fn deliver_to(
    db: &Db,
    hub: &Hub,
    row: &SessionRow,
    kind: DeliveryKind,
    body: String,
) -> Result<Delivered, DeliveryError> {
    // The deck's own words go through Fleetlayer as service:quietdeck where it knows the session; outside data and
    // messages between sessions never do (they'd arrive as the deck's), and go to the node.
    if kind.is_deck_words()
        && let (Some(fl), Some(tab)) = (FLEETLAYER.get(), row.info.tab.as_deref())
    {
        let address = row.address();
        match fl.send(&format!("{tab}@{}", row.node), &kind.render(&body), kind.subkind()).await {
            crate::fleetlayer::Sent::Delivered(how) => {
                count(db, &row.node, true).await;
                return Ok(how);
            }
            crate::fleetlayer::Sent::Unknown => {}
            crate::fleetlayer::Sent::Failed { status, message } => {
                count(db, &row.node, false).await;
                return Err(match status {
                    503 => DeliveryError::SessionOffline { address },
                    _ => DeliveryError::Failed { address, error: message },
                });
            }
        }
    }
    let delivery = Delivery { id: new_delivery_id(), target: target_of(row), kind, body };
    let address = row.address();
    let result = match hub.request(&row.node, DeckRequest::Deliver { delivery }, DELIVERY_TIMEOUT).await {
        Ok(reply) => reply.into_result::<Delivered>().map_err(|error| classify(&address, error)),
        Err(HubError::Offline(_)) => Err(DeliveryError::NodeOffline { address: address.clone() }),
        Err(HubError::Timeout(_)) => {
            Err(DeliveryError::Failed { address: address.clone(), error: "the node didn't answer in time".into() })
        }
    };
    count(db, &row.node, result.is_ok()).await;
    result
}

/// The node reports an offline session with this prefix; anything else is passed back as it is.
pub const NODE_OFFLINE_PREFIX: &str = "offline:";

fn classify(address: &str, error: String) -> DeliveryError {
    if error.starts_with(NODE_OFFLINE_PREFIX) {
        DeliveryError::SessionOffline { address: address.to_string() }
    } else {
        DeliveryError::Failed { address: address.to_string(), error }
    }
}

fn new_delivery_id() -> String {
    format!("d_{}", hex::encode(crate::auth::random_bytes::<12>()))
}

async fn count(db: &Db, node: &str, ok: bool) {
    let sql = if ok {
        "INSERT INTO relay_stats (node, delivered_count) VALUES (?, 1) \
         ON CONFLICT (node) DO UPDATE SET delivered_count = delivered_count + 1"
    } else {
        "INSERT INTO relay_stats (node, failed_count, last_failed_at) VALUES (?, 1, ?2) \
         ON CONFLICT (node) DO UPDATE SET failed_count = failed_count + 1, last_failed_at = ?2"
    };
    let mut q = sqlx::query(sql).bind(node);
    if !ok {
        q = q.bind(now_ms());
    }
    if let Err(e) = q.execute(db).await {
        tracing::warn!(error = %e, "counting a delivery");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quietdeck_types::directory::SessionInfo;
    use quietdeck_types::{AgentKind, AgentStatus};

    fn info(id: &str, tab: &str, running: bool) -> SessionInfo {
        SessionInfo {
            session_id: id.into(),
            agent: AgentKind::Claude,
            workspace: Some("w1".into()),
            tab: Some(tab.into()),
            address: Some("w1:p1".into()),
            cwd: Some("/home/sam/code/app".into()),
            status: AgentStatus::Idle,
            status_changed_at: jiff::Timestamp::now(),
            resume_command: None,
            running,
            open_url: None,
            opens_in: None,
        }
    }

    #[tokio::test]
    async fn a_wrong_name_is_not_found_with_suggestions_and_a_stopped_session_is_offline() {
        let state = crate::state::AppState::for_tests().await;
        let config = crate::config::DeckConfig::default();
        sessions::upsert(&state.db, &config, "agents-triage", &info("s1", "quietdeck triage", true)).await.unwrap();
        sessions::upsert(&state.db, &config, "agents", &info("s2", "old work", false)).await.unwrap();

        let e = find_target(&state.db, &state.hub, "triage@agents-triage").await.unwrap_err();
        assert_eq!(
            e,
            DeliveryError::SessionNotFound {
                tab: "triage".into(),
                node: Some("agents-triage".into()),
                suggestions: vec!["quietdeck triage@agents-triage".into()],
            }
        );
        assert_eq!(
            e.to_string(),
            "no session named 'triage' on agents-triage; did you mean 'quietdeck triage@agents-triage'?"
        );
        let e = find_target(&state.db, &state.hub, "nobody").await.unwrap_err();
        assert_eq!(e.to_string(), "no session named 'nobody'");
        let e = find_target(&state.db, &state.hub, "old work@agents").await.unwrap_err();
        assert_eq!(e, DeliveryError::SessionOffline { address: "old work@agents".into() });
    }
}
