//! The Drafts and Sent tabs (quietdeck_types::mail): the runner hands over what it sees sent and a full list of each
//! account's drafts; the deck keeps them, names the card and session when the deck made the message itself, and counts
//! what's new since the person last opened each tab.

use quietdeck_types::SessionRef;
use quietdeck_types::api::Changed;
use quietdeck_types::mail::{MailBadges, MailItem, MailKind, MailRow};

use crate::db::{Db, now_ms, ts};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Sent messages kept this long.
const SENT_DAYS: i64 = 30;
/// A message on a card's thread within this long of the card's draft or reply action is that action's.
const MATCH_MS: i64 = 15 * 60 * 1000;

fn kind_str(k: MailKind) -> &'static str {
    match k {
        MailKind::Draft => "draft",
        MailKind::Sent => "sent",
    }
}

/// The card (and its poster) whose draft or reply produced this message: one on the same account and thread, with a
/// draft or reply action that finished around then.
async fn made_by(db: &Db, item: &MailItem) -> AppResult<Option<(i64, SessionRef)>> {
    let Some(thread) = &item.thread_id else { return Ok(None) };
    let at = item.at.as_millisecond();
    let row: Option<(i64, String, String, Option<String>)> = sqlx::query_as(
        "SELECT c.id, c.posted_node, c.posted_tab, c.posted_session_id FROM cards c \
         JOIN actions a ON a.card_id = c.id AND a.card_version = c.version \
         WHERE c.account = ? AND c.thread_key = ? AND a.type IN ('draft', 'reply') AND a.status = 'done' \
           AND a.finished_at BETWEEN ? AND ? ORDER BY a.finished_at DESC LIMIT 1",
    )
    .bind(&item.account)
    .bind(thread)
    .bind(at - MATCH_MS)
    .bind(at + MATCH_MS)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|(id, node, tab, session_id)| (id, SessionRef { node, tab, session_id })))
}

async fn upsert(db: &Db, item: &MailItem, now: i64) -> AppResult<bool> {
    let made = made_by(db, item).await?;
    let done = sqlx::query(
        "INSERT INTO mail_items (account, kind, message_id, thread_id, draft_id, from_addr, to_addr, subject, snippet, \
           at, card_id, posted_node, posted_tab, posted_session_id, received_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT (account, kind, message_id) DO NOTHING",
    )
    .bind(&item.account)
    .bind(kind_str(item.kind))
    .bind(&item.message_id)
    .bind(&item.thread_id)
    .bind(&item.draft_id)
    .bind(&item.from)
    .bind(&item.to)
    .bind(&item.subject)
    .bind(&item.snippet)
    .bind(item.at.as_millisecond())
    .bind(made.as_ref().map(|m| m.0))
    .bind(made.as_ref().map(|m| m.1.node.clone()))
    .bind(made.as_ref().map(|m| m.1.tab.clone()))
    .bind(made.as_ref().and_then(|m| m.1.session_id.clone()))
    .bind(now)
    .execute(db)
    .await?;
    Ok(done.rows_affected() > 0)
}

fn check(items: &[MailItem]) -> AppResult<()> {
    if items.len() > 1000 {
        return Err(AppError::BadRequest("send at most 1000 messages at a time".into()));
    }
    Ok(())
}

