//! Cards and their actions.

use quietdeck_types::SessionRef;
use quietdeck_types::card::{
    ActionRecord, ActionStatus, Card, CardInput, CardResolution, CardSection, CardState, RunBy,
};

use crate::db::{Db, enum_str, from_json, json, now_ms, parse_enum, ts, ts_opt};
use crate::error::AppError;

#[derive(sqlx::FromRow)]
struct CardRow {
    id: i64,
    version: i64,
    posted_node: String,
    posted_tab: String,
    posted_session_id: Option<String>,
    section: String,
    state: String,
    handoff_reason: Option<String>,
    grant_id: Option<i64>,
    input: String,
    prepared_at: i64,
    rechecked_at: Option<i64>,
    recheck_note: Option<String>,
    updated_at: i64,
}

const CARD_COLUMNS: &str = "id, version, posted_node, posted_tab, posted_session_id, section, state, handoff_reason, grant_id, \
    input, prepared_at, rechecked_at, recheck_note, updated_at";

#[derive(sqlx::FromRow)]
struct ActionRow {
    id: i64,
    card_id: i64,
    card_version: i64,
    seq: i64,
    payload: String,
    status: String,
    run_by: Option<String>,
    outcome: Option<String>,
    error: Option<String>,
    started_at: Option<i64>,
    finished_at: Option<i64>,
}

const ACTION_COLUMNS: &str =
    "id, card_id, card_version, seq, payload, status, run_by, outcome, error, started_at, finished_at";

impl ActionRow {
    fn into_record(self) -> anyhow::Result<ActionRecord> {
        Ok(ActionRecord {
            id: self.id,
            card_id: self.card_id,
            card_version: self.card_version,
            seq: self.seq,
            action: from_json(&self.payload)?,
            status: parse_enum(&self.status)?,
            run_by: self.run_by.as_deref().map(parse_enum::<RunBy>).transpose()?,
            outcome: self.outcome.as_deref().map(from_json).transpose()?,
            error: self.error,
            started_at: ts_opt(self.started_at),
            finished_at: ts_opt(self.finished_at),
        })
    }
}

pub async fn actions_for(db: &Db, card_id: i64, version: i64) -> anyhow::Result<Vec<ActionRecord>> {
    let rows: Vec<ActionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ACTION_COLUMNS} FROM actions WHERE card_id = ? AND card_version = ? ORDER BY seq"
    )))
    .bind(card_id)
    .bind(version)
    .fetch_all(db)
    .await?;
    rows.into_iter().map(ActionRow::into_record).collect()
}

async fn hydrate(db: &Db, r: CardRow) -> anyhow::Result<Card> {
    let action_records = actions_for(db, r.id, r.version).await?;
    let mut input: CardInput = from_json(&r.input)?;
    // The deck moves cards between sections (Move up, Move down, handled elsewhere): its column wins.
    input.section = parse_enum(&r.section)?;
    Ok(Card {
        id: r.id,
        version: r.version,
        posted_by: SessionRef { node: r.posted_node, tab: r.posted_tab, session_id: r.posted_session_id },
        state: parse_enum(&r.state)?,
        handoff_reason: r.handoff_reason,
        grant_id: r.grant_id,
        prepared_at: ts(r.prepared_at),
        rechecked_at: ts_opt(r.rechecked_at),
        recheck_note: r.recheck_note,
        updated_at: ts(r.updated_at),
        input,
        action_records,
    })
}

pub async fn get(db: &Db, id: i64) -> anyhow::Result<Option<Card>> {
    let row: Option<CardRow> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {CARD_COLUMNS} FROM cards WHERE id = ?")))
            .bind(id)
            .fetch_optional(db)
            .await?;
    match row {
        Some(r) => Ok(Some(hydrate(db, r).await?)),
        None => Ok(None),
    }
}

/// States in which a card is still the thread's card: a new post replaces its content. That includes approved and
/// executing cards: the agent posts a new version when a newer message arrives before it sends a reply (3.2a).
const LIVE_STATES: &str = "('open', 'revising', 'approved', 'executing', 'later', 'failed', 'handed_off')";

