//! `/healthz/runners`: whether each runner is doing its job, for an uptime checker (Gatus). It fails (503) when a
//! runner isn't connected, hasn't read every mailbox lately, or has left an approved action waiting, and says which
//! and since when, so the alert explains itself. Times and node names only: no subjects, addresses or mail text.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use jiff::Timestamp;
use quietdeck_types::node::RunnerInfo;
use serde_json::{Value, json};

use crate::db::{Db, from_json, now_ms};
use crate::state::AppState;

/// A runner's last good mail check may be this old.
const MAIL_STALE_MS: i64 = 10 * 60_000;
/// An approved action may wait this long for its runner.
const ACTION_STALE_MS: i64 = 15 * 60_000;

/// The oldest release of an action still waiting for the runner of `agent` (node, tab), in epoch ms.
async fn oldest_waiting(db: &Db, node: &str, tab: &str) -> anyhow::Result<Option<i64>> {
    let row: (Option<i64>,) = sqlx::query_as(
        "SELECT MIN(d.released_at) FROM actions a \
         JOIN decisions d ON d.card_id = a.card_id AND d.card_version = a.card_version \
         JOIN cards c ON c.id = a.card_id AND c.version = a.card_version \
         WHERE a.status = 'released' AND d.released_at IS NOT NULL AND d.skipped IS NULL \
           AND d.handoff_reason IS NULL AND c.posted_node = ? AND c.posted_tab = ?",
    )
    .bind(node)
    .bind(tab)
    .fetch_one(db)
    .await?;
    Ok(row.0)
}

fn iso(ms: i64) -> String {
    Timestamp::from_millisecond(ms).map(|t| t.to_string()).unwrap_or_default()
}

/// Each runner's state and what's wrong with it, at `now` (epoch ms).
pub async fn report(state: &AppState, now: i64) -> anyhow::Result<(bool, Value)> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, runner_info FROM nodes WHERE runner_info IS NOT NULL AND revoked_at IS NULL ORDER BY name",
    )
    .fetch_all(&state.db)
    .await?;
    if rows.is_empty() {
        return Ok((false, json!({"ok": false, "problems": ["no node has its runner switched on"], "runners": []})));
    }
    let health = state.runner_health.lock().map(|h| h.clone()).unwrap_or_default();
    let mut all_ok = true;
    let mut runners = vec![];
    for (name, info) in rows {
        let info: RunnerInfo = from_json(&info)?;
        let (agent_node, agent_tab) = info.agent(&name);
        let mut problems: Vec<String> = vec![];
        let connected = state.hub.is_connected(&name).await;
        if !connected {
            problems.push(format!("{name} isn't connected to the deck"));
        }
        let mail_ok_at = health.get(&name).and_then(|(h, _)| h.mail_ok_at).map(|t| t.as_millisecond());
        match mail_ok_at {
            Some(at) if now - at <= MAIL_STALE_MS => {}
            Some(at) => {
                problems.push(format!("no complete mail check since {} ({} min)", iso(at), (now - at) / 60_000))
            }
            None => problems.push("no complete mail check reported since the deck started".into()),
        }
        // What went wrong in its last check: an account, the support inbox, the lease filters. Each clears on the
        // next check that works (28 Nov 2026: the lease filters failed for 11 minutes and nothing here said so).
        if let Some(e) = health.get(&name).and_then(|(h, _)| h.mail_error.clone()) {
            problems.push(format!("its last check failed: {e}"));
        }
        // Its wake-ups follow the person's presence: a watch that keeps failing leaves it on its fixed quiet hours.
        if let Some(p) = crate::presence::problem(now) {
            problems.push(p);
        }
        // The oldest approved action waiting on any of its agents' cards.
        let mut waiting: Option<i64> = None;
        for (n, t) in info.agents(&name) {
            if let Some(at) = oldest_waiting(&state.db, &n, &t).await? {
                waiting = Some(waiting.map_or(at, |w| w.min(at)));
            }
        }
        if let Some(at) = waiting.filter(|at| now - at > ACTION_STALE_MS) {
            problems.push(format!("an approved action has waited since {} ({} min)", iso(at), (now - at) / 60_000));
        }
        all_ok &= problems.is_empty();
        runners.push(json!({
            "node": name,
            "agent": format!("{agent_tab}@{agent_node}"),
            "agents": info.agents(&name).iter().map(|(n, t)| format!("{t}@{n}")).collect::<Vec<_>>(),
            "connected": connected,
            "mail_ok_at": mail_ok_at.map(iso),
            "oldest_waiting_action_since": waiting.map(iso),
            "problems": problems,
        }));
    }
    Ok((all_ok, json!({"ok": all_ok, "runners": runners})))
}

