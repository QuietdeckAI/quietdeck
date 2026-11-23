//! What the REST API and the MCP endpoint both do, so the two accept exactly the same shapes and rules.

use std::time::Duration;

use jiff::Timestamp;
use quietdeck_types::SessionRef;
use quietdeck_types::api::{Caller, DeckInfo};
use quietdeck_types::card::{CardAdded, CardInput, CardState};
use quietdeck_types::directory::{HitSource, PartialSearch, TranscriptHit, WhoHas, WhoHasHit};
use quietdeck_types::node::{DeckRequest, Delivered, TranscriptSearch};
use quietdeck_types::relay::{DeliveryError, DeliveryKind, MsgSend};
use quietdeck_types::rules::MatchResult;
use quietdeck_types::timeline::{EntrySource, History, TimelineAdd, TimelineEntry};

use crate::db::now_ms;
use crate::error::{AppError, AppResult, bad};
use crate::state::{AppState, VERSION};
use crate::store::{cards, sessions, timeline};

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
    Ok(timeline::add(&state.db, source, &e).await?)
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
