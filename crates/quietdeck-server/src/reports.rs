//! The triage report (quietdeck_types::report): the agent keeps one living document per round, the Inbox shows it,
//! and the person's answers and chat go straight back to the agent that wrote it. Answers are kept so the Inbox can
//! show them and an agent that was away can take them; chat is only passed on.

use garde::Validate;
use quietdeck_types::action::SessionRef;
use quietdeck_types::api::Changed;
use quietdeck_types::node::Delivered;
use quietdeck_types::relay::{DeliveryError, DeliveryKind};
use quietdeck_types::report::{
    AnswerKind, Report, ReportAnswer, ReportAnswerAdd, ReportChat, ReportReply, ReportSet, ReportSnapshot,
    ReportSnapshotSummary, default_cards, defaults_changed,
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
    finished_at: Option<i64>,
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
    cards: Option<String>,
}

impl From<AnswerRow> for ReportAnswer {
    fn from(r: AnswerRow) -> ReportAnswer {
        ReportAnswer {
            id: r.id,
            report_version: r.report_version,
            control: r.control,
            kind: match r.kind.as_str() {
                KIND_CHANGE => AnswerKind::Change,
                KIND_REPLY => AnswerKind::Reply,
                _ => AnswerKind::Choice,
            },
            about: r.about,
            value: r.value,
            answered_at: ts(r.answered_at),
            picked_up: r.picked_up_at.is_some(),
            cards: r.cards,
        }
    }
}

const KIND_CHANGE: &str = "change";
const KIND_CHOICE: &str = "choice";
const KIND_REPLY: &str = "reply";

fn kind_str(k: AnswerKind) -> &'static str {
    match k {
        AnswerKind::Change => KIND_CHANGE,
        AnswerKind::Choice => KIND_CHOICE,
        AnswerKind::Reply => KIND_REPLY,
    }
}

async fn open_row(db: &Db) -> AppResult<Option<ReportRow>> {
    Ok(sqlx::query_as(
        "SELECT id, version, markdown, posted_node, posted_tab, posted_session_id, created_at, updated_at, \
         finished_at FROM reports WHERE closed_at IS NULL ORDER BY id DESC LIMIT 1",
    )
    .fetch_optional(db)
    .await?)
}