pub struct Added {
    pub card: Card,
    pub replaced: bool,
}

/// Post a card. One card per thread: a thread's live card gets a new version. A thread left for later takes a card
/// only for a message newer than when it was left.
pub async fn add(
    db: &Db,
    posted_by: &SessionRef,
    input: &CardInput,
    state: CardState,
    grant_id: Option<i64>,
) -> Result<Added, AppError> {
    let now = now_ms();
    let mut tx = db.begin().await?;

    let later: Option<(i64,)> = sqlx::query_as("SELECT left_at FROM later WHERE account = ? AND thread_key = ?")
        .bind(&input.account)
        .bind(&input.thread_key)
        .fetch_optional(&mut *tx)
        .await?;
    if let Some((left_at,)) = later {
        match input.last_message_at {
            Some(at) if at.as_millisecond() > left_at => {
                // A newer message: the thread comes back from Later.
                sqlx::query("DELETE FROM later WHERE account = ? AND thread_key = ?")
                    .bind(&input.account)
                    .bind(&input.thread_key)
                    .execute(&mut *tx)
                    .await?;
            }
            _ => {
                return Err(AppError::Conflict(
                    "this thread was left for later: post again only with last_message_at newer than when it was left"
                        .into(),
                ));
            }
        }
    }

    let existing: Option<(i64, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT id, version FROM cards WHERE account = ? AND thread_key = ? AND state IN {LIVE_STATES} \
         ORDER BY id DESC LIMIT 1"
    )))
    .bind(&input.account)
    .bind(&input.thread_key)
    .fetch_optional(&mut *tx)
    .await?;

    let section = input.section;
    let (id, version, replaced) = match existing {
        Some((id, version)) => {
            let version = version + 1;
            sqlx::query(
                "UPDATE cards SET version = ?, posted_node = ?, posted_tab = ?, posted_session_id = ?, section = ?, \
                   state = ?, handoff_reason = NULL, grant_id = ?, input = ?, prepared_at = ?, rechecked_at = NULL, \
                   recheck_note = NULL, updated_at = ? WHERE id = ?",
            )
            .bind(version)
            .bind(&posted_by.node)
            .bind(&posted_by.tab)
            .bind(&posted_by.session_id)
            .bind(enum_str(&section))
            .bind(enum_str(&state))
            .bind(grant_id)
            .bind(json(input))
            .bind(now)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
            // Actions on older versions that never started are superseded.
            sqlx::query(
                "UPDATE actions SET status = 'stopped' WHERE card_id = ? AND card_version < ? \
                 AND status IN ('pending', 'released')",
            )
            .bind(id)
            .bind(version)
            .execute(&mut *tx)
            .await?;
            (id, version, true)
        }
        None => {
            let res = sqlx::query(
                "INSERT INTO cards (version, posted_node, posted_tab, posted_session_id, account, thread_key, section, \
                   state, grant_id, input, prepared_at, created_at, updated_at) \
                 VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&posted_by.node)
            .bind(&posted_by.tab)
            .bind(&posted_by.session_id)
            .bind(&input.account)
            .bind(&input.thread_key)
            .bind(enum_str(&section))
            .bind(enum_str(&state))
            .bind(grant_id)
            .bind(json(input))
            .bind(now)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            (res.last_insert_rowid(), 1, false)
        }
    };

    for (seq, action) in input.actions.iter().enumerate() {
        sqlx::query(
            "INSERT INTO actions (card_id, card_version, seq, type, payload, status) VALUES (?, ?, ?, ?, ?, 'pending')",
        )
        .bind(id)
        .bind(version)
        .bind(seq as i64)
        .bind(action.action_type().as_str())
        .bind(json(action))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    let card = get(db, id).await?.ok_or_else(|| anyhow::anyhow!("card {id} vanished"))?;
    Ok(Added { card, replaced })
}

#[derive(Debug, Default, Clone, serde::Deserialize, schemars::JsonSchema)]
pub struct ListFilter {
    pub account: Option<String>,
    pub section: Option<CardSection>,
    pub state: Option<CardState>,
    /// Only cards still on the Inbox (not finished).
    #[serde(default)]
    pub open: bool,
    pub limit: Option<i64>,
}

pub async fn list(db: &Db, f: &ListFilter) -> anyhow::Result<Vec<Card>> {
    let mut sql = format!("SELECT {CARD_COLUMNS} FROM cards WHERE 1 = 1");
    if f.account.is_some() {
        sql.push_str(" AND account = ?");
    }
    if f.section.is_some() {
        sql.push_str(" AND section = ?");
    }
    if f.state.is_some() {
        sql.push_str(" AND state = ?");
    }
    if f.open {
        sql.push_str(" AND state NOT IN ('done', 'handled_elsewhere', 'automatic')");
    }
    sql.push_str(" ORDER BY updated_at DESC LIMIT ?");
    let mut q = sqlx::query_as::<_, CardRow>(sqlx::AssertSqlSafe(sql));
    if let Some(a) = &f.account {
        q = q.bind(a);
    }
    if let Some(s) = &f.section {
        q = q.bind(enum_str(s));
    }
    if let Some(s) = &f.state {
        q = q.bind(enum_str(s));
    }
    let rows = q.bind(f.limit.unwrap_or(200).clamp(1, 1000)).fetch_all(db).await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(hydrate(db, r).await?);
    }
    Ok(out)
}

