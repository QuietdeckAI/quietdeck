//! Pings (3.8) and thread watches. An agent pings only for what can't wait a few hours: at first, replies on watched
//! threads and urgent internal events. Each ping goes to every device by web push and is recorded; the weekly budget
//! is shown against them on the Rules screen and suppresses nothing. "Don't ping for this again" goes back to the
//! agent that sent it, which picks it up with `quietdeck pings feedback` and turns it into a rule of its own.

use garde::Validate;
use quietdeck_types::Action;
use quietdeck_types::action::SessionRef;
use quietdeck_types::api::Changed;
use quietdeck_types::timeline::{Ping, PingFeedback, PingRequest, PingsView, ThreadWatch, WatchAdd};
use serde_json::json;
use sqlx::SqliteConnection;

use crate::db::{Db, now_ms, ts, ts_opt};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

const WEEK_MS: i64 = 7 * 24 * 3600 * 1000;

#[derive(sqlx::FromRow)]
struct PingRow {
    id: i64,
    sent_node: String,
    sent_tab: String,
    sent_session_id: Option<String>,
    title: String,
    body: String,
    link: Option<String>,
    reason: String,
    account: Option<String>,
    sent_at: i64,
    result: Option<String>,
    feedback: Option<String>,
    feedback_at: Option<i64>,
    feedback_picked_up_at: Option<i64>,
}

const PING_COLUMNS: &str = "id, sent_node, sent_tab, sent_session_id, title, body, link, reason, account, sent_at, \
    result, feedback, feedback_at, feedback_picked_up_at";

impl From<PingRow> for Ping {
    fn from(r: PingRow) -> Ping {
        Ping {
            id: r.id,
            sent_by: SessionRef { node: r.sent_node, tab: r.sent_tab, session_id: r.sent_session_id },
            title: r.title,
            body: r.body,
            link: r.link,
            reason: r.reason,
            account: r.account,
            sent_at: ts(r.sent_at),
            result: r.result,
            feedback: r.feedback,
            feedback_at: ts_opt(r.feedback_at),
            feedback_picked_up_at: ts_opt(r.feedback_picked_up_at),
        }
    }
}

async fn ping_by_id(db: &Db, id: i64) -> AppResult<Ping> {
    let row: Option<PingRow> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {PING_COLUMNS} FROM pings WHERE id = ?")))
            .bind(id)
            .fetch_optional(db)
            .await?;
    row.map(Ping::from).ok_or_else(|| AppError::NotFound(format!("no ping {id}")))
}

/// `quietdeck ping`: record it, push it to every device, and say how many took it.
pub async fn send(state: &AppState, me: &SessionRef, req: PingRequest) -> AppResult<Ping> {
    req.validate().map_err(|e| AppError::BadRequest(e.to_string()))?;
    if let Some(a) = &req.account
        && !state.config.knows_account(a)
    {
        return Err(AppError::BadRequest(format!(
            "unknown account {a:?}: deck.yaml lists {}",
            state.config.accounts.iter().map(|a| a.id.as_str()).collect::<Vec<_>>().join(", ")
        )));
    }
    let id: (i64,) = sqlx::query_as(
        "INSERT INTO pings (sent_node, sent_tab, sent_session_id, title, body, link, reason, account, thread_key, \
         sent_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(&me.node)
    .bind(&me.tab)
    .bind(&me.session_id)
    .bind(&req.title)
    .bind(&req.body)
    .bind(&req.link)
    .bind(&req.reason)
    .bind(&req.account)
    .bind(&req.thread_key)
    .bind(now_ms())
    .fetch_one(&state.db)
    .await?;
    let id = id.0;
    let result = match &state.pusher {
        None => "Not pushed: web push is off on the deck (no VAPID keys).".to_string(),
        Some(pusher) => {
            let message = crate::push::Message {
                title: req.title.clone(),
                body: req.body.clone(),
                navigate: format!("{}/rules?ping={id}", state.public_url.trim_end_matches('/')),
                topic: req.thread_key.as_ref().map(|t| format!("{}:{t}", req.account.as_deref().unwrap_or(""))),
            };
            match pusher.send_all(&state.db, &message).await {
                Ok(s) if s.devices == 0 => "Not pushed: no device takes pushes yet (Settings, on the phone).".into(),
                Ok(s) => format!(
                    "Pushed to {} of {} device{}.",
                    s.delivered,
                    s.devices,
                    if s.devices == 1 { "" } else { "s" }
                ),
                Err(e) => {
                    tracing::error!(error = %e, "sending a ping");
                    format!("Not pushed: {e}")
                }
            }
        }
    };
    sqlx::query("UPDATE pings SET result = ? WHERE id = ?").bind(&result).bind(id).execute(&state.db).await?;
    state.changed(Changed::Pings);
    ping_by_id(&state.db, id).await
}

/// The Rules screen's pings: the newest 100, and how many of the last seven days' count against the budget.
pub async fn view(state: &AppState) -> AppResult<PingsView> {
    let rows: Vec<PingRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {PING_COLUMNS} FROM pings ORDER BY sent_at DESC, id DESC LIMIT 100"
    )))
    .fetch_all(&state.db)
    .await?;
    let last_week: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM pings WHERE sent_at > ?")
        .bind(now_ms() - WEEK_MS)
        .fetch_one(&state.db)
        .await?;
    Ok(PingsView {
        weekly_budget: state.config.pings.weekly_budget,
        last_week: last_week.0 as u32,
        pings: rows.into_iter().map(Ping::from).collect(),
    })
}

