//! The person's comments on timeline entries and History rows. Each goes to the session behind what it's on: the
//! card's poster, or the session that posted the entry.

use quietdeck_types::SessionRef;
use quietdeck_types::timeline::{Comment, CommentTarget};

use crate::db::{Db, now_ms, ts, ts_opt};
use crate::error::AppError;

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    target_type: String,
    target_id: i64,
    text: String,
    to_node: Option<String>,
    to_tab: Option<String>,
    created_at: i64,
    picked_up_at: Option<i64>,
}

async fn hydrate(db: &Db, r: Row) -> anyhow::Result<Comment> {
    let target = match r.target_type.as_str() {
        "card" => CommentTarget::Card(r.target_id),
        _ => CommentTarget::Timeline(r.target_id),
    };
    let (on, _) = describe(db, target).await?.unwrap_or_else(|| ("(deleted)".into(), None));
    Ok(Comment {
        id: r.id,
        target,
        on,
        text: r.text,
        to: match (r.to_node, r.to_tab) {
            (Some(node), Some(tab)) => Some(SessionRef { node, tab, session_id: None }),
            _ => None,
        },
        created_at: ts(r.created_at),
        picked_up_at: ts_opt(r.picked_up_at),
    })
}

/// What a target is, in a line, and the session behind it.
async fn describe(db: &Db, target: CommentTarget) -> anyhow::Result<Option<(String, Option<SessionRef>)>> {
    let row: Option<(String, Option<String>, Option<String>)> = match target {
        CommentTarget::Card(id) => {
            sqlx::query_as("SELECT json_extract(input, '$.subject'), posted_node, posted_tab FROM cards WHERE id = ?")
                .bind(id)
                .fetch_optional(db)
                .await?
        }
        CommentTarget::Timeline(id) => {
            sqlx::query_as("SELECT text, posted_node, posted_tab FROM timeline_entries WHERE id = ?")
                .bind(id)
                .fetch_optional(db)
                .await?
        }
    };
    Ok(row.map(|(on, node, tab)| {
        let to = match (node, tab) {
            (Some(node), Some(tab)) => Some(SessionRef { node, tab, session_id: None }),
            _ => None,
        };
        (on, to)
    }))
}

/// Store a comment for the session behind its target, or for `fallback` when nothing posted it (the deck's own
/// entries).
pub async fn add(
    db: &Db,
    target: CommentTarget,
    text: &str,
    fallback: Option<&SessionRef>,
) -> Result<Comment, AppError> {
    let (_, to) =
        describe(db, target).await?.ok_or_else(|| AppError::NotFound("that entry or card doesn't exist".into()))?;
    let to = to.or_else(|| fallback.cloned());
    let (kind, id) = match target {
        CommentTarget::Card(id) => ("card", id),
        CommentTarget::Timeline(id) => ("timeline", id),
    };
    let id = sqlx::query(
        "INSERT INTO comments (target_type, target_id, text, to_node, to_tab, created_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(kind)
    .bind(id)
    .bind(text)
    .bind(to.as_ref().map(|t| t.node.clone()))
    .bind(to.as_ref().map(|t| t.tab.clone()))
    .bind(now_ms())
    .execute(db)
    .await?
    .last_insert_rowid();
    Ok(get(db, id).await?.ok_or_else(|| anyhow::anyhow!("comment {id} vanished"))?)
}

const COLUMNS: &str = "id, target_type, target_id, text, to_node, to_tab, created_at, picked_up_at";

pub async fn get(db: &Db, id: i64) -> anyhow::Result<Option<Comment>> {
    let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM comments WHERE id = ?")))
        .bind(id)
        .fetch_optional(db)
        .await?;
    match row {
        Some(r) => Ok(Some(hydrate(db, r).await?)),
        None => Ok(None),
    }
}

pub async fn mark_picked_up(db: &Db, id: i64) -> anyhow::Result<()> {
    sqlx::query("UPDATE comments SET picked_up_at = ? WHERE id = ? AND picked_up_at IS NULL")
        .bind(now_ms())
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

/// The comments for this session not picked up yet, now picked up.
pub async fn take(db: &Db, me: &SessionRef) -> anyhow::Result<Vec<Comment>> {
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM comments WHERE to_node = ? AND to_tab = ? AND picked_up_at IS NULL ORDER BY id"
    )))
    .bind(&me.node)
    .bind(&me.tab)
    .fetch_all(db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        mark_picked_up(db, r.id).await?;
        let mut c = hydrate(db, r).await?;
        c.picked_up_at = Some(jiff::Timestamp::now());
        out.push(c);
    }
    Ok(out)
}

/// Every comment made in a window, for the History screen.
pub async fn between(db: &Db, since_ms: i64, until_ms: i64) -> anyhow::Result<Vec<Comment>> {
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM comments WHERE created_at >= ? AND created_at < ? ORDER BY id LIMIT 1000"
    )))
    .bind(since_ms)
    .bind(until_ms)
    .fetch_all(db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(hydrate(db, r).await?);
    }
    Ok(out)
}
