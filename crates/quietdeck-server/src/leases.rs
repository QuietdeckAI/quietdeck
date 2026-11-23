//! Leases: a session's claim on something for a while. Two kinds ship.
//!
//! - **gmail-filter**: a temporary Gmail filter tied to its holder's session. Approved through a proposal card, or at
//!   once when the person approved the same filter on the same account before. The runner creates and removes it
//!   (without a runner, the holder does, told through the relay).
//! - **heavy-build**: N slots per machine with a FIFO queue, no approval and no side effects.
//!
//! A lease ends when its holder releases it, when its time limit passes, when its holder has been idle or offline
//! for its idle minutes, or when its session closes. One ticker on the deck checks those every 15 seconds.

use std::time::Duration;

use quietdeck_types::action::SessionRef;
use quietdeck_types::api::{Caller, Changed};
use quietdeck_types::lease::{
    CleanupBy, KIND_GMAIL_FILTER, KIND_HEAVY_BUILD, Lease, LeaseAnswer, LeaseKindInfo, LeasePattern, LeaseRequest,
    LeaseState, LeaseStep, LeasesView,
};
use quietdeck_types::node::DeckRequest;
use quietdeck_types::relay::DeliveryKind;
use quietdeck_types::timeline::{EntrySource, TimelineAdd, TimelineKind};
use quietdeck_types::{Action, AgentStatus};
use serde_json::{Value, json};
use sqlx::SqliteConnection;

use crate::db::{Db, enum_str, json as to_json, now_ms, parse_enum, ts, ts_opt};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::store::timeline;

const DEFAULT_TTL_SECONDS: i64 = 24 * 3600;
const DEFAULT_IDLE_MINUTES: i64 = 30;
const MAX_QUERY: usize = 500;
const MAX_FILTERS_PER_SESSION: i64 = 5;
const CLEANUP_OVERDUE_MS: i64 = 15 * 60 * 1000;
const STEP_TIMEOUT: Duration = Duration::from_secs(60);
const TICK: Duration = Duration::from_secs(15);

pub fn kinds() -> Vec<LeaseKindInfo> {
    vec![
        LeaseKindInfo {
            kind: KIND_GMAIL_FILTER.into(),
            slots: None,
            needs_approval: true,
            describe:
                "A temporary Gmail filter that keeps mail out of the inbox while its session works. Removed when \
                       the session goes idle or closes."
                    .into(),
        },
        LeaseKindInfo {
            kind: KIND_HEAVY_BUILD.into(),
            slots: Some(2),
            needs_approval: false,
            describe: "A slot for a big compile or spec run on a machine (deck.yaml sets how many). Queues when every \
                       slot is taken."
                .into(),
        },
    ]
}

// --- gmail-filter ---------------------------------------------------------------------------------------------------

/// The query, checked: it needs a `from:` with a full address or domain.
pub fn check_query(query: &str) -> Result<String, String> {
    let q = query.trim();
    if q.is_empty() {
        return Err("a gmail-filter lease needs its --query".into());
    }
    if q.chars().count() > MAX_QUERY {
        return Err(format!("the query is longer than {MAX_QUERY} characters"));
    }
    let lower = q.to_lowercase();
    let mut rest = lower.as_str();
    while let Some(i) = rest.find("from:") {
        let after = rest[i + 5..].trim_start_matches(['(', '"', '{']);
        let value: String = after.chars().take_while(|c| !c.is_whitespace() && !matches!(c, ')' | '"' | '}')).collect();
        if full_sender(&value) {
            return Ok(q.to_string());
        }
        rest = &rest[i + 5..];
    }
    Err("the query needs a from: with a full address or domain (from:notifications@github.com or from:github.com)"
        .into())
}

fn full_sender(v: &str) -> bool {
    let domain = v.rsplit_once('@').map_or(v, |(_, d)| d);
    !v.contains('*')
        && v.chars().all(|c| c.is_ascii_alphanumeric() || "._%+-@".contains(c))
        && domain.split('.').count() >= 2
        && domain.split('.').all(|part| !part.is_empty())
}

/// The same filter on the same account counts as approved, whichever session asks.
pub fn fingerprint(account: &str, query: &str) -> String {
    format!("{account}\n{}", query.trim())
}

fn query_of(params: &Value) -> &str {
    params.get("query").and_then(Value::as_str).unwrap_or("")
}

fn has_filter(lease: &Lease) -> bool {
    lease.kind_state.as_ref().and_then(|s| s.get("filter_id")).is_some_and(|f| !f.is_null())
}

// --- Rows -----------------------------------------------------------------------------------------------------------

const COLUMNS: &str = "id, kind, resource, params, holder_node, holder_tab, holder_session_id, reason, state, \
    ttl_seconds, idle_minutes, created_at, activated_at, last_renewed_at, expires_at, cleanup_requested_at, cleanup_by, \
    ended_at, end_reason, pattern_id, kind_state, card_id, \
    (SELECT position FROM lease_queue q WHERE q.lease_id = leases.id) AS queue_position";

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    kind: String,
    resource: String,
    params: String,
    holder_node: String,
    holder_tab: String,
    holder_session_id: Option<String>,
    reason: String,
    state: String,
    ttl_seconds: i64,
    idle_minutes: i64,
    created_at: i64,
    activated_at: Option<i64>,
    last_renewed_at: Option<i64>,
    expires_at: Option<i64>,
    cleanup_requested_at: Option<i64>,
    cleanup_by: Option<String>,
    ended_at: Option<i64>,
    end_reason: Option<String>,
    pattern_id: Option<i64>,
    kind_state: Option<String>,
    card_id: Option<i64>,
    queue_position: Option<i64>,
}

fn from_row(r: Row) -> anyhow::Result<Lease> {
    Ok(Lease {
        id: r.id,
        kind: r.kind,
        resource: r.resource,
        params: serde_json::from_str(&r.params)?,
        holder: SessionRef { node: r.holder_node, tab: r.holder_tab, session_id: r.holder_session_id },
        reason: r.reason,
        state: parse_enum(&r.state)?,
        ttl_seconds: r.ttl_seconds,
        idle_minutes: r.idle_minutes,
        created_at: ts(r.created_at),
        activated_at: ts_opt(r.activated_at),
        last_renewed_at: ts_opt(r.last_renewed_at),
        expires_at: ts_opt(r.expires_at),
        cleanup_requested_at: ts_opt(r.cleanup_requested_at),
        cleanup_by: r.cleanup_by.as_deref().map(parse_enum).transpose()?,
        ended_at: ts_opt(r.ended_at),
        end_reason: r.end_reason,
        pattern_id: r.pattern_id,
        kind_state: r.kind_state.as_deref().map(serde_json::from_str).transpose()?,
        card_id: r.card_id,
        queue_position: r.queue_position,
    })
}

