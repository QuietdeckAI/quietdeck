//! While the person reads the Inbox (quietdeck_types::reading): the page's heartbeat, the runner's question "let this
//! through now?", the "while I'm reading" rules that answer it, and what the runner holds meanwhile.
//!
//! The deck decides; the runner holds. Going idle (no heartbeat for two minutes, or every tab hidden) tells the
//! runner, which then wakes the agent for everything it held.

use std::sync::Mutex;
use std::time::Duration;

use quietdeck_types::api::Changed;
use quietdeck_types::node::DeckRequest;
use quietdeck_types::reading::{
    HoldVerdict, IDLE_AFTER_SECONDS, ItemFacts, Presence, PresenceBeat, ReadingRule, ReadingRuleInput,
};

use crate::db::{Db, from_json, json, now_ms, ts, ts_opt};
use crate::error::{AppError, AppResult, bad};
use crate::state::AppState;

/// What the runners were last told, so a change is told once.
static TOLD: Mutex<Option<bool>> = Mutex::new(None);

async fn active_until(db: &Db) -> AppResult<Option<i64>> {
    let row: (Option<i64>,) = sqlx::query_as("SELECT MAX(active_until) FROM presence WHERE active_until > ?")
        .bind(now_ms())
        .fetch_one(db)
        .await?;
    Ok(row.0)
}

pub async fn is_active(db: &Db) -> AppResult<bool> {
    Ok(active_until(db).await?.is_some())
}

/// The Inbox's view: whether the person is reading, and what the runners hold.
pub async fn current(db: &Db) -> AppResult<Presence> {
    let until = active_until(db).await?;
    let held: Vec<(Option<String>, i64)> =
        sqlx::query_as("SELECT summary, count FROM runner_held WHERE count > 0 ORDER BY node").fetch_all(db).await?;
    let count: i64 = held.iter().map(|h| h.1).sum();
    let summary: Vec<String> = held.into_iter().filter_map(|h| h.0).collect();
    Ok(Presence {
        active: until.is_some(),
        active_until: ts_opt(until),
        held: (!summary.is_empty()).then(|| summary.join(", ")),
        held_count: count.max(0) as u64,
    })
}

/// A heartbeat from one browser tab.
pub async fn beat(state: &AppState, b: PresenceBeat) -> AppResult<Presence> {
    garde::Validate::validate(&b).map_err(bad)?;
    if b.active {
        sqlx::query(
            "INSERT INTO presence (device, active_until) VALUES (?, ?) \
             ON CONFLICT (device) DO UPDATE SET active_until = excluded.active_until",
        )
        .bind(&b.device)
        .bind(now_ms() + IDLE_AFTER_SECONDS * 1000)
        .execute(&state.db)
        .await?;
    } else {
        sqlx::query("DELETE FROM presence WHERE device = ?").bind(&b.device).execute(&state.db).await?;
    }
    settle_told(state).await?;
    current(&state.db).await
}

/// Tell the runners when activity started or ended since they were last told.
async fn settle_told(state: &AppState) -> AppResult<()> {
    let active = is_active(&state.db).await?;
    let changed = {
        let mut told = TOLD.lock().expect("presence lock");
        let changed = *told != Some(active);
        *told = Some(active);
        changed
    };
    if changed {
        tracing::info!(active, "the person's reading the Inbox changed");
        state.changed(Changed::Cards);
        let state = state.clone();
        tokio::spawn(async move { crate::runner::tell_runners(&state, DeckRequest::Presence { active }).await });
    }
    Ok(())
}

/// Notices activity ending without a last heartbeat (a closed laptop), and drops old rows.
pub async fn ticker(state: AppState) {
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        if let Err(e) = settle_told(&state).await {
            tracing::warn!(error = %e, "checking whether the person is still reading");
        }
        let _ = sqlx::query("DELETE FROM presence WHERE active_until < ?")
            .bind(now_ms() - 3600 * 1000)
            .execute(&state.db)
            .await;
    }
}

/// What a runner that just connected is told first.
pub async fn presence_request(db: &Db) -> AppResult<DeckRequest> {
    Ok(DeckRequest::Presence { active: is_active(db).await? })
}