/// Messages the runner saw sent. A draft that was sent leaves the Drafts tab now rather than at the next draft list.
pub async fn sent(state: &AppState, items: &[MailItem]) -> AppResult<()> {
    check(items)?;
    let now = now_ms();
    let mut changed = false;
    for item in items.iter().filter(|i| i.kind == MailKind::Sent) {
        changed |= upsert(&state.db, item, now).await?;
        if let Some(thread) = &item.thread_id {
            let gone = sqlx::query(
                "DELETE FROM mail_items WHERE kind = 'draft' AND account = ? AND thread_id = ? AND at <= ?",
            )
            .bind(&item.account)
            .bind(thread)
            .bind(item.at.as_millisecond())
            .execute(&state.db)
            .await?;
            changed |= gone.rows_affected() > 0;
        }
    }
    sqlx::query("DELETE FROM mail_items WHERE kind = 'sent' AND at < ?")
        .bind(now - SENT_DAYS * 24 * 3600 * 1000)
        .execute(&state.db)
        .await?;
    if changed {
        state.changed(Changed::Mail);
    }
    // A Work entry waiting for him to send this himself is answered for him.
    let threads: Vec<String> =
        items.iter().filter(|i| i.kind == MailKind::Sent).filter_map(|i| i.thread_id.clone()).collect();
    if let Err(e) = crate::reports::sent_threads(state, &threads).await {
        tracing::warn!(error = %e, "matching sent mail to the reports' questions");
    }
    Ok(())
}

/// Every draft on one account now: new ones are added, and drafts no longer there (sent, deleted) go.
pub async fn drafts(state: &AppState, account: &str, items: &[MailItem]) -> AppResult<()> {
    check(items)?;
    let now = now_ms();
    let mut changed = false;
    for item in items.iter().filter(|i| i.kind == MailKind::Draft && i.account == account) {
        changed |= upsert(&state.db, item, now).await?;
    }
    let keep: Vec<&str> = items.iter().map(|i| i.message_id.as_str()).collect();
    let existing: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, message_id FROM mail_items WHERE kind = 'draft' AND account = ?")
            .bind(account)
            .fetch_all(&state.db)
            .await?;
    for (id, message_id) in existing {
        if !keep.contains(&message_id.as_str()) {
            sqlx::query("DELETE FROM mail_items WHERE id = ?").bind(id).execute(&state.db).await?;
            changed = true;
        }
    }
    if changed {
        state.changed(Changed::Mail);
    }
    Ok(())
}

async fn seen_until(db: &Db, kind: MailKind) -> AppResult<i64> {
    let row: Option<(i64,)> = sqlx::query_as("SELECT seen_until FROM mail_seen WHERE kind = ?")
        .bind(kind_str(kind))
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| r.0).unwrap_or(0))
}

type Row = (
    i64,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    i64,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
    i64,
);

/// One tab's rows, newest first.
pub async fn list(db: &Db, kind: MailKind, account: Option<&str>) -> AppResult<Vec<MailRow>> {
    let seen = seen_until(db, kind).await?;
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, account, kind, message_id, thread_id, draft_id, from_addr, to_addr, subject, snippet, at, card_id, \
           posted_node, posted_tab, posted_session_id, received_at \
         FROM mail_items WHERE kind = ? AND (? IS NULL OR account = ?) ORDER BY at DESC LIMIT 500",
    )
    .bind(kind_str(kind))
    .bind(account)
    .bind(account)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| MailRow {
            id: r.0,
            item: MailItem {
                account: r.1,
                kind,
                message_id: r.3,
                thread_id: r.4,
                draft_id: r.5,
                from: r.6,
                to: r.7,
                subject: r.8,
                snippet: r.9,
                at: ts(r.10),
            },
            card_id: r.11,
            posted_by: match (r.12, r.13) {
                (Some(node), Some(tab)) => Some(SessionRef { node, tab, session_id: r.14 }),
                _ => None,
            },
            new: r.15 > seen,
        })
        .collect())
}

/// The person opened a tab: its badge clears.
pub async fn seen(state: &AppState, kind: MailKind) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO mail_seen (kind, seen_until) VALUES (?, ?) \
         ON CONFLICT (kind) DO UPDATE SET seen_until = excluded.seen_until",
    )
    .bind(kind_str(kind))
    .bind(now_ms())
    .execute(&state.db)
    .await?;
    state.changed(Changed::Mail);
    Ok(())
}

