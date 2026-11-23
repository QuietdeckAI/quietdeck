//! Nodes: enrolment, tokens, what each says about itself.

use quietdeck_types::ActionType;
use quietdeck_types::api::{Enrolled, EnrolmentCode};
use quietdeck_types::directory::NodeRow;
use quietdeck_types::node::RunnerInfo;

use crate::auth::{ENROLMENT_CODE_TTL_MS, new_enrolment_code, new_node_token, normalise_code, sha256_hex};
use crate::db::{Db, from_json, json, now_ms, ts_opt};
use crate::error::AppError;
use crate::hub::Hub;

pub async fn create_enrolment_code(db: &Db) -> anyhow::Result<EnrolmentCode> {
    let code = new_enrolment_code();
    let expires_at = now_ms() + ENROLMENT_CODE_TTL_MS;
    sqlx::query("INSERT INTO enrolment_codes (code_hash, expires_at) VALUES (?, ?)")
        .bind(sha256_hex(&normalise_code(&code)))
        .bind(expires_at)
        .execute(db)
        .await?;
    Ok(EnrolmentCode { code, expires_at: expires_at / 1000 })
}

/// Use a one-time code to enrol a machine. Enrolling a name again (a reinstalled machine) replaces its token.
pub async fn enrol(db: &Db, code: &str, name: &str) -> Result<Enrolled, AppError> {
    let now = now_ms();
    let mut tx = db.begin().await?;
    let hash = sha256_hex(&normalise_code(code));
    let used = sqlx::query(
        "UPDATE enrolment_codes SET used_at = ?, used_by_node = ? \
         WHERE code_hash = ? AND purpose = 'node' AND used_at IS NULL AND expires_at > ?",
    )
    .bind(now)
    .bind(name)
    .bind(&hash)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    if used.rows_affected() != 1 {
        return Err(AppError::Unauthorized(
            "that code is unknown, used or expired: make a new one in the deck's Settings".into(),
        ));
    }
    let token = new_node_token();
    sqlx::query(
        "INSERT INTO nodes (name, token_hash, created_at) VALUES (?, ?, ?) \
         ON CONFLICT (name) DO UPDATE SET token_hash = excluded.token_hash, revoked_at = NULL",
    )
    .bind(name)
    .bind(sha256_hex(&token))
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Enrolled { node: name.to_string(), token })
}

pub async fn hello(db: &Db, node: &str, version: &str, runner: Option<&RunnerInfo>) -> anyhow::Result<()> {
    sqlx::query("UPDATE nodes SET version = ?, last_seen_at = ?, runner = ?, runner_info = ? WHERE name = ?")
        .bind(version)
        .bind(now_ms())
        .bind(runner.is_some() as i64)
        .bind(runner.map(json))
        .bind(node)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn seen(db: &Db, node: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE nodes SET last_seen_at = ? WHERE name = ?").bind(now_ms()).bind(node).execute(db).await?;
    Ok(())
}

pub async fn runner_info(db: &Db, node: &str) -> anyhow::Result<Option<RunnerInfo>> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT runner_info FROM nodes WHERE name = ?").bind(node).fetch_optional(db).await?;
    row.and_then(|r| r.0).map(|s| from_json(&s)).transpose()
}

pub async fn list(db: &Db, hub: &Hub) -> anyhow::Result<Vec<NodeRow>> {
    type NodeRowDb = (String, Option<i64>, Option<String>, Option<String>);
    let rows: Vec<NodeRowDb> = sqlx::query_as(
        "SELECT name, last_seen_at, version, runner_info FROM nodes WHERE revoked_at IS NULL ORDER BY name",
    )
    .fetch_all(db)
    .await?;
    let mut out = Vec::new();
    for (name, last_seen, version, runner) in rows {
        let runner: Option<RunnerInfo> = runner.map(|s| from_json(&s)).transpose()?;
        let no_handler: Vec<ActionType> = sqlx::query_as::<_, (String,)>(
            "SELECT action_type FROM runner_handoff_stats WHERE node = ? ORDER BY count DESC, last_at DESC",
        )
        .bind(&name)
        .fetch_all(db)
        .await?
        .into_iter()
        .filter_map(|(t,)| ActionType::parse(&t))
        .collect();
        out.push(NodeRow {
            no_handler,
            online: hub.is_online(&name).await,
            name,
            last_seen_at: ts_opt(last_seen),
            version,
            runner: runner.is_some(),
            runner_types: runner.map(|r| r.eligible).unwrap_or_default().into_iter().collect::<Vec<ActionType>>(),
        });
    }
    Ok(out)
}

pub async fn revoke(db: &Db, node: &str) -> Result<(), AppError> {
    let r = sqlx::query("UPDATE nodes SET revoked_at = ? WHERE name = ? AND revoked_at IS NULL")
        .bind(now_ms())
        .bind(node)
        .execute(db)
        .await?;
    if r.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("no enrolled node {node}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_code_works_once() {
        let db = crate::db::memory().await;
        let c = create_enrolment_code(&db).await.unwrap();
        let e = enrol(&db, &c.code.to_lowercase(), "laptop").await.unwrap();
        assert!(e.token.starts_with("qdn_"));
        assert_eq!(crate::auth::node_for_token(&db, &e.token).await.unwrap().as_deref(), Some("laptop"));
        assert!(enrol(&db, &c.code, "other").await.is_err());
    }

    #[tokio::test]
    async fn re_enrolling_replaces_the_token() {
        let db = crate::db::memory().await;
        let first = enrol(&db, &create_enrolment_code(&db).await.unwrap().code, "laptop").await.unwrap();
        let second = enrol(&db, &create_enrolment_code(&db).await.unwrap().code, "laptop").await.unwrap();
        assert!(crate::auth::node_for_token(&db, &first.token).await.unwrap().is_none());
        assert!(crate::auth::node_for_token(&db, &second.token).await.unwrap().is_some());
    }
}