pub async fn get(db: &Db, id: i64) -> AppResult<Lease> {
    let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM leases WHERE id = ?")))
        .bind(id)
        .fetch_optional(db)
        .await?;
    match row {
        Some(r) => Ok(from_row(r)?),
        None => Err(AppError::NotFound(format!("no lease {id}"))),
    }
}

async fn select(db: &Db, filter: &str) -> anyhow::Result<Vec<Lease>> {
    let rows: Vec<Row> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM leases WHERE {filter} ORDER BY id")))
            .fetch_all(db)
            .await?;
    rows.into_iter().map(from_row).collect()
}

/// Held, queued, waiting or overdue leases, the ones that ended in the last day, and the approved filters.
pub async fn view(db: &Db) -> anyhow::Result<LeasesView> {
    let since = now_ms() - 24 * 3600 * 1000;
    let mut leases = select(db, &format!("ended_at IS NULL OR ended_at > {since}")).await?;
    leases.reverse();
    type PatternRow = (i64, String, String, Option<i64>, Option<i64>, Option<i64>);
    let rows: Vec<PatternRow> = sqlx::query_as(
        "SELECT id, kind, fingerprint, confirmed_by_card_id, confirmed_at, revoked_at FROM lease_patterns \
         ORDER BY revoked_at IS NOT NULL, id DESC",
    )
    .fetch_all(db)
    .await?;
    let patterns = rows
        .into_iter()
        .map(|(id, kind, fp, card, confirmed, revoked)| {
            let (resource, pattern) = fp.split_once('\n').unwrap_or(("", fp.as_str()));
            LeasePattern {
                id,
                kind,
                resource: resource.to_string(),
                pattern: pattern.to_string(),
                confirmed_by_card_id: card,
                confirmed_at: ts_opt(confirmed),
                revoked_at: ts_opt(revoked),
            }
        })
        .collect();
    Ok(LeasesView { leases, patterns, kinds: kinds() })
}

async fn insert(
    db: &Db,
    kind: &str,
    resource: &str,
    params: &Value,
    holder: &SessionRef,
    req: &LeaseRequest,
    state: LeaseState,
) -> anyhow::Result<i64> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO leases (kind, resource, params, holder_node, holder_tab, holder_session_id, reason, state, \
           ttl_seconds, idle_minutes, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(kind)
    .bind(resource)
    .bind(to_json(params))
    .bind(&holder.node)
    .bind(&holder.tab)
    .bind(&holder.session_id)
    .bind(&req.reason)
    .bind(enum_str(&state))
    .bind(req.ttl_seconds.unwrap_or(DEFAULT_TTL_SECONDS))
    .bind(req.idle_minutes.unwrap_or(DEFAULT_IDLE_MINUTES))
    .bind(now_ms())
    .fetch_one(db)
    .await?;
    Ok(id)
}

async fn set_state(db: &Db, id: i64, state: LeaseState) -> anyhow::Result<()> {
    sqlx::query("UPDATE leases SET state = ? WHERE id = ?").bind(enum_str(&state)).bind(id).execute(db).await?;
    Ok(())
}

/// The confirmed, unrevoked pattern for a fingerprint.
async fn confirmed_pattern(db: &Db, kind: &str, fp: &str) -> anyhow::Result<Option<i64>> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM lease_patterns WHERE kind = ? AND fingerprint = ? AND confirmed_at IS NOT NULL \
         AND revoked_at IS NULL",
    )
    .bind(kind)
    .bind(fp)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.0))
}

async fn upsert_pattern(
    conn: &mut SqliteConnection,
    kind: &str,
    fp: &str,
    card_id: Option<i64>,
    now: i64,
) -> anyhow::Result<i64> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO lease_patterns (kind, fingerprint, confirmed_by_card_id, confirmed_at) VALUES (?, ?, ?, ?) \
         ON CONFLICT (owner_type, owner_id, kind, fingerprint) DO UPDATE SET \
           confirmed_by_card_id = COALESCE(excluded.confirmed_by_card_id, confirmed_by_card_id), \
           confirmed_at = excluded.confirmed_at, revoked_at = NULL \
         RETURNING id",
    )
    .bind(kind)
    .bind(fp)
    .bind(card_id)
    .bind(now)
    .fetch_one(&mut *conn)
    .await?;
    Ok(id)
}

pub async fn revoke_pattern(state: &AppState, id: i64) -> AppResult<LeasesView> {
    let done = sqlx::query("UPDATE lease_patterns SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
        .bind(now_ms())
        .bind(id)
        .execute(&state.db)
        .await?;
    if done.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("no approved filter {id}")));
    }
    state.changed(Changed::Leases);
    Ok(view(&state.db).await?)
}

// --- Proposing and acquiring ----------------------------------------------------------------------------------------

/// `lease propose`: a gmail-filter waits on a card unless the person approved the same filter before. A heavy-build
/// slot needs no approval, so it's the same as acquiring one.
pub async fn propose(state: &AppState, holder: &SessionRef, req: LeaseRequest) -> AppResult<LeaseAnswer> {
    request(state, holder, req, true).await
}

/// `lease acquire`: no card. A gmail-filter needs an approved pattern while "Require a proposal for every new filter"
/// is on.
pub async fn acquire(state: &AppState, holder: &SessionRef, req: LeaseRequest) -> AppResult<LeaseAnswer> {
    request(state, holder, req, false).await
}