/// The Inbox's cards, newest first: everything not finished or left for later, and "handled elsewhere" lines the
/// person hasn't seen yet (an Approve All marks them seen).
pub async fn inbox(db: &Db, account: Option<&str>) -> anyhow::Result<Vec<Card>> {
    let mut sql = format!(
        "SELECT {CARD_COLUMNS} FROM cards c WHERE (state NOT IN ('done', 'automatic', 'handled_elsewhere', 'later') \
           OR (state = 'handled_elsewhere' AND NOT EXISTS (SELECT 1 FROM decisions d WHERE d.card_id = c.id \
             AND d.released_at IS NOT NULL AND d.skipped IS NULL)))"
    );
    if account.is_some() {
        sql.push_str(" AND account = ?");
    }
    sql.push_str(" ORDER BY prepared_at DESC LIMIT 500");
    let mut q = sqlx::query_as::<_, CardRow>(sqlx::AssertSqlSafe(sql));
    if let Some(a) = account {
        q = q.bind(a);
    }
    let rows = q.fetch_all(db).await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(hydrate(db, r).await?);
    }
    Ok(out)
}

/// Which of these cards were carried out after an approval: done, done by a rule, or handled elsewhere after a
/// released decision (the person approved it and the work landed another way).
pub async fn finished_of(db: &Db, ids: &[i64]) -> anyhow::Result<Vec<i64>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let list = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    let rows: Vec<(i64,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT id FROM cards c WHERE id IN ({list}) AND (state IN ('done', 'automatic') \
           OR (state = 'handled_elsewhere' AND EXISTS (SELECT 1 FROM decisions d WHERE d.card_id = c.id \
             AND d.released_at IS NOT NULL AND d.skipped IS NULL))) ORDER BY id"
    )))
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

/// Whether any thread is left for later.
pub async fn has_later(db: &Db, account: Option<&str>) -> anyhow::Result<bool> {
    let row: Option<(i64,)> = match account {
        Some(a) => {
            sqlx::query_as("SELECT 1 FROM cards WHERE state = 'later' AND account = ? LIMIT 1")
                .bind(a)
                .fetch_optional(db)
                .await?
        }
        None => sqlx::query_as("SELECT 1 FROM cards WHERE state = 'later' LIMIT 1").fetch_optional(db).await?,
    };
    Ok(row.is_some())
}

