//! Messages from chat sources other than mail (Matrix first, sc-7779). A node that reads a source hands new messages
//! over its connection (`messages_ingest`); the deck keeps each once, by room and ID, and tells every connected runner
//! how many arrived, so it counts them into its agent's next wake-up. Agents read them with `quietdeck messages`.

use quietdeck_types::SessionRef;
use quietdeck_types::message::{
    ChannelCount, ChannelMessage, IncomingMessage, MAX_INGEST, MessagesPage, MessagesStored, SOURCES, clip_body,
};
use quietdeck_types::node::DeckRequest;
use quietdeck_types::reading::{HoldVerdict, ItemFacts};

use crate::db::{Db, now_ms, ts};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Most messages one read returns.
pub const MAX_PAGE: u32 = 200;

fn check(m: &IncomingMessage) -> AppResult<()> {
    if !SOURCES.contains(&m.source.as_str()) {
        return Err(AppError::BadRequest(format!("unknown message source {:?}", m.source)));
    }
    for (name, v, max) in [
        ("channel", &m.channel, 255),
        ("channel_name", &m.channel_name, 255),
        ("external_id", &m.external_id, 255),
        ("sender", &m.sender, 255),
    ] {
        if v.trim().is_empty() || v.len() > max {
            return Err(AppError::BadRequest(format!("a message's {name} must be 1 to {max} bytes")));
        }
    }
    Ok(())
}

/// Keep what's new of `messages` from `node`. Returns the new ones' counts per room, in the order they came.
pub async fn store(db: &Db, node: &str, messages: &[IncomingMessage]) -> AppResult<Vec<ChannelCount>> {
    if messages.len() > MAX_INGEST {
        return Err(AppError::BadRequest(format!("send at most {MAX_INGEST} messages at a time")));
    }
    for m in messages {
        check(m)?;
    }
    let now = now_ms();
    let mut counts: Vec<ChannelCount> = vec![];
    for m in messages {
        let done = sqlx::query(
            "INSERT INTO messages (source, channel, channel_name, external_id, sender, body, sent_at, node, \
             received_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (source, channel, external_id) DO NOTHING",
        )
        .bind(&m.source)
        .bind(&m.channel)
        .bind(&m.channel_name)
        .bind(&m.external_id)
        .bind(&m.sender)
        .bind(clip_body(&m.body))
        .bind(m.sent_at.as_millisecond())
        .bind(node)
        .bind(now)
        .execute(db)
        .await?;
        if done.rows_affected() == 0 {
            continue;
        }
        match counts.iter_mut().find(|c| c.source == m.source && c.channel_name == m.channel_name) {
            Some(c) => c.count += 1,
            None => counts.push(ChannelCount {
                source: m.source.clone(),
                channel_name: m.channel_name.clone(),
                count: 1,
                let_through: vec![],
                handled_by: None,
            }),
        }
    }
    Ok(counts)
}

/// `messages_ingest` from a node: keep the new messages and tell the runners.
pub async fn ingest(state: &AppState, node: &str, messages: Vec<IncomingMessage>) -> AppResult<MessagesStored> {
    let mut counts = store(&state.db, node, &messages).await?;
    // While the person reads the Inbox, a rule may let a message through now; the rest wait for the runner's next
    // wake-up after they go idle.
    if crate::reading::is_active(&state.db).await? && !counts.is_empty() {
        let rules = crate::reading::list(&state.db).await?;
        for m in &messages {
            let item = ItemFacts {
                source: m.source.clone(),
                account: m.channel_name.clone(),
                thread_key: Some(m.channel.clone()),
                from: Some(m.sender.clone()),
                text: Some(m.body.clone()),
                ..ItemFacts::default()
            };
            if let HoldVerdict::Let { reason } = crate::reading::verdict(&state.db, &rules, &item).await?
                && let Some(c) = counts.iter_mut().find(|c| c.channel_name == m.channel_name)
            {
                c.let_through.push(reason);
            }
        }
    }
    let new: u32 = counts.iter().map(|c| c.count).sum();
    if new > 0 {
        tracing::info!(%node, new, "messages arrived");
        let state = state.clone();
        tokio::spawn(async move { notify_runners(&state, counts).await });
    }
    Ok(MessagesStored { new })
}

/// Tell every connected runner what arrived. A runner that's offline misses the count, not the messages: its agent
/// reads them with `quietdeck messages --new` on its next round.
async fn notify_runners(state: &AppState, channels: Vec<ChannelCount>) {
    if crate::runner::tell_runners(state, DeckRequest::MessagesArrived { channels }).await == 0 {
        tracing::info!("no runner was connected to hear about new messages");
    }
}

fn cursor_key(me: &SessionRef) -> String {
    me.session_id.clone().unwrap_or_else(|| format!("tab:{}", me.tab))
}

type Row = (i64, String, String, String, String, String, String, i64, String, i64);

fn from_row(r: Row) -> ChannelMessage {
    ChannelMessage {
        id: r.0,
        source: r.1,
        channel: r.2,
        channel_name: r.3,
        external_id: r.4,
        sender: r.5,
        body: r.6,
        sent_at: ts(r.7),
        node: r.8,
        received_at: ts(r.9),
    }
}

