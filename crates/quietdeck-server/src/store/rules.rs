//! Auto-approve rules (grants): stored, listed, edited, archived when their session closes, and counted when they
//! match.

use quietdeck_types::SessionRef;
use quietdeck_types::rules::{Rule, RuleInput, RuleStatus};

use crate::db::{Db, enum_str, from_json, json, now_ms, parse_enum, ts_opt};
use crate::error::AppError;

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    mode: String,
    instruction: Option<String>,
    criteria: String,
    action_template: String,
    session_ref: Option<String>,
    project: Option<String>,
    topic: Option<String>,
    known_sender_domains: String,
    status: String,
    archived_reason: Option<String>,
    match_count: i64,
    last_matched_at: Option<i64>,
    created_from_card_id: Option<i64>,
    builtin: Option<String>,
}

impl Row {
    fn into_rule(self) -> anyhow::Result<Rule> {
        Ok(Rule {
            id: self.id,
            mode: parse_enum(&self.mode)?,
            instruction: self.instruction,
            criteria: from_json(&self.criteria)?,
            action_template: from_json(&self.action_template)?,
            session_ref: self.session_ref.as_deref().map(from_json).transpose()?,
            project: self.project,
            topic: self.topic,
            known_sender_domains: from_json(&self.known_sender_domains)?,
            status: parse_enum(&self.status)?,
            archived_reason: self.archived_reason,
            match_count: self.match_count,
            last_matched_at: ts_opt(self.last_matched_at),
            created_from_card_id: self.created_from_card_id,
            builtin: self.builtin.is_some(),
        })
    }
}

const COLUMNS: &str = "id, mode, instruction, criteria, action_template, session_ref, project, topic, \
    known_sender_domains, status, archived_reason, match_count, last_matched_at, created_from_card_id, builtin";

/// Active first, then archived and disabled; newest first within each.
pub async fn list(db: &Db) -> anyhow::Result<Vec<Rule>> {
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM rules ORDER BY status = 'active' DESC, id DESC LIMIT 1000"
    )))
    .fetch_all(db)
    .await?;
    rows.into_iter().map(Row::into_rule).collect()
}

pub async fn get(db: &Db, id: i64) -> anyhow::Result<Option<Rule>> {
    let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM rules WHERE id = ?")))
        .bind(id)
        .fetch_optional(db)
        .await?;
    row.map(Row::into_rule).transpose()
}

/// Lower-case the addresses and domains, so matching never depends on how they were typed.
fn normalised(input: &RuleInput) -> RuleInput {
    let mut r = input.clone();
    let lower = |s: &mut Option<String>| {
        if let Some(v) = s {
            *v = v.trim().to_lowercase();
        }
    };
    lower(&mut r.criteria.from_address);
    lower(&mut r.criteria.from_domain);
    r.known_sender_domains =
        r.known_sender_domains.iter().map(|d| d.trim().trim_start_matches('@').to_lowercase()).collect();
    r.known_sender_domains.sort();
    r.known_sender_domains.dedup();
    if r.session_ref.is_none() {
        r.session_ref = r.action_template.target.clone();
    }
    r
}

pub async fn create(db: &Db, input: &RuleInput) -> anyhow::Result<Rule> {
    let r = normalised(input);
    let id = sqlx::query(
        "INSERT INTO rules (mode, instruction, criteria, action_template, session_ref, project, topic, \
           known_sender_domains, status, created_from_card_id, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?)",
    )
    .bind(enum_str(&r.mode))
    .bind(&r.instruction)
    .bind(json(&r.criteria))
    .bind(json(&r.action_template))
    .bind(r.session_ref.as_ref().map(json))
    .bind(&r.project)
    .bind(&r.topic)
    .bind(json(&r.known_sender_domains))
    .bind(r.created_from_card_id)
    .bind(now_ms())
    .execute(db)
    .await?
    .last_insert_rowid();
    get(db, id).await?.ok_or_else(|| anyhow::anyhow!("rule {id} vanished"))
}

pub async fn update(db: &Db, id: i64, input: &RuleInput) -> Result<Rule, AppError> {
    let r = normalised(input);
    let res = sqlx::query(
        "UPDATE rules SET mode = ?, instruction = ?, criteria = ?, action_template = ?, session_ref = ?, project = ?, \
           topic = ?, known_sender_domains = ? WHERE id = ?",
    )
    .bind(enum_str(&r.mode))
    .bind(&r.instruction)
    .bind(json(&r.criteria))
    .bind(json(&r.action_template))
    .bind(r.session_ref.as_ref().map(json))
    .bind(&r.project)
    .bind(&r.topic)
    .bind(json(&r.known_sender_domains))
    .bind(id)
    .execute(db)
    .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("no rule {id}")));
    }
    Ok(get(db, id).await?.ok_or_else(|| anyhow::anyhow!("rule {id} vanished"))?)
}

