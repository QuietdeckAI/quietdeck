//! The SQLite database: WAL mode, migrations at start-up, and helpers for the columns every table shares.

use std::time::Duration;

use jiff::Timestamp;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

pub type Db = SqlitePool;

pub async fn open(path: &std::path::Path) -> anyhow::Result<Db> {
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await
        .map_err(|e| anyhow::anyhow!("opening {}: {e}", path.display()))?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// An in-memory database for tests, migrated.
#[cfg(test)]
pub async fn memory() -> Db {
    let opts = <SqliteConnectOptions as std::str::FromStr>::from_str("sqlite::memory:").unwrap().foreign_keys(true);
    // One connection: every connection to :memory: is its own database.
    let pool = SqlitePoolOptions::new().max_connections(1).connect_with(opts).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn a_path_with_spaces_and_a_question_mark_opens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("my data?/deck.db");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        super::open(&path).await.unwrap();
        assert!(path.exists());
    }
}

pub fn now_ms() -> i64 {
    Timestamp::now().as_millisecond()
}

pub fn ts(ms: i64) -> Timestamp {
    Timestamp::from_millisecond(ms).unwrap_or(Timestamp::UNIX_EPOCH)
}

pub fn ts_opt(ms: Option<i64>) -> Option<Timestamp> {
    ms.map(ts)
}

/// A serde enum's snake_case name, for TEXT columns.
pub fn enum_str<T: serde::Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        other => panic!("enum_str on a value that isn't a unit variant: {other:?}"),
    }
}

/// The reverse of [`enum_str`]. A value the code doesn't know is a corrupt row, reported as an error.
pub fn parse_enum<T: serde::de::DeserializeOwned>(s: &str) -> anyhow::Result<T> {
    serde_json::from_value(serde_json::Value::String(s.to_string()))
        .map_err(|e| anyhow::anyhow!("unknown value {s:?} in the database: {e}"))
}

pub fn json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).expect("serialising a quietdeck type")
}

pub fn from_json<T: serde::de::DeserializeOwned>(s: &str) -> anyhow::Result<T> {
    serde_json::from_str(s).map_err(|e| anyhow::anyhow!("corrupt JSON in the database: {e}"))
}
