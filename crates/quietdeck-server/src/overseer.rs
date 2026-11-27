//! The overseer watch: the deck wakes the agent that writes a Work document when one of the person's sessions needs a
//! look. A session that went idle (and stayed idle two minutes) or is blocked on a prompt is named once per change (a
//! new status time, and for an idle session, work since it was last named: done to idle isn't new work), a
//! few at a time in one message, and a session is named at most once every five minutes. A session that went idle more
//! than six hours ago (or got blocked more than three days ago) stops counting, so a new watch doesn't wake its agent
//! for everything that was already there. Left out: the overseer itself, the writers of other documents, the agents runners work for (the
//! triage agent), ignored sessions and parked ones, except when blocked on a prompt. Every three hours without
//! another wake-up it asks for a refresh (tasks due today, stale entries).
//!
//! `/healthz/overseer` fails while the watch has stopped ticking, or while the last wake-up couldn't be delivered for
//! ten minutes, so an uptime checker sees a watch that's up but no longer reaching its agent.

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use quietdeck_types::relay::DeliveryKind;
use serde_json::{Value, json};

use crate::db::{Db, now_ms};
use crate::state::AppState;

const TICK: Duration = Duration::from_secs(30);
/// An idle session is named once it has stayed idle this long.
const SETTLE_MS: i64 = 2 * 60 * 1000;
/// At most once per session in this window.
const PER_SESSION_MS: i64 = 5 * 60 * 1000;
/// Idle longer than this: no longer a change worth a look (the overseer's own rounds cover what was already there).
const STALE_IDLE_MS: i64 = 6 * 3600 * 1000;
/// Blocked longer than this: left alone too, though the Work tab still shows its prompt.
const STALE_MS: i64 = 3 * 24 * 3600 * 1000;
const REFRESH_MS: i64 = 3 * 3600 * 1000;
const MAX_NAMED: usize = 8;

pub async fn ticker(state: AppState) {
    loop {
        tokio::time::sleep(TICK).await;
        if let Err(e) = tick(&state, now_ms()).await {
            tracing::warn!(error = %e, "the overseer watch");
            let _ = record_error(&state.db, &e.to_string()).await;
        }
    }
}

/// A session the overseer should look at.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub node: String,
    pub session_id: String,
    pub tab: String,
    pub status: String,
    pub status_changed_at: i64,
}

/// One pass: for each Work document with a writer, the changes it hasn't been told about, in one message.
pub async fn tick(state: &AppState, now: i64) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO overseer_watch (id, ticked_at) VALUES (1, ?) ON CONFLICT (id) DO UPDATE SET ticked_at = excluded.ticked_at",
    )
    .bind(now)
    .execute(&state.db)
    .await?;
    // Seen working since it was named: its next idle is new work.
    sqlx::query(
        "UPDATE overseer_told SET worked = 1 WHERE worked = 0 AND EXISTS (SELECT 1 FROM sessions s \
         WHERE s.node = overseer_told.node AND s.session_id = overseer_told.session_id AND s.status = 'working')",
    )
    .execute(&state.db)
    .await?;
    let writers: Vec<(String, String)> = sqlx::query_as(
        "SELECT writer_node, writer_tab FROM documents WHERE kind = 'work' AND removed_at IS NULL \
         AND writer_node IS NOT NULL AND writer_tab IS NOT NULL",
    )
    .fetch_all(&state.db)
    .await?;
    for (node, tab) in writers {
        let changes = changes_for(state, &node, &tab, now).await?;
        let to = format!("{tab}@{node}");
        let (refresh_at,): (Option<i64>,) =
            sqlx::query_as("SELECT refresh_at FROM overseer_watch WHERE id = 1").fetch_one(&state.db).await?;
        let body = if !changes.is_empty() {
            message(&changes, now)
        } else if refresh_at.is_none_or(|at| now - at > REFRESH_MS) {
            "From the deck's overseer watch: a periodic refresh. Bring the Work document up to date: today's tasks \
             (due today or started), entries that are done or no longer relevant, and any snoozed session that's back \
             (`quietdeck marks`). Then end your turn."
                .to_string()
        } else {
            continue;
        };
        match crate::relay::send(&state.db, &state.hub, &to, DeliveryKind::Deck, body).await {
            Ok(_) => {
                for c in &changes {
                    sqlx::query(
                        "INSERT INTO overseer_told (node, session_id, status, status_changed_at, told_at) \
                         VALUES (?, ?, ?, ?, ?) ON CONFLICT (node, session_id) DO UPDATE SET status = excluded.status, \
                         status_changed_at = excluded.status_changed_at, told_at = excluded.told_at, worked = 0",
                    )
                    .bind(&c.node)
                    .bind(&c.session_id)
                    .bind(&c.status)
                    .bind(c.status_changed_at)
                    .bind(now)
                    .execute(&state.db)
                    .await?;
                }
                sqlx::query(
                    "UPDATE overseer_watch SET told_at = ?, refresh_at = ?, error = NULL, error_at = NULL WHERE id = 1",
                )
                .bind(now)
                .bind(now)
                .execute(&state.db)
                .await?;
            }
            Err(e) => {
                tracing::info!(%to, error = %e, "the overseer didn't get its wake-up");
                record_error(&state.db, &format!("{to}: {e}")).await?;
            }
        }
    }
    Ok(())
}