async fn request(state: &AppState, holder: &SessionRef, req: LeaseRequest, proposing: bool) -> AppResult<LeaseAnswer> {
    use garde::Validate;
    req.validate().map_err(|e| AppError::BadRequest(e.to_string()))?;
    match req.kind.as_str() {
        KIND_GMAIL_FILTER => gmail_filter(state, holder, req, proposing).await,
        KIND_HEAVY_BUILD => heavy_build(state, holder, req).await,
        other => Err(AppError::BadRequest(format!(
            "unknown lease kind {other:?}: the kinds are gmail-filter and heavy-build"
        ))),
    }
}

async fn gmail_filter(
    state: &AppState,
    holder: &SessionRef,
    req: LeaseRequest,
    proposing: bool,
) -> AppResult<LeaseAnswer> {
    let account =
        req.resource.clone().ok_or_else(|| AppError::BadRequest("a gmail-filter lease needs its --account".into()))?;
    if !state.config.knows_account(&account) {
        return Err(AppError::BadRequest(format!("unknown account {account:?}")));
    }
    let query = check_query(query_of(&req.params)).map_err(AppError::BadRequest)?;
    let held: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM leases WHERE kind = ? AND ended_at IS NULL AND state NOT IN ('released', 'rejected') \
         AND holder_node = ? AND holder_tab = ? AND COALESCE(holder_session_id, '') = COALESCE(?, '')",
    )
    .bind(KIND_GMAIL_FILTER)
    .bind(&holder.node)
    .bind(&holder.tab)
    .bind(&holder.session_id)
    .fetch_one(&state.db)
    .await?;
    if held.0 >= MAX_FILTERS_PER_SESSION {
        return Err(AppError::Conflict(format!(
            "this session already holds {MAX_FILTERS_PER_SESSION} filter leases: release one first"
        )));
    }
    let params = json!({ "query": query });
    let fp = fingerprint(&account, &query);
    let pattern = confirmed_pattern(&state.db, KIND_GMAIL_FILTER, &fp).await?;
    let require = crate::api::load_settings(&state.db).await?.require_filter_proposal;

    let (lease_state, pattern_id, message) = match (pattern, proposing, require) {
        (Some(p), _, _) => (LeaseState::Approved, Some(p), "approved at once: the same filter was approved before"),
        (None, false, true) => {
            return Err(AppError::Forbidden(
                "\"Require a proposal for every new filter\" is on: use `quietdeck lease propose`".into(),
            ));
        }
        (None, false, false) => (LeaseState::Approved, None, "approved without a proposal, which the timeline shows"),
        (None, true, _) => (LeaseState::Proposed, None, "waiting on a card in the person's Inbox"),
    };
    let id = insert(&state.db, KIND_GMAIL_FILTER, &account, &params, holder, &req, lease_state).await?;
    if lease_state == LeaseState::Approved {
        let now = now_ms();
        let pid = match pattern_id {
            Some(p) => p,
            None => {
                let mut conn = state.db.acquire().await?;
                upsert_pattern(&mut conn, KIND_GMAIL_FILTER, &fp, None, now).await?
            }
        };
        sqlx::query("UPDATE leases SET pattern_id = ?, expires_at = ? WHERE id = ?")
            .bind(pid)
            .bind(now + req.ttl_seconds.unwrap_or(DEFAULT_TTL_SECONDS) * 1000)
            .bind(id)
            .execute(&state.db)
            .await?;
    } else {
        let card = proposal_card(&account, &query, holder, &req, id);
        let added = crate::ops::add_card(state, holder, card).await?;
        sqlx::query("UPDATE leases SET card_id = ? WHERE id = ?")
            .bind(added.card.id)
            .bind(id)
            .execute(&state.db)
            .await?;
    }
    state.changed(Changed::Leases);
    if lease_state == LeaseState::Approved {
        let s = state.clone();
        tokio::spawn(async move { dispatch_creates(&s).await });
    }
    Ok(LeaseAnswer { lease: get(&state.db, id).await?, message: message.into() })
}

fn proposal_card(
    account: &str,
    query: &str,
    holder: &SessionRef,
    req: &LeaseRequest,
    lease_id: i64,
) -> quietdeck_types::card::CardInput {
    let idle = req.idle_minutes.unwrap_or(DEFAULT_IDLE_MINUTES);
    let ttl_hours = (req.ttl_seconds.unwrap_or(DEFAULT_TTL_SECONDS) as f64 / 3600.0 * 10.0).round() / 10.0;
    let summary = format!(
        "It ends when that session releases it, has been idle for {idle} minutes or closes, and after {ttl_hours} \
         hours at most. The filter is removed then; what it caught stays labelled Quietdeck/Lease/{}.",
        holder.tab
    );
    let action = Action::Lease {
        kind: KIND_GMAIL_FILTER.into(),
        resource: account.into(),
        params: json!({ "query": query }),
        holder: holder.clone(),
        reason: req.reason.clone(),
    };
    serde_json::from_value(json!({
        "account": account,
        "thread_key": format!("lease-{lease_id}"),
        "subject": format!("{} asks for a temporary Gmail filter", holder.tab),
        "summary": summary,
        "section": "actions",
        "reason": "A session asked for a Gmail filter. Approving it approves this filter on this account for any session.",
        "refs": [format!("lease {lease_id}")],
        "actions": [action],
    }))
    .expect("a proposal card is a CardInput")
}

async fn heavy_build(state: &AppState, holder: &SessionRef, req: LeaseRequest) -> AppResult<LeaseAnswer> {
    let machine = req.resource.clone().unwrap_or_else(|| holder.node.clone());
    let id = insert(&state.db, KIND_HEAVY_BUILD, &machine, &json!({}), holder, &req, LeaseState::Queued).await?;
    let pos: (i64,) = sqlx::query_as("SELECT COALESCE(MAX(position), 0) + 1 FROM lease_queue WHERE resource = ?")
        .bind(&machine)
        .fetch_one(&state.db)
        .await?;
    sqlx::query("INSERT INTO lease_queue (resource, lease_id, position) VALUES (?, ?, ?)")
        .bind(&machine)
        .bind(id)
        .bind(pos.0)
        .execute(&state.db)
        .await?;
    promote(state, &machine).await?;
    let lease = get(&state.db, id).await?;
    let message = if lease.state == LeaseState::Active {
        format!("a heavy-build slot on {machine} is yours")
    } else {
        format!("every heavy-build slot on {machine} is taken: queued")
    };
    state.changed(Changed::Leases);
    Ok(LeaseAnswer { lease, message })
}

