//! The automatic-actions timeline.

use quietdeck_types::SessionRef;
use quietdeck_types::timeline::{EntrySource, TimelineAdd, TimelineEntry, TimelineKind};

use crate::db::{Db, enum_str, from_json, json, now_ms, parse_enum, ts, ts_opt};

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    account: Option<String>,
    kind: String,
    source: String,
    refs: String,
    text: String,
    count: i64,
    created_at: i64,
    read_at: Option<i64>,
}

impl Row {
    fn into_entry(self) -> anyhow::Result<TimelineEntry> {
        Ok(TimelineEntry {
            id: self.id,
            account: self.account,
            kind: parse_enum(&self.kind)?,
            source: parse_enum(&self.source)?,
            refs: from_json(&self.refs)?,
            text: self.text,
            count: self.count,
            created_at: ts(self.created_at),
            read_at: ts_opt(self.read_at),
        })
    }
}

const COLUMNS: &str = "id, account, kind, source, refs, text, count, created_at, read_at";

/// An unread entry. `posted_by` is the session behind it, which the person's comments on it go to.
pub async fn add(
    db: &Db,
    source: EntrySource,
    posted_by: Option<&SessionRef>,
    e: &TimelineAdd,
) -> anyhow::Result<TimelineEntry> {
    let id = sqlx::query(
        "INSERT INTO timeline_entries (account, kind, source, refs, text, lease_id, posted_node, posted_tab, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&e.account)
    .bind(enum_str(&e.kind))
    .bind(enum_str(&source))
    .bind(json(&e.refs))
    .bind(&e.text)
    .bind(e.lease_id)
    .bind(posted_by.map(|p| p.node.clone()))
    .bind(posted_by.map(|p| p.tab.clone()))
    .bind(now_ms())
    .execute(db)
    .await?
    .last_insert_rowid();
    get(db, id).await?.ok_or_else(|| anyhow::anyhow!("timeline entry {id} vanished"))
}

/// Something the person did that History should show, already read so it never waits on the Inbox: a session
/// started, resumed or closed from the deck.
pub async fn log(db: &Db, kind: TimelineKind, refs: &[String], text: &str) -> anyhow::Result<TimelineEntry> {
    let now = now_ms();
    let id = sqlx::query(
        "INSERT INTO timeline_entries (kind, source, refs, text, created_at, read_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(enum_str(&kind))
    .bind(enum_str(&EntrySource::Deck))
    .bind(json(&refs))
    .bind(text)
    .bind(now)
    .bind(now)
    .execute(db)
    .await?
    .last_insert_rowid();
    get(db, id).await?.ok_or_else(|| anyhow::anyhow!("timeline entry {id} vanished"))
}

pub async fn get(db: &Db, id: i64) -> anyhow::Result<Option<TimelineEntry>> {
    let row: Option<Row> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM timeline_entries WHERE id = ?")))
            .bind(id)
            .fetch_optional(db)
            .await?;
    row.map(Row::into_entry).transpose()
}

pub async fn between(db: &Db, since_ms: i64, until_ms: i64) -> anyhow::Result<Vec<TimelineEntry>> {
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM timeline_entries WHERE created_at > ? AND created_at <= ? ORDER BY created_at"
    )))
    .bind(since_ms)
    .bind(until_ms)
    .fetch_all(db)
    .await?;
    rows.into_iter().map(Row::into_entry).collect()
}

/// Unread entries: the Inbox's timeline.
pub async fn unread(db: &Db, limit: i64) -> anyhow::Result<Vec<TimelineEntry>> {
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM timeline_entries WHERE read_at IS NULL ORDER BY created_at DESC LIMIT ?"
    )))
    .bind(limit)
    .fetch_all(db)
    .await?;
    rows.into_iter().map(Row::into_entry).collect()
}