pub async fn runners(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    match report(&state, now_ms()).await {
        Ok((true, body)) => (StatusCode::OK, Json(body)),
        Ok((false, body)) => (StatusCode::SERVICE_UNAVAILABLE, Json(body)),
        Err(e) => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"ok": false, "problems": [e.to_string()]}))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quietdeck_types::node::RunnerHealth;

    #[tokio::test]
    async fn a_runner_that_stops_reading_mail_or_leaves_work_waiting_fails_and_says_why() {
        let state = AppState::for_tests().await;
        let (ok, body) = report(&state, now_ms()).await.unwrap();
        assert!(!ok && body["problems"][0].as_str().unwrap().contains("no node"), "{body}");

        sqlx::query("INSERT INTO nodes (name, token_hash, created_at) VALUES ('box-runner', 'h1', 0)")
            .execute(&state.db)
            .await
            .unwrap();
        let info =
            RunnerInfo { eligible: vec![], agent_tab: "triage@box".into(), accounts: vec![], agent_tabs: vec![] };
        crate::store::nodes::hello(&state.db, "box-runner", "test", Some(&info)).await.unwrap();
        let now = now_ms();
        let fresh = RunnerHealth { mail_ok_at: Timestamp::from_millisecond(now - 60_000).ok(), ..Default::default() };
        state.runner_health.lock().unwrap().insert("box-runner".into(), (fresh, Timestamp::now()));
        let (ok, body) = report(&state, now).await.unwrap();
        let problems = body["runners"][0]["problems"].as_array().unwrap().clone();
        assert!(!ok, "not connected in this test");
        assert_eq!(problems.len(), 1, "a fresh mail check is fine: {problems:?}");
        assert!(problems[0].as_str().unwrap().contains("isn't connected"));
        assert_eq!(body["runners"][0]["agent"], "triage@box");

        let stale =
            RunnerHealth { mail_ok_at: Timestamp::from_millisecond(now - 30 * 60_000).ok(), ..Default::default() };
        state.runner_health.lock().unwrap().insert("box-runner".into(), (stale, Timestamp::now()));
        let (_, body) = report(&state, now).await.unwrap();
        assert!(body["runners"][0]["problems"].to_string().contains("30 min"), "{body}");

        // A check that failed part-way (the lease filters here) is a problem too, though mail was read.
        let failing = RunnerHealth {
            mail_ok_at: Timestamp::from_millisecond(now - 60_000).ok(),
            mail_error: Some("Lease filters: no leases authority found".into()),
            ..Default::default()
        };
        state.runner_health.lock().unwrap().insert("box-runner".into(), (failing, Timestamp::now()));
        let (_, body) = report(&state, now).await.unwrap();
        assert!(body["runners"][0]["problems"].to_string().contains("Lease filters: no leases"), "{body}");

        // An approved action the runner hasn't run: fine at first, a problem after 15 minutes.
        let me = quietdeck_types::SessionRef { node: "box".into(), tab: "triage".into(), session_id: None };
        let input: quietdeck_types::card::CardInput = serde_json::from_value(json!({"account": "personal",
            "thread_key": "t1", "subject": "s", "summary": "s", "section": "actions", "reason": "r",
            "actions": [{"type": "archive", "account": "personal", "thread_id": "t1"}]}))
        .unwrap();
        let card = crate::store::cards::add(&state.db, &me, &input, quietdeck_types::card::CardState::Open, None)
            .await
            .unwrap()
            .card;
        let d = quietdeck_types::decision::CardDecision {
            card_id: card.id,
            version: card.version,
            kind: quietdeck_types::card::DecisionKind::Approve,
            fields: None,
            text: None,
        };
        let req = quietdeck_types::decision::ApproveAll { cards: vec![d], timeline: vec![], single: None };
        crate::store::decisions::approve_all(&state.db, &req, 0).await.unwrap();
        crate::store::decisions::release_due(&state.db).await.unwrap();
        let (_, body) = report(&state, now_ms()).await.unwrap();
        assert!(!body["runners"][0]["problems"].to_string().contains("approved action"), "{body}");
        let (_, body) = report(&state, now_ms() + 20 * 60_000).await.unwrap();
        assert!(body["runners"][0]["problems"].to_string().contains("approved action has waited"), "{body}");
    }
}