/// Fill a machine's free slots from its queue, first come first served.
async fn promote(state: &AppState, machine: &str) -> anyhow::Result<()> {
    let slots = state.config.leases.heavy_build_slots(machine);
    loop {
        let active: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM leases WHERE kind = ? AND resource = ? AND state = 'active'")
                .bind(KIND_HEAVY_BUILD)
                .bind(machine)
                .fetch_one(&state.db)
                .await?;
        if active.0 >= slots {
            return Ok(());
        }
        let next: Option<(i64, i64)> = sqlx::query_as(
            "SELECT q.lease_id, l.ttl_seconds FROM lease_queue q JOIN leases l ON l.id = q.lease_id \
             WHERE q.resource = ? ORDER BY q.position LIMIT 1",
        )
        .bind(machine)
        .fetch_optional(&state.db)
        .await?;
        let Some((id, ttl)) = next else { return Ok(()) };
        let now = now_ms();
        sqlx::query("DELETE FROM lease_queue WHERE lease_id = ?").bind(id).execute(&state.db).await?;
        sqlx::query(
            "UPDATE leases SET state = 'active', activated_at = ?, last_renewed_at = ?, expires_at = ? WHERE id = ?",
        )
        .bind(now)
        .bind(now)
        .bind(now + ttl * 1000)
        .bind(id)
        .execute(&state.db)
        .await?;
    }
}

// --- Approval through the card --------------------------------------------------------------------------------------

/// Inside a decision's release: the card's released lease actions are carried out by the deck itself. Each confirms
/// its filter's pattern and approves its lease; the create step follows after the commit. Returns how many it took.
pub async fn approve_released(
    conn: &mut SqliteConnection,
    card_id: i64,
    version: i64,
    now: i64,
) -> anyhow::Result<u64> {
    let actions: Vec<(i64,)> = sqlx::query_as(
        "SELECT id FROM actions WHERE card_id = ? AND card_version = ? AND type = 'lease' AND status = 'released'",
    )
    .bind(card_id)
    .bind(version)
    .fetch_all(&mut *conn)
    .await?;
    if actions.is_empty() {
        return Ok(0);
    }
    let lease: Option<(i64, String, String, String, i64)> = sqlx::query_as(
        "SELECT id, kind, resource, params, ttl_seconds FROM leases WHERE card_id = ? AND state = 'proposed'",
    )
    .bind(card_id)
    .fetch_optional(&mut *conn)
    .await?;
    let outcome = match lease {
        Some((id, kind, resource, params, ttl)) => {
            let params: Value = serde_json::from_str(&params)?;
            let pid =
                upsert_pattern(conn, &kind, &fingerprint(&resource, query_of(&params)), Some(card_id), now).await?;
            sqlx::query("UPDATE leases SET state = 'approved', pattern_id = ?, expires_at = ? WHERE id = ?")
                .bind(pid)
                .bind(now + ttl * 1000)
                .bind(id)
                .execute(&mut *conn)
                .await?;
            json!({ "lease_id": id, "approved": true })
        }
        None => json!({ "approved": false, "reason": "the lease had already ended" }),
    };
    for (action_id,) in &actions {
        sqlx::query("UPDATE actions SET status = 'done', outcome = ? WHERE id = ?")
            .bind(to_json(&outcome))
            .bind(action_id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(actions.len() as u64)
}

// --- Steps ------------------------------------------------------------------------------------------------------------

/// A connected runner whose accounts include this one.
async fn runner_for(state: &AppState, account: &str) -> Option<String> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT name, runner_info FROM nodes WHERE runner_info IS NOT NULL AND revoked_at IS NULL")
            .fetch_all(&state.db)
            .await
            .ok()?;
    for (node, info) in rows {
        let Ok(info) = serde_json::from_str::<quietdeck_types::node::RunnerInfo>(&info) else { continue };
        if info.accounts.iter().any(|a| a == account) && state.hub.is_connected(&node).await {
            return Some(node);
        }
    }
    None
}

async fn ask_runner(state: &AppState, node: &str, lease: &Lease, step: LeaseStep) -> Result<Value, String> {
    let req = DeckRequest::LeaseStep { lease: Box::new(lease.clone()), step };
    match state.hub.request(node, req, STEP_TIMEOUT).await {
        Ok(reply) => reply.into_result::<Value>(),
        Err(e) => Err(format!("{e:?}")),
    }
}

async fn tell(state: &AppState, to: &SessionRef, body: String) -> bool {
    let address = format!("{}@{}", to.tab, to.node);
    crate::relay::send(&state.db, &state.hub, &address, DeliveryKind::Deck, body).await.is_ok()
}

/// Approved leases whose create step hasn't been asked for: the runner creates each filter, or the holder is told
/// to.
pub async fn dispatch_creates(state: &AppState) {
    let leases = match select(&state.db, "state = 'approved' AND create_requested_at IS NULL").await {
        Ok(l) => l,
        Err(e) => return tracing::warn!(error = %e, "reading approved leases"),
    };
    for lease in leases {
        let claimed =
            sqlx::query("UPDATE leases SET create_requested_at = ? WHERE id = ? AND create_requested_at IS NULL")
                .bind(now_ms())
                .bind(lease.id)
                .execute(&state.db)
                .await;
        if !matches!(claimed, Ok(r) if r.rows_affected() == 1) {
            continue;
        }
        // Gmail refuses a second identical filter, so a lease on a filter another session already has shares it.
        match active_with_same_filter(&state.db, &lease).await {
            Ok(Some(other)) => {
                let mut kind_state = other.kind_state.clone().unwrap_or_else(|| json!({}));
                kind_state["shared_with"] = json!(other.id);
                if let Err(e) = activate(state, lease.id, kind_state).await {
                    tracing::warn!(lease = lease.id, error = %e, "sharing a filter");
                }
                continue;
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(lease = lease.id, error = %e, "looking for a filter to share"),
        }
        let why = match runner_for(state, &lease.resource).await {
            Some(node) => match ask_runner(state, &node, &lease, LeaseStep::Create).await {
                Ok(kind_state) => {
                    if let Err(e) = activate(state, lease.id, kind_state).await {
                        tracing::warn!(lease = lease.id, error = %e, "activating a lease");
                    }
                    continue;
                }
                Err(e) => format!("The runner couldn't create it ({e})."),
            },
            None => "No runner creates filters for this account.".into(),
        };
        let body = format!(
            "Your gmail-filter lease {id} on {account} is approved. {why} Create the Gmail filter yourself: query \
             `{query}`, add the label Quietdeck/Lease/{tab}, remove INBOX and UNREAD; move matching inbox mail the same \
             way; then run `quietdeck lease register {id} --filter-id <the filter's id>`.",
            id = lease.id,
            account = lease.resource,
            query = query_of(&lease.params),
            tab = lease.holder.tab,
        );
        tell(state, &lease.holder, body).await;
    }
}

/// Another active lease whose filter has this lease's account and query.
async fn active_with_same_filter(db: &Db, lease: &Lease) -> anyhow::Result<Option<Lease>> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM leases WHERE id != ? AND kind = ? AND resource = ? AND state = 'active' \
         AND json_extract(params, '$.query') = ? AND json_extract(kind_state, '$.filter_id') IS NOT NULL \
         ORDER BY id LIMIT 1",
    )
    .bind(lease.id)
    .bind(&lease.kind)
    .bind(&lease.resource)
    .bind(query_of(&lease.params))
    .fetch_optional(db)
    .await?;
    match row {
        Some((id,)) => Ok(Some(get(db, id).await?)),
        None => Ok(None),
    }
}

