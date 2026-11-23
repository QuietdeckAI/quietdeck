//! What the REST API and the MCP endpoint both do, so the two accept exactly the same shapes and rules.

use std::time::Duration;

use jiff::Timestamp;
use quietdeck_types::SessionRef;
use quietdeck_types::api::{Caller, Changed, DeckInfo, Settings};
use quietdeck_types::card::{ActionRecord, ActionReport, Card, CardAdded, CardInput, CardSection, CardState, Released};
use quietdeck_types::decision::{ApproveAll, Approving, Decide, Inbox, ReportReceipt, StopResult};
use quietdeck_types::directory::{HitSource, PartialSearch, TranscriptHit, WhoHas, WhoHasHit};
use quietdeck_types::node::{DeckRequest, Delivered, TranscriptSearch};
use quietdeck_types::relay::{DeliveryError, DeliveryKind, MsgSend};
use quietdeck_types::rules::MatchResult;
use quietdeck_types::timeline::{EntrySource, History, TimelineAdd, TimelineEntry};

use crate::db::now_ms;
use crate::error::{AppError, AppResult, bad};
use crate::state::{AppState, VERSION};
use crate::store::{cards, decisions, sessions, timeline};

pub async fn info(state: &AppState, caller: Caller) -> AppResult<DeckInfo> {
    Ok(DeckInfo {
        version: VERSION.to_string(),
        caller,
        accounts: state.config.accounts.clone(),
        boards: state.config.boards.clone(),
        settings: crate::api::load_settings(&state.db).await?,
    })
}

pub async fn add_card(state: &AppState, me: &SessionRef, input: CardInput) -> AppResult<CardAdded> {
    input.check().map_err(bad)?;
    if !state.config.knows_account(&input.account) {
        return Err(AppError::BadRequest(format!(
            "unknown account {:?}: deck.yaml lists {}",
            input.account,
            state.config.accounts.iter().map(|a| a.id.as_str()).collect::<Vec<_>>().join(", ")
        )));
    }
    for a in &input.actions {
        if let Some(acct) = a.account()
            && !state.config.knows_account(acct)
        {
            return Err(AppError::BadRequest(format!("an action names unknown account {acct:?}")));
        }
    }
    // Automatic cards go through `rules match`, which arrives with the rules (build step 6). Until then every card
    // waits for the person.
    let auto = input.auto_grant.map(|grant_id| MatchResult {
        allow: false,
        grant_id,
        reason: "no automatic actions on this deck yet: stored as a normal card".into(),
    });
    let added = cards::add(&state.db, me, &input, CardState::Open, None).await?;
    state.changed(Changed::Cards);
    Ok(CardAdded { card: added.card, replaced: added.replaced, auto })
}

/// "Who has this ID?": the summaries on the deck, and the raw transcripts on every connected node.
pub async fn who_has(state: &AppState, query: &str) -> AppResult<WhoHas> {
    let query = query.trim();
    if query.is_empty() {
        return Err(AppError::BadRequest("who-has needs something to look for".into()));
    }
    let mut hits = sessions::summary_hits(&state.db, query).await?;
    let enrolled: Vec<String> =
        sqlx::query_as::<_, (String,)>("SELECT name FROM nodes WHERE revoked_at IS NULL ORDER BY name")
            .fetch_all(&state.db)
            .await?
            .into_iter()
            .map(|r| r.0)
            .collect();
    let connected = state.hub.connected_nodes().await;
    let asks = connected.iter().map(|node| {
        let hub = state.hub.clone();
        let q = query.to_string();
        async move {
            let r =
                hub.request(node, DeckRequest::SearchTranscripts { query: q, limit: 50 }, Duration::from_secs(9)).await;
            (node.clone(), r.ok().and_then(|reply| reply.into_result::<TranscriptSearch>().ok()))
        }
    });
    let answers = futures::future::join_all(asks).await;
    let mut offline: Vec<String> = enrolled.iter().filter(|n| !connected.contains(n)).cloned().collect();
    let mut partial = Vec::new();
    for (node, answer) in answers {
        let Some(TranscriptSearch { hits: found, searched_back_to }) = answer else {
            offline.push(node);
            continue;
        };
        if let Some(t) = searched_back_to {
            partial.push(PartialSearch { node: node.clone(), searched_back_to: t });
        }
        for TranscriptHit { session_id, last_mentioned_at } in found {
            let tab = sessions::tab_of(&state.db, &node, &session_id).await?;
            match hits.iter_mut().find(|h| h.node == node && h.session_id == session_id) {
                // A transcript mention is the stronger evidence, and usually the newer one.
                Some(h) => {
                    h.source = HitSource::Transcript;
                    if last_mentioned_at > h.last_mentioned_at {
                        h.last_mentioned_at = last_mentioned_at;
                    }
                }
                None => hits.push(WhoHasHit {
                    node: node.clone(),
                    session_id,
                    tab,
                    source: HitSource::Transcript,
                    last_mentioned_at,
                }),
            }
        }
    }
    hits.sort_by_key(|h| std::cmp::Reverse(h.last_mentioned_at));
    offline.sort();
    offline.dedup();
    Ok(WhoHas { query: query.to_string(), hits, offline_nodes: offline, partial })
}

