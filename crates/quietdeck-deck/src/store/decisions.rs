//! Decisions: Approve All and its undo window, release, Stop, Retry, the agent's `decisions take`, and the actions'
//! start and report.
//!
//! A decision names the card version the person saw. Release checks it again, so an approval of version 2 never
//! releases version 3's actions. Released actions go only to the agent that posted the card (or, from build step 5,
//! that agent's runner), and an action runs only after `action start`, which Stop makes fail.

use quietdeck_types::SessionRef;
use quietdeck_types::card::{
    ActionRecord, ActionReport, ActionResult, ActionStatus, Card, CardSection, CardState, Decision, DecisionKind,
    Released, RunBy,
};
use quietdeck_types::decision::{ApproveAll, Approving, CardDecision, Decide, ReportReceipt, Skipped, StopResult};

use crate::db::{Db, enum_str, from_json, json, now_ms, parse_enum, ts, ts_opt};
use crate::error::AppError;
use crate::store::cards;

fn new_batch_id() -> String {
    format!("b_{}", hex::encode(crate::auth::random_bytes::<8>()))
}

/// Record an Approve All (or one card's adjustment, sent the same way). Nothing is released until the undo window
/// ends; the batch is released then by [`release_due`].
pub async fn approve_all(db: &Db, req: &ApproveAll, undo_seconds: u32) -> Result<Approving, AppError> {
    for d in &req.cards {
        d.check().map_err(|e| AppError::BadRequest(format!("card {}: {e}", d.card_id)))?;
        if !matches!(d.kind, DecisionKind::Approve | DecisionKind::Edit | DecisionKind::Adjust) {
            return Err(AppError::BadRequest(format!(
                "card {}: {} isn't an Approve All decision (send it on its own)",
                d.card_id,
                enum_str(&d.kind)
            )));
        }
        if d.kind == DecisionKind::Adjust && d.text.is_none() {
            return Err(AppError::BadRequest(format!("card {}: an adjustment needs its text", d.card_id)));
        }
        if d.kind == DecisionKind::Edit && d.fields.is_none() {
            return Err(AppError::BadRequest(format!("card {}: an edit needs the edited actions", d.card_id)));
        }
    }
    let now = now_ms();
    let release_after = now + i64::from(undo_seconds) * 1000;
    let batch = new_batch_id();
    let mut tx = db.begin().await?;
    sqlx::query("INSERT INTO decision_batches (id, timeline, release_after, created_at) VALUES (?, ?, ?, ?)")
        .bind(&batch)
        .bind(json(&req.timeline))
        .bind(release_after)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    let mut decided = Vec::new();
    let mut skipped = Vec::new();
    for d in &req.cards {
        match why_not(&mut tx, d).await? {
            Some(reason) => skipped.push(Skipped { card_id: d.card_id, reason }),
            None => {
                sqlx::query(
                    "INSERT INTO decisions (card_id, card_version, kind, fields, text, release_after, batch, at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(d.card_id)
                .bind(d.version)
                .bind(enum_str(&d.kind))
                .bind(d.fields.as_ref().map(json))
                .bind(&d.text)
                .bind(release_after)
                .bind(&batch)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                decided.push(d.card_id);
            }
        }
    }
    tx.commit().await?;
    Ok(Approving { batch, release_at: ts(release_after), cards: decided, skipped })
}

/// Why Approve All leaves this card alone, if it does.
async fn why_not(tx: &mut sqlx::SqliteConnection, d: &CardDecision) -> anyhow::Result<Option<String>> {
    let row: Option<(i64, String, String, String)> =
        sqlx::query_as("SELECT version, state, section, input FROM cards WHERE id = ?")
            .bind(d.card_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((version, state, section, input)) = row else { return Ok(Some("no such card".into())) };
    if version != d.version {
        return Ok(Some(format!("changed: version {version} is newer than the one on screen")));
    }
    let state: CardState = parse_enum(&state)?;
    let section: CardSection = parse_enum(&section)?;
    let reason = match state {
        CardState::Revising => Some("the agent is revising it"),
        CardState::Later => Some("left for later"),
        CardState::Approved | CardState::Executing => Some("already approved"),
        CardState::Failed => Some("an action failed: use Retry"),
        CardState::Done | CardState::Automatic => Some("already done"),
        CardState::Open | CardState::HandedOff | CardState::HandledElsewhere => None,
    };
    if let Some(r) = reason {
        return Ok(Some(r.into()));
    }
    if section == CardSection::Suggestion && d.text.is_none() && d.fields.is_none() {
        return Ok(Some("a suggestion: it's approved only on its own".into()));
    }
    let input: quietdeck_types::card::CardInput = from_json(&input)?;
    if section == CardSection::Actions && input.actions.is_empty() && d.text.is_none() && d.fields.is_none() {
        return Ok(Some("nothing proposed and nothing typed".into()));
    }
    let pending: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM decisions WHERE card_id = ? AND card_version = ? AND released_at IS NULL \
         AND kind IN ('approve', 'edit', 'adjust') LIMIT 1",
    )
    .bind(d.card_id)
    .bind(d.version)
    .fetch_optional(&mut *tx)
    .await?;
    if pending.is_some() {
        return Ok(Some("already waiting to be released".into()));
    }
    Ok(None)
}

/// Cancel a batch still in its undo window.
pub async fn undo(db: &Db, batch: &str) -> Result<(), AppError> {
    let res = sqlx::query("DELETE FROM decision_batches WHERE id = ? AND released_at IS NULL").bind(batch).execute(db).await?;
    if res.rows_affected() == 0 {
        let exists: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM decision_batches WHERE id = ?").bind(batch).fetch_optional(db).await?;
        return Err(match exists {
            Some(_) => AppError::Conflict("too late to undo: it's released (use Stop)".into()),
            None => AppError::NotFound(format!("no batch {batch}")),
        });
    }
    Ok(())
}

/// When the next batch is due, if one is waiting.
pub async fn next_due(db: &Db) -> anyhow::Result<Option<i64>> {
    let row: (Option<i64>,) =
        sqlx::query_as("SELECT MIN(release_after) FROM decision_batches WHERE released_at IS NULL").fetch_one(db).await?;
    Ok(row.0)
}

/// A session with released decisions to take.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Waiting {
    pub node: String,
    pub tab: String,
    pub decisions: usize,
}