/// Whether another lease that hasn't ended still uses this lease's filter.
async fn filter_still_used(db: &Db, lease: &Lease) -> anyhow::Result<bool> {
    let Some(filter_id) = lease.kind_state.as_ref().and_then(|s| s.get("filter_id")).and_then(Value::as_str) else {
        return Ok(false);
    };
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM leases WHERE id != ? AND resource = ? AND ended_at IS NULL \
         AND state IN ('approved', 'active', 'expiring') AND json_extract(kind_state, '$.filter_id') = ? LIMIT 1",
    )
    .bind(lease.id)
    .bind(&lease.resource)
    .bind(filter_id)
    .fetch_optional(db)
    .await?;
    Ok(row.is_some())
}

/// The create step is done: the lease is active, and the timeline says what its filter does.
pub async fn activate(state: &AppState, id: i64, kind_state: Value) -> AppResult<Lease> {
    let lease = get(&state.db, id).await?;
    if !matches!(lease.state, LeaseState::Approved | LeaseState::Active) {
        return Err(AppError::Conflict(format!(
            "lease {id} is {}, so there's nothing to register",
            enum_str(&lease.state)
        )));
    }
    let now = now_ms();
    sqlx::query("UPDATE leases SET state = 'active', kind_state = ?, activated_at = COALESCE(activated_at, ?), last_renewed_at = ? WHERE id = ?")
        .bind(to_json(&kind_state))
        .bind(now)
        .bind(now)
        .bind(id)
        .execute(&state.db)
        .await?;
    if lease.kind == KIND_GMAIL_FILTER && lease.state == LeaseState::Approved {
        let confirmed_by: Option<(Option<i64>,)> = match lease.pattern_id {
            Some(p) => {
                sqlx::query_as("SELECT confirmed_by_card_id FROM lease_patterns WHERE id = ?")
                    .bind(p)
                    .fetch_optional(&state.db)
                    .await?
            }
            None => None,
        };
        let query = query_of(&lease.params);
        let shared_with = kind_state.get("shared_with").and_then(Value::as_i64);
        let (kind, text) = match (lease.card_id, confirmed_by.and_then(|c| c.0)) {
            _ if shared_with.is_some() => {
                let other = get(&state.db, shared_with.unwrap_or_default()).await?;
                (TimelineKind::LeaseAbsorbed, {
                    let owner = if other.holder.node == lease.holder.node && other.holder.tab == lease.holder.tab {
                        "its other lease".to_string()
                    } else {
                        other.holder.tab.clone()
                    };
                    format!("{} shares the filter {owner} already has: `{query}`", lease.holder.tab)
                })
            }
            (Some(_), _) => {
                (TimelineKind::LeaseStarted, format!("{} started the filter you approved: `{query}`", lease.holder.tab))
            }
            (None, Some(_)) => {
                (TimelineKind::LeaseStarted, format!("{} re-created your approved filter: `{query}`", lease.holder.tab))
            }
            (None, None) => (
                TimelineKind::FilterWithoutProposal,
                format!("{} created a filter without a proposal: `{query}`", lease.holder.tab),
            ),
        };
        entry(state, &lease, kind, text).await;
    }
    state.changed(Changed::Leases);
    get(&state.db, id).await
}

async fn entry(state: &AppState, lease: &Lease, kind: TimelineKind, text: String) {
    let e = TimelineAdd {
        kind,
        account: (lease.kind == KIND_GMAIL_FILTER).then(|| lease.resource.clone()),
        refs: vec![format!("lease {}", lease.id)],
        text,
        lease_id: Some(lease.id),
    };
    if let Err(e) = timeline::add(&state.db, EntrySource::Deck, None, &e).await {
        tracing::warn!(error = %e, "a lease's timeline entry");
    }
    state.changed(Changed::Timeline);
}

// --- Ending ---------------------------------------------------------------------------------------------------------

/// End a lease: one with a filter asks for its removal first; anything else is released at once.
pub async fn end(state: &AppState, lease: &Lease, reason: &str) -> AppResult<()> {
    if lease.state.is_ended() || matches!(lease.state, LeaseState::CleanupRequested | LeaseState::Overdue) {
        return Ok(());
    }
    let now = now_ms();
    if has_filter(lease) && !filter_still_used(&state.db, lease).await? {
        sqlx::query(
            "UPDATE leases SET state = 'cleanup_requested', cleanup_requested_at = ?, end_reason = ? WHERE id = ?",
        )
        .bind(now)
        .bind(reason)
        .bind(lease.id)
        .execute(&state.db)
        .await?;
        let lease = get(&state.db, lease.id).await?;
        remove(state, &lease).await;
    } else {
        let to = if lease.state == LeaseState::Proposed { LeaseState::Rejected } else { LeaseState::Released };
        sqlx::query("UPDATE leases SET state = ?, ended_at = ?, end_reason = ? WHERE id = ?")
            .bind(enum_str(&to))
            .bind(now)
            .bind(reason)
            .bind(lease.id)
            .execute(&state.db)
            .await?;
        sqlx::query("DELETE FROM lease_queue WHERE lease_id = ?").bind(lease.id).execute(&state.db).await?;
        if lease.kind == KIND_HEAVY_BUILD {
            promote(state, &lease.resource).await?;
        }
    }
    state.changed(Changed::Leases);
    Ok(())
}

