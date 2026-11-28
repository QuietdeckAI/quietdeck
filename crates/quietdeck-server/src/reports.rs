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
    ReportSnapshotSummary, default_cards, default_keys, defaults_changed,
};

use quietdeck_types::document::{Document, DocumentKind, INBOX_ID};

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
    document_id: i64,
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

const ROW: &str = "SELECT id, version, markdown, posted_node, posted_tab, posted_session_id, created_at, updated_at, \
                   finished_at, document_id FROM reports";

/// A document's open report.
async fn open_row(db: &Db, doc: i64) -> AppResult<Option<ReportRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{ROW} WHERE closed_at IS NULL AND document_id = ? ORDER BY id DESC LIMIT 1"
    )))
    .bind(doc)
    .fetch_optional(db)
    .await?)
}

/// An open report by its ID, whichever document it's on.
async fn open_row_by_id(db: &Db, id: i64) -> AppResult<Option<ReportRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!("{ROW} WHERE closed_at IS NULL AND id = ?")))
        .bind(id)
        .fetch_optional(db)
        .await?)
}

/// The report the Inbox shows, with its answers.
/// Whether the deck keeps a report (Settings: `report`). Off, every read here answers as if there were none.
pub async fn enabled(db: &Db) -> anyhow::Result<bool> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM settings WHERE name = 'report'").fetch_optional(db).await?;
    Ok(row.is_none_or(|(v,)| v == "true"))
}

/// What the report's routes answer while it's off.
pub const OFF: &str = "this deck has no report (Settings: report is off): use the cards and decisions";

pub async fn current(db: &Db, doc: i64) -> AppResult<Option<Report>> {
    if !enabled(db).await? {
        return Ok(None);
    }
    let Some(r) = open_row(db, doc).await? else { return Ok(None) };
    from_row(db, r).await.map(Some)
}