/// "Don't ping for this again". Changing the note later hands it to the agent again.
pub async fn feedback(state: &AppState, id: i64, f: PingFeedback) -> AppResult<Ping> {
    f.validate().map_err(|e| AppError::BadRequest(e.to_string()))?;
    let note = f.note.map(|n| n.trim().to_string()).unwrap_or_default();
    let r = sqlx::query("UPDATE pings SET feedback = ?, feedback_at = ?, feedback_picked_up_at = NULL WHERE id = ?")
        .bind(&note)
        .bind(now_ms())
        .bind(id)
        .execute(&state.db)
        .await?;
    if r.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("no ping {id}")));
    }
    state.changed(Changed::Pings);
    ping_by_id(&state.db, id).await
}

/// `quietdeck pings feedback`: the feedback on this session's tab's pings that it hasn't picked up, marked picked up.
pub async fn take_feedback(state: &AppState, me: &SessionRef) -> AppResult<Vec<Ping>> {
    let now = now_ms();
    let rows: Vec<PingRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "UPDATE pings SET feedback_picked_up_at = ? WHERE sent_node = ? AND sent_tab = ? \
         AND feedback_at IS NOT NULL AND feedback_picked_up_at IS NULL RETURNING {PING_COLUMNS}"
    )))
    .bind(now)
    .bind(&me.node)
    .bind(&me.tab)
    .fetch_all(&state.db)
    .await?;
    if !rows.is_empty() {
        state.changed(Changed::Pings);
    }
    let mut pings: Vec<Ping> = rows.into_iter().map(Ping::from).collect();
    pings.sort_by_key(|p| p.id);
    Ok(pings)
}

#[derive(sqlx::FromRow)]
struct WatchRow {
    id: i64,
    account: String,
    thread_key: String,
    subject: Option<String>,
    created_at: i64,
    active: bool,
}

impl From<WatchRow> for ThreadWatch {
    fn from(r: WatchRow) -> ThreadWatch {
        ThreadWatch {
            id: r.id,
            account: r.account,
            thread_key: r.thread_key,
            subject: r.subject,
            created_at: ts(r.created_at),
            active: r.active,
        }
    }
}

/// Watched threads, the newest first: the active ones, or with `all` the stopped ones too (up to 200).
pub async fn watches(db: &Db, all: bool) -> AppResult<Vec<ThreadWatch>> {
    let rows: Vec<WatchRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT id, account, thread_key, subject, created_at, active FROM thread_watches {} \
         ORDER BY created_at DESC, id DESC LIMIT 200",
        if all { "" } else { "WHERE active = 1" }
    )))
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(ThreadWatch::from).collect())
}

async fn upsert_watch(
    conn: &mut SqliteConnection,
    account: &str,
    thread_key: &str,
    subject: Option<&str>,
) -> anyhow::Result<i64> {
    let id: (i64,) = sqlx::query_as(
        "INSERT INTO thread_watches (account, thread_key, subject, created_at) VALUES (?, ?, ?, ?) \
         ON CONFLICT (owner_type, owner_id, account, thread_key) DO UPDATE SET active = 1, \
         subject = COALESCE(excluded.subject, subject) RETURNING id",
    )
    .bind(account)
    .bind(thread_key)
    .bind(subject)
    .bind(now_ms())
    .fetch_one(&mut *conn)
    .await?;
    Ok(id.0)
}

/// Watch thread, from a card's menu.
pub async fn watch(state: &AppState, w: WatchAdd) -> AppResult<ThreadWatch> {
    w.validate().map_err(|e| AppError::BadRequest(e.to_string()))?;
    if !state.config.knows_account(&w.account) {
        return Err(AppError::BadRequest(format!("unknown account {:?}", w.account)));
    }
    let mut conn = state.db.acquire().await?;
    let id = upsert_watch(&mut conn, &w.account, &w.thread_key, w.subject.as_deref()).await?;
    drop(conn);
    state.changed(Changed::Pings);
    let rows = watches(&state.db, true).await?;
    rows.into_iter().find(|x| x.id == id).ok_or_else(|| AppError::NotFound(format!("no watch {id}")))
}

/// Stop watching, from the Rules screen.
pub async fn unwatch(state: &AppState, id: i64) -> AppResult<()> {
    let r = sqlx::query("UPDATE thread_watches SET active = 0 WHERE id = ?").bind(id).execute(&state.db).await?;
    if r.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("no watch {id}")));
    }
    state.changed(Changed::Pings);
    Ok(())
}