/// Ask for a filter's removal: the runner, else the holder, else the fallback tab. Until someone confirms, the lease
/// waits in cleanup, and becomes overdue after 15 minutes.
async fn remove(state: &AppState, lease: &Lease) {
    if let Some(node) = runner_for(state, &lease.resource).await
        && ask_runner(state, &node, lease, LeaseStep::Remove).await.is_ok()
    {
        let _ = cleaned(state, lease.id, CleanupBy::Runner).await;
        return;
    }
    if lease.cleanup_by.is_some() {
        return; // the holder or the fallback was asked already
    }
    let filter_id = lease.kind_state.as_ref().and_then(|s| s.get("filter_id")).and_then(Value::as_str).unwrap_or("?");
    let body = format!(
        "Gmail-filter lease {id} has ended ({why}). Delete filter {filter_id} on {account} (query `{query}`), then run \
         `quietdeck lease cleaned {id}`.",
        id = lease.id,
        why = lease.end_reason.as_deref().unwrap_or("its session ended"),
        account = lease.resource,
        query = query_of(&lease.params),
    );
    let mut by = CleanupBy::Holder;
    if !tell(state, &lease.holder, body.clone()).await {
        by = CleanupBy::Fallback;
        if let Some(f) = &state.config.leases.fallback {
            tell(state, &SessionRef { node: f.node.clone(), tab: f.tab.clone(), session_id: None }, body).await;
        }
    }
    let _ = sqlx::query("UPDATE leases SET cleanup_by = ? WHERE id = ?")
        .bind(enum_str(&by))
        .bind(lease.id)
        .execute(&state.db)
        .await;
}

/// Whoever removed the filter confirms it: the lease is released.
pub async fn cleaned(state: &AppState, id: i64, by: CleanupBy) -> AppResult<Lease> {
    let lease = get(&state.db, id).await?;
    if lease.state.is_ended() {
        return Ok(lease);
    }
    sqlx::query(
        "UPDATE leases SET state = 'released', ended_at = ?, cleanup_by = COALESCE(cleanup_by, ?), \
                 end_reason = COALESCE(end_reason, 'released') WHERE id = ?",
    )
    .bind(now_ms())
    .bind(enum_str(&by))
    .bind(id)
    .execute(&state.db)
    .await?;
    if lease.kind == KIND_GMAIL_FILTER && has_filter(&lease) {
        let text = format!(
            "Filter `{}` removed: {}",
            query_of(&lease.params),
            lease.end_reason.as_deref().unwrap_or("its lease ended")
        );
        entry(state, &lease, TimelineKind::LeaseExpired, text).await;
    }
    state.changed(Changed::Leases);
    get(&state.db, id).await
}

/// The holder ends its own lease (or the person does, from the deck).
pub async fn release(state: &AppState, caller: &Caller, id: i64) -> AppResult<Lease> {
    let lease = get(&state.db, id).await?;
    let reason = match caller {
        Caller::Session { session: me } if me.node == lease.holder.node && me.tab == lease.holder.tab => {
            "released by its holder"
        }
        Caller::Session { .. } => {
            return Err(AppError::Forbidden(format!(
                "lease {id} belongs to {}@{}",
                lease.holder.tab, lease.holder.node
            )));
        }
        _ => "ended by the person",
    };
    end(state, &lease, reason).await?;
    get(&state.db, id).await
}

/// A session closed (its tab or its agent ended): its leases end.
pub async fn session_closed(state: &AppState, node: &str, session_id: &str, tab: Option<&str>) -> AppResult<()> {
    let rows: Vec<(i64,)> = sqlx::query_as(
        "SELECT id FROM leases WHERE ended_at IS NULL AND holder_node = ? \
         AND (holder_session_id = ? OR (holder_session_id IS NULL AND holder_tab = ?))",
    )
    .bind(node)
    .bind(session_id)
    .bind(tab.unwrap_or(""))
    .fetch_all(&state.db)
    .await?;
    for (id,) in rows {
        let lease = get(&state.db, id).await?;
        end(state, &lease, "its session closed").await?;
    }
    Ok(())
}

// --- The ticker -----------------------------------------------------------------------------------------------------

pub async fn ticker(state: AppState) {
    let mut every = tokio::time::interval(TICK);
    loop {
        every.tick().await;
        if let Err(e) = tick(&state).await {
            tracing::warn!(error = %e, "lease ticker");
        }
    }
}

pub async fn tick(state: &AppState) -> anyhow::Result<()> {
    let now = now_ms();
    for lease in select(&state.db, "ended_at IS NULL").await? {
        match lease.state {
            LeaseState::Proposed => {
                if let Some(card) = lease.card_id {
                    let st: Option<(String,)> = sqlx::query_as("SELECT state FROM cards WHERE id = ?")
                        .bind(card)
                        .fetch_optional(&state.db)
                        .await?;
                    if st.is_some_and(|s| matches!(s.0.as_str(), "later" | "done" | "handled_elsewhere")) {
                        end(state, &lease, "the person didn't approve it").await?;
                    }
                }
            }
            LeaseState::Approved | LeaseState::Active | LeaseState::Queued | LeaseState::Expiring => {
                if lease.expires_at.is_some_and(|t| t.as_millisecond() <= now) {
                    end(state, &lease, "its time limit passed").await?;
                } else if matches!(lease.state, LeaseState::Active | LeaseState::Queued)
                    && let Some(minutes) = idle_for(&state.db, &lease, now).await?
                    && minutes >= lease.idle_minutes
                {
                    end(state, &lease, &format!("its session was idle or offline for {minutes} minutes")).await?;
                }
            }
            LeaseState::CleanupRequested | LeaseState::Overdue => {
                // A runner that has come back removes it now.
                if lease.cleanup_by != Some(CleanupBy::Runner)
                    && let Some(node) = runner_for(state, &lease.resource).await
                    && ask_runner(state, &node, &lease, LeaseStep::Remove).await.is_ok()
                {
                    cleaned(state, lease.id, CleanupBy::Runner).await?;
                } else if lease.state == LeaseState::CleanupRequested
                    && lease.cleanup_requested_at.is_some_and(|t| now - t.as_millisecond() > CLEANUP_OVERDUE_MS)
                {
                    set_state(&state.db, lease.id, LeaseState::Overdue).await?;
                    let text = format!(
                        "Cleanup overdue: nobody has confirmed removing the filter `{}` (lease {}). Delete it in Gmail, \
                         then run `quietdeck lease cleaned {}`.",
                        query_of(&lease.params),
                        lease.id,
                        lease.id
                    );
                    entry(state, &lease, TimelineKind::LeaseCleanupOverdue, text).await;
                    state.changed(Changed::Leases);
                }
            }
            LeaseState::Released | LeaseState::Rejected => {}
        }
    }
    dispatch_creates(state).await;
    Ok(())
}