/// Cards changed since a moment, for `history`.
pub async fn changed_since(db: &Db, since_ms: i64, until_ms: i64) -> anyhow::Result<Vec<Card>> {
    let rows: Vec<CardRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {CARD_COLUMNS} FROM cards WHERE id IN ( \
           SELECT id FROM cards WHERE updated_at > ? AND updated_at <= ? \
           UNION SELECT card_id FROM actions WHERE (finished_at > ? AND finished_at <= ?) \
             OR (started_at > ? AND started_at <= ?)) \
         ORDER BY updated_at"
    )))
    .bind(since_ms)
    .bind(until_ms)
    .bind(since_ms)
    .bind(until_ms)
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

/// The agent's recheck result.
pub async fn resolve(db: &Db, id: i64, how: &CardResolution) -> Result<Card, AppError> {
    let now = now_ms();
    let card = get(db, id).await?.ok_or_else(|| AppError::NotFound(format!("no card {id}")))?;
    if card.state.is_finished() {
        return Err(AppError::Conflict(format!("card {id} is already finished ({})", enum_str(&card.state))));
    }
    match how {
        CardResolution::ArchivedInGmail => {
            sqlx::query(
                "UPDATE cards SET state = 'done', rechecked_at = ?, recheck_note = 'archived in Gmail', updated_at = ? \
                 WHERE id = ?",
            )
            .bind(now)
            .bind(now)
            .bind(id)
            .execute(db)
            .await?;
            stop_unstarted(db, id).await?;
        }
        CardResolution::HandledElsewhere { note } => {
            sqlx::query(
                "UPDATE cards SET state = 'handled_elsewhere', section = 'archive_list', rechecked_at = ?, \
                   recheck_note = ?, updated_at = ? WHERE id = ?",
            )
            .bind(now)
            .bind(note)
            .bind(now)
            .bind(id)
            .execute(db)
            .await?;
            stop_unstarted(db, id).await?;
        }
        CardResolution::Rechecked { note } => {
            sqlx::query("UPDATE cards SET rechecked_at = ?, recheck_note = ?, updated_at = ? WHERE id = ?")
                .bind(now)
                .bind(note)
                .bind(now)
                .bind(id)
                .execute(db)
                .await?;
        }
    }
    Ok(get(db, id).await?.ok_or_else(|| anyhow::anyhow!("card {id} vanished"))?)
}

async fn stop_unstarted(db: &Db, card_id: i64) -> anyhow::Result<()> {
    sqlx::query("UPDATE actions SET status = 'stopped' WHERE card_id = ? AND status IN ('pending', 'released')")
        .bind(card_id)
        .execute(db)
        .await?;
    Ok(())
}

/// A card's state from its current version's actions, after something changed them (a release, a start, a report,
/// Stop). Cards that are revising, left for later or finished keep their state.
pub fn state_from_actions(current: CardState, actions: &[ActionRecord]) -> CardState {
    let has = |s: ActionStatus| actions.iter().any(|a| a.status == s);
    if current == CardState::Automatic {
        // An automatic card stays out of the Inbox while it runs and once it's done. A failure, or Stop before it
        // ran, brings it to the Inbox like any card.
        if has(ActionStatus::Started) || has(ActionStatus::Released) {
            return current;
        }
        if has(ActionStatus::Failed) {
            return CardState::Failed;
        }
        if has(ActionStatus::Stopped) || has(ActionStatus::Pending) {
            return CardState::Open;
        }
        return current;
    }
    if !matches!(
        current,
        CardState::Approved | CardState::HandedOff | CardState::Executing | CardState::Failed | CardState::Open
    ) {
        return current;
    }
    if has(ActionStatus::Started) {
        CardState::Executing
    } else if has(ActionStatus::Released) {
        if current == CardState::Open { CardState::Approved } else { current }
    } else if has(ActionStatus::Failed) {
        CardState::Failed
    } else if has(ActionStatus::Pending) || has(ActionStatus::Stopped) {
        CardState::Open
    } else if !actions.is_empty() {
        CardState::Done
    } else {
        current
    }
}

