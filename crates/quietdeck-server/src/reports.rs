//! The triage report (quietdeck_types::report): the agent keeps one living document per round, the Inbox shows it,
//! and the person's answers and chat go straight back to the agent that wrote it. Answers are kept so the Inbox can
//! show them and an agent that was away can take them; chat is only passed on.

use garde::Validate;
use quietdeck_types::action::SessionRef;
use quietdeck_types::api::Changed;
use quietdeck_types::node::Delivered;
use quietdeck_types::relay::{DeliveryError, DeliveryKind};
use quietdeck_types::report::{
    AnswerKind, Report, ReportAnswer, ReportAnswerAdd, ReportChat, ReportSet, default_cards,
};

use crate::db::{Db, now_ms, ts};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(sqlx::FromRow)]
struct ReportRow {
    id: i64,
    version: i64,
    markdown: String,
    posted_node: String,
    posted_tab: String,
    posted_session_id: Option<String>,
    created_at: i64,
    updated_at: i64,
}

#[derive(sqlx::FromRow)]
struct AnswerRow {
    id: i64,
    report_version: i64,
    control: String,
    kind: String,
    about: String,
    value: String,
    answered_at: i64,
    picked_up_at: Option<i64>,
}

impl From<AnswerRow> for ReportAnswer {
    fn from(r: AnswerRow) -> ReportAnswer {
        ReportAnswer {
            id: r.id,
            report_version: r.report_version,
            control: r.control,
            kind: if r.kind == "change" { AnswerKind::Change } else { AnswerKind::Choice },
            about: r.about,
            value: r.value,
            answered_at: ts(r.answered_at),
            picked_up: r.picked_up_at.is_some(),
        }
    }
}

const KIND_CHANGE: &str = "change";
const KIND_CHOICE: &str = "choice";

fn kind_str(k: AnswerKind) -> &'static str {
    match k {
        AnswerKind::Change => KIND_CHANGE,
        AnswerKind::Choice => KIND_CHOICE,
    }
}

async fn open_row(db: &Db) -> AppResult<Option<ReportRow>> {
    Ok(sqlx::query_as(
        "SELECT id, version, markdown, posted_node, posted_tab, posted_session_id, created_at, updated_at \
         FROM reports WHERE closed_at IS NULL ORDER BY id DESC LIMIT 1",
    )
    .fetch_optional(db)
    .await?)
}

/// The report the Inbox shows, with its answers.
pub async fn current(db: &Db) -> AppResult<Option<Report>> {
    let Some(r) = open_row(db).await? else { return Ok(None) };
    let answers: Vec<AnswerRow> = sqlx::query_as(
        "SELECT id, report_version, control, kind, about, value, answered_at, picked_up_at \
         FROM report_answers WHERE report_id = ? ORDER BY id",
    )
    .bind(r.id)
    .fetch_all(db)
    .await?;
    let (named, _) = default_cards(&r.markdown);
    let open: Vec<(i64,)> = sqlx::query_as(
        "SELECT id FROM cards WHERE state IN ('open', 'handed_off', 'failed') AND section != 'sent' ORDER BY id",
    )
    .fetch_all(db)
    .await?;
    let cards_not_in_report = open.into_iter().map(|(id,)| id).filter(|id| !named.contains(id)).collect();
    Ok(Some(Report {
        id: r.id,
        version: r.version,
        markdown: r.markdown,
        posted_by: SessionRef { node: r.posted_node, tab: r.posted_tab, session_id: r.posted_session_id },
        created_at: ts(r.created_at),
        updated_at: ts(r.updated_at),
        answers: answers.into_iter().map(ReportAnswer::from).collect(),
        cards_not_in_report,
    }))
}

