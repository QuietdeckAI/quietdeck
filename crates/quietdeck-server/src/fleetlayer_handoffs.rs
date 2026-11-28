//! Handoffs made through Fleetlayer (`fleetlayer session handoff`, Fleetlayer v0.4.0), copied into the deck.
//!
//! Fleetlayer starts the new session, keeps the note and links old and new in its fleet view. What only the deck
//! knows stays the deck's: the old session's open cards and report move to the new session (`handoffs::continued`).
//! The deck reads the control plane's feed (`GET /v1/handoffs?since=`, every minute) and copies each handoff into its
//! own `handoffs` table once, so `sessions show`, the Sessions screen and `handoff show` treat it like one written
//! with `quietdeck handoff add`. A control plane older than v0.4.0 answers 404, and the deck waits for it quietly.

use std::time::Duration;

use quietdeck_types::action::SessionRef;
use serde::Deserialize;

use crate::db::now_ms;
use crate::fleetlayer::Client;
use crate::state::AppState;

const EVERY: Duration = Duration::from_secs(60);
const PAGE: usize = 500;
const CURSOR: &str = "fleetlayer.handoffs_cursor";
/// A handoff whose new session hasn't started yet (Fleetlayer keeps the handoff first and fills in the session when
/// the start answers, within about two minutes) is read again later, up to this age. Past it, it's taken as it is.
const STARTING_FOR: Duration = Duration::from_secs(5 * 60);

/// A page of `/v1/handoffs?since=`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Feed {
    pub handoffs: Vec<Wire>,
    pub next_cursor: String,
}

/// One handoff as the feed serves it (without its note).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Wire {
    pub id: String,
    /// `<tab>@<node>`.
    pub from: String,
    pub from_session_id: Option<String>,
    pub to: String,
    pub to_session_id: Option<String>,
    pub created_at: String,
}

fn session(address: &str, session_id: Option<String>) -> Option<SessionRef> {
    let (tab, node) = address.rsplit_once('@')?;
    (!tab.is_empty() && !node.is_empty()).then(|| SessionRef { node: node.into(), tab: tab.into(), session_id })
}

fn created_ms(w: &Wire) -> i64 {
    w.created_at.parse::<jiff::Timestamp>().map(|t| t.as_millisecond()).unwrap_or_else(|_| now_ms())
}

/// Whether a handoff's new session is still starting: read it again on the next pull.
fn still_starting(w: &Wire, now: i64) -> bool {
    w.to_session_id.is_none() && now - created_ms(w) < STARTING_FOR.as_millis() as i64
}

/// Copies one handoff and moves the old session's work to the new one. Copying it again does nothing.
async fn copy(state: &AppState, w: &Wire) -> anyhow::Result<bool> {
    let (Some(from), Some(to)) = (session(&w.from, w.from_session_id.clone()), session(&w.to, w.to_session_id.clone()))
    else {
        tracing::warn!(handoff = %w.id, from = %w.from, to = %w.to, "a Fleetlayer handoff the deck can't read");
        return Ok(false);
    };
    let text =
        format!("Handed off through Fleetlayer as {id}. Its note: `fleetlayer session handoff-note {id}`.", id = w.id);
    let id: Option<(i64,)> = sqlx::query_as(
        "INSERT INTO handoffs (from_node, from_tab, from_session_id, handoff, manifest, created_at, fleetlayer_id) \
         VALUES (?, ?, ?, ?, '', ?, ?) ON CONFLICT DO NOTHING RETURNING id",
    )
    .bind(&from.node)
    .bind(&from.tab)
    .bind(&from.session_id)
    .bind(&text)
    .bind(created_ms(w))
    .bind(&w.id)
    .fetch_optional(&state.db)
    .await?;
    let Some((id,)) = id else { return Ok(false) };
    crate::handoffs::continued(&state.db, id, &to).await?;
    Ok(true)
}

async fn cursor(state: &AppState) -> anyhow::Result<String> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM settings WHERE name = ?").bind(CURSOR).fetch_optional(&state.db).await?;
    Ok(row.map(|r| r.0).unwrap_or_default())
}

async fn set_cursor(state: &AppState, value: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO settings (name, value) VALUES (?, ?) \
         ON CONFLICT (owner_type, owner_id, name) DO UPDATE SET value = excluded.value",
    )
    .bind(CURSOR)
    .bind(value)
    .execute(&state.db)
    .await?;
    Ok(())
}

/// The cursor just before `w`, so the next pull reads it again.
fn cursor_before(w: &Wire) -> String {
    // The feed's cursor is `<created ms>:<id>` and compares the ID as text; an empty ID sorts before every real one.
    format!("{}:", created_ms(w))
}

/// Applies one page in order. Stops at a handoff whose session is still starting, and returns the cursor to keep
/// and how many handoffs it copied.
async fn apply(state: &AppState, feed: Feed, now: i64) -> anyhow::Result<(String, usize, bool)> {
    let mut copied = 0;
    for w in &feed.handoffs {
        if still_starting(w, now) {
            return Ok((cursor_before(w), copied, true));
        }
        if copy(state, w).await? {
            copied += 1;
        }
    }
    Ok((feed.next_cursor, copied, false))
}

/// Pulls everything new once. `None` when the control plane has no handoffs feed (older than v0.4.0).
pub async fn pull(state: &AppState, client: &Client) -> anyhow::Result<Option<usize>> {
    let mut total = 0;
    loop {
        let from = cursor(state).await?;
        let Some(feed) = client.handoffs_since(&from, PAGE).await? else { return Ok(None) };
        let n = feed.handoffs.len();
        let (next, copied, waiting) = apply(state, feed, now_ms()).await?;
        total += copied;
        let moved = next != from;
        if moved {
            set_cursor(state, &next).await?;
        }
        if waiting || n < PAGE || !moved {
            break;
        }
    }
    if total > 0 {
        state.changed(crate::events::Changed::Sessions);
        state.changed(crate::events::Changed::Cards);
    }
    Ok(Some(total))
}