/// The tabs' counts.
pub async fn badges(db: &Db) -> AppResult<MailBadges> {
    let count = |kind: MailKind| async move {
        let seen = seen_until(db, kind).await?;
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM mail_items WHERE kind = ? AND received_at > ?")
            .bind(kind_str(kind))
            .bind(seen)
            .fetch_one(db)
            .await?;
        AppResult::Ok(n.max(0) as u32)
    };
    // Emails waiting for approval, as the person counts them: the distinct threads across the open cards (one card
    // often groups many related emails), each card's own thread included. Work's cards (about sessions, not mail)
    // count on the Work tab, never here.
    let open: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT account, thread_key, input FROM cards WHERE state = 'open' \
         AND section IN ('actions', 'suggestion', 'archive_list') AND account != ?",
    )
    .bind(quietdeck_types::document::SESSIONS_ACCOUNT)
    .fetch_all(db)
    .await?;
    let mut threads = std::collections::HashSet::new();
    for (account, thread_key, input) in open {
        let thread_key = thread_key.rsplit(':').next().unwrap_or(&thread_key).to_string();
        threads.insert((account.clone(), thread_key));
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&input) {
            for a in v.get("actions").and_then(|a| a.as_array()).into_iter().flatten() {
                if let Some(t) = a.get("thread_id").and_then(|t| t.as_str()) {
                    let acct = a.get("account").and_then(|x| x.as_str()).unwrap_or(&account);
                    threads.insert((acct.to_string(), t.to_string()));
                }
            }
        }
    }
    let inbox = threads.len() as i64;
    // Drafts: how many there are now. Sent: what's new since the person last opened it.
    let (drafts,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM mail_items WHERE kind = 'draft'").fetch_one(db).await?;
    Ok(MailBadges { inbox: inbox as u32, drafts: drafts.max(0) as u32, sent: count(MailKind::Sent).await? })
}

/// The threads of a card: its own thread and every thread its actions name, per account.
fn card_threads(account: &str, thread_key: &str, input: &str) -> Vec<(String, String)> {
    let mut out = vec![(account.to_string(), thread_key.rsplit(':').next().unwrap_or(thread_key).to_string())];
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(input) {
        for a in v.get("actions").and_then(|a| a.as_array()).into_iter().flatten() {
            if let Some(t) = a.get("thread_id").and_then(|t| t.as_str()) {
                let acct = a.get("account").and_then(|x| x.as_str()).unwrap_or(account);
                let pair = (acct.to_string(), t.to_string());
                if !out.contains(&pair) {
                    out.push(pair);
                }
            }
        }
    }
    out
}