/// The runner's question for new items. Not reading: all go through. Reading: an update to a thread an open card
/// shows goes through, and so does whatever a rule lets through; the rest waits.
pub async fn hold_check(db: &Db, items: &[ItemFacts]) -> AppResult<Vec<HoldVerdict>> {
    if items.len() > 500 {
        return Err(AppError::BadRequest("ask about at most 500 items at a time".into()));
    }
    if !is_active(db).await? {
        return Ok(items.iter().map(|_| HoldVerdict::Let { reason: String::new() }).collect());
    }
    let rules = list(db).await?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        out.push(verdict(db, &rules, item).await?);
    }
    Ok(out)
}

/// One item's verdict while the person reads.
pub async fn verdict(db: &Db, rules: &[ReadingRule], item: &ItemFacts) -> AppResult<HoldVerdict> {
    if item.source == "gmail"
        && let Some(thread) = &item.thread_key
    {
        let card: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM cards WHERE account = ? AND thread_key = ? \
             AND state IN ('open', 'handed_off', 'failed', 'revising') ORDER BY id DESC LIMIT 1",
        )
        .bind(&item.account)
        .bind(thread)
        .fetch_optional(db)
        .await?;
        if let Some((id,)) = card {
            return Ok(HoldVerdict::Let { reason: format!("an update on c_{id}'s thread") });
        }
    }
    for r in rules.iter().filter(|r| r.enabled) {
        if r.input.matches(item) {
            sqlx::query("UPDATE reading_rules SET match_count = match_count + 1, last_matched_at = ? WHERE id = ?")
                .bind(now_ms())
                .bind(r.id)
                .execute(db)
                .await?;
            return Ok(HoldVerdict::Let { reason: format!("rule \"{}\"", r.input.name) });
        }
    }
    Ok(HoldVerdict::Hold)
}

/// What a runner holds now.
pub async fn held(state: &AppState, node: &str, summary: Option<String>, count: u64) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO runner_held (node, summary, count, at) VALUES (?, ?, ?, ?) \
         ON CONFLICT (node) DO UPDATE SET summary = excluded.summary, count = excluded.count, at = excluded.at",
    )
    .bind(node)
    .bind(summary)
    .bind(count as i64)
    .bind(now_ms())
    .execute(&state.db)
    .await?;
    state.changed(Changed::Cards);
    Ok(())
}

/// "Triage now": the runners wake their agents for what they hold.
pub async fn release(state: &AppState) -> AppResult<()> {
    crate::runner::tell_runners(state, DeckRequest::ReleaseHeld).await;
    Ok(())
}

// --- Rules -----------------------------------------------------------------------------------------------------------

type RuleRow = (i64, String, bool, i64, Option<i64>);

fn rule(r: RuleRow) -> AppResult<ReadingRule> {
    Ok(ReadingRule { id: r.0, input: from_json(&r.1)?, enabled: r.2, match_count: r.3, last_matched_at: r.4.map(ts) })
}

pub async fn list(db: &Db) -> AppResult<Vec<ReadingRule>> {
    let rows: Vec<RuleRow> =
        sqlx::query_as("SELECT id, input, enabled, match_count, last_matched_at FROM reading_rules ORDER BY id")
            .fetch_all(db)
            .await?;
    rows.into_iter().map(rule).collect()
}

fn check(input: &ReadingRuleInput) -> AppResult<()> {
    garde::Validate::validate(input).map_err(bad)?;
    input.check().map_err(AppError::BadRequest)
}

async fn get(db: &Db, id: i64) -> AppResult<ReadingRule> {
    let row: Option<RuleRow> =
        sqlx::query_as("SELECT id, input, enabled, match_count, last_matched_at FROM reading_rules WHERE id = ?")
            .bind(id)
            .fetch_optional(db)
            .await?;
    rule(row.ok_or_else(|| AppError::NotFound(format!("no rule {id}")))?)
}

pub async fn create(state: &AppState, input: ReadingRuleInput) -> AppResult<ReadingRule> {
    check(&input)?;
    let id = sqlx::query("INSERT INTO reading_rules (input, created_at) VALUES (?, ?)")
        .bind(json(&input))
        .bind(now_ms())
        .execute(&state.db)
        .await?
        .last_insert_rowid();
    state.changed(Changed::Rules);
    get(&state.db, id).await
}