/// The report the Inbox shows, with its answers.
pub async fn current(db: &Db) -> AppResult<Option<Report>> {
    let Some(r) = open_row(db).await? else { return Ok(None) };
    let answers: Vec<AnswerRow> = sqlx::query_as(
        "SELECT id, report_version, control, kind, about, value, answered_at, picked_up_at, cards \
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
        changes: changes(db, r.id).await?,
        finished: r.finished_at.is_some(),
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
    let open: Option<(i64, i64, String)> =
        sqlx::query_as("SELECT id, version, markdown FROM reports WHERE closed_at IS NULL ORDER BY id DESC LIMIT 1")
            .fetch_optional(&mut *tx)
            .await?;
    // What changed, for the person's toast and Approve All's settle window.
    let before = match (&open, req.new) {
        (Some((_, _, md)), false) => md.clone(),
        _ => String::new(),
    };
    let worked_out = defaults_changed(&before, &req.markdown);
    let open = open.map(|(id, version, _)| (id, version));
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
                 posted_tab = ?, posted_session_id = ?, finished_at = NULL WHERE id = ?",
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
    let (report_id, version): (i64, i64) =
        sqlx::query_as("SELECT id, version FROM reports WHERE closed_at IS NULL ORDER BY id DESC LIMIT 1")
            .fetch_one(&mut *tx)
            .await?;
    let cards = worked_out.as_ref().map(|w| w.0.clone()).unwrap_or_default();
    let summary = req.change.clone().or_else(|| worked_out.map(|w| w.1));
    if let Some(summary) = summary {
        sqlx::query("INSERT INTO report_changes (report_id, version, summary, cards, at) VALUES (?, ?, ?, ?, ?)")
            .bind(report_id)
            .bind(version)
            .bind(&summary)
            .bind(crate::db::json(&cards))
            .bind(now)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    state.changed(Changed::Cards);
    current(&state.db).await?.ok_or_else(|| AppError::Unavailable("the report vanished".into()))
}

/// `quietdeck report done`: nothing on the report waits for the person any more, so the Inbox shows "All done." (its
/// bar and chat stay) until the agent edits it or starts the next report. Refused while a card still waits: "All
/// done." would hide it.
pub async fn done(state: &AppState) -> AppResult<()> {
    let waiting: Vec<(i64,)> = sqlx::query_as(
        "SELECT id FROM cards WHERE state IN ('open', 'handed_off', 'failed', 'approved', 'executing', 'revising') \
         AND section != 'sent' ORDER BY id",
    )
    .fetch_all(&state.db)
    .await?;
    if !waiting.is_empty() {
        let ids = waiting.iter().map(|(id,)| format!("c_{id}")).collect::<Vec<_>>().join(", ");
        return Err(AppError::Conflict(format!(
            "cards still wait: {ids}. Resolve them (or name them on the report) before marking it done"
        )));
    }
    let now = now_ms();
    let r = sqlx::query("UPDATE reports SET finished_at = ?, updated_at = ? WHERE closed_at IS NULL")
        .bind(now)
        .bind(now)
        .execute(&state.db)
        .await?;
    if r.rows_affected() == 0 {
        return Err(AppError::NotFound("there's no report: start one with `quietdeck report set --new`".into()));
    }
    state.changed(Changed::Cards);
    Ok(())
}

/// The report's latest changes, oldest first.
async fn changes(db: &Db, report_id: i64) -> AppResult<Vec<quietdeck_types::reading::ReportChange>> {
    let rows: Vec<(i64, i64, String, String, i64)> = sqlx::query_as(
        "SELECT id, version, summary, cards, at FROM report_changes WHERE report_id = ? ORDER BY id DESC LIMIT 20",
    )
    .bind(report_id)
    .fetch_all(db)
    .await?;
    let mut out: Vec<_> = rows
        .into_iter()
        .map(|r| quietdeck_types::reading::ReportChange {
            id: r.0,
            version: r.1,
            summary: r.2,
            cards: crate::db::from_json(&r.3).unwrap_or_default(),
            at: ts(r.4),
        })
        .collect();
    out.reverse();
    Ok(out)
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
    if a.kind == AnswerKind::Reply {
        return Err(AppError::BadRequest("replies come from the agent (`quietdeck report reply`)".into()));
    }
    // An Ask that names cards joins the thread of the default that names them.
    let cards = a
        .cards
        .as_deref()
        .map(str::trim)
        .filter(|c| a.kind == AnswerKind::Choice && !quietdeck_types::report::card_ids(c).is_empty());
    // A default's messages are a thread: a later one continues the conversation.
    let (earlier,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM report_answers WHERE report_id = ? AND control = ? AND kind IN ('change', 'reply')",
    )
    .bind(r.id)
    .bind(&a.control)
    .fetch_one(&state.db)
    .await?;
    let id: (i64,) = sqlx::query_as(
        "INSERT INTO report_answers (report_id, report_version, control, kind, about, value, answered_at, cards) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(r.id)
    .bind(a.report_version)
    .bind(&a.control)
    .bind(kind_str(a.kind))
    .bind(a.about.trim())
    .bind(value)
    .bind(now_ms())
    .bind(cards)
    .fetch_one(&state.db)
    .await?;
    let to = SessionRef { node: r.posted_node, tab: r.posted_tab, session_id: None };
    let body = match a.kind {
        AnswerKind::Choice => match cards {
            Some(cards) => format!(
                "On the report (version {}), \"{}\" [{}]: {value}\n\nThe question is about {cards}, so the Inbox shows \
                 his answer in the thread of the default that names them. Act on it, update the report where it \
                 changes things, and answer him in that thread: `quietdeck report reply 'cards:{cards}' \"…\"` (what \
                 you did, or a question).",
                a.report_version,
                a.about.trim(),
                a.control
            ),
            None => format!(
                "On the report (version {}), \"{}\" [{}]: {value}\n\nUpdate the report: follow up on the answer there.",
                a.report_version,
                a.about.trim(),
                a.control
            ),
        },
        AnswerKind::Change if earlier > 0 => format!(
            "On the report (version {}), in the thread on the default \"{}\" [{}]: {value}\n\nIt continues that \
             conversation (`quietdeck report show` lists it). Approve All leaves those cards to you. Answer him in \
             the thread: `quietdeck report reply '{}' \"…\"`.",
            a.report_version,
            a.about.trim(),
            a.control,
            a.control
        ),
        AnswerKind::Change => format!(
            "On the report (version {}), a change to the default \"{}\" [{}]: {value}\n\nUsually it adjusts the \
             default or adds to it rather than replacing it. Approve All leaves those cards to you. Do what this \
             says, or propose it in the report, and answer him in the default's thread: \
             `quietdeck report reply '{}' \"…\"` (what you did, or a question).",
            a.report_version,
            a.about.trim(),
            a.control,
            a.control
        ),
        AnswerKind::Reply => unreachable!("refused above"),
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
         RETURNING id, report_version, control, kind, about, value, answered_at, picked_up_at, cards",
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

/// `quietdeck report reply`: the agent's answer in a default's thread. `to` is the default's key as its messages
/// carry it, or a card it names (`c_12`, kept as `cards:c_12`: the Inbox puts it in the thread of whichever default
/// names that card).
pub async fn reply(state: &AppState, req: ReportReply) -> AppResult<Report> {
    req.validate().map_err(|e| AppError::BadRequest(e.to_string()))?;
    let Some(r) = open_row(&state.db).await? else {
        return Err(AppError::NotFound("there's no report to reply on".into()));
    };
    let to = req.to.trim();
    let keys = quietdeck_types::report::default_keys(&r.markdown);
    let control = if keys.iter().any(|(k, _)| k == to) || to.starts_with("cards:") {
        to.to_string()
    } else if !quietdeck_types::report::card_ids(to).is_empty() {
        format!("cards:{to}")
    } else {
        return Err(AppError::BadRequest(format!(
            "no default {to:?} in the report: give the [key] from his message (cards:c_12, archive-all) or a card (c_12)"
        )));
    };
    let now = now_ms();
    sqlx::query(
        "INSERT INTO report_answers (report_id, report_version, control, kind, about, value, answered_at, picked_up_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(r.id)
    .bind(r.version)
    .bind(&control)
    .bind(KIND_REPLY)
    .bind("")
    .bind(req.text.trim())
    .bind(now)
    .bind(now)
    .execute(&state.db)
    .await?;
    state.changed(Changed::Cards);
    current(&state.db).await?.ok_or_else(|| AppError::Unavailable("the report vanished".into()))
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

/// How far back a card's changes are looked for: a card waits days at most.
const CHANGES_WINDOW_MS: i64 = 30 * 24 * 3600 * 1000;

/// The person's changes to a card's default, oldest first: those on a `<Default>` that names it in the current
/// report (by its `id` or its `cards`), and any `cards:` change that names it, whatever the report says now.
pub async fn changes_for_card(db: &Db, card_id: i64) -> anyhow::Result<Vec<String>> {
    let markdown: Option<(String,)> =
        sqlx::query_as("SELECT markdown FROM reports WHERE closed_at IS NULL ORDER BY id DESC LIMIT 1")
            .fetch_optional(db)
            .await?;
    let keys: Vec<String> = markdown
        .map(|(m,)| quietdeck_types::report::default_keys(&m))
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, cards)| cards.contains(&card_id))
        .map(|(key, _)| key)
        .collect();
    let rows: Vec<(String, String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT control, kind, about, value, cards FROM report_answers \
         WHERE kind IN ('change', 'choice') AND answered_at > ? ORDER BY id",
    )
    .bind(now_ms() - CHANGES_WINDOW_MS)
    .fetch_all(db)
    .await?;
    let names = |c: &str| quietdeck_types::report::card_ids(c).contains(&card_id);
    Ok(rows
        .into_iter()
        .filter_map(|(control, kind, about, value, cards)| {
            if kind == KIND_CHOICE {
                // His answer to a question about the card, with the question, so it reads on its own.
                let about = about.trim();
                return cards.as_deref().is_some_and(names).then(|| match about.ends_with('?') {
                    true => format!("{about} {value}"),
                    false => format!("{about}: {value}"),
                });
            }
            (keys.contains(&control) || control.strip_prefix("cards:").is_some_and(names)).then_some(value)
        })
        .collect())
}

/// Approve All: keep the Inbox document as it is now, for History, with the person's changes and answers since the
/// report's previous snapshot and the cards it names. Nothing when there's no report.
pub async fn snapshot(db: &Db, batch: &str, approved: &[i64]) -> AppResult<()> {
    let Some(report) = current(db).await? else { return Ok(()) };
    let previous: Option<(Option<i64>,)> =
        sqlx::query_as("SELECT last_answer_id FROM report_snapshots WHERE report_id = ? ORDER BY id DESC LIMIT 1")
            .bind(report.id)
            .fetch_optional(db)
            .await?;
    let after = previous.and_then(|p| p.0).unwrap_or(0);
    let round_answers: Vec<ReportAnswer> = report.answers.iter().filter(|a| a.id > after).cloned().collect();
    let last_answer_id = report.answers.iter().map(|a| a.id).max();
    let (mut ids, _) = default_cards(&report.markdown);
    ids.extend_from_slice(approved);
    ids.sort_unstable();
    ids.dedup();
    let mut cards = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(c) = crate::store::cards::get(db, id).await? {
            cards.push(c);
        }
    }
    let now = now_ms();
    let snap = ReportSnapshot {
        id: 0,
        taken_at: ts(now),
        report: report.clone(),
        cards,
        approved: approved.len() as i64,
        round_answers,
    };
    let json = serde_json::to_string(&snap).map_err(|e| AppError::Internal(e.into()))?;
    sqlx::query(
        "INSERT INTO report_snapshots (report_id, batch, taken_at, last_answer_id, approved, snapshot) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(report.id)
    .bind(batch)
    .bind(now)
    .bind(last_answer_id.or(Some(after)))
    .bind(snap.approved)
    .bind(json)
    .execute(db)
    .await?;
    Ok(())
}

/// An undone Approve All didn't happen: its snapshot goes.
pub async fn drop_snapshot(db: &Db, batch: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM report_snapshots WHERE batch = ?").bind(batch).execute(db).await?;
    Ok(())
}

pub async fn snapshot_get(db: &Db, id: i64) -> AppResult<ReportSnapshot> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT snapshot FROM report_snapshots WHERE id = ?").bind(id).fetch_optional(db).await?;
    let Some((json,)) = row else { return Err(AppError::NotFound(format!("no snapshot {id}"))) };
    let mut snap: ReportSnapshot = serde_json::from_str(&json).map_err(|e| AppError::Internal(e.into()))?;
    snap.id = id;
    Ok(snap)
}

/// The snapshots taken in a time window, newest first, for History.
pub async fn snapshots_between(db: &Db, since_ms: i64, until_ms: i64) -> AppResult<Vec<ReportSnapshotSummary>> {
    let rows: Vec<(i64, i64, i64, String)> = sqlx::query_as(
        "SELECT id, taken_at, approved, snapshot FROM report_snapshots WHERE taken_at >= ? AND taken_at < ? \
         ORDER BY taken_at DESC",
    )
    .bind(since_ms)
    .bind(until_ms)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, taken_at, approved, json)| {
            let answers = serde_json::from_str::<ReportSnapshot>(&json)
                .map(|s| s.round_answers.iter().filter(|a| a.kind != AnswerKind::Reply).count() as i64)
                .unwrap_or(0);
            ReportSnapshotSummary { id, taken_at: ts(taken_at), approved, answers }
        })
        .collect())
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
        let set = |markdown: &str, base: Option<i64>, new: bool| ReportSet {
            markdown: markdown.into(),
            base,
            new,
            change: None,
        };
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
        let r =
            super::set(&state, &me(), ReportSet { markdown: md, base: None, new: false, change: None }).await.unwrap();
        assert_eq!(r.cards_not_in_report, vec![ids[1], ids[2]]);
        let md = format!("<Default cards=\"c_{} c_{} c_{}\">archive</Default>", ids[0], ids[1], ids[2]);
        let r = super::set(&state, &me(), ReportSet { markdown: md, base: Some(1), new: false, change: None })
            .await
            .unwrap();
        assert!(r.cards_not_in_report.is_empty());
    }

    #[tokio::test]
    async fn answers_are_kept_and_taken_once_when_the_agent_was_away() {
        let state = AppState::for_tests().await;
        let r = super::set(&state, &me(), ReportSet { markdown: "x".into(), base: None, new: false, change: None })
            .await
            .unwrap();
        let r = answer(
            &state,
            ReportAnswerAdd {
                report_id: r.id,
                report_version: r.version,
                control: "signin".into(),
                kind: AnswerKind::Choice,
                about: "Was this you?".into(),
                value: "No".into(),
                cards: None,
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

    #[tokio::test]
    async fn an_answer_about_cards_joins_their_thread_and_reaches_the_decision() {
        let state = AppState::for_tests().await;
        let markdown = "<Ask id=\"invoice\" options=\"DocSpring, Inc.|Me\" cards=\"c_5\">Who should BizSpace \
                        invoice?</Ask>";
        let r =
            super::set(&state, &me(), ReportSet { markdown: markdown.into(), base: None, new: false, change: None })
                .await
                .unwrap();
        let pick = |cards: Option<&str>| ReportAnswerAdd {
            report_id: r.id,
            report_version: r.version,
            control: "invoice".into(),
            kind: AnswerKind::Choice,
            about: "Who should BizSpace invoice?".into(),
            value: "DocSpring, Inc.".into(),
            cards: cards.map(str::to_string),
        };
        let r = answer(&state, pick(Some(" c_5 "))).await.unwrap();
        assert_eq!(r.answers[0].cards.as_deref(), Some("c_5"));
        assert_eq!(changes_for_card(&state.db, 5).await.unwrap(), vec!["Who should BizSpace invoice? DocSpring, Inc."]);
        assert!(changes_for_card(&state.db, 6).await.unwrap().is_empty());

        // Without cards, or with a value that names none, the answer stays on its question.
        let r = answer(&state, pick(Some("soon"))).await.unwrap();
        assert_eq!(r.answers[1].cards, None);
        answer(&state, pick(None)).await.unwrap();
        assert_eq!(changes_for_card(&state.db, 5).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn approve_all_keeps_the_document_with_this_rounds_answers_and_undo_drops_it() {
        let state = AppState::for_tests().await;
        let r =
            super::set(&state, &me(), ReportSet { markdown: "# Round".into(), base: None, new: false, change: None })
                .await
                .unwrap();
        let change = |value: &str| ReportAnswerAdd {
            report_id: r.id,
            report_version: r.version,
            control: "archive-all".into(),
            kind: AnswerKind::Change,
            about: "Archive all".into(),
            value: value.into(),
            cards: None,
        };
        answer(&state, change("keep the Stripe one")).await.unwrap();
        snapshot(&state.db, "b1", &[7]).await.unwrap();
        let listed = snapshots_between(&state.db, 0, i64::MAX).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!((listed[0].approved, listed[0].answers), (1, 1));
        let first = snapshot_get(&state.db, listed[0].id).await.unwrap();
        assert_eq!(first.report.markdown, "# Round");
        assert_eq!(first.round_answers[0].value, "keep the Stripe one");

        // The next Approve All lists only what was said after the first one.
        answer(&state, change("and the Amex one")).await.unwrap();
        snapshot(&state.db, "b2", &[8, 9]).await.unwrap();
        let listed = snapshots_between(&state.db, 0, i64::MAX).await.unwrap();
        let second = snapshot_get(&state.db, listed.iter().find(|s| s.approved == 2).unwrap().id).await.unwrap();
        let values: Vec<&str> = second.round_answers.iter().map(|a| a.value.as_str()).collect();
        assert_eq!(values, ["and the Amex one"]);
        assert_eq!(second.report.answers.len(), 2, "the report itself keeps every answer");

        drop_snapshot(&state.db, "b2").await.unwrap();
        assert_eq!(snapshots_between(&state.db, 0, i64::MAX).await.unwrap().len(), 1);
    }
}
