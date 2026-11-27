//! Documents (quietdeck_types::document): the pages agents write, each under a top-level tab, and the person's marks
//! on sessions (ignored, parked). The Inbox document is id 1 and can't be removed or re-slugged.

use garde::Validate;
use quietdeck_types::SessionRef;
use quietdeck_types::api::Changed;
use quietdeck_types::document::{
    Document, DocumentAdd, DocumentEdit, DocumentKind, INBOX_ID, MarkKind, SessionMark, SessionMarkRemove,
    SessionMarkSet,
};

use crate::db::{Db, now_ms, ts};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    slug: String,
    name: String,
    tab: String,
    kind: String,
    writer_node: Option<String>,
    writer_tab: Option<String>,
    position: i64,
}

impl From<Row> for Document {
    fn from(r: Row) -> Document {
        Document {
            id: r.id,
            slug: r.slug,
            name: r.name,
            tab: r.tab,
            kind: DocumentKind::parse(&r.kind).unwrap_or(DocumentKind::Inbox),
            writer_node: r.writer_node,
            writer_tab: r.writer_tab,
            position: r.position,
        }
    }
}

const COLUMNS: &str = "id, slug, name, tab, kind, writer_node, writer_tab, position";

/// Every live document, in order.
pub async fn list(db: &Db) -> AppResult<Vec<Document>> {
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM documents WHERE removed_at IS NULL ORDER BY position, id"
    )))
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(Document::from).collect())
}

pub async fn get(db: &Db, id: i64) -> AppResult<Document> {
    let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM documents WHERE id = ? AND removed_at IS NULL"
    )))
    .bind(id)
    .fetch_optional(db)
    .await?;
    row.map(Document::from).ok_or_else(|| AppError::NotFound(format!("no document {id}")))
}

/// The document a request is about: `?doc=<slug>` when given; else, for an agent, the one it writes; else the Inbox.
pub async fn resolve(db: &Db, slug: Option<&str>, caller: Option<&SessionRef>) -> AppResult<Document> {
    if let Some(slug) = slug.map(str::trim).filter(|s| !s.is_empty()) {
        let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM documents WHERE slug = ? AND removed_at IS NULL"
        )))
        .bind(slug)
        .fetch_optional(db)
        .await?;
        return row.map(Document::from).ok_or_else(|| {
            AppError::NotFound(format!("no document {slug:?}: `GET /documents` lists them (the Inbox is `inbox`)"))
        });
    }
    if let Some(me) = caller {
        let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM documents WHERE writer_node = ? AND writer_tab = ? AND removed_at IS NULL"
        )))
        .bind(&me.node)
        .bind(&me.tab)
        .fetch_optional(db)
        .await?;
        if let Some(r) = row {
            return Ok(r.into());
        }
    }
    get(db, INBOX_ID).await
}

/// Whether `me` may write `doc`: anyone when it has no writer, else only its writer.
pub fn may_write(doc: &Document, me: &SessionRef) -> AppResult<()> {
    match (&doc.writer_node, &doc.writer_tab) {
        (Some(node), Some(tab)) if node != &me.node || tab != &me.tab => Err(AppError::Forbidden(format!(
            "the {} document is written by {tab}@{node}, not {}",
            doc.name,
            me.address()
        ))),
        _ => Ok(()),
    }
}

fn writer_pair(node: &Option<String>, tab: &Option<String>) -> AppResult<()> {
    if node.is_some() != tab.is_some() {
        return Err(AppError::BadRequest("give the writer's node and tab together".into()));
    }
    Ok(())
}

fn unique(e: sqlx::Error, what: &str) -> AppError {
    match &e {
        sqlx::Error::Database(d) if d.message().contains("UNIQUE") => AppError::Conflict(what.to_string()),
        _ => e.into(),
    }
}