/// What a session reads. `after`: a message ID, or None for "after what this session last read" (from now, the
/// first time: the deck's last day). Reading without `after` moves the session's mark to the last message returned.
pub async fn read(
    db: &Db,
    me: &SessionRef,
    source: Option<&str>,
    after: Option<i64>,
    limit: u32,
) -> AppResult<MessagesPage> {
    if let Some(s) = source
        && !SOURCES.contains(&s)
    {
        return Err(AppError::BadRequest(format!("unknown message source {s:?}: try {}", SOURCES.join(", "))));
    }
    let limit = limit.clamp(1, MAX_PAGE);
    let cursor_source = source.unwrap_or("*");
    let start = match after {
        Some(id) => id,
        None => {
            let row: Option<(i64,)> = sqlx::query_as(
                "SELECT read_until FROM message_cursors WHERE node = ? AND session_key = ? AND source = ?",
            )
            .bind(&me.node)
            .bind(cursor_key(me))
            .bind(cursor_source)
            .fetch_optional(db)
            .await?;
            match row {
                Some((id,)) => id,
                None => {
                    // First read: the last day's messages.
                    let since = now_ms() - 24 * 3600 * 1000;
                    let r: (Option<i64>,) = sqlx::query_as("SELECT MAX(id) FROM messages WHERE received_at < ?")
                        .bind(since)
                        .fetch_one(db)
                        .await?;
                    r.0.unwrap_or(0)
                }
            }
        }
    };
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, source, channel, channel_name, external_id, sender, body, sent_at, node, received_at \
         FROM messages WHERE id > ? AND (? IS NULL OR source = ?) ORDER BY id LIMIT ?",
    )
    .bind(start)
    .bind(source)
    .bind(source)
    .bind(i64::from(limit) + 1)
    .fetch_all(db)
    .await?;
    let more = rows.len() > limit as usize;
    let messages: Vec<ChannelMessage> = rows.into_iter().take(limit as usize).map(from_row).collect();
    if after.is_none()
        && let Some(last) = messages.last()
    {
        sqlx::query(
            "INSERT INTO message_cursors (node, session_key, source, read_until) VALUES (?, ?, ?, ?) \
             ON CONFLICT (node, session_key, source) DO UPDATE SET read_until = MAX(read_until, excluded.read_until)",
        )
        .bind(&me.node)
        .bind(cursor_key(me))
        .bind(cursor_source)
        .bind(last.id)
        .execute(db)
        .await?;
    }
    Ok(MessagesPage { messages, more })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn by_room(counts: &[ChannelCount]) -> BTreeMap<String, u32> {
        counts.iter().map(|c| (c.channel_name.clone(), c.count)).collect()
    }

    fn msg(id: &str, body: &str) -> IncomingMessage {
        IncomingMessage {
            source: "matrix".into(),
            channel: "!room:example.com".into(),
            channel_name: "Infrastructure Alerts".into(),
            external_id: id.into(),
            sender: "@alertmanager:example.com".into(),
            body: body.into(),
            sent_at: jiff::Timestamp::now(),
        }
    }

    fn me(tab: &str) -> SessionRef {
        SessionRef { node: "agents-triage".into(), tab: tab.into(), session_id: Some(format!("s-{tab}")) }
    }

    #[tokio::test]
    async fn each_message_is_kept_once_and_read_once_per_session() {
        let db = crate::db::memory().await;
        let counts = store(&db, "agents", &[msg("$1", "disk full"), msg("$2", "resolved")]).await.unwrap();
        assert_eq!(by_room(&counts), BTreeMap::from([("Infrastructure Alerts".into(), 2)]));
        let again = store(&db, "agents", &[msg("$2", "resolved"), msg("$3", "cpu")]).await.unwrap();
        assert_eq!(by_room(&again), BTreeMap::from([("Infrastructure Alerts".into(), 1)]));

        let triage = me("email triage");
        let page = read(&db, &triage, Some("matrix"), None, 50).await.unwrap();
        assert_eq!(page.messages.iter().map(|m| m.external_id.as_str()).collect::<Vec<_>>(), ["$1", "$2", "$3"]);
        assert!(!page.more);
        assert!(read(&db, &triage, Some("matrix"), None, 50).await.unwrap().messages.is_empty());

        // Another session has its own mark; reading after an ID moves no mark.
        let other = me("homeserver");
        let page = read(&db, &other, None, Some(page.messages[0].id), 1).await.unwrap();
        assert_eq!(page.messages[0].external_id, "$2");
        assert!(page.more);
        assert_eq!(read(&db, &other, None, None, 50).await.unwrap().messages.len(), 3);

        store(&db, "agents", &[msg("$4", "new")]).await.unwrap();
        let page = read(&db, &triage, Some("matrix"), None, 50).await.unwrap();
        assert_eq!(page.messages.iter().map(|m| m.external_id.as_str()).collect::<Vec<_>>(), ["$4"]);
    }

    #[tokio::test]
    async fn bad_messages_are_refused_whole() {
        let db = crate::db::memory().await;
        let mut bad = msg("$1", "x");
        bad.source = "irc".into();
        assert!(store(&db, "agents", &[msg("$0", "fine"), bad]).await.is_err());
        let mut empty = msg("", "x");
        empty.external_id = " ".into();
        assert!(store(&db, "agents", &[empty]).await.is_err());
        assert!(read(&db, &me("t"), None, None, 50).await.unwrap().messages.is_empty());
        assert!(read(&db, &me("t"), Some("irc"), None, 50).await.is_err());
    }
}