/// How many minutes the holder has been idle or offline, or none while it works (or waits on the person).
async fn idle_for(db: &Db, lease: &Lease, now: i64) -> anyhow::Result<Option<i64>> {
    let row: Option<(i64, String, i64)> = sqlx::query_as(
        "SELECT running, status, status_changed_at FROM sessions WHERE node = ? \
         AND (session_id = ? OR (? IS NULL AND tab = ?)) ORDER BY running DESC, status_changed_at DESC LIMIT 1",
    )
    .bind(&lease.holder.node)
    .bind(&lease.holder.session_id)
    .bind(&lease.holder.session_id)
    .bind(&lease.holder.tab)
    .fetch_optional(db)
    .await?;
    let since_start = lease.activated_at.unwrap_or(lease.created_at).as_millisecond();
    let Some((running, status, changed)) = row else {
        return Ok(Some((now - since_start) / 60_000));
    };
    let status: AgentStatus = parse_enum(&status)?;
    if running != 0 && !status.is_idle() {
        return Ok(None);
    }
    Ok(Some((now - changed.max(since_start)) / 60_000))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use quietdeck_types::card::DecisionKind;
    use quietdeck_types::decision::{ApproveAll, CardDecision};

    use crate::store::{cards, decisions};

    const CONFIG: &str = "accounts:\n  - { id: work, label: Work }\n\
        leases:\n  fallback: { node: server, tab: email triage }\n  heavy_build_slots: { laptop: 1 }\n";

    async fn deck() -> AppState {
        let mut state = AppState::for_tests().await;
        state.config = Arc::new(yaml_serde::from_str(CONFIG).unwrap());
        state
    }

    fn holder(tab: &str) -> SessionRef {
        SessionRef { node: "laptop".into(), tab: tab.into(), session_id: Some(format!("s-{tab}")) }
    }

    fn filter(query: &str) -> LeaseRequest {
        LeaseRequest {
            kind: KIND_GMAIL_FILTER.into(),
            resource: Some("work".into()),
            params: json!({ "query": query }),
            reason: "I'm watching CI myself".into(),
            ttl_seconds: None,
            idle_minutes: None,
        }
    }

    fn slot() -> LeaseRequest {
        LeaseRequest {
            kind: KIND_HEAVY_BUILD.into(),
            resource: None,
            params: json!({}),
            reason: "a release build".into(),
            ttl_seconds: Some(7200),
            idle_minutes: None,
        }
    }

    async fn approve(state: &AppState, card_id: i64) {
        let card = cards::get(&state.db, card_id).await.unwrap().unwrap();
        let d = CardDecision { card_id, version: card.version, kind: DecisionKind::Approve, fields: None, text: None };
        decisions::approve_all(&state.db, &ApproveAll { cards: vec![d], timeline: vec![] }, 0).await.unwrap();
        decisions::release_due(&state.db).await.unwrap();
    }

    #[tokio::test]
    async fn a_filter_waits_on_its_card_and_approving_it_approves_the_same_filter_for_any_session() {
        let state = deck().await;
        let q = "from:notifications@github.com subject:(\"Run failed\")";
        let a = propose(&state, &holder("api"), filter(q)).await.unwrap();
        assert_eq!(a.lease.state, LeaseState::Proposed);
        let card_id = a.lease.card_id.expect("a proposal card");
        let card = cards::get(&state.db, card_id).await.unwrap().unwrap();
        assert_eq!(card.input.subject, "api asks for a temporary Gmail filter");

        approve(&state, card_id).await;
        let lease = get(&state.db, a.lease.id).await.unwrap();
        assert_eq!(lease.state, LeaseState::Approved);
        let card = cards::get(&state.db, card_id).await.unwrap().unwrap();
        assert_eq!(card.state, quietdeck_types::card::CardState::Done, "the deck carried out the lease action");

        // No runner here: the holder is told to create it, and registers it.
        dispatch_creates(&state).await;
        let lease = activate(&state, lease.id, json!({ "filter_id": "f1" })).await.unwrap();
        assert_eq!(lease.state, LeaseState::Active);

        // Another session asks for the same filter on the same account: approved at once, no card.
        let b = propose(&state, &holder("web"), filter(&format!("  {q} "))).await.unwrap();
        assert_eq!(b.lease.state, LeaseState::Approved);
        assert!(b.lease.card_id.is_none());
        // The same query on a slightly different form is a new proposal.
        let c = propose(&state, &holder("web"), filter("from:github.com")).await.unwrap();
        assert_eq!(c.lease.state, LeaseState::Proposed);

        // The second session shares the first one's filter: Gmail refuses an identical second filter.
        dispatch_creates(&state).await;
        let shared = get(&state.db, b.lease.id).await.unwrap();
        assert_eq!(shared.state, LeaseState::Active);
        assert_eq!(shared.kind_state.as_ref().unwrap()["filter_id"], "f1");

        // The first lease ends: the filter stays for the second.
        end(&state, &lease, "released by its holder").await.unwrap();
        assert_eq!(get(&state.db, lease.id).await.unwrap().state, LeaseState::Released);

        // The last one ends: its filter waits for removal; confirming it releases it and the timeline says so.
        end(&state, &shared, "released by its holder").await.unwrap();
        let ending = get(&state.db, shared.id).await.unwrap();
        assert_eq!(ending.state, LeaseState::CleanupRequested);
        assert_eq!(ending.cleanup_by, Some(CleanupBy::Fallback), "the holder and the fallback can't be reached here");
        let done = cleaned(&state, shared.id, CleanupBy::Holder).await.unwrap();
        assert_eq!(done.state, LeaseState::Released);
        let kinds: Vec<(String,)> =
            sqlx::query_as("SELECT kind FROM timeline_entries ORDER BY id").fetch_all(&state.db).await.unwrap();
        let kinds: Vec<&str> = kinds.iter().map(|t| t.0.as_str()).collect();
        assert_eq!(kinds, ["lease_started", "lease_absorbed", "lease_expired"]);
    }

    #[tokio::test]
    async fn acquiring_a_new_filter_needs_a_proposal_while_the_setting_is_on() {
        let state = deck().await;
        let e = acquire(&state, &holder("api"), filter("from:ci.example.com")).await.unwrap_err();
        assert!(e.to_string().contains("Require a proposal"), "{e}");
        let mut s = crate::api::load_settings(&state.db).await.unwrap();
        s.require_filter_proposal = false;
        crate::ops::save_settings(&state, s).await.unwrap();
        let a = acquire(&state, &holder("api"), filter("from:ci.example.com")).await.unwrap();
        assert_eq!(a.lease.state, LeaseState::Approved);
        let lease = activate(&state, a.lease.id, json!({ "filter_id": "f2" })).await.unwrap();
        let kinds: Vec<(String,)> =
            sqlx::query_as("SELECT kind FROM timeline_entries").fetch_all(&state.db).await.unwrap();
        assert_eq!(kinds[0].0, "filter_without_proposal");
        assert_eq!(lease.state, LeaseState::Active);
    }

    #[tokio::test]
    async fn a_filter_query_is_checked_and_a_session_holds_at_most_five() {
        let state = deck().await;
        assert!(propose(&state, &holder("api"), filter("subject:hello")).await.is_err());
        let mut no_account = filter("from:a.com");
        no_account.resource = Some("elsewhere".into());
        assert!(propose(&state, &holder("api"), no_account).await.is_err());
        for i in 0..5 {
            propose(&state, &holder("api"), filter(&format!("from:a{i}.example.com"))).await.unwrap();
        }
        let e = propose(&state, &holder("api"), filter("from:six.example.com")).await.unwrap_err();
        assert!(e.to_string().contains("already holds 5"), "{e}");
    }

    #[tokio::test]
    async fn heavy_build_slots_queue_and_the_next_waiter_starts_when_one_frees() {
        let state = deck().await;
        let first = acquire(&state, &holder("api"), slot()).await.unwrap();
        assert_eq!(first.lease.state, LeaseState::Active);
        assert_eq!(first.lease.resource, "laptop", "the holder's machine by default");
        let second = propose(&state, &holder("web"), slot()).await.unwrap();
        assert_eq!(second.lease.state, LeaseState::Queued, "deck.yaml gives laptop one slot");
        assert_eq!(second.lease.queue_position, Some(1), "first in line");
        let caller = Caller::Session { session: holder("api") };
        let e = release(&state, &Caller::Session { session: holder("web") }, first.lease.id).await.unwrap_err();
        assert!(e.to_string().contains("belongs to api@laptop"), "{e}");
        release(&state, &caller, first.lease.id).await.unwrap();
        let second = get(&state.db, second.lease.id).await.unwrap();
        assert_eq!(second.state, LeaseState::Active);
        assert!(second.expires_at.is_some());
    }

    #[tokio::test]
    async fn the_ticker_ends_leases_whose_time_passed_or_whose_session_closed_and_marks_cleanup_overdue() {
        let state = deck().await;
        let a = acquire(&state, &holder("api"), slot()).await.unwrap();
        sqlx::query("UPDATE leases SET expires_at = 1 WHERE id = ?").bind(a.lease.id).execute(&state.db).await.unwrap();
        // A running session keeps it from counting as idle; the time limit still applies.
        tick(&state).await.unwrap();
        let ended = get(&state.db, a.lease.id).await.unwrap();
        assert_eq!(ended.state, LeaseState::Released);
        assert_eq!(ended.end_reason.as_deref(), Some("its time limit passed"));

        let mut s = crate::api::load_settings(&state.db).await.unwrap();
        s.require_filter_proposal = false;
        crate::ops::save_settings(&state, s).await.unwrap();
        let f = acquire(&state, &holder("web"), filter("from:ci.example.com")).await.unwrap();
        activate(&state, f.lease.id, json!({ "filter_id": "f3" })).await.unwrap();
        session_closed(&state, "laptop", "s-web", Some("web")).await.unwrap();
        assert_eq!(get(&state.db, f.lease.id).await.unwrap().state, LeaseState::CleanupRequested);
        sqlx::query("UPDATE leases SET cleanup_requested_at = 1 WHERE id = ?")
            .bind(f.lease.id)
            .execute(&state.db)
            .await
            .unwrap();
        tick(&state).await.unwrap();
        assert_eq!(get(&state.db, f.lease.id).await.unwrap().state, LeaseState::Overdue);
        let kinds: Vec<(String,)> =
            sqlx::query_as("SELECT kind FROM timeline_entries ORDER BY id").fetch_all(&state.db).await.unwrap();
        assert_eq!(kinds.last().unwrap().0, "lease_cleanup_overdue");
    }

    #[test]
    fn a_query_needs_a_full_sender() {
        assert!(check_query("from:notifications@github.com subject:(\"Run failed\")").is_ok());
        assert!(check_query("from:github.com").is_ok());
        assert!(check_query("subject:x from:(ci@example.org OR other@example.org)").is_ok());
        assert!(check_query("from:github").is_err());
        assert!(check_query("from:*@github.com").is_err());
        assert!(check_query("subject:\"Run failed\"").is_err());
        assert!(check_query(&format!("from:a.com {}", "x".repeat(600))).is_err());
        assert_eq!(fingerprint("work", "  from:a.com  "), "work\nfrom:a.com");
    }
}