/// Archive, disable or restore a rule. Restoring with `move_to` ties it to that session and makes it the target.
pub async fn set_status(
    db: &Db,
    id: i64,
    status: RuleStatus,
    reason: Option<&str>,
    move_to: Option<&SessionRef>,
) -> Result<Rule, AppError> {
    let mut rule = get(db, id).await?.ok_or_else(|| AppError::NotFound(format!("no rule {id}")))?;
    if let Some(to) = move_to {
        if status != RuleStatus::Active {
            return Err(AppError::BadRequest("a rule moves to another session only when it's restored".into()));
        }
        rule.session_ref = Some(to.clone());
        if rule.action_template.target.is_some() {
            rule.action_template.target = Some(to.clone());
        }
    }
    sqlx::query("UPDATE rules SET status = ?, archived_reason = ?, session_ref = ?, action_template = ? WHERE id = ?")
        .bind(enum_str(&status))
        .bind(if status == RuleStatus::Active { None } else { reason })
        .bind(rule.session_ref.as_ref().map(json))
        .bind(json(&rule.action_template))
        .bind(id)
        .execute(db)
        .await?;
    Ok(get(db, id).await?.ok_or_else(|| anyhow::anyhow!("rule {id} vanished"))?)
}

/// Whether a rule is tied to this session: the same session ID, or the same tab when the rule names no session ID.
fn tied_to(rule: &Rule, node: &str, session_id: &str, tab: Option<&str>) -> bool {
    let Some(r) = &rule.session_ref else { return false };
    r.node == node
        && match &r.session_id {
            Some(id) => id == session_id,
            None => tab == Some(r.tab.as_str()),
        }
}

/// A session closed (its tab closed, or the agent exited): archive the active grants tied to it.
pub async fn archive_for_session(
    db: &Db,
    node: &str,
    session_id: &str,
    tab: Option<&str>,
) -> anyhow::Result<Vec<Rule>> {
    let mut archived = Vec::new();
    for rule in list(db).await? {
        if rule.status != RuleStatus::Active || !tied_to(&rule, node, session_id, tab) {
            continue;
        }
        let reason = format!("its session in {} closed", tab.unwrap_or("a tab"));
        let res =
            sqlx::query("UPDATE rules SET status = 'archived', archived_reason = ? WHERE id = ? AND status = 'active'")
                .bind(&reason)
                .bind(rule.id)
                .execute(db)
                .await?;
        if res.rows_affected() == 1 {
            archived.push(Rule { status: RuleStatus::Archived, archived_reason: Some(reason), ..rule });
        }
    }
    Ok(archived)
}

/// Grants archived when an earlier session in this tab closed, which a card targeting the tab could take over.
pub async fn archived_for_tab(db: &Db, target: &SessionRef) -> anyhow::Result<Vec<i64>> {
    Ok(list(db)
        .await?
        .into_iter()
        .filter(|r| {
            r.status == RuleStatus::Archived
                && r.session_ref.as_ref().is_some_and(|s| {
                    s.node == target.node
                        && s.tab == target.tab
                        && (target.session_id.is_none() || s.session_id != target.session_id)
                })
        })
        .map(|r| r.id)
        .collect())
}

/// A match the deck allowed: counted on the rule.
pub async fn record_match(tx: &mut sqlx::SqliteConnection, id: i64) -> anyhow::Result<()> {
    sqlx::query("UPDATE rules SET match_count = match_count + 1, last_matched_at = ? WHERE id = ?")
        .bind(now_ms())
        .bind(id)
        .execute(tx)
        .await?;
    Ok(())
}

/// Automatic resumes and new sessions in the last 24 hours.
pub async fn session_starts_in_last_day(db: &Db) -> anyhow::Result<u32> {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM auto_session_starts WHERE at > ?")
        .bind(now_ms() - 24 * 3600 * 1000)
        .fetch_one(db)
        .await?;
    Ok(row.0 as u32)
}

pub async fn record_session_starts(tx: &mut sqlx::SqliteConnection, card_id: i64, n: usize) -> anyhow::Result<()> {
    let now = now_ms();
    for _ in 0..n {
        sqlx::query("INSERT INTO auto_session_starts (card_id, at) VALUES (?, ?)")
            .bind(card_id)
            .bind(now)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}