/// Release every batch whose undo window has ended. Returns who now has decisions to take.
pub async fn release_due(db: &Db) -> anyhow::Result<Vec<Waiting>> {
    let now = now_ms();
    let batches: Vec<(String, String)> =
        sqlx::query_as("SELECT id, timeline FROM decision_batches WHERE released_at IS NULL AND release_after <= ?")
            .bind(now)
            .fetch_all(db)
            .await?;
    let mut waiting: std::collections::BTreeMap<(String, String), usize> = Default::default();
    let mut touched = Vec::new();
    for (batch, timeline) in batches {
        let mut tx = db.begin().await?;
        let claimed = sqlx::query("UPDATE decision_batches SET released_at = ? WHERE id = ? AND released_at IS NULL")
            .bind(now)
            .bind(&batch)
            .execute(&mut *tx)
            .await?;
        if claimed.rows_affected() == 0 {
            continue;
        }
        let ids: Vec<i64> = from_json(&timeline)?;
        for id in ids {
            sqlx::query("UPDATE timeline_entries SET read_at = ? WHERE id = ? AND read_at IS NULL")
                .bind(now)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        let decisions: Vec<(i64, i64, i64, String, Option<String>)> =
            sqlx::query_as("SELECT id, card_id, card_version, kind, fields FROM decisions WHERE batch = ? ORDER BY id")
                .bind(&batch)
                .fetch_all(&mut *tx)
                .await?;
        for (id, card_id, version, kind, fields) in decisions {
            let kind: DecisionKind = parse_enum(&kind)?;
            let fields: Option<Vec<quietdeck_types::Action>> = fields.as_deref().map(from_json).transpose()?;
            if let Some(who) = release_one(&mut tx, id, card_id, version, kind, fields, now).await? {
                *waiting.entry(who).or_default() += 1;
            }
            touched.push(card_id);
        }
        tx.commit().await?;
    }
    for card_id in touched {
        cards::refresh_state(db, card_id).await?;
    }
    Ok(waiting.into_iter().map(|((node, tab), decisions)| Waiting { node, tab, decisions }).collect())
}

/// Release one decision inside its batch's transaction. Returns the posting agent when it has something to take.
async fn release_one(
    tx: &mut sqlx::SqliteConnection,
    id: i64,
    card_id: i64,
    version: i64,
    kind: DecisionKind,
    fields: Option<Vec<quietdeck_types::Action>>,
    now: i64,
) -> anyhow::Result<Option<(String, String)>> {
    let (current, state, node, tab): (i64, String, String, String) =
        sqlx::query_as("SELECT version, state, posted_node, posted_tab FROM cards WHERE id = ?")
            .bind(card_id)
            .fetch_one(&mut *tx)
            .await?;
    let state: CardState = parse_enum(&state)?;
    let skip = if current != version {
        Some(format!("version {current} arrived during the undo window"))
    } else if matches!(state, CardState::Later | CardState::Done | CardState::Automatic | CardState::Revising) {
        Some(format!("the card became {} during the undo window", enum_str(&state)))
    } else {
        None
    };
    if let Some(reason) = skip {
        sqlx::query("UPDATE decisions SET released_at = ?, skipped = ? WHERE id = ?")
            .bind(now)
            .bind(reason)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        return Ok(None);
    }
    sqlx::query("UPDATE decisions SET released_at = ? WHERE id = ?").bind(now).bind(id).execute(&mut *tx).await?;

    if kind == DecisionKind::Adjust {
        sqlx::query("UPDATE cards SET state = 'revising', updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(card_id)
            .execute(&mut *tx)
            .await?;
        return Ok(Some((node, tab)));
    }
    if state == CardState::HandledElsewhere {
        // Seen: it leaves the archive list. Nothing to run.
        return Ok(None);
    }
    if let Some(fields) = fields {
        let ran: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM actions WHERE card_id = ? AND card_version = ? \
             AND status NOT IN ('pending', 'stopped')",
        )
        .bind(card_id)
        .bind(version)
        .fetch_one(&mut *tx)
        .await?;
        if ran.0 > 0 {
            sqlx::query("UPDATE decisions SET skipped = 'some actions already ran: edits not applied' WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            return Ok(None);
        }
        sqlx::query("DELETE FROM actions WHERE card_id = ? AND card_version = ?")
            .bind(card_id)
            .bind(version)
            .execute(&mut *tx)
            .await?;
        for (seq, action) in fields.iter().enumerate() {
            sqlx::query(
                "INSERT INTO actions (card_id, card_version, seq, type, payload, status) \
                 VALUES (?, ?, ?, ?, ?, 'pending')",
            )
            .bind(card_id)
            .bind(version)
            .bind(seq as i64)
            .bind(action.action_type().as_str())
            .bind(json(action))
            .execute(&mut *tx)
            .await?;
        }
    }
    let released = sqlx::query(
        "UPDATE actions SET status = 'released', run_by = NULL, error = NULL \
         WHERE card_id = ? AND card_version = ? AND status IN ('pending', 'stopped')",
    )
    .bind(card_id)
    .bind(version)
    .execute(&mut *tx)
    .await?;
    if released.rows_affected() == 0 {
        // Nothing to run (a sent row he only looked at): seen, so done.
        sqlx::query("UPDATE cards SET state = 'done', updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(card_id)
            .execute(&mut *tx)
            .await?;
        return Ok(None);
    }
    sqlx::query("UPDATE cards SET state = 'approved', handoff_reason = NULL, updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(card_id)
        .execute(&mut *tx)
        .await?;
    Ok(Some((node, tab)))
}

#[derive(sqlx::FromRow)]
struct DecisionRow {
    id: i64,
    card_id: i64,
    card_version: i64,
    kind: String,
    fields: Option<String>,
    text: Option<String>,
    released_at: Option<i64>,
    claimed_by: Option<String>,
    picked_up_at: Option<i64>,
    handoff_reason: Option<String>,
    at: i64,
}

impl DecisionRow {
    fn into_decision(self) -> anyhow::Result<Decision> {
        Ok(Decision {
            id: self.id,
            card_id: self.card_id,
            card_version: self.card_version,
            kind: parse_enum(&self.kind)?,
            fields: self.fields.as_deref().map(from_json).transpose()?,
            text: self.text,
            released_at: ts_opt(self.released_at),
            claimed_by: self.claimed_by.as_deref().map(parse_enum::<RunBy>).transpose()?,
            picked_up_at: ts_opt(self.picked_up_at),
            at: ts(self.at),
        })
    }
}

/// `quietdeck decisions take`: every released decision on cards this agent posted, claimed for it in one go.
pub async fn take(db: &Db, me: &SessionRef) -> anyhow::Result<Vec<Released>> {
    let now = now_ms();
    let rows: Vec<DecisionRow> = sqlx::query_as(
        "SELECT d.id, d.card_id, d.card_version, d.kind, d.fields, d.text, d.released_at, d.claimed_by, \
           d.picked_up_at, d.handoff_reason, d.at \
         FROM decisions d JOIN cards c ON c.id = d.card_id \
         WHERE d.released_at IS NOT NULL AND d.claimed_by IS NULL AND d.skipped IS NULL \
           AND d.kind IN ('approve', 'edit', 'adjust') AND c.version = d.card_version \
           AND c.posted_node = ? AND c.posted_tab = ? \
         ORDER BY d.id",
    )
    .bind(&me.node)
    .bind(&me.tab)
    .fetch_all(db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let claimed = sqlx::query(
            "UPDATE decisions SET claimed_by = 'agent', picked_up_at = ? WHERE id = ? AND claimed_by IS NULL",
        )
        .bind(now)
        .bind(row.id)
        .execute(db)
        .await?;
        if claimed.rows_affected() == 0 {
            continue;
        }
        sqlx::query(
            "UPDATE actions SET run_by = 'agent' WHERE card_id = ? AND card_version = ? AND status = 'released'",
        )
        .bind(row.card_id)
        .bind(row.card_version)
        .execute(db)
        .await?;
        let card = cards::get(db, row.card_id).await?.ok_or_else(|| anyhow::anyhow!("card {} vanished", row.card_id))?;
        let handoff_reason = row.handoff_reason.clone();
        let mut decision = row.into_decision()?;
        decision.claimed_by = Some(RunBy::Agent);
        decision.picked_up_at = Some(ts(now));
        out.push(Released { actions: card.action_records.clone(), decision, card, handoff_reason });
    }
    Ok(out)
}

/// The action, checked as one this agent may run: on a card it posted, at the card's current version.
async fn own_action(db: &Db, me: &SessionRef, action_id: i64) -> Result<(ActionRecord, Card), AppError> {
    let card_id: Option<(i64,)> =
        sqlx::query_as("SELECT card_id FROM actions WHERE id = ?").bind(action_id).fetch_optional(db).await?;
    let Some((card_id,)) = card_id else { return Err(AppError::NotFound(format!("no action {action_id}"))) };
    let card = cards::get(db, card_id).await?.ok_or_else(|| AppError::NotFound(format!("no card {card_id}")))?;
    if card.posted_by.node != me.node || card.posted_by.tab != me.tab {
        return Err(AppError::Forbidden(format!(
            "action {action_id} is on a card {} posted; only that session runs it",
            card.posted_by.address()
        )));
    }
    let Some(action) = card.action_records.iter().find(|a| a.id == action_id).cloned() else {
        return Err(AppError::Conflict(format!(
            "action {action_id} belongs to an older version of card {card_id}: take the card again"
        )));
    };
    Ok((action, card))
}

/// `quietdeck action start`: claim an action just before running it. Refused once Stop has been pressed.
pub async fn start(db: &Db, me: &SessionRef, action_id: i64) -> Result<ActionRecord, AppError> {
    let (action, card) = own_action(db, me, action_id).await?;
    let now = now_ms();
    let res = sqlx::query(
        "UPDATE actions SET status = 'started', started_at = ?, run_by = COALESCE(run_by, 'agent') \
         WHERE id = ? AND status = 'released'",
    )
    .bind(now)
    .bind(action_id)
    .execute(db)
    .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::Conflict(match action.status {
            ActionStatus::Stopped => format!("action {action_id} was stopped: don't run it"),
            ActionStatus::Pending => format!("action {action_id} isn't approved"),
            ActionStatus::Started => format!("action {action_id} has already started"),
            ActionStatus::Done | ActionStatus::Failed => format!("action {action_id} has already finished"),
            ActionStatus::Released => format!("action {action_id} changed while starting: try again"),
        }));
    }
    cards::refresh_state(db, card.id).await?;
    let updated = cards::actions_for(db, card.id, card.version).await?;
    Ok(updated.into_iter().find(|a| a.id == action_id).ok_or_else(|| anyhow::anyhow!("action {action_id} vanished"))?)
}

/// `quietdeck actions report` (and `action done|fail`): every result in one call. Each is checked on its own.
pub async fn report(db: &Db, me: &SessionRef, reports: &[ActionReport]) -> Result<Vec<ReportReceipt>, AppError> {
    let now = now_ms();
    let mut out = Vec::with_capacity(reports.len());
    let mut touched = Vec::new();
    for r in reports {
        let result = async {
            garde::Validate::validate(r).map_err(|e| AppError::BadRequest(e.to_string()))?;
            let (action, card) = own_action(db, me, r.action_id).await?;
            if action.status != ActionStatus::Started {
                return Err(AppError::Conflict(format!(
                    "action {} is {}, not started: run `quietdeck action start {}` before carrying it out",
                    r.action_id,
                    enum_str(&action.status),
                    r.action_id
                )));
            }
            let (status, outcome, error) = match &r.result {
                ActionResult::Done { outcome } => ("done", outcome.as_ref().map(json), None),
                ActionResult::Failed { error } => ("failed", None, Some(error.clone())),
            };
            sqlx::query(
                "UPDATE actions SET status = ?, outcome = ?, error = ?, finished_at = ? WHERE id = ? AND status = 'started'",
            )
            .bind(status)
            .bind(outcome)
            .bind(error)
            .bind(now)
            .bind(r.action_id)
            .execute(db)
            .await?;
            Ok::<i64, AppError>(card.id)
        }
        .await;
        match result {
            Ok(card_id) => {
                touched.push(card_id);
                out.push(ReportReceipt { action_id: r.action_id, error: None });
            }
            Err(e) => out.push(ReportReceipt { action_id: r.action_id, error: Some(e.to_string()) }),
        }
    }
    touched.sort_unstable();
    touched.dedup();
    for id in touched {
        cards::refresh_state(db, id).await?;
    }
    Ok(out)
}

/// Stop: cancel every batch still in its undo window, and stop every approved action that hasn't started. Their cards
/// go back to Actions for another try.
pub async fn stop(db: &Db) -> anyhow::Result<StopResult> {
    let mut tx = db.begin().await?;
    let decisions: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM decisions WHERE batch IN (SELECT id FROM decision_batches WHERE released_at IS NULL)",
    )
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM decision_batches WHERE released_at IS NULL").execute(&mut *tx).await?;
    let cards_hit: Vec<(i64,)> =
        sqlx::query_as("SELECT DISTINCT card_id FROM actions WHERE status = 'released'").fetch_all(&mut *tx).await?;
    let actions = sqlx::query("UPDATE actions SET status = 'stopped' WHERE status = 'released'").execute(&mut *tx).await?;
    tx.commit().await?;
    for (id,) in cards_hit {
        cards::refresh_state(db, id).await?;
    }
    Ok(StopResult { actions: actions.rows_affected() as i64, decisions: decisions.0 })
}

/// Retry a card whose action failed: its failed actions are approved again, released at once.
pub async fn retry(db: &Db, card_id: i64) -> Result<Approving, AppError> {
    let card = cards::get(db, card_id).await?.ok_or_else(|| AppError::NotFound(format!("no card {card_id}")))?;
    if card.state != CardState::Failed {
        return Err(AppError::Conflict(format!("card {card_id} is {}, not failed", enum_str(&card.state))));
    }
    sqlx::query(
        "UPDATE actions SET status = 'pending', error = NULL, started_at = NULL, finished_at = NULL \
         WHERE card_id = ? AND card_version = ? AND status = 'failed'",
    )
    .bind(card_id)
    .bind(card.version)
    .execute(db)
    .await?;
    sqlx::query("UPDATE cards SET state = 'open', updated_at = ? WHERE id = ?")
        .bind(now_ms())
        .bind(card_id)
        .execute(db)
        .await?;
    let req = ApproveAll {
        cards: vec![CardDecision {
            card_id,
            version: card.version,
            kind: DecisionKind::Approve,
            fields: None,
            text: None,
        }],
        timeline: vec![],
    };
    approve_all(db, &req, 0).await
}

/// A card's own decision: Leave for later, Move up or Move down. They take effect at once.
pub async fn decide(db: &Db, card_id: i64, d: &Decide) -> Result<Card, AppError> {
    garde::Validate::validate(d).map_err(|e| AppError::BadRequest(e.to_string()))?;
    let card = cards::get(db, card_id).await?.ok_or_else(|| AppError::NotFound(format!("no card {card_id}")))?;
    if card.version != d.version {
        return Err(AppError::Conflict(format!("card {card_id} has a newer version ({})", card.version)));
    }
    if card.state.is_finished() || matches!(card.state, CardState::Approved | CardState::Executing) {
        return Err(AppError::Conflict(format!("card {card_id} is {}", enum_str(&card.state))));
    }
    let now = now_ms();
    let mut tx = db.begin().await?;
    match d.kind {
        DecisionKind::Later => {
            sqlx::query(
                "INSERT INTO later (account, thread_key, left_at, last_message_ref) VALUES (?, ?, ?, ?) \
                 ON CONFLICT (owner_type, owner_id, account, thread_key) \
                 DO UPDATE SET left_at = excluded.left_at, last_message_ref = excluded.last_message_ref",
            )
            .bind(&card.input.account)
            .bind(&card.input.thread_key)
            .bind(now)
            .bind(&card.input.last_message_ref)
            .execute(&mut *tx)
            .await?;
            sqlx::query("UPDATE cards SET state = 'later', updated_at = ? WHERE id = ?")
                .bind(now)
                .bind(card_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE actions SET status = 'stopped' WHERE card_id = ? AND status IN ('pending', 'released')")
                .bind(card_id)
                .execute(&mut *tx)
                .await?;
        }
        DecisionKind::MoveUp | DecisionKind::MoveDown => {
            let section = if d.kind == DecisionKind::MoveUp { CardSection::Actions } else { CardSection::ArchiveList };
            sqlx::query("UPDATE cards SET section = ?, updated_at = ? WHERE id = ?")
                .bind(enum_str(&section))
                .bind(now)
                .bind(card_id)
                .execute(&mut *tx)
                .await?;
        }
        other => {
            return Err(AppError::BadRequest(format!(
                "{} goes through Approve All (POST /api/decisions)",
                enum_str(&other)
            )));
        }
    }
    // Kept for the learning loop, already released and never taken.
    sqlx::query(
        "INSERT INTO decisions (card_id, card_version, kind, text, released_at, at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(card_id)
    .bind(card.version)
    .bind(enum_str(&d.kind))
    .bind(&d.text)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(cards::get(db, card_id).await?.ok_or_else(|| anyhow::anyhow!("card {card_id} vanished"))?)
}

/// The batch still in its undo window, for the Inbox's bar after a reload.
pub async fn approving(db: &Db) -> anyhow::Result<Option<Approving>> {
    let row: Option<(String, i64)> = sqlx::query_as(
        "SELECT id, release_after FROM decision_batches WHERE released_at IS NULL ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_optional(db)
    .await?;
    let Some((batch, release_after)) = row else { return Ok(None) };
    let cards: Vec<(i64,)> =
        sqlx::query_as("SELECT card_id FROM decisions WHERE batch = ? ORDER BY id").bind(&batch).fetch_all(db).await?;
    Ok(Some(Approving {
        batch,
        release_at: ts(release_after),
        cards: cards.into_iter().map(|r| r.0).collect(),
        skipped: vec![],
    }))
}

/// Whether any approved action is still waiting or running.
pub async fn running(db: &Db) -> anyhow::Result<bool> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM actions WHERE status IN ('released', 'started') LIMIT 1").fetch_optional(db).await?;
    Ok(row.is_some())
}

/// When an agent last posted a card or took decisions.
pub async fn last_agent_at(db: &Db) -> anyhow::Result<Option<i64>> {
    let row: (Option<i64>,) = sqlx::query_as(
        "SELECT MAX(t) FROM (SELECT MAX(prepared_at) AS t FROM cards UNION ALL SELECT MAX(picked_up_at) FROM decisions)",
    )
    .fetch_one(db)
    .await?;
    Ok(row.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use quietdeck_types::card::CardInput;
    use serde_json::json;

    fn agent() -> SessionRef {
        SessionRef { node: "server".into(), tab: "email triage".into(), session_id: Some("s1".into()) }
    }

    fn input(thread: &str, actions: serde_json::Value) -> CardInput {
        serde_json::from_value(json!({"account": "personal", "thread_key": thread, "subject": "Invoice",
            "summary": "s", "section": "actions", "reason": "r", "actions": actions}))
        .unwrap()
    }

    fn archive(thread: &str) -> serde_json::Value {
        json!([{"type": "archive", "account": "personal", "thread_id": thread}])
    }

    async fn post(db: &Db, thread: &str) -> Card {
        cards::add(db, &agent(), &input(thread, archive(thread)), CardState::Open, None).await.unwrap().card
    }

    fn approve(card: &Card) -> CardDecision {
        CardDecision { card_id: card.id, version: card.version, kind: DecisionKind::Approve, fields: None, text: None }
    }

    async fn approve_now(db: &Db, cards: &[&Card]) -> Approving {
        let req = ApproveAll { cards: cards.iter().map(|c| approve(c)).collect(), timeline: vec![] };
        let a = approve_all(db, &req, 0).await.unwrap();
        release_due(db).await.unwrap();
        a
    }

    #[tokio::test]
    async fn nothing_is_released_during_the_undo_window_and_undo_cancels_it() {
        let db = crate::db::memory().await;
        let c = post(&db, "t1").await;
        let a = approve_all(&db, &ApproveAll { cards: vec![approve(&c)], timeline: vec![] }, 60).await.unwrap();
        assert_eq!(a.cards, vec![c.id]);
        assert!(release_due(&db).await.unwrap().is_empty());
        assert!(take(&db, &agent()).await.unwrap().is_empty());
        undo(&db, &a.batch).await.unwrap();
        assert!(matches!(undo(&db, &a.batch).await, Err(AppError::NotFound(_))));
        assert_eq!(cards::get(&db, c.id).await.unwrap().unwrap().state, CardState::Open);
    }

    #[tokio::test]
    async fn released_decisions_go_to_the_posting_agent_only() {
        let db = crate::db::memory().await;
        let c = post(&db, "t1").await;
        let req = ApproveAll { cards: vec![approve(&c)], timeline: vec![] };
        approve_all(&db, &req, 0).await.unwrap();
        let waiting = release_due(&db).await.unwrap();
        assert_eq!(waiting, vec![Waiting { node: "server".into(), tab: "email triage".into(), decisions: 1 }]);
        let other = SessionRef { node: "laptop".into(), tab: "api".into(), session_id: None };
        assert!(take(&db, &other).await.unwrap().is_empty());
        let action_id = c.action_records[0].id;
        assert!(matches!(start(&db, &other, action_id).await, Err(AppError::Forbidden(_))));
        let taken = take(&db, &agent()).await.unwrap();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].actions[0].status, ActionStatus::Released);
        assert!(take(&db, &agent()).await.unwrap().is_empty(), "taken once");
    }

    #[tokio::test]
    async fn an_approval_of_one_version_never_releases_the_next() {
        let db = crate::db::memory().await;
        let v1 = post(&db, "t1").await;
        approve_all(&db, &ApproveAll { cards: vec![approve(&v1)], timeline: vec![] }, 60).await.unwrap();
        let v2 = post(&db, "t1").await;
        assert_eq!(v2.version, 2);
        sqlx::query("UPDATE decision_batches SET release_after = 0").execute(&db).await.unwrap();
        assert!(release_due(&db).await.unwrap().is_empty());
        let card = cards::get(&db, v1.id).await.unwrap().unwrap();
        assert_eq!(card.state, CardState::Open);
        assert_eq!(card.action_records[0].status, ActionStatus::Pending);
        // And a stale version on screen is skipped up front.
        let a = approve_all(&db, &ApproveAll { cards: vec![approve(&v1)], timeline: vec![] }, 0).await.unwrap();
        assert!(a.cards.is_empty());
        assert!(a.skipped[0].reason.contains("version 2"));
    }

    #[tokio::test]
    async fn start_then_report_finishes_the_card() {
        let db = crate::db::memory().await;
        let c = post(&db, "t1").await;
        approve_now(&db, &[&c]).await;
        let taken = take(&db, &agent()).await.unwrap();
        let id = taken[0].actions[0].id;
        let r = report(&db, &agent(), &[ActionReport { action_id: id, result: ActionResult::Done { outcome: None } }])
            .await
            .unwrap();
        assert!(r[0].error.as_deref().unwrap().contains("not started"), "a report needs a start first");
        assert_eq!(start(&db, &agent(), id).await.unwrap().status, ActionStatus::Started);
        assert_eq!(cards::get(&db, c.id).await.unwrap().unwrap().state, CardState::Executing);
        let r = report(&db, &agent(), &[ActionReport { action_id: id, result: ActionResult::Done { outcome: None } }])
            .await
            .unwrap();
        assert_eq!(r[0].error, None);
        assert_eq!(cards::get(&db, c.id).await.unwrap().unwrap().state, CardState::Done);
    }

    #[tokio::test]
    async fn stop_refuses_start_and_returns_the_card_to_actions() {
        let db = crate::db::memory().await;
        let c = post(&db, "t1").await;
        approve_now(&db, &[&c]).await;
        let id = c.action_records[0].id;
        let s = stop(&db).await.unwrap();
        assert_eq!(s.actions, 1);
        assert!(matches!(start(&db, &agent(), id).await, Err(AppError::Conflict(m)) if m.contains("stopped")));
        assert_eq!(cards::get(&db, c.id).await.unwrap().unwrap().state, CardState::Open);
        // Another Approve All releases it again.
        let c = cards::get(&db, c.id).await.unwrap().unwrap();
        approve_now(&db, &[&c]).await;
        assert_eq!(start(&db, &agent(), id).await.unwrap().status, ActionStatus::Started);
    }

    #[tokio::test]
    async fn stop_cancels_batches_in_their_undo_window() {
        let db = crate::db::memory().await;
        let c = post(&db, "t1").await;
        approve_all(&db, &ApproveAll { cards: vec![approve(&c)], timeline: vec![] }, 60).await.unwrap();
        assert_eq!(stop(&db).await.unwrap().decisions, 1);
        sqlx::query("UPDATE decision_batches SET release_after = 0").execute(&db).await.unwrap();
        assert!(release_due(&db).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_failed_action_keeps_the_card_for_retry() {
        let db = crate::db::memory().await;
        let c = post(&db, "t1").await;
        approve_now(&db, &[&c]).await;
        take(&db, &agent()).await.unwrap();
        let id = c.action_records[0].id;
        start(&db, &agent(), id).await.unwrap();
        let failed = ActionResult::Failed { error: "Executor error 401".into() };
        report(&db, &agent(), &[ActionReport { action_id: id, result: failed }]).await.unwrap();
        let card = cards::get(&db, c.id).await.unwrap().unwrap();
        assert_eq!(card.state, CardState::Failed);
        assert_eq!(card.action_records[0].error.as_deref(), Some("Executor error 401"));
        retry(&db, c.id).await.unwrap();
        release_due(&db).await.unwrap();
        assert_eq!(take(&db, &agent()).await.unwrap().len(), 1);
        assert_eq!(start(&db, &agent(), id).await.unwrap().status, ActionStatus::Started);
    }

    #[tokio::test]
    async fn edits_replace_the_proposed_actions() {
        let db = crate::db::memory().await;
        let c = post(&db, "t1").await;
        let fields: Vec<quietdeck_types::Action> = serde_json::from_value(json!([
            {"type": "mark_read", "account": "personal", "thread_id": "t1"},
            {"type": "archive", "account": "personal", "thread_id": "t1"}]))
        .unwrap();
        let d = CardDecision { card_id: c.id, version: 1, kind: DecisionKind::Edit, fields: Some(fields), text: None };
        approve_all(&db, &ApproveAll { cards: vec![d], timeline: vec![] }, 0).await.unwrap();
        release_due(&db).await.unwrap();
        let taken = take(&db, &agent()).await.unwrap();
        assert_eq!(taken[0].actions.len(), 2);
        assert_eq!(taken[0].actions[0].action.action_type().as_str(), "mark_read");
    }

    #[tokio::test]
    async fn an_adjustment_goes_to_the_agent_and_the_card_revises() {
        let db = crate::db::memory().await;
        let c = post(&db, "t1").await;
        let d = CardDecision {
            card_id: c.id,
            version: 1,
            kind: DecisionKind::Adjust,
            fields: None,
            text: Some("forward it to the api tab instead".into()),
        };
        approve_all(&db, &ApproveAll { cards: vec![d], timeline: vec![] }, 0).await.unwrap();
        assert_eq!(release_due(&db).await.unwrap().len(), 1);
        assert_eq!(cards::get(&db, c.id).await.unwrap().unwrap().state, CardState::Revising);
        let taken = take(&db, &agent()).await.unwrap();
        assert_eq!(taken[0].decision.text.as_deref(), Some("forward it to the api tab instead"));
        assert_eq!(taken[0].actions[0].status, ActionStatus::Pending, "nothing runs until it's approved again");
        // The agent's new version is an open card again.
        let v2 = post(&db, "t1").await;
        assert_eq!((v2.version, v2.state), (2, CardState::Open));
    }

    #[tokio::test]
    async fn approve_all_skips_what_it_must() {
        let db = crate::db::memory().await;
        let empty = cards::add(&db, &agent(), &input("t2", json!([])), CardState::Open, None).await.unwrap().card;
        let later = post(&db, "t3").await;
        decide(&db, later.id, &Decide { version: 1, kind: DecisionKind::Later, text: None }).await.unwrap();
        let later = cards::get(&db, later.id).await.unwrap().unwrap();
        let a = approve_all(&db, &ApproveAll { cards: vec![approve(&empty), approve(&later)], timeline: vec![] }, 3)
            .await
            .unwrap();
        assert!(a.cards.is_empty());
        assert_eq!(a.skipped.len(), 2);
    }

    #[tokio::test]
    async fn the_batch_marks_its_timeline_entries_read() {
        let db = crate::db::memory().await;
        let e = crate::store::timeline::add(
            &db,
            quietdeck_types::timeline::EntrySource::Runner,
            &quietdeck_types::timeline::TimelineAdd {
                kind: quietdeck_types::timeline::TimelineKind::AutoAction,
                account: Some("personal".into()),
                refs: vec![],
                text: "archived a receipt".into(),
                lease_id: None,
            },
        )
        .await
        .unwrap();
        approve_all(&db, &ApproveAll { cards: vec![], timeline: vec![e.id] }, 0).await.unwrap();
        release_due(&db).await.unwrap();
        let read: (Option<i64>,) =
            sqlx::query_as("SELECT read_at FROM timeline_entries WHERE id = ?").bind(e.id).fetch_one(&db).await.unwrap();
        assert!(read.0.is_some());
    }

    #[tokio::test]
    async fn move_down_and_up_change_the_section() {
        let db = crate::db::memory().await;
        let c = post(&db, "t1").await;
        let c = decide(&db, c.id, &Decide { version: 1, kind: DecisionKind::MoveDown, text: None }).await.unwrap();
        assert_eq!(c.input.section, CardSection::ArchiveList);
        let c = decide(&db, c.id, &Decide { version: 1, kind: DecisionKind::MoveUp, text: None }).await.unwrap();
        assert_eq!(c.input.section, CardSection::Actions);
        assert!(take(&db, &agent()).await.unwrap().is_empty(), "moves are never taken");
    }
}
