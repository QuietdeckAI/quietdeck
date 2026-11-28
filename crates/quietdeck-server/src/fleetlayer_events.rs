//! What the deck reads from Fleetlayer's event log: session status changes (`session.status`), and Matrix rooms read
//! through Fleetlayer's Matrix plugin in place of the node's own Matrix reader.
//!
//! Each status change is applied to the deck's sessions (`sessions::apply_status`), so the overseer watch and the
//! Sessions screen see Fleetlayer's statuses as they change, for machines without a Quietdeck node too, and a session
//! seen working counts as new work for the overseer even when it went idle again between two of the watch's ticks.
//!
//! The deck reads the control plane's event log (`GET /v1/events?after=<id>&channel=…&wait=30`, with the read token)
//! for the channels in `QUIETDECK_FLEETLAYER_ROOM_CHANNELS`, and stores each room message as the node's reader did
//! (`messages::ingest`): same room ID and event ID, so a message read both ways is kept once. Triage's wake-ups, the
//! hold while the person reads and `quietdeck messages` don't change. The feed's position is kept in `settings`, so
//! nothing is lost while the deck restarts; the first read starts from now. A control plane without the feed (older
//! than its release) answers without `next`, and the deck asks again every minute without complaining.

use std::time::Duration;

use quietdeck_types::message::IncomingMessage;
use serde::Deserialize;

use crate::fleetlayer::Client;
use crate::state::AppState;

const CURSOR: &str = "fleetlayer.events_cursor";
/// How long each request waits for an event (Fleetlayer allows 60).
pub const WAIT_SECS: u64 = 30;
/// After a failure, or while the control plane has no feed.
const RETRY: Duration = Duration::from_secs(60);

/// One answer of the feed.
#[derive(Debug, Deserialize)]
pub struct Feed {
    #[serde(default)]
    pub events: Vec<Event>,
    /// The cursor to ask from next. Missing from a control plane without the feed.
    pub next: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Event {
    pub id: i64,
    #[serde(default)]
    pub channel: String,
    pub kind: String,
    #[serde(default)]
    pub body: serde_json::Value,
    #[serde(default)]
    pub source: Option<String>,
    /// Milliseconds.
    pub at: i64,
}

/// A room message as the deck stores it, or None for an event that isn't one.
fn message(e: &Event) -> Option<IncomingMessage> {
    if e.kind != "matrix.message" && e.kind != "matrix.undecryptable" {
        return None;
    }
    let b = &e.body;
    let text = |k: &str| b[k].as_str().map(str::to_string);
    let room_id = text("roomId")?;
    Some(IncomingMessage {
        source: "matrix".into(),
        channel_name: text("room").unwrap_or_else(|| room_id.clone()),
        channel: room_id,
        external_id: text("eventId")?,
        sender: text("sender")?,
        body: text("text").unwrap_or_default(),
        sent_at: jiff::Timestamp::from_millisecond(e.at).unwrap_or_else(|_| jiff::Timestamp::now()),
    })
}

/// Fleetlayer's status channel.
pub const STATUS_CHANNEL: &str = "session.status";

/// A `session.status` event's time: `statusChangedAt` (RFC 3339 or milliseconds), else the event's own.
fn status_at(e: &Event) -> i64 {
    match &e.body["statusChangedAt"] {
        serde_json::Value::String(t) => t.parse::<jiff::Timestamp>().map(|t| t.as_millisecond()).unwrap_or(e.at),
        serde_json::Value::Number(n) => n.as_i64().unwrap_or(e.at),
        _ => e.at,
    }
}

/// Applies a `session.status` event to the deck's sessions.
async fn apply_status(state: &AppState, e: &Event) -> anyhow::Result<()> {
    use quietdeck_types::AgentStatus;
    let b = &e.body;
    let (Some(node), Some(session_id)) = (b["node"].as_str(), b["sessionId"].as_str()) else { return Ok(()) };
    let status = match e.kind.strip_prefix("session.") {
        Some("working") => Some(AgentStatus::Working),
        Some("idle") => Some(AgentStatus::Idle),
        Some("done") => Some(AgentStatus::Done),
        Some("blocked") => Some(AgentStatus::Blocked),
        Some("gone") => None,
        _ => return Ok(()),
    };
    let at = status_at(e);
    crate::store::sessions::apply_status(
        &state.db,
        &state.config,
        node,
        session_id,
        b["tab"].as_str(),
        b["cwd"].as_str(),
        status,
        at,
    )
    .await?;
    if status == Some(AgentStatus::Working) {
        // Its next idle is new work for the overseer, however soon it comes.
        sqlx::query("UPDATE overseer_told SET worked = 1 WHERE worked = 0 AND node = ? AND session_id = ?")
            .bind(node)
            .bind(session_id)
            .execute(&state.db)
            .await?;
    }
    Ok(())
}

/// The node that read an event (`matrix@agents` → `agents`), for the stored message's `node`.
fn reader(e: &Event) -> String {
    e.source
        .as_deref()
        .and_then(|s| s.rsplit_once('@').map(|(_, n)| n.to_string()))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "fleetlayer".into())
}