pub async fn add(state: &AppState, req: DocumentAdd) -> AppResult<Document> {
    req.validate().map_err(|e| AppError::BadRequest(e.to_string()))?;
    writer_pair(&req.writer_node, &req.writer_tab)?;
    let position = match req.position {
        Some(p) => p,
        None => {
            let (max,): (Option<i64>,) = sqlx::query_as("SELECT MAX(position) FROM documents WHERE removed_at IS NULL")
                .fetch_one(&state.db)
                .await?;
            max.unwrap_or(0) + 1
        }
    };
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO documents (slug, name, tab, kind, writer_node, writer_tab, position, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(&req.slug)
    .bind(req.name.trim())
    .bind(req.tab.trim())
    .bind(req.kind.as_str())
    .bind(&req.writer_node)
    .bind(&req.writer_tab)
    .bind(position)
    .bind(now_ms())
    .fetch_one(&state.db)
    .await
    .map_err(|e| unique(e, "a document has that slug already, or that agent writes another document"))?;
    state.changed(Changed::Documents);
    get(&state.db, id).await
}

pub async fn edit(state: &AppState, id: i64, req: DocumentEdit) -> AppResult<Document> {
    req.validate().map_err(|e| AppError::BadRequest(e.to_string()))?;
    writer_pair(&req.writer_node, &req.writer_tab)?;
    let doc = get(&state.db, id).await?;
    let clear = req.clear_writer == Some(true);
    if clear && req.writer_node.is_some() {
        return Err(AppError::BadRequest("give a writer or clear_writer, not both".into()));
    }
    let (writer_node, writer_tab) = match (clear, &req.writer_node) {
        (true, _) => (None, None),
        (false, Some(_)) => (req.writer_node.clone(), req.writer_tab.clone()),
        (false, None) => (doc.writer_node.clone(), doc.writer_tab.clone()),
    };
    sqlx::query("UPDATE documents SET name = ?, tab = ?, writer_node = ?, writer_tab = ?, position = ? WHERE id = ?")
        .bind(req.name.as_deref().map(str::trim).unwrap_or(&doc.name))
        .bind(req.tab.as_deref().map(str::trim).unwrap_or(&doc.tab))
        .bind(writer_node)
        .bind(writer_tab)
        .bind(req.position.unwrap_or(doc.position))
        .bind(id)
        .execute(&state.db)
        .await
        .map_err(|e| unique(e, "that agent writes another document already"))?;
    state.changed(Changed::Documents);
    get(&state.db, id).await
}

pub async fn remove(state: &AppState, id: i64) -> AppResult<()> {
    if id == INBOX_ID {
        return Err(AppError::Conflict("the Inbox document can't be removed".into()));
    }
    get(&state.db, id).await?;
    sqlx::query("UPDATE documents SET removed_at = ? WHERE id = ?").bind(now_ms()).bind(id).execute(&state.db).await?;
    sqlx::query("UPDATE reports SET closed_at = ? WHERE document_id = ? AND closed_at IS NULL")
        .bind(now_ms())
        .bind(id)
        .execute(&state.db)
        .await?;
    state.changed(Changed::Documents);
    Ok(())
}

// Session marks.

#[derive(sqlx::FromRow)]
struct MarkRow {
    node: String,
    tab: String,
    mark: String,
    until: Option<i64>,
    note: Option<String>,
    at: i64,
}

/// Every mark still in force: a parked session whose `until` has passed is back, and its mark goes.
pub async fn marks(db: &Db) -> AppResult<Vec<SessionMark>> {
    sqlx::query("DELETE FROM session_marks WHERE until IS NOT NULL AND until <= ?").bind(now_ms()).execute(db).await?;
    let rows: Vec<MarkRow> =
        sqlx::query_as("SELECT node, tab, mark, until, note, at FROM session_marks ORDER BY at").fetch_all(db).await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            Some(SessionMark {
                node: r.node,
                tab: r.tab,
                mark: MarkKind::parse(&r.mark)?,
                until: r.until.map(ts),
                note: r.note,
                at: ts(r.at),
            })
        })
        .collect())
}

pub async fn mark_set(state: &AppState, req: SessionMarkSet) -> AppResult<Vec<SessionMark>> {
    req.validate().map_err(|e| AppError::BadRequest(e.to_string()))?;
    let until = req.until.map(|t| t.as_millisecond());
    if req.mark == MarkKind::Ignored && until.is_some() {
        return Err(AppError::BadRequest("only a parked session takes `until`".into()));
    }
    if until.is_some_and(|u| u <= now_ms()) {
        return Err(AppError::BadRequest("`until` is in the past".into()));
    }
    sqlx::query(
        "INSERT INTO session_marks (node, tab, mark, until, note, at) VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT (node, tab, mark) DO UPDATE SET until = excluded.until, note = excluded.note, at = excluded.at",
    )
    .bind(&req.node)
    .bind(&req.tab)
    .bind(req.mark.as_str())
    .bind(until)
    .bind(req.note.as_deref().map(str::trim).filter(|n| !n.is_empty()))
    .bind(now_ms())
    .execute(&state.db)
    .await?;
    state.changed(Changed::SessionMarks);
    marks(&state.db).await
}