/// Recompute a card's state from its actions and store it.
pub async fn refresh_state(db: &Db, card_id: i64) -> anyhow::Result<()> {
    let Some(card) = get(db, card_id).await? else { return Ok(()) };
    let next = state_from_actions(card.state, &card.action_records);
    if next != card.state {
        sqlx::query("UPDATE cards SET state = ?, updated_at = ? WHERE id = ?")
            .bind(enum_str(&next))
            .bind(now_ms())
            .bind(card_id)
            .execute(db)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn me() -> SessionRef {
        SessionRef { node: "server".into(), tab: "email triage".into(), session_id: Some("s1".into()) }
    }

    fn input(summary: &str) -> CardInput {
        serde_json::from_value(json!({"account": "personal", "thread_key": "t1", "subject": "Invoice",
            "summary": summary, "section": "actions", "reason": "r",
            "actions": [{"type": "archive", "account": "personal", "thread_id": "t1"}]}))
        .unwrap()
    }

    #[tokio::test]
    async fn finished_of_names_only_cards_carried_out() {
        let db = crate::db::memory().await;
        let mut ids = vec![];
        for (thread, state) in [("t1", "done"), ("t2", "open"), ("t3", "automatic"), ("t4", "later")] {
            let mut i = input("x");
            i.thread_key = thread.into();
            let id = add(&db, &me(), &i, CardState::Open, None).await.unwrap().card.id;
            sqlx::query("UPDATE cards SET state = ? WHERE id = ?").bind(state).bind(id).execute(&db).await.unwrap();
            ids.push(id);
        }
        assert_eq!(finished_of(&db, &ids).await.unwrap(), vec![ids[0], ids[2]]);
        assert!(finished_of(&db, &[]).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn one_card_per_thread_with_versions() {
        let db = crate::db::memory().await;
        let a = add(&db, &me(), &input("first"), CardState::Open, None).await.unwrap();
        assert!(!a.replaced);
        assert_eq!(a.card.version, 1);
        assert_eq!(a.card.action_records.len(), 1);
        let b = add(&db, &me(), &input("second"), CardState::Open, None).await.unwrap();
        assert!(b.replaced);
        assert_eq!(b.card.id, a.card.id);
        assert_eq!(b.card.version, 2);
        assert_eq!(b.card.input.summary, "second");
        let old = actions_for(&db, a.card.id, 1).await.unwrap();
        assert_eq!(old[0].status, ActionStatus::Stopped, "the old version's actions are superseded");
        assert_eq!(list(&db, &ListFilter { open: true, ..Default::default() }).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_finished_card_doesnt_take_new_versions() {
        let db = crate::db::memory().await;
        let a = add(&db, &me(), &input("first"), CardState::Open, None).await.unwrap();
        resolve(&db, a.card.id, &CardResolution::ArchivedInGmail).await.unwrap();
        let b = add(&db, &me(), &input("new mail"), CardState::Open, None).await.unwrap();
        assert_ne!(b.card.id, a.card.id);
    }

    #[tokio::test]
    async fn later_threads_need_a_newer_message() {
        let db = crate::db::memory().await;
        sqlx::query("INSERT INTO later (account, thread_key, left_at) VALUES ('personal', 't1', ?)")
            .bind(now_ms())
            .execute(&db)
            .await
            .unwrap();
        assert!(matches!(add(&db, &me(), &input("x"), CardState::Open, None).await, Err(AppError::Conflict(_))));
        let mut i = input("x");
        i.last_message_at = Some(jiff::Timestamp::now() + jiff::SignedDuration::from_secs(60));
        add(&db, &me(), &i, CardState::Open, None).await.unwrap();
    }

    #[tokio::test]
    async fn handled_elsewhere_moves_to_the_archive_list_with_its_note() {
        let db = crate::db::memory().await;
        let a = add(&db, &me(), &input("first"), CardState::Open, None).await.unwrap();
        let c = resolve(&db, a.card.id, &CardResolution::HandledElsewhere { note: "you replied at 14:20".into() })
            .await
            .unwrap();
        assert_eq!(c.state, CardState::HandledElsewhere);
        assert_eq!(c.recheck_note.as_deref(), Some("you replied at 14:20"));
        assert!(matches!(resolve(&db, a.card.id, &CardResolution::ArchivedInGmail).await, Err(AppError::Conflict(_))));
    }
}