/// Runs for as long as the deck does, when it has a Fleetlayer address and token.
pub async fn ticker(state: AppState) {
    let Some(client) = state.fleetlayer.clone() else { return };
    let mut failing = false;
    let mut absent = false;
    loop {
        match pull(&state, &client).await {
            Ok(Some(n)) => {
                if n > 0 || failing || absent {
                    tracing::info!(handoffs = n, "handoffs from Fleetlayer");
                }
                failing = false;
                absent = false;
            }
            Ok(None) => {
                if !absent {
                    tracing::info!("Fleetlayer's control plane has no handoffs feed yet (v0.4.0); asking every minute");
                }
                absent = true;
            }
            Err(e) => {
                if !failing {
                    tracing::warn!(error = %e, "can't read handoffs from Fleetlayer; trying again every minute");
                }
                failing = true;
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(EVERY) => {}
            _ = crate::state::until_stopping(state.stopping.clone()) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(id: &str, to_session_id: Option<&str>, created_at: &str) -> Wire {
        Wire {
            id: id.into(),
            from: "infra@laptop".into(),
            from_session_id: Some("s-old".into()),
            to: "infra@agents".into(),
            to_session_id: to_session_id.map(str::to_string),
            created_at: created_at.into(),
        }
    }

    async fn card_posted_by(state: &AppState, node: &str, tab: &str, card_state: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(
            "INSERT INTO cards (posted_node, posted_tab, account, thread_key, section, state, input, prepared_at, \
             created_at, updated_at) VALUES (?, ?, 'me', 't', 'needs_you', ?, '{}', 0, 0, 0) RETURNING id",
        )
        .bind(node)
        .bind(tab)
        .bind(card_state)
        .fetch_one(&state.db)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_fleetlayer_handoff_moves_the_old_sessions_open_cards_once() {
        let state = AppState::for_tests().await;
        let open = card_posted_by(&state, "laptop", "infra", "open").await;
        let feed = Feed {
            handoffs: vec![wire("h00000000000000a1", Some("s-new"), "2026-11-29T04:00:00Z")],
            next_cursor: "1795924800000:h00000000000000a1".into(),
        };
        let (next, copied, waiting) = apply(&state, feed, now_ms()).await.unwrap();
        assert_eq!((next.as_str(), copied, waiting), ("1795924800000:h00000000000000a1", 1, false));
        let posted: (String, String, Option<String>) =
            sqlx::query_as("SELECT posted_node, posted_tab, posted_session_id FROM cards WHERE id = ?")
                .bind(open)
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(posted, ("agents".into(), "infra".into(), Some("s-new".into())));
        let (continues, _) = crate::handoffs::links(&state.db, "agents", "s-new").await.unwrap();
        assert_eq!(continues.unwrap().session.tab, "infra");

        // The same handoff again (a cursor kept before a restart): nothing more happens.
        let again = Feed {
            handoffs: vec![wire("h00000000000000a1", Some("s-new"), "2026-11-29T04:00:00Z")],
            next_cursor: "1795924800000:h00000000000000a1".into(),
        };
        assert_eq!(apply(&state, again, now_ms()).await.unwrap().1, 0);
        let n: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM handoffs").fetch_one(&state.db).await.unwrap();
        assert_eq!(n.0, 1);
    }

    #[tokio::test]
    async fn a_handoff_still_starting_is_read_again_and_the_ones_before_it_are_kept() {
        let state = AppState::for_tests().await;
        let now = "2026-11-29T04:10:00Z".parse::<jiff::Timestamp>().unwrap().as_millisecond();
        let feed = Feed {
            handoffs: vec![
                wire("h00000000000000a1", Some("s-new"), "2026-11-29T04:00:00Z"),
                wire("h00000000000000a2", None, "2026-11-29T04:09:00Z"),
                wire("h00000000000000a3", Some("s-3"), "2026-11-29T04:09:30Z"),
            ],
            next_cursor: "x".into(),
        };
        let (next, copied, waiting) = apply(&state, feed, now).await.unwrap();
        assert!(waiting);
        assert_eq!(copied, 1);
        let a2 = "2026-11-29T04:09:00Z".parse::<jiff::Timestamp>().unwrap().as_millisecond();
        assert_eq!(next, format!("{a2}:"));
    }

    #[test]
    fn an_old_handoff_without_a_session_is_taken_as_it_is() {
        let now = "2026-11-29T05:00:00Z".parse::<jiff::Timestamp>().unwrap().as_millisecond();
        assert!(!still_starting(&wire("h1", None, "2026-11-29T04:00:00Z"), now));
        assert!(still_starting(&wire("h1", None, "2026-11-29T04:58:00Z"), now));
        assert!(!still_starting(&wire("h1", Some("s"), "2026-11-29T04:59:59Z"), now));
    }

    #[test]
    fn a_feed_page_parses() {
        let feed: Feed = serde_json::from_value(serde_json::json!({"handoffs": [{
            "id": "h00000000000000a1", "from": "infra@laptop", "fromSessionId": "s-old", "to": "infra@agents",
            "toSessionId": null, "note": null, "by": "infra@laptop", "createdAt": "2026-11-29T04:00:00Z"
        }], "nextCursor": "1795924800000:h00000000000000a1"}))
        .unwrap();
        assert_eq!(feed.handoffs[0].to, "infra@agents");
        assert!(feed.handoffs[0].to_session_id.is_none());
    }
}