async fn cursor(state: &AppState) -> anyhow::Result<Option<String>> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM settings WHERE name = ?").bind(CURSOR).fetch_optional(&state.db).await?;
    Ok(row.map(|r| r.0))
}

async fn set_cursor(state: &AppState, value: i64) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO settings (name, value) VALUES (?, ?) \
         ON CONFLICT (owner_type, owner_id, name) DO UPDATE SET value = excluded.value",
    )
    .bind(CURSOR)
    .bind(value.to_string())
    .execute(&state.db)
    .await?;
    Ok(())
}

/// Stores one answer's room messages, grouped by the node that read them. Returns how many were new.
async fn store(state: &AppState, events: &[Event]) -> anyhow::Result<u32> {
    let mut by_node: Vec<(String, Vec<IncomingMessage>)> = Vec::new();
    let mut statuses = false;
    for e in events {
        if e.channel == STATUS_CHANNEL {
            apply_status(state, e).await?;
            statuses = true;
            continue;
        }
        let Some(m) = message(e) else { continue };
        let node = reader(e);
        match by_node.iter_mut().find(|(n, _)| *n == node) {
            Some((_, ms)) => ms.push(m),
            None => by_node.push((node, vec![m])),
        }
    }
    let mut new = 0;
    for (node, ms) in by_node {
        for chunk in ms.chunks(quietdeck_types::message::MAX_INGEST) {
            new += crate::messages::ingest(state, &node, chunk.to_vec()).await?.new;
        }
    }
    if statuses {
        state.changed(crate::events::Changed::Sessions);
    }
    Ok(new)
}

/// One read: waits up to [`WAIT_SECS`] for events after the kept cursor. `None` when the control plane has no feed.
pub async fn read_once(state: &AppState, client: &Client, channels: &str) -> anyhow::Result<Option<u32>> {
    let after = cursor(state).await?.unwrap_or_else(|| "latest".into());
    let Some(feed) = client.events_after(&after, channels, WAIT_SECS).await? else { return Ok(None) };
    let Some(next) = feed.next else { return Ok(None) };
    let new = store(state, &feed.events).await?;
    set_cursor(state, next).await?;
    Ok(Some(new))
}

/// The channels to read: the status channel and the rooms.
fn channels(rooms: Option<&str>) -> String {
    let mut all = vec![STATUS_CHANNEL];
    all.extend(rooms.unwrap_or_default().split(',').map(str::trim).filter(|c| !c.is_empty() && *c != STATUS_CHANNEL));
    all.join(",")
}

