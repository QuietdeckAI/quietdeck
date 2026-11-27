//! What the REST API and the MCP endpoint both do, so the two accept exactly the same shapes and rules.

use std::time::Duration;

use jiff::Timestamp;
use quietdeck_types::SessionRef;
use quietdeck_types::api::{Caller, Changed, DeckInfo, Settings};
use quietdeck_types::card::{
    ActionRecord, ActionReport, Card, CardAdded, CardInput, CardSection, CardState, Released, RunBy,
};
use quietdeck_types::decision::{ApproveAll, Approving, CardDecision, Decide, Inbox, ReportReceipt, StopResult};
use quietdeck_types::directory::{HitSource, PartialSearch, SessionRow, TranscriptHit, WhoHas, WhoHasHit};
use quietdeck_types::node::{DeckRequest, TranscriptSearch};
use quietdeck_types::relay::DeliveryKind;
use quietdeck_types::rules::{
    MatchRequest, MatchResult, ProjectPolicy, Rule, RuleInput, RuleOffer, RuleStatus, RuleStatusChange, RulesView,
};
use quietdeck_types::timeline::{Comment, CommentAdd, EntrySource, History, TimelineAdd, TimelineEntry, TimelineKind};

use crate::db::now_ms;
use crate::error::{AppError, AppResult, bad};
use crate::state::{AppState, BUILT_AT, COMMIT, STARTED_AT, VERSION};
use crate::store::{cards, comments, decisions, rules, sessions, timeline};