/// Watch actions are the deck's own to carry out: a released one starts or stops the watch at once. Returns how many
/// actions it finished.
pub async fn apply_released(conn: &mut SqliteConnection, card_id: i64, version: i64) -> anyhow::Result<u64> {
    let actions: Vec<(i64, String, Option<String>)> = sqlx::query_as(
        "SELECT a.id, a.payload, json_extract(c.input, '$.subject') FROM actions a JOIN cards c ON c.id = a.card_id \
         WHERE a.card_id = ? AND a.card_version = ? AND a.type = 'watch' AND a.status = 'released'",
    )
    .bind(card_id)
    .bind(version)
    .fetch_all(&mut *conn)
    .await?;
    let now = now_ms();
    for (action_id, payload, subject) in &actions {
        let Action::Watch { account, thread_id, on } = serde_json::from_str(payload)? else {
            anyhow::bail!("action {action_id} is typed watch but isn't one");
        };
        if on {
            upsert_watch(conn, &account, &thread_id, subject.as_deref()).await?;
        } else {
            sqlx::query("UPDATE thread_watches SET active = 0 WHERE account = ? AND thread_key = ?")
                .bind(&account)
                .bind(&thread_id)
                .execute(&mut *conn)
                .await?;
        }
        sqlx::query("UPDATE actions SET status = 'done', outcome = ?, finished_at = ? WHERE id = ?")
            .bind(json!({ "watching": on }).to_string())
            .bind(now)
            .bind(action_id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(actions.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me(tab: &str) -> SessionRef {
        SessionRef { node: "laptop".into(), tab: tab.into(), session_id: Some("s1".into()) }
    }

    fn ping(title: &str) -> PingRequest {
        PingRequest {
            title: title.into(),
            body: "They replied".into(),
            link: None,
            reason: "a reply on a watched thread".into(),
            account: None,
            thread_key: Some("t1".into()),
        }
    }

    #[tokio::test]
    async fn a_ping_without_push_is_recorded_and_says_why_it_wasnt_pushed() {
        let state = AppState::for_tests().await;
        let p = send(&state, &me("email triage"), ping("Bank replied")).await.unwrap();
        assert_eq!(p.sent_by.tab, "email triage");
        assert!(p.result.unwrap().contains("web push is off"));
        let v = view(&state).await.unwrap();
        assert_eq!((v.weekly_budget, v.last_week, v.pings.len()), (10, 1, 1));
    }

    #[tokio::test]
    async fn the_budget_counts_the_last_seven_days_and_suppresses_nothing() {
        let state = AppState::for_tests().await;
        for i in 0..12 {
            send(&state, &me("email triage"), ping(&format!("p{i}"))).await.unwrap();
        }
        sqlx::query("UPDATE pings SET sent_at = ? WHERE title = 'p0'")
            .bind(now_ms() - WEEK_MS - 1000)
            .execute(&state.db)
            .await
            .unwrap();
        let v = view(&state).await.unwrap();
        assert_eq!(v.last_week, 11, "over the budget of 10, and every ping was still sent");
        assert_eq!(v.pings.len(), 12);
    }

    #[tokio::test]
    async fn feedback_goes_back_once_to_the_tab_that_pinged() {
        let state = AppState::for_tests().await;
        let a = send(&state, &me("email triage"), ping("a")).await.unwrap();
        let b = send(&state, &me("sre"), ping("b")).await.unwrap();
        feedback(&state, a.id, PingFeedback { note: Some(" Not for newsletters ".into()) }).await.unwrap();
        feedback(&state, b.id, PingFeedback::default()).await.unwrap();
        let mine = take_feedback(&state, &me("email triage")).await.unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].feedback.as_deref(), Some("Not for newsletters"));
        assert!(mine[0].feedback_picked_up_at.is_some());
        assert!(take_feedback(&state, &me("email triage")).await.unwrap().is_empty(), "picked up once");
        assert_eq!(take_feedback(&state, &me("sre")).await.unwrap()[0].feedback.as_deref(), Some(""));
        // A changed note goes back again.
        feedback(&state, a.id, PingFeedback { note: Some("Not for any bank".into()) }).await.unwrap();
        assert_eq!(take_feedback(&state, &me("email triage")).await.unwrap().len(), 1);
        assert!(feedback(&state, 999, PingFeedback::default()).await.is_err());
    }

    #[tokio::test]
    async fn watching_twice_keeps_one_watch_and_stopping_hides_it() {
        let state = AppState::for_tests().await;
        let w = WatchAdd { account: "personal".into(), thread_key: "t1".into(), subject: Some("Visa".into()) };
        let a = watch(&state, w.clone()).await.unwrap();
        let b = watch(&state, WatchAdd { subject: None, ..w }).await.unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(b.subject.as_deref(), Some("Visa"));
        unwatch(&state, a.id).await.unwrap();
        assert!(watches(&state.db, false).await.unwrap().is_empty());
        assert!(!watches(&state.db, true).await.unwrap()[0].active);
    }
}