pub async fn msg_send(state: &AppState, me: &SessionRef, msg: MsgSend) -> AppResult<Delivered> {
    garde::Validate::validate(&msg).map_err(bad)?;
    crate::relay::send(&state.db, &state.hub, &msg.to, DeliveryKind::Message { from: me.clone() }, msg.text)
        .await
        .map_err(|e| match e {
            DeliveryError::SessionOffline { .. } | DeliveryError::NodeOffline { .. } => {
                AppError::Offline(e.to_string())
            }
            DeliveryError::Ambiguous { .. } => AppError::BadRequest(e.to_string()),
            DeliveryError::Failed { .. } => AppError::Unavailable(e.to_string()),
        })
}

pub async fn timeline_add(state: &AppState, caller: &Caller, e: TimelineAdd) -> AppResult<TimelineEntry> {
    garde::Validate::validate(&e).map_err(bad)?;
    let source = match caller {
        Caller::Session { .. } => EntrySource::Agent,
        Caller::Node { .. } => EntrySource::Runner,
        Caller::Owner { .. } => {
            return Err(AppError::Forbidden("timeline entries come from agents and runners".into()));
        }
    };
    let entry = timeline::add(&state.db, source, &e).await?;
    state.changed(Changed::Timeline);
    Ok(entry)
}

fn cursor_key(me: &SessionRef) -> String {
    me.session_id.clone().unwrap_or_else(|| format!("tab:{}", me.tab))
}

/// What happened since the session last asked (or since a time): timeline entries and cards that changed.
pub async fn history(state: &AppState, me: &SessionRef, since: &str) -> AppResult<History> {
    let until = now_ms();
    let since_ms = if since == "last" {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT read_until FROM history_cursors WHERE node = ? AND session_key = ?")
                .bind(&me.node)
                .bind(cursor_key(me))
                .fetch_optional(&state.db)
                .await?;
        row.map(|r| r.0).unwrap_or(until - 24 * 3600 * 1000)
    } else {
        since
            .parse::<Timestamp>()
            .map_err(|e| AppError::BadRequest(format!("--since takes `last` or an RFC 3339 time: {e}")))?
            .as_millisecond()
    };
    let entries = timeline::between(&state.db, since_ms, until).await?;
    let cards = cards::changed_since(&state.db, since_ms, until).await?;
    sqlx::query(
        "INSERT INTO history_cursors (node, session_key, read_until) VALUES (?, ?, ?) \
         ON CONFLICT (node, session_key) DO UPDATE SET read_until = MAX(read_until, excluded.read_until)",
    )
    .bind(&me.node)
    .bind(cursor_key(me))
    .bind(until)
    .execute(&state.db)
    .await?;
    Ok(History { since: crate::db::ts(since_ms), until: crate::db::ts(until), entries, cards })
}

// Decisions, the Inbox and History.

pub async fn approve_all(state: &AppState, req: ApproveAll) -> AppResult<Approving> {
    garde::Validate::validate(&req).map_err(bad)?;
    let settings = crate::api::load_settings(&state.db).await?;
    let a = decisions::approve_all(&state.db, &req, settings.undo_seconds).await?;
    state.release.notify_one();
    state.changed(Changed::Cards);
    Ok(a)
}

pub async fn undo(state: &AppState, batch: &str) -> AppResult<()> {
    decisions::undo(&state.db, batch).await?;
    state.changed(Changed::Cards);
    Ok(())
}

pub async fn stop(state: &AppState) -> AppResult<StopResult> {
    let r = decisions::stop(&state.db).await?;
    state.changed(Changed::Cards);
    Ok(r)
}

pub async fn retry(state: &AppState, card_id: i64) -> AppResult<Approving> {
    let a = decisions::retry(&state.db, card_id).await?;
    state.release.notify_one();
    state.changed(Changed::Cards);
    Ok(a)
}

pub async fn decide(state: &AppState, card_id: i64, d: Decide) -> AppResult<Card> {
    let c = decisions::decide(&state.db, card_id, &d).await?;
    state.changed(Changed::Cards);
    Ok(c)
}

pub async fn take(state: &AppState, me: &SessionRef) -> AppResult<Vec<Released>> {
    let r = decisions::take(&state.db, me).await?;
    if !r.is_empty() {
        state.changed(Changed::Cards);
    }
    Ok(r)
}

pub async fn action_start(state: &AppState, me: &SessionRef, action_id: i64) -> AppResult<ActionRecord> {
    let a = decisions::start(&state.db, me, action_id).await?;
    state.changed(Changed::Cards);
    Ok(a)
}

pub async fn actions_report(
    state: &AppState,
    me: &SessionRef,
    reports: Vec<ActionReport>,
) -> AppResult<Vec<ReportReceipt>> {
    if reports.is_empty() || reports.len() > 500 {
        return Err(AppError::BadRequest("report 1 to 500 actions at a time".into()));
    }
    let r = decisions::report(&state.db, me, &reports).await?;
    state.changed(Changed::Cards);
    Ok(r)
}