/// `quietdeck report set`: replace the report's text, from the version the agent last saw.
pub async fn set(state: &AppState, me: &SessionRef, req: ReportSet) -> AppResult<Report> {
    req.validate().map_err(|e| AppError::BadRequest(e.to_string()))?;
    let (ids, mut problems) = default_cards(&req.markdown);
    if !ids.is_empty() {
        let list = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let found: Vec<(i64,)> =
            sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT id FROM cards WHERE id IN ({list})")))
                .fetch_all(&state.db)
                .await?;
        for id in &ids {
            if !found.iter().any(|f| f.0 == *id) {
                problems.push(format!("no card c_{id}"));
            }
        }
    }
    if !problems.is_empty() {
        return Err(AppError::BadRequest(format!("the report's <Default> tags: {}", problems.join("; "))));
    }
    let now = now_ms();
    let mut tx = state.db.begin().await?;
    let open: Option<(i64, i64)> =
        sqlx::query_as("SELECT id, version FROM reports WHERE closed_at IS NULL ORDER BY id DESC LIMIT 1")
            .fetch_optional(&mut *tx)
            .await?;
    match (open, req.new) {
        (Some((id, _)), true) => {
            sqlx::query("UPDATE reports SET closed_at = ? WHERE id = ?").bind(now).bind(id).execute(&mut *tx).await?;
            insert(&mut tx, me, &req.markdown, now).await?;
        }
        (None, _) => insert(&mut tx, me, &req.markdown, now).await?,
        (Some((id, version)), false) => {
            if req.base != Some(version) {
                return Err(AppError::Conflict(format!(
                    "the report is at version {version}{}: run `quietdeck report show`, make your edit to that text \
                     and send it with --base {version}",
                    req.base.map(|b| format!(", not {b}")).unwrap_or_default()
                )));
            }
            sqlx::query(
                "UPDATE reports SET markdown = ?, version = version + 1, updated_at = ?, posted_node = ?, \
                 posted_tab = ?, posted_session_id = ? WHERE id = ?",
            )
            .bind(&req.markdown)
            .bind(now)
            .bind(&me.node)
            .bind(&me.tab)
            .bind(&me.session_id)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    state.changed(Changed::Cards);
    current(&state.db).await?.ok_or_else(|| AppError::Unavailable("the report vanished".into()))
}

async fn insert(tx: &mut sqlx::SqliteConnection, me: &SessionRef, markdown: &str, now: i64) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO reports (markdown, posted_node, posted_tab, posted_session_id, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(markdown)
    .bind(&me.node)
    .bind(&me.tab)
    .bind(&me.session_id)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// An answer from the Inbox: kept, and sent to the agent at once when its session is running.
pub async fn answer(state: &AppState, a: ReportAnswerAdd) -> AppResult<Report> {
    a.validate().map_err(|e| AppError::BadRequest(e.to_string()))?;
    let Some(r) = open_row(&state.db).await? else {
        return Err(AppError::NotFound("there's no report to answer".into()));
    };
    if r.id != a.report_id {
        return Err(AppError::Conflict("the agent has started a new report: answer on that one".into()));
    }
    let value = a.value.trim();
    if value.is_empty() {
        return Err(AppError::BadRequest("the answer is empty".into()));
    }
    let id: (i64,) = sqlx::query_as(
        "INSERT INTO report_answers (report_id, report_version, control, kind, about, value, answered_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(r.id)
    .bind(a.report_version)
    .bind(&a.control)
    .bind(kind_str(a.kind))
    .bind(a.about.trim())
    .bind(value)
    .bind(now_ms())
    .fetch_one(&state.db)
    .await?;
    let to = SessionRef { node: r.posted_node, tab: r.posted_tab, session_id: None };
    let body = match a.kind {
        AnswerKind::Choice => format!(
            "On the report (version {}), \"{}\" [{}]: {value}\n\nUpdate the report: follow up on the answer there.",
            a.report_version,
            a.about.trim(),
            a.control
        ),
        AnswerKind::Change => format!(
            "On the report (version {}), instead of the default \"{}\" [{}]: {value}\n\nApprove All leaves those \
             cards to you. Do what this says, or propose it in the report.",
            a.report_version,
            a.about.trim(),
            a.control
        ),
    };
    match crate::relay::send(&state.db, &state.hub, &to.address(), DeliveryKind::Deck, body).await {
        Ok(_) => {
            sqlx::query("UPDATE report_answers SET picked_up_at = ? WHERE id = ?")
                .bind(now_ms())
                .bind(id.0)
                .execute(&state.db)
                .await?;
        }
        Err(e) => {
            tracing::info!(to = %to.address(), error = %e, "a report answer waits for `quietdeck report answers`")
        }
    }
    state.changed(Changed::Cards);
    current(&state.db).await?.ok_or_else(|| AppError::Unavailable("the report vanished".into()))
}

/// `quietdeck report answers`: what the agent hasn't had yet, each once.
pub async fn take_answers(state: &AppState) -> AppResult<Vec<ReportAnswer>> {
    let rows: Vec<AnswerRow> = sqlx::query_as(
        "UPDATE report_answers SET picked_up_at = ? WHERE picked_up_at IS NULL \
         RETURNING id, report_version, control, kind, about, value, answered_at, picked_up_at",
    )
    .bind(now_ms())
    .fetch_all(&state.db)
    .await?;
    let mut out: Vec<ReportAnswer> = rows.into_iter().map(ReportAnswer::from).collect();
    out.sort_by_key(|a| a.id);
    if !out.is_empty() {
        state.changed(Changed::Cards);
    }
    Ok(out)
}

/// The chat box: the words go to the report's agent now, or fail at once when it isn't running. Nothing is kept.
pub async fn chat(state: &AppState, c: ReportChat) -> AppResult<Delivered> {
    c.validate().map_err(|e| AppError::BadRequest(e.to_string()))?;
    let Some(r) = open_row(&state.db).await? else {
        return Err(AppError::NotFound("no triage agent has written a report yet, so there's nobody to tell".into()));
    };
    let to = format!("{}@{}", r.posted_tab, r.posted_node);
    let body = format!("From the Inbox's chat box: {}", c.text.trim());
    crate::relay::send(&state.db, &state.hub, &to, DeliveryKind::Deck, body).await.map_err(|e| match e {
        DeliveryError::SessionOffline { .. } | DeliveryError::NodeOffline { .. } => {
            AppError::Offline(format!("{to} isn't running, so it didn't get that."))
        }
        DeliveryError::Ambiguous { .. } => AppError::BadRequest(e.to_string()),
        DeliveryError::Failed { .. } => AppError::Unavailable(e.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use quietdeck_types::report::ReportSet;

    fn me() -> SessionRef {
        SessionRef { node: "n1".into(), tab: "email triage".into(), session_id: Some("s1".into()) }
    }

    #[tokio::test]
    async fn a_report_is_edited_from_the_version_the_agent_saw() {
        let state = AppState::for_tests().await;
        let set = |markdown: &str, base: Option<i64>, new: bool| ReportSet { markdown: markdown.into(), base, new };
        let r = super::set(&state, &me(), set("# Round", None, false)).await.unwrap();
        assert_eq!((r.version, r.markdown.as_str()), (1, "# Round"));
        let r = super::set(&state, &me(), set("# Round\n- more", Some(1), false)).await.unwrap();
        assert_eq!(r.version, 2);
        let stale = super::set(&state, &me(), set("# Old", Some(1), false)).await.unwrap_err();
        assert!(stale.to_string().contains("version 2"), "{stale}");
        let missing = super::set(&state, &me(), set("<Default cards=\"c_99\">archive</Default>", Some(2), false))
            .await
            .unwrap_err();
        assert!(missing.to_string().contains("no card c_99"), "{missing}");
        let fresh = super::set(&state, &me(), set("# Next round", None, true)).await.unwrap();
        assert_ne!(fresh.id, r.id);
        assert_eq!(fresh.version, 1);
    }

    #[tokio::test]
    async fn the_report_lists_open_cards_no_default_names() {
        let state = AppState::for_tests().await;
        let card = |thread: &str| -> quietdeck_types::card::CardInput {
            serde_json::from_value(serde_json::json!({"account": "personal", "thread_key": thread, "subject": "s",
                "summary": "x", "section": "archive_list", "reason": "r",
                "actions": [{"type": "archive", "account": "personal", "thread_id": thread}]}))
            .unwrap()
        };
        let mut ids = vec![];
        for t in ["t1", "t2", "t3"] {
            let added =
                crate::store::cards::add(&state.db, &me(), &card(t), quietdeck_types::card::CardState::Open, None)
                    .await
                    .unwrap();
            ids.push(added.card.id);
        }
        let md = format!("<Default cards=\"c_{}\">archive</Default>", ids[0]);
        let r = super::set(&state, &me(), ReportSet { markdown: md, base: None, new: false }).await.unwrap();
        assert_eq!(r.cards_not_in_report, vec![ids[1], ids[2]]);
        let md = format!("<Default cards=\"c_{} c_{} c_{}\">archive</Default>", ids[0], ids[1], ids[2]);
        let r = super::set(&state, &me(), ReportSet { markdown: md, base: Some(1), new: false }).await.unwrap();
        assert!(r.cards_not_in_report.is_empty());
    }

    #[tokio::test]
    async fn answers_are_kept_and_taken_once_when_the_agent_was_away() {
        let state = AppState::for_tests().await;
        let r = super::set(&state, &me(), ReportSet { markdown: "x".into(), base: None, new: false }).await.unwrap();
        let r = answer(
            &state,
            ReportAnswerAdd {
                report_id: r.id,
                report_version: r.version,
                control: "signin".into(),
                kind: AnswerKind::Choice,
                about: "Was this you?".into(),
                value: "No".into(),
            },
        )
        .await
        .unwrap();
        // No node is connected in tests, so the answer waits.
        assert_eq!(r.answers.len(), 1);
        assert!(!r.answers[0].picked_up);
        let taken = take_answers(&state).await.unwrap();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].value, "No");
        assert!(take_answers(&state).await.unwrap().is_empty());
        let chat = chat(&state, ReportChat { text: "archive the HelloFresh ones from now on".into() }).await;
        assert!(chat.is_err(), "nobody is running, so the chat fails at once");
    }
}