async fn record_error(db: &Db, error: &str) -> anyhow::Result<()> {
    // The first failure's time stays until a delivery works, so the health check sees how long it's been failing.
    sqlx::query(
        "INSERT INTO overseer_watch (id, ticked_at, error, error_at) VALUES (1, ?1, ?2, ?1) ON CONFLICT (id) DO \
         UPDATE SET error = ?2, error_at = COALESCE(overseer_watch.error_at, ?1)",
    )
    .bind(now_ms())
    .bind(error)
    .execute(db)
    .await?;
    Ok(())
}

/// The sessions the writer `tab@node` should hear about now.
pub async fn changes_for(state: &AppState, node: &str, tab: &str, now: i64) -> anyhow::Result<Vec<Change>> {
    let rows: Vec<(String, String, String, String, i64)> = sqlx::query_as(
        "SELECT s.node, s.session_id, s.tab, s.status, s.status_changed_at FROM sessions s \
         WHERE s.running = 1 AND s.tab IS NOT NULL AND s.closed_at IS NULL \
           AND ((s.status IN ('idle', 'done') AND s.status_changed_at <= ?1 - ?2 AND s.status_changed_at > ?1 - ?7) \
                OR (s.status = 'blocked' AND s.status_changed_at > ?1 - ?3)) \
           AND NOT (s.node = ?4 AND s.tab = ?5) \
           AND NOT EXISTS (SELECT 1 FROM documents d WHERE d.removed_at IS NULL AND d.writer_node = s.node \
                           AND d.writer_tab = s.tab) \
           AND (s.status = 'blocked' OR NOT EXISTS (SELECT 1 FROM session_marks m WHERE m.node = s.node \
                AND m.tab = s.tab AND (m.until IS NULL OR m.until > ?1))) \
           AND NOT EXISTS (SELECT 1 FROM overseer_told t WHERE t.node = s.node AND t.session_id = s.session_id \
                AND (t.status_changed_at = s.status_changed_at OR t.told_at > ?1 - ?6 \
                     OR (s.status IN ('idle', 'done') AND t.status IN ('idle', 'done') AND t.worked = 0))) \
         ORDER BY s.status = 'blocked' DESC, s.status_changed_at",
    )
    .bind(now)
    .bind(SETTLE_MS)
    .bind(STALE_MS)
    .bind(node)
    .bind(tab)
    .bind(PER_SESSION_MS)
    .bind(STALE_IDLE_MS)
    .fetch_all(&state.db)
    .await?;
    // The agents runners work for (the triage agent) aren't the overseer's to watch.
    let runner_agents = runner_agents(&state.db).await?;
    Ok(rows
        .into_iter()
        .filter(|(n, _, t, _, _)| !runner_agents.iter().any(|(rn, rt)| rn == n && rt == t))
        .take(MAX_NAMED)
        .map(|(node, session_id, tab, status, status_changed_at)| Change {
            node,
            session_id,
            tab,
            status,
            status_changed_at,
        })
        .collect())
}