/// The cards that belong to a document when no default names them: on a document with a writer, the open cards its
/// writer posted; on the Inbox, every open card but those the writers of other documents posted.
async fn open_cards_of(db: &Db, doc: i64) -> AppResult<Vec<i64>> {
    let d = crate::documents::get(db, doc).await?;
    let rows: Vec<(i64,)> = match (&d.writer_node, &d.writer_tab) {
        (Some(node), Some(tab)) => {
            sqlx::query_as(
                "SELECT id FROM cards WHERE state IN ('open', 'handed_off', 'failed') AND section != 'sent' \
                 AND posted_node = ? AND posted_tab = ? ORDER BY id",
            )
            .bind(node)
            .bind(tab)
            .fetch_all(db)
            .await?
        }
        _ => {
            sqlx::query_as(
                "SELECT id FROM cards c WHERE state IN ('open', 'handed_off', 'failed') AND section != 'sent' \
                 AND NOT EXISTS (SELECT 1 FROM documents d WHERE d.removed_at IS NULL AND d.id != ? \
                 AND d.writer_node = c.posted_node AND d.writer_tab = c.posted_tab) ORDER BY id",
            )
            .bind(doc)
            .fetch_all(db)
            .await?
        }
    };
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

async fn from_row(db: &Db, r: ReportRow) -> AppResult<Report> {
    let answers: Vec<AnswerRow> = sqlx::query_as(
        "SELECT id, report_version, control, kind, about, value, answered_at, picked_up_at, cards \
         FROM report_answers WHERE report_id = ? ORDER BY id",
    )
    .bind(r.id)
    .fetch_all(db)
    .await?;
    let (named, _) = default_cards(&r.markdown);
    let cards_not_in_report =
        open_cards_of(db, r.document_id).await?.into_iter().filter(|id| !named.contains(id)).collect();
    // Reports saved before the deck fixed item Defaults on the way in (27 Nov 2026) are fixed on the way out.
    let markdown = quietdeck_types::report::item_defaults_indented(&r.markdown);
    Ok(Report {
        id: r.id,
        version: r.version,
        markdown: markdown.clone(),
        posted_by: SessionRef { node: r.posted_node, tab: r.posted_tab, session_id: r.posted_session_id },
        created_at: ts(r.created_at),
        updated_at: ts(r.updated_at),
        answers: answers.into_iter().map(ReportAnswer::from).collect(),
        cards_not_in_report,
        changes: changes(db, r.id).await?,
        finished: r.finished_at.is_some(),
        item_times: item_times(db, r.document_id, &markdown, r.updated_at).await?,
        document_id: r.document_id,
    })
}

/// Notes when each of the report's items was first written as it reads now, and forgets items that are gone.
async fn track_items(tx: &mut sqlx::SqliteConnection, doc: i64, markdown: &str, now: i64) -> AppResult<()> {
    let hashes: Vec<String> =
        quietdeck_types::report::items(markdown).iter().map(|i| crate::auth::sha256_hex(i)).collect();
    for h in &hashes {
        sqlx::query("INSERT OR IGNORE INTO report_items (document_id, hash, changed_at) VALUES (?, ?, ?)")
            .bind(doc)
            .bind(h)
            .bind(now)
            .execute(&mut *tx)
            .await?;
    }
    let kept: Vec<(String,)> =
        sqlx::query_as("SELECT hash FROM report_items WHERE document_id = ?").bind(doc).fetch_all(&mut *tx).await?;
    for (h,) in kept.into_iter().filter(|(h,)| !hashes.contains(h)) {
        sqlx::query("DELETE FROM report_items WHERE document_id = ? AND hash = ?")
            .bind(doc)
            .bind(h)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}

/// When each item last changed, in order; an item the deck never noted (written before it kept track) gets the
/// report's last edit.
async fn item_times(db: &Db, doc: i64, markdown: &str, updated_at: i64) -> AppResult<Vec<jiff::Timestamp>> {
    let rows: Vec<(String, i64)> = sqlx::query_as("SELECT hash, changed_at FROM report_items WHERE document_id = ?")
        .bind(doc)
        .fetch_all(db)
        .await?;
    let known: std::collections::HashMap<String, i64> = rows.into_iter().collect();
    Ok(quietdeck_types::report::items(markdown)
        .iter()
        .map(|i| ts(known.get(&crate::auth::sha256_hex(i)).copied().unwrap_or(updated_at)))
        .collect())
}

/// `quietdeck report set`: replace the report's text, from the version the agent last saw.
pub async fn set(state: &AppState, me: &SessionRef, doc: &Document, mut req: ReportSet) -> AppResult<Report> {
    req.validate().map_err(|e| AppError::BadRequest(e.to_string()))?;
    crate::documents::may_write(doc, me)?;
    // An item's Default written flush left is moved inside its item, so the Inbox never draws it as a group's.
    req.markdown = quietdeck_types::report::item_defaults_indented(&req.markdown);
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
    if doc.kind == DocumentKind::Inbox {
        problems.extend(archive_all_problems(&state.db, &req.markdown).await?);
        problems.extend(finished_problem(&state.db, &ids).await?);
    }
    if !problems.is_empty() {
        return Err(AppError::BadRequest(format!("the report's <Default> tags: {}", problems.join("; "))));
    }
    let now = now_ms();
    let mut tx = state.db.begin().await?;
    let open: Option<(i64, i64, String)> = sqlx::query_as(
        "SELECT id, version, markdown FROM reports WHERE closed_at IS NULL AND document_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(doc.id)
    .fetch_optional(&mut *tx)
    .await?;
    // While an Approve All on this report is being carried out (its undo window, or its cards approved or running),
    // the report holds still: an edit would change what he's watching mid-way (Nathan, 28 Nov: "updates like this to
    // the inbox shouldn't [be] allowed while an 'approve all' is in progress"). Cards handed to the agent don't count:
    // it carries those out itself.
    if let Some((_, _, markdown)) = &open {
        let (named, _) = default_cards(markdown);
        if !named.is_empty() {
            let list = named.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
            let (busy,): (i64,) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT COUNT(*) FROM cards c WHERE c.id IN ({list}) AND (c.state IN ('approved', 'executing') \
                 OR EXISTS (SELECT 1 FROM decisions d JOIN decision_batches b ON b.id = d.batch \
                            WHERE d.card_id = c.id AND b.released_at IS NULL))"
            )))
            .fetch_one(&mut *tx)
            .await?;
            if busy > 0 {
                return Err(AppError::Conflict(format!(
                    "Approve All is being carried out on this report ({busy} card{} still going): leave the report as \
                     it is until they're done, then make your edit (new mail waits for it)",
                    if busy == 1 { "" } else { "s" }
                )));
            }
        }
    }
    if req.new && open.is_some() && doc.kind == DocumentKind::Work {
        return Err(AppError::BadRequest(format!(
            "the {} document is kept up to date, never replaced: edit it with --base (no --new)",
            doc.name
        )));
    }
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
            insert(&mut tx, me, doc.id, &req.markdown, now).await?;
        }
        (None, _) => insert(&mut tx, me, doc.id, &req.markdown, now).await?,
        (Some((id, version)), false) => {
            if req.base != Some(version) {
                return Err(AppError::Conflict(format!(
                    "the report is at version {version}{}: run `quietdeck report show{}`, make your edit to that text \
                     and send it with --base {version}",
                    req.base.map(|b| format!(", not {b}")).unwrap_or_default(),
                    if doc.id == INBOX_ID { String::new() } else { format!(" --doc {}", doc.slug) }
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
    track_items(&mut tx, doc.id, &req.markdown, now).await?;
    let (report_id, version): (i64, i64) = sqlx::query_as(
        "SELECT id, version FROM reports WHERE closed_at IS NULL AND document_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(doc.id)
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
    current(&state.db, doc.id).await?.ok_or_else(|| AppError::Unavailable("the report vanished".into()))
}

/// The document an open report is on.
pub async fn document_of(db: &Db, report_id: i64) -> AppResult<Option<Document>> {
    match open_row_by_id(db, report_id).await? {
        Some(r) => Ok(Some(crate::documents::get(db, r.document_id).await?)),
        None => Ok(None),
    }
}

/// The Inbox document.
pub async fn inbox(db: &Db) -> AppResult<Document> {
    crate::documents::get(db, INBOX_ID).await
}

/// A Work document's "Archive all" on its FYI part (and Approve All there): the FYI items go, as a new version, and the
/// agent sees it on its next `report show`. Answers 0 when there was nothing to clear.
pub async fn fyi_clear(state: &AppState, doc: &Document) -> AppResult<Option<Report>> {
    if doc.kind != DocumentKind::Work {
        return Err(AppError::BadRequest(format!("the {} document has no FYI part to clear", doc.name)));
    }
    let Some(r) = open_row(&state.db, doc.id).await? else { return Ok(None) };
    let Some(cleared) = quietdeck_types::report::fyi_cleared(&r.markdown) else { return Ok(None) };
    let now = now_ms();
    let mut tx = state.db.begin().await?;
    let done = sqlx::query(
        "UPDATE reports SET markdown = ?, version = version + 1, updated_at = ? WHERE id = ? AND version = ?",
    )
    .bind(&cleared)
    .bind(now)
    .bind(r.id)
    .bind(r.version)
    .execute(&mut *tx)
    .await?;
    if done.rows_affected() == 0 {
        return Err(AppError::Conflict("the document changed: read it again, then clear".into()));
    }
    sqlx::query("INSERT INTO report_changes (report_id, version, summary, cards, at) VALUES (?, ?, ?, '[]', ?)")
        .bind(r.id)
        .bind(r.version + 1)
        .bind("Cleared the FYI list")
        .bind(now)
        .execute(&mut *tx)
        .await?;
    track_items(&mut tx, doc.id, &cleared, now).await?;
    tx.commit().await?;
    state.changed(Changed::Cards);
    current(&state.db, doc.id).await
}

/// The Inbox shows only what still waits: a card that's finished (carried out, automatic, handled in Gmail) comes
/// out of the report, with the text about it, the next time the agent edits it. Left in, it came back with the next
/// new mail after "All done." (Nathan, 28 Nov 2026: "i approve, it goes to all done, then i check later and it's back
/// on the old inbox content").
async fn finished_problem(db: &Db, ids: &[i64]) -> AppResult<Option<String>> {
    if ids.is_empty() {
        return Ok(None);
    }
    let list = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    let done: Vec<(i64,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT id FROM cards WHERE id IN ({list}) AND state IN ('done', 'automatic', 'handled_elsewhere') ORDER BY id"
    )))
    .fetch_all(db)
    .await?;
    if done.is_empty() {
        return Ok(None);
    }
    let names: Vec<String> = done.iter().map(|(id,)| format!("c_{id}")).collect();
    Ok(Some(format!(
        "{} {} finished: take {} out of the report, with the text about {}, since the Inbox shows only what still \
         waits for him",
        names.join(", "),
        if names.len() == 1 { "is" } else { "are" },
        if names.len() == 1 { "it" } else { "them" },
        if names.len() == 1 { "it" } else { "them" },
    )))
}

/// The archive part's `archive-all` Default may name only cards whose sole action is an archive (or marking read):
/// anything more, a forward or a reply included, goes in the Inbox part with its own Default (Nathan, 25 Nov 2026:
/// "if there is any other action at all (e.g. forwarding) it should go to inbox. forward + archive is still inbox.").
async fn archive_all_problems(db: &Db, markdown: &str) -> AppResult<Vec<String>> {
    let ids: Vec<i64> =
        default_keys(markdown).into_iter().filter(|(key, _)| key == "archive-all").flat_map(|(_, ids)| ids).collect();
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let list = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    let rows: Vec<(i64, String)> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT id, input FROM cards WHERE id IN ({list}) ORDER BY id")))
            .fetch_all(db)
            .await?;
    let mut problems = vec![];
    for (id, input) in rows {
        let input: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
        let mut others: Vec<String> = input["actions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| a["type"].as_str())
            .filter(|t| !matches!(*t, "archive" | "mark_read"))
            .map(str::to_string)
            .collect();
        others.dedup();
        if !others.is_empty() {
            problems.push(format!(
                "c_{id} has {}: \"Archive all\" takes only cards whose sole action is archive, so give it its own \
                 Default in the Inbox part",
                others.join(", ")
            ));
        }
    }
    Ok(problems)
}

/// `quietdeck report done`: nothing on the report waits for the person any more, so the Inbox shows "All done." (its
/// bar and chat stay) until the agent edits it or starts the next report. Refused while a card still waits: "All
/// done." would hide it.
pub async fn done(state: &AppState, doc: &Document) -> AppResult<()> {
    let mine = open_cards_of(&state.db, doc.id).await?;
    let waiting: Vec<(i64,)> = sqlx::query_as(
        "SELECT id FROM cards WHERE state IN ('open', 'handed_off', 'failed', 'approved', 'executing', 'revising') \
         AND section != 'sent' ORDER BY id",
    )
    .fetch_all(&state.db)
    .await?;
    // Approved and running cards aren't open, so they count for the Inbox as before; on another document, only its own.
    let waiting: Vec<(i64,)> = if doc.id == INBOX_ID || mine.is_empty() {
        waiting.into_iter().filter(|(id,)| doc.id == INBOX_ID || mine.contains(id)).collect()
    } else {
        waiting.into_iter().filter(|(id,)| mine.contains(id)).collect()
    };
    if !waiting.is_empty() {
        let ids = waiting.iter().map(|(id,)| format!("c_{id}")).collect::<Vec<_>>().join(", ");
        return Err(AppError::Conflict(format!(
            "cards still wait: {ids}. Resolve them (or name them on the report) before marking it done"
        )));
    }
    let now = now_ms();
    let r =
        sqlx::query("UPDATE reports SET finished_at = ?, updated_at = ? WHERE closed_at IS NULL AND document_id = ?")
            .bind(now)
            .bind(now)
            .bind(doc.id)
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
    let rows: Vec<(i64, i64, String, String, i64, String)> = sqlx::query_as(
        "SELECT id, version, summary, cards, at, kind FROM report_changes WHERE report_id = ? ORDER BY id DESC LIMIT 20",
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
            reply: r.5 == "reply",
        })
        .collect();
    out.reverse();
    Ok(out)
}

/// `quietdeck chat reply`: the agent answers the person's chat in the Inbox, shown as a toast on the page. For an
/// answer that isn't a change to the report or a card (those show anyway).
pub async fn chat_reply(state: &AppState, doc: &Document, text: &str) -> AppResult<()> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > 2000 {
        return Err(AppError::BadRequest("a reply is 1 to 2000 characters".into()));
    }
    let Some((report_id, version)): Option<(i64, i64)> = sqlx::query_as(
        "SELECT id, version FROM reports WHERE closed_at IS NULL AND document_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(doc.id)
    .fetch_optional(&state.db)
    .await?
    else {
        return Err(AppError::NotFound("there's no report for the reply to show on".into()));
    };
    sqlx::query(
        "INSERT INTO report_changes (report_id, version, summary, cards, at, kind) VALUES (?, ?, ?, '[]', ?, 'reply')",
    )
    .bind(report_id)
    .bind(version)
    .bind(text)
    .bind(now_ms())
    .execute(&state.db)
    .await?;
    state.changed(Changed::Cards);
    Ok(())
}

async fn insert(tx: &mut sqlx::SqliteConnection, me: &SessionRef, doc: i64, markdown: &str, now: i64) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO reports (markdown, posted_node, posted_tab, posted_session_id, created_at, updated_at, \
         document_id) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(markdown)
    .bind(&me.node)
    .bind(&me.tab)
    .bind(&me.session_id)
    .bind(now)
    .bind(now)
    .bind(doc)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// An answer from the Inbox: kept, and sent to the agent at once when its session is running.
pub async fn answer(state: &AppState, a: ReportAnswerAdd) -> AppResult<Report> {
    a.validate().map_err(|e| AppError::BadRequest(e.to_string()))?;
    let Some(r) = open_row_by_id(&state.db, a.report_id).await? else {
        return Err(AppError::Conflict("the agent has started a new report: answer on that one".into()));
    };
    let doc = r.document_id;
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
    // A question panel (an `<Ask>` with no options) is answered in words, in a thread of its own.
    let asks = quietdeck_types::report::ask_keys(&r.markdown);
    let panel = a.kind == AnswerKind::Choice && asks.iter().any(|(k, free)| *free && k == &a.control);
    // A message in an `<Ask>`'s chat: he'd rather talk it over than pick an option, so it isn't his answer.
    let ask_chat = a.kind == AnswerKind::Change && asks.iter().any(|(k, _)| k == &a.control);
    // A default's messages are a thread: a later one continues the conversation.
    let (earlier,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM report_answers WHERE report_id = ? AND control = ? \
         AND (kind IN ('change', 'reply') OR (? AND kind = 'choice'))",
    )
    .bind(r.id)
    .bind(&a.control)
    .bind(panel)
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
    // His answer to a question about one session goes to that session itself, from the deck, so it counts as his
    // own: relayed by the writer, a session rightly refuses to act as him on it (the overseer, 28 Nov 2026: a PR
    // comment he approved with "Post it" wasn't posted). The writer still hears of it, to update the report.
    let direct = match a.kind {
        AnswerKind::Choice => quietdeck_types::report::session_ask(&r.markdown, &a.control),
        _ => None,
    }
    .filter(|ask| ask.session != to.address());
    let direct_note = match &direct {
        Some(ask) => {
            let body = to_session(ask, value);
            match crate::relay::send(&state.db, &state.hub, &ask.session, DeliveryKind::Deck, body).await {
                Ok(_) => format!(
                    "\n\nThe deck has given his answer to {} itself, as his own words: don't pass it on. Follow up \
                     there only on what it does next.",
                    ask.session
                ),
                Err(e) => format!(
                    "\n\nThe deck couldn't give his answer to {} itself ({e}): pass it on when the session is back, \
                     saying it's his answer from the Work tab.",
                    ask.session
                ),
            }
        }
        None => String::new(),
    };
    let body = match a.kind {
        AnswerKind::Choice if panel => format!(
            "On the report (version {}), his answer to the question \"{}\" [{}]: {value}\n\nIt's in the question's \
             own thread{}. Act on it, update the report where it changes things (take the question out, or rewrite it, \
             once it's settled), and answer him in that thread: `quietdeck report reply '{}' \"…\"` (what you did, or \
             what you still need to know).",
            a.report_version,
            a.about.trim(),
            a.control,
            if earlier > 0 { ", continuing the conversation there (`quietdeck report show` lists it)" } else { "" },
            a.control
        ),
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
        AnswerKind::Change if ask_chat => format!(
            "On the report (version {}), a message about the question \"{}\" [{}], instead of picking an answer: \
             {value}\n\nIt's in the question's own thread{}. It isn't his answer: the question stays open. Answer him \
             in that thread (`quietdeck report reply '{}' \"…\"`), and change the question or its options in the \
             report where the conversation settles it.",
            a.report_version,
            a.about.trim(),
            a.control,
            if earlier > 0 { ", continuing the conversation there (`quietdeck report show` lists it)" } else { "" },
            a.control
        ),
        AnswerKind::Change if earlier > 0 => format!(
            "On the report (version {}), in the thread on the default \"{}\" [{}]: {value}\n\nIt continues that \
             conversation (`quietdeck report show` lists it). Like every message he writes in the Inbox, it's an \
             instruction to update this entry, not to carry anything out now. Update the cards and the default to \
             match, answer him in the thread (`quietdeck report reply '{}' \"…\"`), and wait for his approval.",
            a.report_version,
            a.about.trim(),
            a.control,
            a.control
        ),
        AnswerKind::Change => format!(
            "On the report (version {}), a change to the default \"{}\" [{}]: {value}\n\nUsually it adjusts the \
             default or adds to it rather than replacing it. Like every message he writes in the Inbox, it's an \
             instruction to update this entry, not to carry anything out now. Post the cards' new versions with \
             actions to match, change the default's text in the report, answer him in the default's thread \
             (`quietdeck report reply '{}' \"…\"`: what you changed, or a question), and wait for his approval.",
            a.report_version,
            a.about.trim(),
            a.control,
            a.control
        ),
        AnswerKind::Reply => unreachable!("refused above"),
    } + &direct_note;
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
    current(&state.db, doc).await?.ok_or_else(|| AppError::Unavailable("the report vanished".into()))
}

/// What a session reads when he answers its question on the Work tab: the question as he saw it and his answer,
/// delivered by the deck (through Fleetlayer as the Quietdeck service), never by another session.
fn to_session(ask: &quietdeck_types::report::SessionAsk, value: &str) -> String {
    let options =
        if ask.options.is_empty() { String::new() } else { format!(" (options: {})", ask.options.join(", ")) };
    format!(
        "The deck's owner answered your question on the Work tab of his Quietdeck deck, signed in there. The deck \
         delivers it directly, not through another session. If what it asks is something you'd only do on his word \
         in this tab, say so here and wait for him.\n\nThe question, as he saw it: \"{}\"{options}\nHis answer: {value}",
        ask.text
    )
}

/// He sent mail on these Gmail threads himself: every open report's `<Ask thread="…">` that names one of them and
/// has no answer yet is answered for him, as if he'd pressed its button ("Done" when the question offers it), so the
/// agent that wrote it takes the entry off (Nathan, 28 Nov: "if i send it myself in Gmail … mark this off the list
/// automatically"). Returns how many were answered.
pub async fn sent_threads(state: &AppState, threads: &[String]) -> AppResult<usize> {
    if threads.is_empty() {
        return Ok(0);
    }
    let rows: Vec<ReportRow> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("{ROW} WHERE closed_at IS NULL ORDER BY id")))
            .fetch_all(&state.db)
            .await?;
    let mut answered = 0;
    for r in rows {
        for ask in quietdeck_types::report::sent_asks(&r.markdown) {
            if !ask.threads.iter().any(|t| threads.contains(t)) {
                continue;
            }
            let (already,): (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM report_answers WHERE report_id = ? AND control = ? AND kind = 'choice'",
            )
            .bind(r.id)
            .bind(&ask.key)
            .fetch_one(&state.db)
            .await?;
            if already > 0 {
                continue;
            }
            let value = ask
                .options
                .iter()
                .find(|o| o.eq_ignore_ascii_case("done"))
                .cloned()
                .unwrap_or_else(|| "Sent: he sent it in Gmail".to_string());
            let add = ReportAnswerAdd {
                report_id: r.id,
                report_version: r.version,
                control: ask.key.clone(),
                kind: AnswerKind::Choice,
                about: ask.text.clone(),
                value,
                cards: None,
            };
            match answer(state, add).await {
                Ok(_) => answered += 1,
                Err(e) => tracing::warn!(report = r.id, ask = %ask.key, error = %e, "answering an Ask for sent mail"),
            }
        }
    }
    Ok(answered)
}

/// `quietdeck report answers`: what the agent hasn't had yet, each once.
pub async fn take_answers(state: &AppState, doc: &Document) -> AppResult<Vec<ReportAnswer>> {
    let rows: Vec<AnswerRow> = sqlx::query_as(
        "UPDATE report_answers SET picked_up_at = ? WHERE picked_up_at IS NULL \
         AND report_id IN (SELECT id FROM reports WHERE document_id = ?) \
         RETURNING id, report_version, control, kind, about, value, answered_at, picked_up_at, cards",
    )
    .bind(now_ms())
    .bind(doc.id)
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
/// carry it (still there, or one he answered on this report before the agent took it off), or a card it names (`c_12`,
/// kept as `cards:c_12`: the Inbox puts it in the thread of whichever default names that card).
pub async fn reply(state: &AppState, doc: &Document, req: ReportReply) -> AppResult<Report> {
    req.validate().map_err(|e| AppError::BadRequest(e.to_string()))?;
    let Some(r) = open_row(&state.db, doc.id).await? else {
        return Err(AppError::NotFound("there's no report to reply on".into()));
    };
    let to = req.to.trim();
    let keys = quietdeck_types::report::default_keys(&r.markdown);
    let asks = quietdeck_types::report::ask_keys(&r.markdown);
    // A default he commented on keeps its thread after the agent takes it off the report, so his answers name it too.
    let answered: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM report_answers WHERE report_id = ? AND control = ? LIMIT 1")
            .bind(r.id)
            .bind(to)
            .fetch_optional(&state.db)
            .await?;
    let control = if keys.iter().any(|(k, _)| k == to)
        || asks.iter().any(|(k, _)| k == to)
        || to.starts_with("cards:")
        || answered.is_some()
    {
        to.to_string()
    } else if !quietdeck_types::report::card_ids(to).is_empty() {
        format!("cards:{to}")
    } else {
        return Err(AppError::BadRequest(format!(
            "no default or question {to:?} in the report: give the [key] from his message (cards:c_12, archive-all, a \
             question's id) or a card (c_12)"
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
    current(&state.db, doc.id).await?.ok_or_else(|| AppError::Unavailable("the report vanished".into()))
}

/// The chat box: the words go to the report's agent now, or fail at once when it isn't running. Nothing is kept.
pub async fn chat(state: &AppState, doc: &Document, c: ReportChat) -> AppResult<Delivered> {
    c.validate().map_err(|e| AppError::BadRequest(e.to_string()))?;
    // A document with a writer reaches it even before its first report.
    let to = match (open_row(&state.db, doc.id).await?, &doc.writer_node, &doc.writer_tab) {
        (_, Some(node), Some(tab)) => format!("{tab}@{node}"),
        (Some(r), _, _) => format!("{}@{}", r.posted_tab, r.posted_node),
        (None, _, _) => {
            return Err(AppError::NotFound("no agent has written this document yet, so there's nobody to tell".into()));
        }
    };
    let body = if doc.id == INBOX_ID {
        format!("From the Inbox's chat box: {}", c.text.trim())
    } else {
        format!("From the chat box under the {} document: {}", doc.name, c.text.trim())
    };
    crate::relay::send(&state.db, &state.hub, &to, DeliveryKind::Deck, body).await.map_err(|e| match e {
        DeliveryError::SessionOffline { .. } | DeliveryError::NodeOffline { .. } => {
            AppError::Offline(format!("{to} isn't running, so it didn't get that."))
        }
        DeliveryError::SessionNotFound { .. } => AppError::NotFound(e.to_string()),
        DeliveryError::Ambiguous { .. } => AppError::BadRequest(e.to_string()),
        DeliveryError::Failed { .. } => AppError::Unavailable(e.to_string()),
    })
}

/// How far back a card's changes are looked for: a card waits days at most.
const CHANGES_WINDOW_MS: i64 = 30 * 24 * 3600 * 1000;

/// The person's changes to a card's default, oldest first: those on a `<Default>` that names it in the current
/// report (by its `id` or its `cards`), and any `cards:` change that names it, whatever the report says now.
pub async fn changes_for_card(db: &Db, card_id: i64) -> anyhow::Result<Vec<String>> {
    if !enabled(db).await? {
        return Ok(vec![]);
    }
    let markdown: Vec<(String,)> =
        sqlx::query_as("SELECT markdown FROM reports WHERE closed_at IS NULL ORDER BY id").fetch_all(db).await?;
    let keys: Vec<String> = markdown
        .iter()
        .flat_map(|(m,)| quietdeck_types::report::default_keys(m))
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
    let Some(report) = current(db, INBOX_ID).await? else { return Ok(()) };
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
    if !enabled(db).await? {
        return Ok(vec![]);
    }
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
    async fn a_chat_reply_shows_with_the_reports_changes_as_a_reply() {
        let state = AppState::for_tests().await;
        assert!(chat_reply(&state, &inbox(&state.db).await.unwrap(), "Done").await.is_err(), "no report to show it on");
        let set = ReportSet { markdown: "# Inbox".into(), base: None, new: true, change: None };
        super::set(&state, &me(), &inbox(&state.db).await.unwrap(), set).await.unwrap();
        assert!(chat_reply(&state, &inbox(&state.db).await.unwrap(), "  ").await.is_err());
        chat_reply(&state, &inbox(&state.db).await.unwrap(), "I'll keep the Acme thread in your inbox until Friday.")
            .await
            .unwrap();
        let r = current(&state.db, INBOX_ID).await.unwrap().unwrap();
        let last = r.changes.last().unwrap();
        assert!(last.reply && last.cards.is_empty());
        assert_eq!(last.summary, "I'll keep the Acme thread in your inbox until Friday.");
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
        let r = super::set(&state, &me(), &inbox(&state.db).await.unwrap(), set("# Round", None, false)).await.unwrap();
        assert_eq!((r.version, r.markdown.as_str()), (1, "# Round"));
        let r = super::set(&state, &me(), &inbox(&state.db).await.unwrap(), set("# Round\n- more", Some(1), false))
            .await
            .unwrap();
        assert_eq!(r.version, 2);
        let stale = super::set(&state, &me(), &inbox(&state.db).await.unwrap(), set("# Old", Some(1), false))
            .await
            .unwrap_err();
        assert!(stale.to_string().contains("version 2"), "{stale}");
        let missing = super::set(
            &state,
            &me(),
            &inbox(&state.db).await.unwrap(),
            set("<Default cards=\"c_99\">archive</Default>", Some(2), false),
        )
        .await
        .unwrap_err();
        assert!(missing.to_string().contains("no card c_99"), "{missing}");
        let fresh =
            super::set(&state, &me(), &inbox(&state.db).await.unwrap(), set("# Next round", None, true)).await.unwrap();
        assert_ne!(fresh.id, r.id);
        assert_eq!(fresh.version, 1);
    }

    #[tokio::test]
    async fn approve_report_takes_the_cards_of_every_default_he_didnt_change() {
        let state = AppState::for_tests().await;
        let card = |thread: &str| -> quietdeck_types::card::CardInput {
            serde_json::from_value(serde_json::json!({"account": "personal", "thread_key": thread, "subject": "s",
                "summary": "x", "section": "actions", "reason": "r",
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
        let md = format!(
            "1. One\n   <Default cards=\"c_{a} c_{b}\">Archive both</Default>\n2. Two\n   <Default cards=\"c_{c}\">Archive</Default>\n",
            a = ids[0],
            b = ids[1],
            c = ids[2]
        );
        let r = super::set(
            &state,
            &me(),
            &inbox(&state.db).await.unwrap(),
            ReportSet { markdown: md, base: None, new: true, change: None },
        )
        .await
        .unwrap();
        // He changed the second default: its card waits for the agent.
        let r = answer(
            &state,
            quietdeck_types::report::ReportAnswerAdd {
                report_id: r.id,
                report_version: r.version,
                control: format!("cards:c_{}", ids[2]),
                kind: AnswerKind::Change,
                about: "Archive".into(),
                value: "forward it to api instead".into(),
                cards: None,
            },
        )
        .await
        .unwrap();
        let req = |version| quietdeck_types::decision::ApproveReport {
            report_id: r.id,
            report_version: version,
            timeline: vec![],
        };
        let stale = crate::ops::approve_report(&state, req(r.version - 1)).await;
        assert!(matches!(stale, Err(AppError::Conflict(_))), "an older version on screen is refused");
        let a = crate::ops::approve_report(&state, req(r.version)).await.unwrap();
        let mut took = a.cards.clone();
        took.sort_unstable();
        assert_eq!(took, vec![ids[0], ids[1]], "the changed default's card isn't approved");
    }

    #[tokio::test]
    async fn approve_all_passes_question_panels_by_and_the_agent_replies_in_their_thread() {
        let state = AppState::for_tests().await;
        let input: quietdeck_types::card::CardInput = serde_json::from_value(serde_json::json!({"account": "personal",
            "thread_key": "t1", "subject": "s", "summary": "x", "section": "actions", "reason": "r",
            "actions": [{"type": "archive", "account": "personal", "thread_id": "t1"}]}))
        .unwrap();
        let card = crate::store::cards::add(&state.db, &me(), &input, quietdeck_types::card::CardState::Open, None)
            .await
            .unwrap()
            .card;
        let md = format!(
            "1. One\n   <Default cards=\"c_{id}\">Archive</Default>\n2. Venue\n   \
             <Ask id=\"budget\">How much can I spend?</Ask>\n\n<Ask id=\"me\" options=\"Yes|No\">Was this you?</Ask>\n",
            id = card.id
        );
        let r = super::set(
            &state,
            &me(),
            &inbox(&state.db).await.unwrap(),
            ReportSet { markdown: md, base: None, new: true, change: None },
        )
        .await
        .unwrap();
        answer(
            &state,
            quietdeck_types::report::ReportAnswerAdd {
                report_id: r.id,
                report_version: r.version,
                control: "budget".into(),
                kind: AnswerKind::Choice,
                about: "How much can I spend?".into(),
                value: "Up to $2k".into(),
                cards: None,
            },
        )
        .await
        .unwrap();
        let r = reply(
            &state,
            &inbox(&state.db).await.unwrap(),
            ReportReply { to: "budget".into(), text: "Booked at $1.8k".into() },
        )
        .await
        .unwrap();
        let thread: Vec<_> = r.answers.iter().filter(|a| a.control == "budget").map(|a| a.kind).collect();
        assert_eq!(thread, vec![AnswerKind::Choice, AnswerKind::Reply]);
        let bad = reply(&state, &inbox(&state.db).await.unwrap(), ReportReply { to: "nope".into(), text: "x".into() })
            .await
            .unwrap_err();
        assert!(bad.to_string().contains("question"), "{bad}");
        // He'd rather talk "Was this you?" over than pick: a message in its chat, which doesn't hold up Approve All.
        answer(
            &state,
            quietdeck_types::report::ReportAnswerAdd {
                report_id: r.id,
                report_version: r.version,
                control: "me".into(),
                kind: AnswerKind::Change,
                about: "Was this you?".into(),
                value: "Which device was it?".into(),
                cards: None,
            },
        )
        .await
        .unwrap();
        let r =
            reply(&state, &inbox(&state.db).await.unwrap(), ReportReply { to: "me".into(), text: "An iPad".into() })
                .await
                .unwrap();
        let thread: Vec<_> = r.answers.iter().filter(|a| a.control == "me").map(|a| a.kind).collect();
        assert_eq!(thread, vec![AnswerKind::Change, AnswerKind::Reply]);
        let req =
            quietdeck_types::decision::ApproveReport { report_id: r.id, report_version: r.version, timeline: vec![] };
        let a = crate::ops::approve_report(&state, req).await.unwrap();
        assert_eq!(a.cards, vec![card.id], "the default's card, and nothing for either question");
    }

    #[tokio::test]
    async fn mail_he_sends_himself_answers_the_question_waiting_for_it_once() {
        let state = AppState::for_tests().await;
        let md = "- <Ask id=\"patronus\" options=\"Done\" thread=\"t1\">Done when you've sent the Patronus reply</Ask>\n\
                  - <Ask id=\"fp\" thread=\"t2\">Send the FinancePals reply?</Ask>\n\
                  - <Ask id=\"other\" options=\"Done\">Done when the key is in</Ask>\n";
        let r = super::set(
            &state,
            &me(),
            &inbox(&state.db).await.unwrap(),
            ReportSet { markdown: md.into(), base: None, new: true, change: None },
        )
        .await
        .unwrap();
        assert_eq!(sent_threads(&state, &["t9".into()]).await.unwrap(), 0, "another thread answers nothing");
        assert_eq!(sent_threads(&state, &["t1".into(), "t2".into()]).await.unwrap(), 2);
        assert_eq!(sent_threads(&state, &["t1".into()]).await.unwrap(), 0, "answered once");
        let r = current(&state.db, r.document_id).await.unwrap().unwrap();
        let got: Vec<(&str, &str)> = r.answers.iter().map(|a| (a.control.as_str(), a.value.as_str())).collect();
        assert_eq!(got, [("patronus", "Done"), ("fp", "Sent: he sent it in Gmail")]);
    }

    #[tokio::test]
    async fn a_work_document_sits_beside_the_inbox_and_approve_all_clears_its_fyi() {
        let state = AppState::for_tests().await;
        let overseer = SessionRef { node: "agents".into(), tab: "overseer".into(), session_id: None };
        let work = crate::documents::add(
            &state,
            quietdeck_types::document::DocumentAdd {
                slug: "work".into(),
                name: "Work".into(),
                tab: "Work".into(),
                kind: DocumentKind::Work,
                writer_node: Some("agents".into()),
                writer_tab: Some("overseer".into()),
                position: None,
            },
        )
        .await
        .unwrap();
        let input = |thread: &str| -> quietdeck_types::card::CardInput {
            serde_json::from_value(serde_json::json!({"account": "personal", "thread_key": thread, "subject": "s",
                "summary": "x", "section": "actions", "reason": "r",
                "actions": [{"type": "archive", "account": "personal", "thread_id": thread}]}))
            .unwrap()
        };
        let open = quietdeck_types::card::CardState::Open;
        let mail = crate::store::cards::add(&state.db, &me(), &input("t1"), open, None).await.unwrap().card;
        let session = crate::store::cards::add(&state.db, &overseer, &input("t2"), open, None).await.unwrap().card;
        let inbox_doc = inbox(&state.db).await.unwrap();
        super::set(
            &state,
            &me(),
            &inbox_doc,
            ReportSet { markdown: "# Inbox".into(), base: None, new: true, change: None },
        )
        .await
        .unwrap();
        // The triage agent can't write Work; its writer can.
        assert!(
            super::set(
                &state,
                &me(),
                &work,
                ReportSet { markdown: "# x".into(), base: None, new: false, change: None }
            )
            .await
            .is_err()
        );
        let md = format!(
            "# Needs you\n\n1. **enterprise:** wrap it up?\n   <Default cards=\"c_{}\">Wrap up session</Default>\n\n\
             ## FYI\n\n- `ci` finished: green.\n- `docs` shipped.\n",
            session.id
        );
        let w = super::set(&state, &overseer, &work, ReportSet { markdown: md, base: None, new: false, change: None })
            .await
            .unwrap();
        assert_eq!(w.document_id, work.id);
        assert!(w.cards_not_in_report.is_empty());
        // Each document lists only its own unnamed cards.
        let i = current(&state.db, INBOX_ID).await.unwrap().unwrap();
        assert_eq!(i.cards_not_in_report, vec![mail.id]);
        assert_eq!(i.markdown, "# Inbox");
        // A Work document is never replaced.
        let again = ReportSet { markdown: "# new".into(), base: None, new: true, change: None };
        assert!(super::set(&state, &overseer, &work, again).await.is_err());
        let req =
            quietdeck_types::decision::ApproveReport { report_id: w.id, report_version: w.version, timeline: vec![] };
        let a = crate::ops::approve_report(&state, req).await.unwrap();
        assert_eq!(a.cards, vec![session.id]);
        let w = current(&state.db, work.id).await.unwrap().unwrap();
        assert!(!w.markdown.contains("`ci` finished"), "{}", w.markdown);
        assert!(
            !w.markdown.contains("## FYI") && w.markdown.contains("Wrap up session"),
            "an empty FYI loses its heading"
        );
        assert!(snapshots_between(&state.db, 0, i64::MAX).await.unwrap().is_empty(), "History keeps the Inbox only");
    }

    #[tokio::test]
    async fn each_item_keeps_the_time_its_text_last_changed() {
        let state = AppState::for_tests().await;
        let write = |md: &'static str, now: i64| {
            let db = state.db.clone();
            async move {
                let mut tx = db.begin().await.unwrap();
                track_items(&mut tx, 7, md, now).await.unwrap();
                tx.commit().await.unwrap();
            }
        };
        write("# Needs you\n\n1. a\n2. b\n", 1_000).await;
        write("# Needs you\n\n1. a\n2. b, reworded\n3. c\n", 5_000).await;
        let times =
            item_times(&state.db, 7, "# Needs you\n\n1. a\n2. b, reworded\n3. c\n- unknown\n", 9_000).await.unwrap();
        let ms: Vec<i64> = times.iter().map(|t| t.as_millisecond()).collect();
        assert_eq!(ms, vec![1_000, 5_000, 5_000, 9_000]);
        let (left,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM report_items WHERE document_id = 7")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(left, 3, "the old wording of b is forgotten");
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
        let r = super::set(
            &state,
            &me(),
            &inbox(&state.db).await.unwrap(),
            ReportSet { markdown: md, base: None, new: false, change: None },
        )
        .await
        .unwrap();
        assert_eq!(r.cards_not_in_report, vec![ids[1], ids[2]]);
        let md = format!("<Default cards=\"c_{} c_{} c_{}\">archive</Default>", ids[0], ids[1], ids[2]);
        let r = super::set(
            &state,
            &me(),
            &inbox(&state.db).await.unwrap(),
            ReportSet { markdown: md, base: Some(1), new: false, change: None },
        )
        .await
        .unwrap();
        assert!(r.cards_not_in_report.is_empty());

        let mut forward = card("t4");
        forward.actions.insert(
            0,
            serde_json::from_value(serde_json::json!({"type": "forward", "to": {"node": "n", "tab": "ci"},
                "text": "see thread t4"}))
            .unwrap(),
        );
        let fwd = crate::store::cards::add(&state.db, &me(), &forward, quietdeck_types::card::CardState::Open, None)
            .await
            .unwrap()
            .card
            .id;
        let md = format!("<Default id=\"archive-all\" cards=\"c_{} c_{fwd}\">Archive all</Default>", ids[0]);
        let refused = super::set(
            &state,
            &me(),
            &inbox(&state.db).await.unwrap(),
            ReportSet { markdown: md, base: Some(2), new: false, change: None },
        )
        .await
        .unwrap_err();
        assert!(refused.to_string().contains(&format!("c_{fwd} has forward")), "{refused}");
        let md = format!(
            "<Default cards=\"c_{fwd}\">Forward to `ci`, then archive</Default>\n\
             <Default id=\"archive-all\" cards=\"c_{} c_{} c_{}\">Archive all</Default>",
            ids[0], ids[1], ids[2]
        );
        super::set(
            &state,
            &me(),
            &inbox(&state.db).await.unwrap(),
            ReportSet { markdown: md, base: Some(2), new: false, change: None },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn the_report_holds_still_while_its_approve_all_is_carried_out() {
        let state = AppState::for_tests().await;
        let input: quietdeck_types::card::CardInput = serde_json::from_value(serde_json::json!({
            "account": "personal", "thread_key": "t1", "subject": "s", "summary": "x", "section": "archive_list",
            "reason": "r", "actions": [{"type": "archive", "account": "personal", "thread_id": "t1"}]}))
        .unwrap();
        let c = crate::store::cards::add(&state.db, &me(), &input, quietdeck_types::card::CardState::Open, None)
            .await
            .unwrap()
            .card;
        let set = |markdown: String, base: Option<i64>| ReportSet { markdown, base, new: false, change: None };
        let r = super::set(
            &state,
            &me(),
            &inbox(&state.db).await.unwrap(),
            set(format!("<Default cards=\"c_{}\">Archive</Default>", c.id), None),
        )
        .await
        .unwrap();
        let req =
            quietdeck_types::decision::ApproveReport { report_id: r.id, report_version: r.version, timeline: vec![] };
        let a = crate::ops::approve_report(&state, req).await.unwrap();
        assert_eq!(a.cards, vec![c.id]);
        let refused =
            super::set(&state, &me(), &inbox(&state.db).await.unwrap(), set("# New mail".into(), Some(r.version)))
                .await
                .unwrap_err();
        assert!(matches!(refused, AppError::Conflict(_)), "{refused}");
        assert!(refused.to_string().contains("Approve All"), "{refused}");
        crate::ops::undo(&state, &a.batch).await.unwrap();
        super::set(&state, &me(), &inbox(&state.db).await.unwrap(), set("# New mail".into(), Some(r.version)))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn approve_all_keeps_a_snapshot_and_one_defaults_approve_does_not() {
        let state = AppState::for_tests().await;
        let input: quietdeck_types::card::CardInput = serde_json::from_value(serde_json::json!({
            "account": "personal", "thread_key": "t1", "subject": "s", "summary": "x", "section": "archive_list",
            "reason": "r", "actions": [{"type": "archive", "account": "personal", "thread_id": "t1"}]}))
        .unwrap();
        let mut ids = vec![];
        for t in ["t1", "t2"] {
            let mut i = input.clone();
            i.thread_key = t.into();
            let c = crate::store::cards::add(&state.db, &me(), &i, quietdeck_types::card::CardState::Open, None)
                .await
                .unwrap()
                .card;
            ids.push((c.id, c.version));
        }
        let md = format!("<Default id=\"archive-all\" cards=\"c_{} c_{}\">Archive all</Default>", ids[0].0, ids[1].0);
        super::set(
            &state,
            &me(),
            &inbox(&state.db).await.unwrap(),
            ReportSet { markdown: md, base: None, new: false, change: None },
        )
        .await
        .unwrap();
        let decision = |(id, version): (i64, i64)| quietdeck_types::decision::CardDecision {
            card_id: id,
            version,
            kind: quietdeck_types::card::DecisionKind::Approve,
            fields: None,
            text: None,
        };
        let all =
            |single| quietdeck_types::decision::ApproveAll { cards: vec![decision(ids[0])], timeline: vec![], single };
        crate::ops::approve_all(&state, all(Some(true))).await.unwrap();
        let count = || async { snapshots_between(&state.db, 0, i64::MAX).await.unwrap().len() };
        assert_eq!(count().await, 0, "one default's Approve keeps no snapshot");
        let mut whole = all(None);
        whole.cards = vec![decision(ids[1])];
        crate::ops::approve_all(&state, whole).await.unwrap();
        assert_eq!(count().await, 1, "Approve All does");
    }

    #[tokio::test]
    async fn answers_are_kept_and_taken_once_when_the_agent_was_away() {
        let state = AppState::for_tests().await;
        let r = super::set(
            &state,
            &me(),
            &inbox(&state.db).await.unwrap(),
            ReportSet { markdown: "x".into(), base: None, new: false, change: None },
        )
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
        let taken = take_answers(&state, &inbox(&state.db).await.unwrap()).await.unwrap();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].value, "No");
        assert!(take_answers(&state, &inbox(&state.db).await.unwrap()).await.unwrap().is_empty());
        // "signin" isn't in the report any more (it never was here), but he answered it, so its thread takes a reply.
        let r = reply(
            &state,
            &inbox(&state.db).await.unwrap(),
            ReportReply { to: "signin".into(), text: "Signed it out".into() },
        )
        .await
        .unwrap();
        assert_eq!(r.answers.last().map(|a| a.control.as_str()), Some("signin"));
        let chat = chat(
            &state,
            &inbox(&state.db).await.unwrap(),
            ReportChat { text: "archive the HelloFresh ones from now on".into() },
        )
        .await;
        assert!(chat.is_err(), "nobody is running, so the chat fails at once");
    }

    #[tokio::test]
    async fn an_answer_about_cards_joins_their_thread_and_reaches_the_decision() {
        let state = AppState::for_tests().await;
        let markdown = "<Ask id=\"invoice\" options=\"DocSpring, Inc.|Me\" cards=\"c_5\">Who should BizSpace \
                        invoice?</Ask>";
        let r = super::set(
            &state,
            &me(),
            &inbox(&state.db).await.unwrap(),
            ReportSet { markdown: markdown.into(), base: None, new: false, change: None },
        )
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
        let r = super::set(
            &state,
            &me(),
            &inbox(&state.db).await.unwrap(),
            ReportSet { markdown: "# Round".into(), base: None, new: false, change: None },
        )
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