pub async fn inbox(state: &AppState, account: Option<&str>) -> AppResult<Inbox> {
    let mut inbox = Inbox {
        actions: vec![],
        suggestions: vec![],
        archive_list: vec![],
        sent: vec![],
        timeline: timeline::unread(&state.db, 500).await?,
        has_later: cards::has_later(&state.db, account).await?,
        approving: decisions::approving(&state.db).await?,
        running: decisions::running(&state.db).await?,
        last_agent_at: decisions::last_agent_at(&state.db).await?.map(crate::db::ts),
    };
    if let Some(a) = account {
        inbox.timeline.retain(|e| e.account.as_deref() == Some(a));
    }
    for card in cards::inbox(&state.db, account).await? {
        match card.input.section {
            CardSection::Actions => inbox.actions.push(card),
            CardSection::Suggestion => inbox.suggestions.push(card),
            CardSection::ArchiveList => inbox.archive_list.push(card),
            CardSection::Sent => inbox.sent.push(card),
            CardSection::Later => {}
        }
    }
    Ok(inbox)
}

/// The History screen: finished cards and read timeline entries in a window (default: the last seven days).
pub async fn owner_history(
    state: &AppState,
    since: Option<&str>,
    until: Option<&str>,
    account: Option<&str>,
) -> AppResult<History> {
    let parse = |s: &str| {
        s.parse::<Timestamp>()
            .map(|t| t.as_millisecond())
            .map_err(|e| AppError::BadRequest(format!("times are RFC 3339: {e}")))
    };
    let until_ms = until.map(parse).transpose()?.unwrap_or_else(now_ms);
    let since_ms = since.map(parse).transpose()?.unwrap_or(until_ms - 7 * 24 * 3600 * 1000);
    let mut entries = timeline::between(&state.db, since_ms, until_ms).await?;
    entries.retain(|e| e.read_at.is_some() && account.is_none_or(|a| e.account.as_deref() == Some(a)));
    let mut cards = cards::changed_since(&state.db, since_ms, until_ms).await?;
    cards.retain(|c| {
        (c.state.is_finished() || c.state == CardState::Automatic) && account.is_none_or(|a| c.input.account == a)
    });
    Ok(History { since: crate::db::ts(since_ms), until: crate::db::ts(until_ms), entries, cards })
}

pub async fn later(state: &AppState, account: Option<&str>) -> AppResult<Vec<Card>> {
    let f = cards::ListFilter {
        account: account.map(str::to_string),
        state: Some(CardState::Later),
        limit: Some(500),
        ..Default::default()
    };
    Ok(cards::list(&state.db, &f).await?)
}

pub async fn save_settings(state: &AppState, s: Settings) -> AppResult<Settings> {
    garde::Validate::validate(&s).map_err(bad)?;
    for (name, value) in [
        ("undo_seconds", s.undo_seconds.to_string()),
        ("require_filter_proposal", s.require_filter_proposal.to_string()),
    ] {
        sqlx::query(
            "INSERT INTO settings (name, value) VALUES (?, ?) \
             ON CONFLICT (owner_type, owner_id, name) DO UPDATE SET value = excluded.value",
        )
        .bind(name)
        .bind(value)
        .execute(&state.db)
        .await?;
    }
    state.changed(Changed::Settings);
    Ok(crate::api::load_settings(&state.db).await?)
}

/// Releases each Approve All batch when its undo window ends, and tells each agent it has decisions to take. One
/// task for the deck, woken when a batch is recorded.
pub async fn releaser(state: AppState) {
    loop {
        let wait = match decisions::next_due(&state.db).await {
            Ok(Some(due)) => Duration::from_millis(due.saturating_sub(now_ms()).max(0) as u64),
            Ok(None) => Duration::from_secs(3600),
            Err(e) => {
                tracing::error!(error = %e, "releaser: reading the next batch");
                Duration::from_secs(5)
            }
        };
        if !wait.is_zero() {
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = state.release.notified() => continue,
            }
        }
        match decisions::release_due(&state.db).await {
            Ok(waiting) => {
                state.changed(Changed::Cards);
                state.changed(Changed::Timeline);
                for w in waiting {
                    tell_agent(&state, &w).await;
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "releaser: releasing");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

/// With no runner yet (build step 5), the agent that posted the cards carries them out: tell it they're waiting. An
/// agent that's offline finds them with `quietdeck decisions take` when it next runs.
async fn tell_agent(state: &AppState, w: &decisions::Waiting) {
    let body = format!(
        "{} decision{} on your cards {} waiting. Take them with `quietdeck decisions take`, run each action between \
         `quietdeck action start <id>` and your own tools, then report every result in one call with \
         `quietdeck actions report --file <results.json>`.",
        w.decisions,
        if w.decisions == 1 { "" } else { "s" },
        if w.decisions == 1 { "is" } else { "are" }
    );
    let address = format!("{}@{}", w.tab, w.node);
    if let Err(e) = crate::relay::send(&state.db, &state.hub, &address, DeliveryKind::Deck, body).await {
        tracing::info!(%address, error = %e, "decisions wait for an agent that isn't reachable");
    }
}