pub async fn info(state: &AppState, caller: Caller) -> AppResult<DeckInfo> {
    Ok(DeckInfo {
        version: VERSION.to_string(),
        commit: COMMIT.filter(|c| !c.is_empty()).map(str::to_string),
        built_at: BUILT_AT.and_then(|s| s.parse::<i64>().ok()).and_then(|s| jiff::Timestamp::from_second(s).ok()),
        started_at: *STARTED_AT,
        caller,
        accounts: state.config.accounts.clone(),
        boards: state.config.boards.clone(),
        settings: crate::api::load_settings(&state.db).await?,
        solo: state.solo,
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
    crate::runner::check_send_as(state, me, &input.actions).await?;
    // An automatic card goes through `rules match`. Allowed, it's released at once; denied, it waits for the person
    // like any card.
    let (auto, starts) = match input.auto_grant {
        Some(grant_id) => {
            let (m, starts) = crate::rules::check(&state.db, &state.config, &input, grant_id).await?;
            (Some(m), starts)
        }
        None => (None, 0),
    };
    let allowed = auto.as_ref().filter(|m| m.allow).map(|m| m.grant_id);
    let state_now = if allowed.is_some() { CardState::Automatic } else { CardState::Open };
    let added = cards::add(&state.db, me, &input, state_now, allowed).await?;
    let mut card = added.card;
    if let Some(grant_id) = allowed {
        let waiting = decisions::release_automatic(&state.db, card.id, card.version, grant_id, starts).await?;
        let entry = TimelineAdd {
            kind: TimelineKind::AutoAction,
            account: Some(card.input.account.clone()),
            refs: vec![format!("c_{}", card.id), format!("g_{grant_id}")],
            text: format!("{}: {} (rule {grant_id})", card.input.subject, describe_actions(&card.input.actions)),
            lease_id: None,
        };
        timeline::add(&state.db, EntrySource::Deck, Some(me), &entry).await?;
        state.changed(Changed::Timeline);
        announce(state, &[waiting]).await;
        card = cards::get(&state.db, card.id).await?.ok_or_else(|| anyhow::anyhow!("card {} vanished", card.id))?;
    }
    state.changed(Changed::Cards);
    Ok(CardAdded { card, replaced: added.replaced, auto })
}

/// The actions in a few words, for a timeline entry.
fn describe_actions(actions: &[quietdeck_types::Action]) -> String {
    use quietdeck_types::Action;
    let words: Vec<String> = actions
        .iter()
        .map(|a| match a {
            Action::Forward { to, .. } => format!("forward to {}", to.tab),
            Action::Resume { tab, .. } => format!("resume {tab}"),
            Action::NewSession { tab, .. } => format!("start a session in {tab}"),
            other => other.action_type().as_str().replace('_', " "),
        })
        .collect();
    words.join(", ")
}

// Rules and comments.

pub async fn rules_view(state: &AppState) -> AppResult<RulesView> {
    let policies = state
        .config
        .projects
        .iter()
        .map(|(name, p)| ProjectPolicy {
            name: name.clone(),
            dirs: p.dirs.clone(),
            accounts: p.accounts.clone(),
            automatic: p.automatic.clone(),
        })
        .collect();
    Ok(RulesView {
        rules: rules::list(&state.db).await?,
        policies,
        session_starts_per_day: crate::rules::SESSION_STARTS_PER_DAY,
    })
}

/// Fill in the session a rule is tied to with the running session in that tab, so a later session in the same tab
/// isn't covered by it.
async fn with_session_id(state: &AppState, mut input: RuleInput) -> AppResult<RuleInput> {
    input.check().map_err(bad)?;
    if let Some(p) = &input.project
        && !state.config.projects.contains_key(p)
    {
        return Err(AppError::BadRequest(format!("project {p} isn't in deck.yaml")));
    }
    if let Some(a) = &input.criteria.account
        && !state.config.knows_account(a)
    {
        return Err(AppError::BadRequest(format!("unknown account {a:?}")));
    }
    let tie = input.session_ref.clone().or_else(|| input.action_template.target.clone());
    if let Some(mut r) = tie {
        if r.session_id.is_none() {
            r.session_id = sessions::running_in_tab(&state.db, &r.node, &r.tab).await?;
        }
        input.session_ref = Some(r);
    }
    Ok(input)
}

pub async fn rule_create(state: &AppState, input: RuleInput) -> AppResult<Rule> {
    let input = with_session_id(state, input).await?;
    let r = rules::create(&state.db, &input).await?;
    state.changed(Changed::Rules);
    Ok(r)
}

pub async fn rule_update(state: &AppState, id: i64, input: RuleInput) -> AppResult<Rule> {
    let input = with_session_id(state, input).await?;
    let r = rules::update(&state.db, id, &input).await?;
    state.changed(Changed::Rules);
    Ok(r)
}

pub async fn rule_status(state: &AppState, id: i64, change: RuleStatusChange) -> AppResult<Rule> {
    garde::Validate::validate(&change).map_err(bad)?;
    let reason = (change.status != RuleStatus::Active).then_some("by the person");
    let r = rules::set_status(&state.db, id, change.status, reason, change.move_to.as_ref()).await?;
    state.changed(Changed::Rules);
    Ok(r)
}

/// `quietdeck rules match` on its own: the answer card add --auto would get, without posting or counting anything.
pub async fn rules_match(state: &AppState, req: MatchRequest) -> AppResult<MatchResult> {
    req.card.check().map_err(bad)?;
    Ok(crate::rules::check(&state.db, &state.config, &req.card, req.grant_id).await?.0)
}

/// A session closed: archive the grants tied to it, each with an entry he sees.
pub async fn session_closed(state: &AppState, node: &str, session_id: &str) -> AppResult<()> {
    let tab = sessions::tab_of(&state.db, node, session_id).await?;
    crate::leases::session_closed(state, node, session_id, tab.as_deref()).await?;
    let archived = rules::archive_for_session(&state.db, node, session_id, tab.as_deref()).await?;
    for r in &archived {
        let entry = TimelineAdd {
            kind: TimelineKind::GrantArchived,
            account: r.criteria.account.clone(),
            refs: vec![format!("g_{}", r.id)],
            text: format!(
                "Rule {} archived: {}. It moves to a new session from the Rules screen, or from a card that forwards there.",
                r.id,
                r.archived_reason.as_deref().unwrap_or("its session closed")
            ),
            lease_id: None,
        };
        timeline::add(&state.db, EntrySource::Deck, None, &entry).await?;
    }
    if !archived.is_empty() {
        state.changed(Changed::Rules);
        state.changed(Changed::Timeline);
    }
    Ok(())
}

/// The person comments on a timeline entry or a History row: it goes to the session behind it (the triage tab for
/// the deck's own entries) as a message, and waits in that session's `history` if the message can't be delivered.
pub async fn comment_add(state: &AppState, c: CommentAdd) -> AppResult<Comment> {
    garde::Validate::validate(&c).map_err(bad)?;
    let fallback = state.config.leases.fallback.as_ref().map(|f| SessionRef {
        node: f.node.clone(),
        tab: f.tab.clone(),
        session_id: None,
    });
    let mut comment = comments::add(&state.db, c.target, &c.text, fallback.as_ref()).await?;
    if let Some(to) = &comment.to {
        let body = format!(
            "The person commented on \"{}\": {}\n\nPropose anything it asks for as a card.",
            comment.on, comment.text
        );
        match crate::relay::send(&state.db, &state.hub, &to.address(), DeliveryKind::Deck, body).await {
            Ok(_) => {
                comments::mark_picked_up(&state.db, comment.id).await?;
                comment = comments::get(&state.db, comment.id).await?.unwrap_or(comment);
            }
            Err(e) => tracing::info!(to = %to.address(), error = %e, "a comment waits for its session's history"),
        }
    }
    state.changed(Changed::Timeline);
    Ok(comment)
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
                    moved_to: None,
                }),
            }
        }
    }
    for h in &mut hits {
        let (_, moved_to) = crate::handoffs::links(&state.db, &h.node, &h.session_id).await?;
        h.moved_to = moved_to.map(|m| m.session.address());
    }
    hits.sort_by_key(|h| std::cmp::Reverse(h.last_mentioned_at));
    offline.sort();
    offline.dedup();
    Ok(WhoHas { query: query.to_string(), hits, offline_nodes: offline, partial })
}