async fn runner_agents(db: &Db) -> anyhow::Result<Vec<(String, String)>> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT name, runner_info FROM nodes WHERE runner_info IS NOT NULL AND revoked_at IS NULL")
            .fetch_all(db)
            .await?;
    let mut out = vec![];
    for (name, info) in rows {
        if let Ok(info) = serde_json::from_str::<quietdeck_types::node::RunnerInfo>(&info) {
            out.push(info.agent(&name));
        }
    }
    Ok(out)
}

fn message(changes: &[Change], now: i64) -> String {
    let lines: Vec<String> = changes
        .iter()
        .map(|c| {
            let mins = (now - c.status_changed_at).max(0) / 60_000;
            match c.status.as_str() {
                "blocked" => format!("- `{}@{}` is blocked on a prompt (for {mins} min)", c.tab, c.node),
                _ => format!("- `{}@{}` went idle {mins} min ago", c.tab, c.node),
            }
        })
        .collect();
    format!(
        "From the deck's overseer watch: {} to look at.\n{}\n\nFor each, read Fleetlayer's summary first, then only \
         what's new in its transcript if you need detail, and skip one that still has a CI watcher or another \
         background task running. Update the Work document (`quietdeck report show --doc work`, then `report set`).",
        if changes.len() == 1 { "a session" } else { "sessions" },
        lines.join("\n")
    )
}

/// `/healthz/overseer`: 200 while the watch ticks and its wake-ups are delivered; 503 otherwise, saying why. 200 with
/// `"watching": false` on a deck with no Work document.
pub async fn health(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    match report(&state.db, now_ms()).await {
        Ok((ok, body)) => (if ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE }, Json(body)),
        Err(e) => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"ok": false, "problems": [e.to_string()]}))),
    }
}