/// Threads the person archived or deleted in Gmail themselves (the runner checked every message has left the inbox),
/// and threads that came back with new mail. Each card still waiting for the person (open, or failed) whose threads
/// have all left is resolved as archived in Gmail; the runner tells the agent, which tidies the report. Cards the
/// runner or the agent are carrying out aren't touched: their own archiving shows up here too.
pub async fn threads_left(
    state: &AppState,
    left: &[quietdeck_types::mail::LeftThread],
    back: &[quietdeck_types::mail::LeftThread],
) -> AppResult<Vec<quietdeck_types::mail::LeftCard>> {
    let now = now_ms();
    for t in back {
        sqlx::query("DELETE FROM left_inbox WHERE account = ? AND thread_id = ?")
            .bind(&t.account)
            .bind(&t.thread_id)
            .execute(&state.db)
            .await?;
    }
    for t in left {
        sqlx::query("INSERT OR REPLACE INTO left_inbox (account, thread_id, at) VALUES (?, ?, ?)")
            .bind(&t.account)
            .bind(&t.thread_id)
            .bind(now)
            .execute(&state.db)
            .await?;
    }
    // A month is plenty: a card older than that is long gone.
    sqlx::query("DELETE FROM left_inbox WHERE at < ?").bind(now - 30 * 86_400_000).execute(&state.db).await?;
    if left.is_empty() {
        return Ok(vec![]);
    }
    let gone: std::collections::HashSet<(String, String)> =
        sqlx::query_as::<_, (String, String)>("SELECT account, thread_id FROM left_inbox")
            .fetch_all(&state.db)
            .await?
            .into_iter()
            .collect();
    let waiting: Vec<(i64, String, String, String, String)> = sqlx::query_as(
        "SELECT id, account, thread_key, input, COALESCE(json_extract(input, '$.subject'), '') FROM cards \
         WHERE state IN ('open', 'failed') AND section != 'sent' ORDER BY id",
    )
    .fetch_all(&state.db)
    .await?;
    let mut resolved = vec![];
    for (id, account, thread_key, input, subject) in waiting {
        let threads = card_threads(&account, &thread_key, &input);
        // Only a card one of these threads belongs to, and only once all of its threads have gone.
        let touched = threads.iter().any(|(a, t)| left.iter().any(|l| &l.account == a && &l.thread_id == t));
        if !touched || !threads.iter().all(|p| gone.contains(p)) {
            continue;
        }
        // Only while it still waits for the person: one the runner picked up meanwhile is left to it.
        let changed = sqlx::query(
            "UPDATE cards SET state = 'done', rechecked_at = ?, recheck_note = 'you archived it in Gmail', \
             updated_at = ? WHERE id = ? AND state IN ('open', 'failed')",
        )
        .bind(now)
        .bind(now)
        .bind(id)
        .execute(&state.db)
        .await?
        .rows_affected();
        if changed == 1 {
            sqlx::query(
                "UPDATE actions SET status = 'stopped' WHERE card_id = ? AND status IN ('pending', 'released')",
            )
            .bind(id)
            .execute(&state.db)
            .await?;
            resolved.push(quietdeck_types::mail::LeftCard { id, subject });
        }
    }
    if !resolved.is_empty() {
        tracing::info!(cards = resolved.len(), "cards resolved: the person archived their threads in Gmail");
        state.changed(Changed::Cards);
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: MailKind, id: &str, thread: &str, minutes_ago: i64) -> MailItem {
        MailItem {
            account: "work".into(),
            kind,
            message_id: id.into(),
            thread_id: Some(thread.into()),
            draft_id: (kind == MailKind::Draft).then(|| format!("r-{id}")),
            from: Some("me@example.org".into()),
            to: Some("acme@example.com".into()),
            subject: Some("Invoice".into()),
            snippet: Some("Paid today".into()),
            at: jiff::Timestamp::from_millisecond(now_ms() - minutes_ago * 60_000).unwrap(),
        }
    }

    #[tokio::test]
    async fn drafts_follow_the_list_and_a_sent_draft_moves_to_sent() {
        let state = AppState::for_tests().await;
        drafts(&state, "work", &[item(MailKind::Draft, "d1", "t1", 10), item(MailKind::Draft, "d2", "t2", 5)])
            .await
            .unwrap();
        assert_eq!(list(&state.db, MailKind::Draft, None).await.unwrap().len(), 2);
        assert_eq!(badges(&state.db).await.unwrap().drafts, 2);

        // d1 was sent: it's on Sent, and off Drafts at once.
        sent(&state, &[item(MailKind::Sent, "s1", "t1", 1)]).await.unwrap();
        let d = list(&state.db, MailKind::Draft, None).await.unwrap();
        assert_eq!(d.iter().map(|r| r.item.message_id.as_str()).collect::<Vec<_>>(), ["d2"]);
        assert_eq!(list(&state.db, MailKind::Sent, None).await.unwrap()[0].item.message_id, "s1");

        // d2 was deleted in Gmail: the next list drops it.
        drafts(&state, "work", &[]).await.unwrap();
        assert!(list(&state.db, MailKind::Draft, None).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn opening_a_tab_clears_its_badge() {
        let state = AppState::for_tests().await;
        sent(&state, &[item(MailKind::Sent, "s1", "t1", 1), item(MailKind::Sent, "s2", "t2", 1)]).await.unwrap();
        assert_eq!(badges(&state.db).await.unwrap().sent, 2);
        assert!(list(&state.db, MailKind::Sent, None).await.unwrap().iter().all(|r| r.new));
        seen(&state, MailKind::Sent).await.unwrap();
        assert_eq!(badges(&state.db).await.unwrap().sent, 0);
        assert!(list(&state.db, MailKind::Sent, None).await.unwrap().iter().all(|r| !r.new));
    }

    fn left(t: &str) -> quietdeck_types::mail::LeftThread {
        quietdeck_types::mail::LeftThread { account: "work".into(), thread_id: t.into() }
    }

    async fn card(state: &AppState, thread: &str, extra: &[&str], card_state: &str) -> i64 {
        let actions: Vec<serde_json::Value> = std::iter::once(thread)
            .chain(extra.iter().copied())
            .map(|t| serde_json::json!({"type": "archive", "thread_id": t}))
            .collect();
        let input = serde_json::json!({"subject": format!("About {thread}"), "actions": actions}).to_string();
        sqlx::query_scalar(
            "INSERT INTO cards (posted_node, posted_tab, account, thread_key, section, state, input, prepared_at, \
             created_at, updated_at) VALUES ('n', 't', 'work', ?, 'archive_list', ?, ?, 0, 0, 0) RETURNING id",
        )
        .bind(thread)
        .bind(card_state)
        .bind(input)
        .fetch_one(&state.db)
        .await
        .unwrap()
    }

    async fn state_of(state: &AppState, id: i64) -> String {
        sqlx::query_scalar("SELECT state FROM cards WHERE id = ?").bind(id).fetch_one(&state.db).await.unwrap()
    }

    #[tokio::test]
    async fn a_card_resolves_once_the_person_archived_all_its_threads() {
        let state = AppState::for_tests().await;
        let single = card(&state, "t1", &[], "open").await;
        let group = card(&state, "t2", &["t3"], "open").await;
        let running = card(&state, "t4", &[], "executing").await;

        // One thread of the group, the single card's thread and the running card's: the single card goes, the group
        // waits for its other thread, and the card being carried out is left alone (its own archiving shows up too).
        let r = threads_left(&state, &[left("t1"), left("t2"), left("t4")], &[]).await.unwrap();
        assert_eq!(r, vec![quietdeck_types::mail::LeftCard { id: single, subject: "About t1".into() }]);
        assert_eq!(state_of(&state, single).await, "done");
        assert_eq!(state_of(&state, group).await, "open");
        assert_eq!(state_of(&state, running).await, "executing");

        // t2 came back with new mail before t3 was archived: the group stays.
        assert!(threads_left(&state, &[], &[left("t2")]).await.unwrap().is_empty());
        assert!(threads_left(&state, &[left("t3")], &[]).await.unwrap().is_empty());
        assert_eq!(state_of(&state, group).await, "open");

        // Archived again: now every thread has gone.
        let r = threads_left(&state, &[left("t2")], &[]).await.unwrap();
        assert_eq!(r.iter().map(|c| c.id).collect::<Vec<_>>(), [group]);
        assert_eq!(state_of(&state, group).await, "done");
    }

    #[tokio::test]
    async fn the_report_is_done_only_when_nothing_waits() {
        let state = AppState::for_tests().await;
        let me = SessionRef { node: "n".into(), tab: "t".into(), session_id: None };
        let c = card(&state, "t1", &[], "open").await;
        crate::reports::set(
            &state,
            &me,
            &crate::reports::inbox(&state.db).await.unwrap(),
            quietdeck_types::report::ReportSet { markdown: "# Inbox\n".into(), base: None, new: true, change: None },
        )
        .await
        .unwrap();
        assert!(
            crate::reports::done(&state, &crate::reports::inbox(&state.db).await.unwrap()).await.is_err(),
            "c_{c} still waits"
        );
        threads_left(&state, &[left("t1")], &[]).await.unwrap();
        crate::reports::done(&state, &crate::reports::inbox(&state.db).await.unwrap()).await.unwrap();
        assert!(
            crate::reports::current(&state.db, quietdeck_types::document::INBOX_ID).await.unwrap().unwrap().finished
        );
    }
}