pub async fn timeline_add(state: &AppState, caller: &Caller, e: TimelineAdd) -> AppResult<TimelineEntry> {
    garde::Validate::validate(&e).map_err(bad)?;
    let (source, posted_by) = match caller {
        Caller::Session { session } => (EntrySource::Agent, Some(session.clone())),
        Caller::Node { node } => {
            let agent = crate::store::nodes::runner_info(&state.db, node).await?.map(|r| {
                let (node, tab) = r.agent(node);
                SessionRef { node, tab, session_id: None }
            });
            (EntrySource::Runner, agent)
        }
        Caller::Owner { .. } => {
            return Err(AppError::Forbidden("timeline entries come from agents and runners".into()));
        }
    };
    let entry = timeline::add(&state.db, source, posted_by.as_ref(), &e).await?;
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
    let comments = comments::take(&state.db, me).await?;
    sqlx::query(
        "INSERT INTO history_cursors (node, session_key, read_until) VALUES (?, ?, ?) \
         ON CONFLICT (node, session_key) DO UPDATE SET read_until = MAX(read_until, excluded.read_until)",
    )
    .bind(&me.node)
    .bind(cursor_key(me))
    .bind(until)
    .execute(&state.db)
    .await?;
    Ok(History {
        since: crate::db::ts(since_ms),
        until: crate::db::ts(until),
        entries,
        cards,
        comments,
        snapshots: vec![],
    })
}

// Decisions, the Inbox and History.

pub async fn approve_all(state: &AppState, req: ApproveAll) -> AppResult<Approving> {
    garde::Validate::validate(&req).map_err(bad)?;
    let settings = crate::api::load_settings(&state.db).await?;
    let a = decisions::approve_all(&state.db, &req, settings.undo_seconds).await?;
    // History keeps the Inbox document as it was at this moment (Approve All only, not one default's Approve).
    // Approve All doesn't fail over it.
    if !a.cards.is_empty()
        && req.single != Some(true)
        && let Err(e) = crate::reports::snapshot(&state.db, &a.batch, &a.cards).await
    {
        tracing::warn!(error = %e, "couldn't keep the Inbox document for History");
    }
    state.release.notify_one();
    state.changed(Changed::Cards);
    Ok(a)
}

/// Approve All for the report on screen: the cards of every `<Default>` the person didn't change on this version,
/// open or handed off, each approved as it stands (a suggestion with its actions as they are).
pub async fn approve_report(state: &AppState, req: quietdeck_types::decision::ApproveReport) -> AppResult<Approving> {
    garde::Validate::validate(&req).map_err(bad)?;
    let report = crate::reports::current(&state.db)
        .await?
        .filter(|r| r.id == req.report_id)
        .ok_or_else(|| AppError::Conflict("that report isn't the current one: read the new one first".into()))?;
    if report.version != req.report_version {
        return Err(AppError::Conflict(format!(
            "the report changed (version {} is newer than the {} on screen): read it, then approve",
            report.version, req.report_version
        )));
    }
    let changed: std::collections::HashSet<&str> = report
        .answers
        .iter()
        .filter(|a| a.kind == quietdeck_types::report::AnswerKind::Change && a.report_version == report.version)
        .map(|a| a.control.as_str())
        .collect();
    let mut seen = std::collections::HashSet::new();
    let mut decisions = Vec::new();
    for (key, ids) in quietdeck_types::report::default_keys(&report.markdown) {
        if changed.contains(key.as_str()) {
            continue;
        }
        for id in ids {
            if !seen.insert(id) {
                continue;
            }
            let Some(card) = cards::get(&state.db, id).await? else { continue };
            if !matches!(card.state, CardState::Open | CardState::HandedOff) {
                continue;
            }
            decisions.push(if card.input.section == CardSection::Suggestion {
                CardDecision {
                    card_id: card.id,
                    version: card.version,
                    kind: quietdeck_types::card::DecisionKind::Edit,
                    fields: Some(card.input.actions.clone()),
                    text: None,
                }
            } else {
                CardDecision {
                    card_id: card.id,
                    version: card.version,
                    kind: quietdeck_types::card::DecisionKind::Approve,
                    fields: None,
                    text: None,
                }
            });
        }
    }
    approve_all(state, ApproveAll { cards: decisions, timeline: req.timeline, single: None }).await
}