/// Runs for as long as the deck does, when it has a Fleetlayer address and token.
pub async fn ticker(state: AppState, rooms: Option<String>) {
    let Some(client) = state.fleetlayer.clone() else { return };
    let channels = channels(rooms.as_deref());
    tracing::info!(%channels, "reading Fleetlayer's event log");
    let (mut failing, mut absent) = (false, false);
    loop {
        // A read waits up to WAIT_SECS for an event; stopping the deck doesn't wait for it.
        let read = tokio::select! {
            r = read_once(&state, &client, &channels) => r,
            _ = crate::state::until_stopping(state.stopping.clone()) => return,
        };
        let pause = match read {
            Ok(Some(n)) => {
                if n > 0 {
                    tracing::info!(new = n, "room messages from Fleetlayer");
                }
                if failing || absent {
                    tracing::info!("reading Fleetlayer's event log again");
                }
                (failing, absent) = (false, false);
                None
            }
            Ok(None) => {
                if !absent {
                    tracing::info!("Fleetlayer's control plane has no event feed yet; asking every minute");
                }
                absent = true;
                Some(RETRY)
            }
            Err(e) => {
                if !failing {
                    tracing::warn!(error = %e, "can't read Fleetlayer's event log; trying again every minute");
                }
                failing = true;
                Some(RETRY)
            }
        };
        if let Some(p) = pause {
            tokio::select! {
                _ = tokio::time::sleep(p) => {}
                _ = crate::state::until_stopping(state.stopping.clone()) => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room_event(id: i64, event_id: &str) -> Event {
        serde_json::from_value(serde_json::json!({
            "id": id, "channel": "infrastructure-alerts", "kind": "matrix.message", "subject": null,
            "summary": "Gatus in Infrastructure alerts: down", "source": "matrix@agents", "at": 1764300000000i64,
            "body": {"room": "Infrastructure alerts", "roomId": "!abc:example.com", "sender": "@gatus:example.com",
                     "eventId": event_id, "text": "pages is down", "link": "https://matrix.to/#/!abc/$e"}
        }))
        .unwrap()
    }

    #[test]
    fn a_room_message_is_stored_as_the_nodes_reader_stored_it() {
        let m = message(&room_event(4, "$e1")).unwrap();
        assert_eq!(
            (m.source.as_str(), m.channel.as_str(), m.channel_name.as_str(), m.external_id.as_str()),
            ("matrix", "!abc:example.com", "Infrastructure alerts", "$e1")
        );
        assert_eq!((m.sender.as_str(), m.body.as_str()), ("@gatus:example.com", "pages is down"));
        assert_eq!(reader(&room_event(4, "$e1")), "agents");
        let other: Event =
            serde_json::from_value(serde_json::json!({"id": 5, "kind": "alertmanager.firing", "at": 0})).unwrap();
        assert!(message(&other).is_none(), "only room messages");
    }

    fn status_event(id: i64, kind: &str, changed_at: &str) -> Event {
        serde_json::from_value(serde_json::json!({
            "id": id, "channel": "session.status", "kind": kind, "subject": "api@laptop", "summary": "",
            "source": "control", "at": 1764300000000i64,
            "body": {"node": "laptop", "sessionId": "s-api", "tab": "api", "cwd": "/home/me/code/api",
                     "status": kind.trim_start_matches("session."), "statusChangedAt": changed_at}
        }))
        .unwrap()
    }

    async fn status_of(state: &AppState) -> (String, i64, i64) {
        sqlx::query_as(
            "SELECT status, status_changed_at, running FROM sessions WHERE node = 'laptop' AND session_id = 's-api'",
        )
        .fetch_one(&state.db)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn status_changes_reach_the_deck_and_an_older_one_doesnt_undo_a_newer() {
        let state = AppState::for_tests().await;
        store(&state, &[status_event(1, "session.working", "2026-11-29T05:00:00Z")]).await.unwrap();
        let t = |s: &str| s.parse::<jiff::Timestamp>().unwrap().as_millisecond();
        assert_eq!(status_of(&state).await, ("working".into(), t("2026-11-29T05:00:00Z"), 1));
        store(&state, &[status_event(2, "session.idle", "2026-11-29T05:02:00Z")]).await.unwrap();
        store(&state, &[status_event(3, "session.blocked", "2026-11-29T05:01:00Z")]).await.unwrap();
        assert_eq!(status_of(&state).await.0, "idle", "the older change is ignored");
        store(&state, &[status_event(4, "session.gone", "2026-11-29T05:03:00Z")]).await.unwrap();
        assert_eq!(status_of(&state).await.2, 0);
    }

    #[tokio::test]
    async fn working_counts_as_new_work_for_the_overseer() {
        let state = AppState::for_tests().await;
        sqlx::query(
            "INSERT INTO overseer_told (node, session_id, status, status_changed_at, told_at, worked) \
             VALUES ('laptop', 's-api', 'idle', 1, 1, 0)",
        )
        .execute(&state.db)
        .await
        .unwrap();
        store(&state, &[status_event(1, "session.working", "2026-11-29T05:00:00Z")]).await.unwrap();
        let (worked,): (i64,) = sqlx::query_as("SELECT worked FROM overseer_told").fetch_one(&state.db).await.unwrap();
        assert_eq!(worked, 1);
    }

    #[test]
    fn the_status_channel_is_always_read() {
        assert_eq!(channels(None), "session.status");
        assert_eq!(channels(Some("infrastructure-alerts, family,")), "session.status,infrastructure-alerts,family");
    }

    #[test]
    fn an_older_control_planes_answer_has_no_cursor() {
        let old: Feed =
            serde_json::from_value(serde_json::json!({"events": [{"id": 1, "kind": "x", "at": 0}]})).unwrap();
        assert!(old.next.is_none());
    }

    #[tokio::test]
    async fn a_message_read_twice_is_kept_once_and_the_cursor_is_kept() {
        let state = AppState::for_tests().await;
        assert_eq!(store(&state, &[room_event(4, "$e1"), room_event(5, "$e2")]).await.unwrap(), 2);
        assert_eq!(store(&state, &[room_event(5, "$e2")]).await.unwrap(), 0, "the same event from the node's reader");
        assert!(cursor(&state).await.unwrap().is_none());
        set_cursor(&state, 5).await.unwrap();
        assert_eq!(cursor(&state).await.unwrap().as_deref(), Some("5"));
    }
}