pub async fn update(state: &AppState, id: i64, input: ReadingRuleInput) -> AppResult<ReadingRule> {
    check(&input)?;
    get(&state.db, id).await?;
    sqlx::query("UPDATE reading_rules SET input = ? WHERE id = ?")
        .bind(json(&input))
        .bind(id)
        .execute(&state.db)
        .await?;
    state.changed(Changed::Rules);
    get(&state.db, id).await
}

pub async fn set_enabled(state: &AppState, id: i64, enabled: bool) -> AppResult<ReadingRule> {
    get(&state.db, id).await?;
    sqlx::query("UPDATE reading_rules SET enabled = ? WHERE id = ?").bind(enabled).bind(id).execute(&state.db).await?;
    state.changed(Changed::Rules);
    get(&state.db, id).await
}

pub async fn delete(state: &AppState, id: i64) -> AppResult<()> {
    get(&state.db, id).await?;
    sqlx::query("DELETE FROM reading_rules WHERE id = ?").bind(id).execute(&state.db).await?;
    state.changed(Changed::Rules);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(thread: &str) -> ItemFacts {
        ItemFacts {
            source: "gmail".into(),
            account: "personal".into(),
            thread_key: Some(thread.into()),
            from: Some("no-reply@accounts.google.com".into()),
            subject: Some("Security alert: new sign-in".into()),
            ..ItemFacts::default()
        }
    }

    async fn reading(state: &AppState) {
        beat(state, PresenceBeat { device: "tab1".into(), active: true }).await.unwrap();
    }

    #[tokio::test]
    async fn nothing_waits_while_nobody_reads() {
        let state = AppState::for_tests().await;
        let v = hold_check(&state.db, &[mail("t1")]).await.unwrap();
        assert_eq!(v, vec![HoldVerdict::Let { reason: String::new() }]);
    }

    #[tokio::test]
    async fn while_reading_only_updates_and_rule_matches_get_through() {
        let state = AppState::for_tests().await;
        reading(&state).await;
        assert!(current(&state.db).await.unwrap().active);
        assert_eq!(hold_check(&state.db, &[mail("t1")]).await.unwrap(), vec![HoldVerdict::Hold]);

        let rule = create(
            &state,
            ReadingRuleInput {
                name: "Sign-in alerts".into(),
                source: Some("gmail".into()),
                from: Some("accounts.google.com".into()),
                text_regex: Some("(?i)sign-in".into()),
                ..ReadingRuleInput::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            hold_check(&state.db, &[mail("t1")]).await.unwrap(),
            vec![HoldVerdict::Let { reason: "rule \"Sign-in alerts\"".into() }]
        );
        assert_eq!(get(&state.db, rule.id).await.unwrap().match_count, 1);
        set_enabled(&state, rule.id, false).await.unwrap();
        assert_eq!(hold_check(&state.db, &[mail("t1")]).await.unwrap(), vec![HoldVerdict::Hold]);

        // A new message on a thread an open card shows is an update to what's on screen.
        sqlx::query(
            "INSERT INTO cards (posted_node, posted_tab, account, thread_key, section, state, input, prepared_at, \
             created_at, updated_at) VALUES ('n', 't', 'personal', 't1', 'actions', 'open', '{}', 0, 0, 0)",
        )
        .execute(&state.db)
        .await
        .unwrap();
        assert_eq!(
            hold_check(&state.db, &[mail("t1")]).await.unwrap(),
            vec![HoldVerdict::Let { reason: "an update on c_1's thread".into() }]
        );

        // The last tab going away ends the reading at once.
        beat(&state, PresenceBeat { device: "tab1".into(), active: false }).await.unwrap();
        assert!(!current(&state.db).await.unwrap().active);
    }

    #[tokio::test]
    async fn rules_need_a_condition_and_a_sound_regex() {
        let state = AppState::for_tests().await;
        assert!(create(&state, ReadingRuleInput { name: "x".into(), ..Default::default() }).await.is_err());
        let bad = ReadingRuleInput { name: "x".into(), text_regex: Some("(".into()), ..Default::default() };
        assert!(create(&state, bad).await.is_err());
    }
}