pub async fn undo(state: &AppState, batch: &str) -> AppResult<()> {
    decisions::undo(&state.db, batch).await?;
    crate::reports::drop_snapshot(&state.db, batch).await?;
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
    let a = decisions::start(&state.db, me, RunBy::Agent, action_id).await?;
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
    let r = decisions::report(&state.db, me, RunBy::Agent, &reports).await?;
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
        rule_offers: vec![],
        report: crate::reports::current(&state.db).await?,
        presence: crate::reading::current(&state.db).await?,
        finished_cards: vec![],
    };
    if let Some(report) = &inbox.report {
        let (named, _) = quietdeck_types::report::default_cards(&report.markdown);
        inbox.finished_cards = cards::finished_of(&state.db, &named).await?;
    }
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
    for card in inbox.actions.iter().chain(&inbox.suggestions) {
        if !matches!(card.state, CardState::Open | CardState::HandedOff) {
            continue;
        }
        let target = card.input.actions.iter().find_map(|a| match a {
            quietdeck_types::Action::Forward { to, .. } => Some(to.clone()),
            quietdeck_types::Action::Resume { node, tab, session_id, .. } => {
                Some(SessionRef { node: node.clone(), tab: tab.clone(), session_id: Some(session_id.clone()) })
            }
            quietdeck_types::Action::NewSession { node, tab, .. } => {
                Some(SessionRef { node: node.clone(), tab: tab.clone(), session_id: None })
            }
            _ => None,
        });
        let Some(mut to) = target else { continue };
        if to.session_id.is_none() {
            to.session_id = sessions::running_in_tab(&state.db, &to.node, &to.tab).await?;
        }
        let rule_ids = rules::archived_for_tab(&state.db, &to).await?;
        if !rule_ids.is_empty() {
            inbox.rule_offers.push(RuleOffer { card_id: card.id, rule_ids, to });
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
    let comments = comments::between(&state.db, since_ms, until_ms).await?;
    // The Inbox document spans both accounts, so it's in History's view of everything.
    let snapshots = match account {
        None => crate::reports::snapshots_between(&state.db, since_ms, until_ms).await?,
        Some(_) => vec![],
    };
    Ok(History { since: crate::db::ts(since_ms), until: crate::db::ts(until_ms), entries, cards, comments, snapshots })
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
        ("show_pending_count", s.show_pending_count.to_string()),
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
/// The watchdog over approved work (Nathan, 25 November: "don't let anything get stuck in progress"): every minute,
/// a card the runner claimed 10 minutes ago and hasn't finished, or one released 5 minutes ago that nothing took, goes
/// to its agent with the reason, and the agent is told at once.
pub async fn stuck_watch(state: AppState) {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let stuck = match decisions::unstick(&state.db, now_ms(), 10 * 60_000, 5 * 60_000).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "the stuck-work watchdog");
                continue;
            }
        };
        if stuck.is_empty() {
            continue;
        }
        tracing::warn!(cards = stuck.len(), "approved work was stuck: handed to its agent");
        state.changed(Changed::Cards);
        let mut by_agent: std::collections::BTreeMap<(String, String), Vec<String>> = Default::default();
        for s in stuck {
            by_agent.entry((s.node, s.tab)).or_default().push(format!("c_{} ({})", s.card_id, s.reason));
        }
        for ((node, tab), lines) in by_agent {
            let body = format!(
                "Watchdog: {} approved {} stuck and {} now handed to you: {}. Take {} with `quietdeck decisions take`.",
                lines.len(),
                if lines.len() == 1 { "card was" } else { "cards were" },
                if lines.len() == 1 { "is" } else { "are" },
                lines.join("; "),
                if lines.len() == 1 { "it" } else { "them" },
            );
            let to = format!("{tab}@{node}");
            if let Err(e) = crate::relay::send(&state.db, &state.hub, &to, DeliveryKind::Deck, body).await {
                tracing::warn!(%to, error = %e, "telling the agent about stuck work: it'll see it on its next take");
            }
        }
    }
}

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
                state.changed(Changed::Leases);
                announce(&state, &waiting).await;
                crate::leases::dispatch_creates(&state).await;
            }
            Err(e) => {
                tracing::error!(error = %e, "releaser: releasing");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

/// Tell each agent's runner it has released decisions, or the agent itself when no runner takes them.
async fn announce(state: &AppState, waiting: &[decisions::Waiting]) {
    for w in waiting {
        if w.decisions == 0 {
            continue;
        }
        if !crate::runner::notify(state, &w.node, &w.tab).await {
            tell_agent(state, w).await;
        }
    }
}

/// With no runner connected for it, the agent that posted the cards carries them out: tell it they're waiting. An
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

/// One session by ID, `tab@node` or tab, with its summary points when asked.
/// One session, which must be on a single machine.
async fn one_session(state: &AppState, address: &str) -> AppResult<SessionRow> {
    Ok(show_session(state, address, false).await?.session)
}

/// A page of a session's transcript, read on its machine. The deck passes it on and keeps nothing.
pub async fn transcript(
    state: &AppState,
    address: &str,
    query: quietdeck_types::transcript::TranscriptQuery,
) -> AppResult<quietdeck_types::transcript::TranscriptPage> {
    let s = one_session(state, address).await?;
    let request = DeckRequest::Transcript { session_id: s.info.session_id.clone(), query };
    let v = crate::control::ask(state, &s.node, request, Duration::from_secs(20)).await?;
    serde_json::from_value(v).map_err(|e| AppError::Internal(e.into()))
}

/// "Since your last message", from the session's machine. The first one for a session waits for a model call. A
/// machine whose summaries Fleetlayer writes can't answer itself, so then Fleetlayer asks it.
pub async fn catchup(state: &AppState, address: &str) -> AppResult<quietdeck_types::summary::Catchup> {
    let s = one_session(state, address).await?;
    let request = DeckRequest::Catchup { session_id: s.info.session_id.clone() };
    let from_node = async {
        let v = crate::control::ask(state, &s.node, request, Duration::from_secs(120)).await?;
        let reply: quietdeck_types::node::CatchupReply =
            serde_json::from_value(v).map_err(|e| AppError::Internal(e.into()))?;
        Ok::<_, AppError>(reply.catchup)
    }
    .await;
    match (from_node, &state.fleetlayer) {
        (Ok(c), _) => Ok(c),
        (Err(e), Some(fleetlayer)) => match fleetlayer.catchup(&s.info.session_id).await {
            Ok(c) => Ok(c),
            Err(fe) => {
                tracing::info!(error = %fe, "Fleetlayer has no catch-up either");
                Err(e)
            }
        },
        (Err(e), None) => Err(e),
    }
}

pub async fn show_session(
    state: &AppState,
    address: &str,
    with_timeline: bool,
) -> AppResult<quietdeck_types::directory::SessionDetail> {
    let rows = sessions::find_any(&state.db, &state.hub, address).await?;
    let mut nodes: Vec<&str> = rows.iter().map(|r| r.node.as_str()).collect();
    nodes.dedup();
    if nodes.len() > 1 {
        let names: Vec<String> = rows.iter().map(|r| r.address()).collect();
        return Err(AppError::BadRequest(format!(
            "{address} names sessions on several machines: {}",
            names.join(", ")
        )));
    }
    let Some(session) = rows.into_iter().next() else {
        return Err(AppError::NotFound(format!("no session {address}")));
    };
    let timeline = if with_timeline {
        sessions::timeline(&state.db, &session.node, &session.info.session_id).await?
    } else {
        Vec::new()
    };
    let (continues, moved_to) = crate::handoffs::links(&state.db, &session.node, &session.info.session_id).await?;
    Ok(quietdeck_types::directory::SessionDetail { session, timeline, continues, moved_to })
}