pub async fn report(db: &Db, now: i64) -> anyhow::Result<(bool, Value)> {
    let (docs,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM documents WHERE kind = 'work' AND removed_at IS NULL AND writer_node IS NOT NULL",
    )
    .fetch_one(db)
    .await?;
    type Row = (i64, Option<i64>, Option<String>, Option<i64>);
    let row: Option<Row> =
        sqlx::query_as("SELECT ticked_at, told_at, error, error_at FROM overseer_watch WHERE id = 1")
            .fetch_optional(db)
            .await?;
    let iso = |ms: i64| jiff::Timestamp::from_millisecond(ms).map(|t| t.to_string()).unwrap_or_default();
    let mut problems = vec![];
    if docs > 0 {
        match row.as_ref().map(|r| r.0) {
            Some(at) if now - at <= 3 * TICK.as_millis() as i64 => {}
            Some(at) => problems.push(format!("the watch hasn't ticked since {}", iso(at))),
            None => problems.push("the watch hasn't ticked since the deck started".into()),
        }
        if let Some((_, _, Some(error), Some(since))) = &row
            && now - since > 10 * 60 * 1000
        {
            problems.push(format!("wake-ups have failed since {}: {error}", iso(*since)));
        }
    }
    let ok = problems.is_empty();
    Ok((
        ok,
        json!({
            "ok": ok,
            "watching": docs > 0,
            "ticked_at": row.as_ref().map(|r| iso(r.0)),
            "told_at": row.as_ref().and_then(|r| r.1).map(iso),
            "error": row.as_ref().and_then(|r| r.2.clone()),
            "problems": problems,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use quietdeck_types::document::{DocumentAdd, DocumentKind, MarkKind, SessionMarkSet};

    async fn session(state: &AppState, id: &str, tab: &str, status: &str, changed: i64) {
        sqlx::query(
            "INSERT OR REPLACE INTO sessions (node, session_id, agent, tab, status, status_changed_at, running, pushed_at) \
             VALUES ('laptop', ?, 'claude', ?, ?, ?, 1, ?)",
        )
        .bind(id)
        .bind(tab)
        .bind(status)
        .bind(changed)
        .bind(changed)
        .execute(&state.db)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn settled_idle_and_blocked_sessions_are_named_once_and_marks_are_respected() {
        let state = AppState::for_tests().await;
        let now = now_ms();
        crate::documents::add(
            &state,
            DocumentAdd {
                slug: "work".into(),
                name: "Work".into(),
                tab: "Work".into(),
                kind: DocumentKind::Work,
                writer_node: Some("laptop".into()),
                writer_tab: Some("overseer".into()),
                position: None,
            },
        )
        .await
        .unwrap();
        session(&state, "s1", "search", "idle", now - 5 * 60_000).await;
        session(&state, "s2", "billing", "idle", now - 30_000).await; // not settled yet
        session(&state, "s3", "onboarding", "blocked", now - 10_000).await;
        session(&state, "s4", "old", "idle", now - 7 * 3_600_000).await; // idle for hours
        session(&state, "s5", "overseer", "idle", now - 5 * 60_000).await; // itself
        session(&state, "s6", "sso", "idle", now - 5 * 60_000).await; // parked
        session(&state, "s7", "talira worker", "blocked", now - 5 * 60_000).await; // ignored, but blocked
        let mark =
            |tab: &str, mark| SessionMarkSet { node: "laptop".into(), tab: tab.into(), mark, until: None, note: None };
        crate::documents::mark_set(&state, mark("sso", MarkKind::Parked)).await.unwrap();
        crate::documents::mark_set(&state, mark("talira worker", MarkKind::Ignored)).await.unwrap();
        let tabs = |c: Vec<Change>| c.into_iter().map(|c| c.tab).collect::<Vec<_>>();
        let first = changes_for(&state, "laptop", "overseer", now).await.unwrap();
        assert_eq!(tabs(first.clone()), vec!["talira worker", "onboarding", "search"]);
        assert!(message(&first, now).contains("`onboarding@laptop` is blocked on a prompt"));
        // Told: not again for the same change.
        for c in &first {
            sqlx::query("INSERT INTO overseer_told (node, session_id, status, status_changed_at, told_at) VALUES (?, ?, ?, ?, ?)")
                .bind(&c.node)
                .bind(&c.session_id)
                .bind(&c.status)
                .bind(c.status_changed_at)
                .bind(now - 10 * 60_000)
                .execute(&state.db)
                .await
                .unwrap();
        }
        assert!(changes_for(&state, "laptop", "overseer", now).await.unwrap().is_empty());
        // idle to done at the same moment (herdr reports both) isn't a new change.
        sqlx::query("UPDATE sessions SET status = 'done' WHERE session_id = 's1'").execute(&state.db).await.unwrap();
        assert!(changes_for(&state, "laptop", "overseer", now).await.unwrap().is_empty());
        // done, then idle a few minutes later (seen in herdr): no new work, not named.
        session(&state, "s1", "search", "idle", now - 3 * 60_000).await;
        assert!(changes_for(&state, "laptop", "overseer", now).await.unwrap().is_empty());
        // It works again (a tick sees it), then goes idle: a new change.
        session(&state, "s1", "search", "working", now - 2 * 60_000).await;
        sqlx::query("UPDATE overseer_told SET worked = 1 WHERE session_id = 's1'").execute(&state.db).await.unwrap();
        session(&state, "s1", "search", "idle", now - 3 * 60_000 + 1).await;
        assert_eq!(tabs(changes_for(&state, "laptop", "overseer", now).await.unwrap()), vec!["search"]);
    }

    #[tokio::test]
    async fn the_health_check_fails_when_wake_ups_keep_failing() {
        let state = AppState::for_tests().await;
        let now = now_ms();
        assert!(report(&state.db, now).await.unwrap().0, "nothing to watch is fine");
        sqlx::query("INSERT INTO documents (slug, name, tab, kind, writer_node, writer_tab, created_at) VALUES ('work', 'Work', 'Work', 'work', 'laptop', 'overseer', 0)")
            .execute(&state.db)
            .await
            .unwrap();
        assert!(!report(&state.db, now).await.unwrap().0, "never ticked");
        tick(&state, now).await.unwrap();
        // The overseer isn't running, so the refresh couldn't be delivered: an error, but not yet for ten minutes.
        let (ok, body) = report(&state.db, now).await.unwrap();
        assert!(ok, "{body}");
        assert!(body["error"].as_str().is_some());
        sqlx::query("UPDATE overseer_watch SET error_at = ?").bind(now - 11 * 60_000).execute(&state.db).await.unwrap();
        assert!(!report(&state.db, now).await.unwrap().0);
    }
}