pub async fn mark_remove(state: &AppState, req: SessionMarkRemove) -> AppResult<Vec<SessionMark>> {
    req.validate().map_err(|e| AppError::BadRequest(e.to_string()))?;
    sqlx::query("DELETE FROM session_marks WHERE node = ? AND tab = ? AND mark = ?")
        .bind(&req.node)
        .bind(&req.tab)
        .bind(req.mark.as_str())
        .execute(&state.db)
        .await?;
    state.changed(Changed::SessionMarks);
    marks(&state.db).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me(tab: &str) -> SessionRef {
        SessionRef { node: "agents".into(), tab: tab.into(), session_id: None }
    }

    fn work() -> DocumentAdd {
        DocumentAdd {
            slug: "work".into(),
            name: "Work".into(),
            tab: "Work".into(),
            kind: DocumentKind::Work,
            writer_node: Some("agents".into()),
            writer_tab: Some("overseer".into()),
            position: None,
        }
    }

    #[tokio::test]
    async fn an_agent_writes_its_own_document_and_everyone_else_gets_the_inbox() {
        let state = AppState::for_tests().await;
        let docs = list(&state.db).await.unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!((docs[0].id, docs[0].slug.as_str(), docs[0].kind), (INBOX_ID, "inbox", DocumentKind::Inbox));
        let w = add(&state, work()).await.unwrap();
        assert_eq!(resolve(&state.db, None, Some(&me("overseer"))).await.unwrap().id, w.id);
        assert_eq!(resolve(&state.db, None, Some(&me("email triage"))).await.unwrap().id, INBOX_ID);
        assert_eq!(resolve(&state.db, None, None).await.unwrap().id, INBOX_ID);
        assert_eq!(resolve(&state.db, Some("work"), None).await.unwrap().id, w.id);
        assert!(resolve(&state.db, Some("nope"), None).await.is_err());
        assert!(may_write(&w, &me("email triage")).is_err());
        may_write(&w, &me("overseer")).unwrap();
        // One live document per slug and per writer.
        let again = add(&state, work()).await.unwrap_err();
        assert!(matches!(again, AppError::Conflict(_)), "{again}");
        // The Inbox stays.
        assert!(matches!(remove(&state, INBOX_ID).await, Err(AppError::Conflict(_))));
        let edited = edit(&state, w.id, DocumentEdit { clear_writer: Some(true), ..Default::default() }).await.unwrap();
        assert_eq!(edited.writer_node, None);
        remove(&state, w.id).await.unwrap();
        add(&state, work()).await.expect("a removed slug can be used again");
    }

    #[tokio::test]
    async fn a_snoozed_session_comes_back_on_its_own() {
        let state = AppState::for_tests().await;
        let set = |mark, until| SessionMarkSet {
            node: "agents".into(),
            tab: "enterprise".into(),
            mark,
            until,
            note: Some("SSO work".into()),
        };
        mark_set(&state, set(MarkKind::Ignored, None)).await.unwrap();
        let later = jiff::Timestamp::now() + jiff::SignedDuration::from_hours(24);
        let m = mark_set(&state, set(MarkKind::Parked, Some(later))).await.unwrap();
        assert_eq!(m.len(), 2);
        assert!(mark_set(&state, set(MarkKind::Ignored, Some(later))).await.is_err());
        // Its time passes.
        sqlx::query("UPDATE session_marks SET until = 1 WHERE mark = 'parked'").execute(&state.db).await.unwrap();
        let m = marks(&state.db).await.unwrap();
        assert_eq!(m.iter().map(|m| m.mark).collect::<Vec<_>>(), vec![MarkKind::Ignored]);
        let m = mark_remove(
            &state,
            SessionMarkRemove { node: "agents".into(), tab: "enterprise".into(), mark: MarkKind::Ignored },
        )
        .await
        .unwrap();
        assert!(m.is_empty());
    }
}
