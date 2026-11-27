//! The REST API. Agents call it with the `quietdeck` CLI (session tokens from their node); the web app with the
//! owner's sign-in. The MCP endpoint calls the same functions (`ops`).

use aide::axum::ApiRouter;
use aide::axum::routing::{delete, get, patch as axum_patch, post, put};
use axum::Json;
use axum::extract::{Path, Query, State};
use quietdeck_types::api::{
    Caller, Changed, DeckInfo, EnrolRequest, Enrolled, EnrolmentCode, RoundDone, RoundStatus, Settings,
};
use quietdeck_types::card::{ActionRecord, ActionReport, Card, CardAdded, CardInput, CardResolution, Released};
use quietdeck_types::decision::{ApproveAll, Approving, Decide, Inbox, ReportReceipt, StopResult, UndoBatch};
use quietdeck_types::directory::{NodeRow, SessionRow, SessionStart, SessionStarted, SessionTarget, WhoHas};
use quietdeck_types::node::Delivered;
use quietdeck_types::relay::MsgSend;
use quietdeck_types::rules::{MatchRequest, MatchResult, Rule, RuleInput, RuleStatusChange, RulesView};
use quietdeck_types::timeline::{Comment, CommentAdd, History, TimelineAdd, TimelineEntry};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::auth::{Agent, Authed, Owner};
use crate::error::{AppError, AppResult, bad};
use crate::openapi::{Id, NodeName, TextId};
use crate::state::AppState;
use crate::store::{cards, nodes, sessions};
use crate::{control, ops};

/// Every route, without the `/api` prefix: `crate::openapi` mounts them at `/api/v1` (the published, versioned API) and
/// at `/api` (the same routes, for nodes, CLIs and web apps that predate v1). Routes go in with `api_route`, so the
/// spec (`api/openapi.json`) takes their types from the handlers; the one `route` is the event stream, whose entry
/// `crate::openapi` writes by hand.
pub fn router() -> ApiRouter<AppState> {
    ApiRouter::new()
        .api_route("/info", get(info))
        .api_route("/nodes", get(list_nodes))
        .api_route("/nodes/enroll", post(enroll))
        .api_route("/nodes/enrolment-codes", post(new_code))
        .api_route("/nodes/ws", get(crate::ws::upgrade))
        .api_route("/nodes/{name}", delete(revoke_node))
        .api_route("/cards", get(list_cards).post(add_card))
        .api_route("/cards/{id}", get(get_card))
        .api_route("/cards/{id}/resolve", post(resolve_card))
        .api_route("/sessions", get(list_sessions))
        .api_route("/sessions/search", get(search_sessions))
        .api_route("/sessions/show", get(show_session))
        .api_route("/sessions/transcript", get(session_transcript))
        .api_route("/sessions/catchup", get(session_catchup))
        .api_route("/sessions/start", post(start_session))
        .api_route("/sessions/resume", post(resume_session))
        .api_route("/sessions/close", post(close_session))
        .api_route("/sessions/interrupt", post(interrupt_session))
        .api_route("/handoffs", post(handoff_add))
        .api_route("/handoffs/{id}", get(handoff_show))
        .api_route("/handoffs/{id}/claim", post(handoff_claim))
        .api_route("/who-has", get(who_has))
        .api_route("/msg", post(msg_send))
        .api_route("/timeline", post(timeline_add))
        .api_route("/history", get(history))
        .api_route("/messages", get(messages_read))
        .api_route("/mail", get(mail_list))
        .api_route("/mail/seen", post(mail_seen))
        .api_route("/mail/badges", get(mail_badges))
        .api_route("/presence", post(presence_beat))
        .api_route("/presence/release", post(presence_release))
        .api_route("/reading-rules", get(reading_rules_list).post(reading_rule_create))
        .api_route("/reading-rules/{id}", put(reading_rule_update).delete(reading_rule_delete))
        .api_route("/reading-rules/{id}/enabled", post(reading_rule_enabled))
        .api_route("/inbox", get(inbox))
        .api_route("/later", get(later))
        .route("/events", axum::routing::get(crate::events::stream))
        .api_route("/settings", get(get_settings).put(put_settings))
        .api_route("/decisions/approve-all", post(approve_all))
        .api_route("/report/approve", post(approve_report))
        .api_route("/decisions/undo", post(undo))
        .api_route("/decisions/stop", post(stop))
        .api_route("/decisions/take", post(take))
        .api_route("/round", get(round_status))
        .api_route("/round/done", post(round_done))
        .api_route("/cards/{id}/decide", post(decide))
        .api_route("/cards/{id}/retry", post(retry))
        .api_route("/actions/{id}/start", post(action_start))
        .api_route("/actions/report", post(actions_report))
        .api_route("/rules", get(rules_view).post(rule_create))
        .api_route("/rules/match", post(rules_match))
        .api_route("/rules/{id}", put(rule_update))
        .api_route("/rules/{id}/status", post(rule_status))
        .api_route("/leases", get(leases_view))
        .api_route("/leases/propose", post(lease_propose))
        .api_route("/leases/acquire", post(lease_acquire))
        .api_route("/leases/{id}", get(lease_get))
        .api_route("/leases/{id}/register", post(lease_register))
        .api_route("/leases/{id}/release", post(lease_release))
        .api_route("/leases/{id}/cleaned", post(lease_cleaned))
        .api_route("/lease-patterns/{id}/revoke", post(lease_pattern_revoke))
        .api_route("/comments", post(comment_add))
        .api_route("/report", get(report_show).post(report_set))
        .api_route("/report/done", post(report_done))
        .api_route("/chat/reply", post(chat_reply))
        .api_route("/hooks", get(hooks_list).post(hooks_create))
        .api_route("/hooks/{id}/approve", post(hooks_approve))
        .api_route("/hooks/{id}", delete(hooks_delete))
        .api_route("/report/answers", post(report_answer))
        .api_route("/report/answers/take", post(report_answers_take))
        .api_route("/report/chat", post(report_chat))
        .api_route("/report/replies", post(report_reply))
        .api_route("/report/snapshots/{id}", get(report_snapshot))
        .api_route("/report/fyi/clear", post(report_fyi_clear))
        .api_route("/documents", get(documents_list).post(documents_add))
        .api_route("/documents/{id}", axum_patch(documents_edit))
        .api_route("/documents/{id}/remove", post(documents_remove))
        .api_route("/session-marks", get(session_marks_list).post(session_marks_set))
        .api_route("/session-marks/remove", post(session_marks_remove))
        .api_route("/pings", get(pings_view).post(ping_send))
        .api_route("/pings/{id}/feedback", post(ping_feedback))
        .api_route("/pings/feedback/take", post(ping_feedback_take))
        .api_route("/watches", get(watches_list).post(watch_add))
        .api_route("/watches/{id}/stop", post(watch_stop))
        .api_route("/push", get(push_config))
        .api_route("/push/devices", get(push_devices).post(push_subscribe))
        .api_route("/push/devices/remove", post(push_remove))
}

#[derive(Deserialize, JsonSchema)]
pub struct MailQuery {
    pub kind: quietdeck_types::mail::MailKind,
    pub account: Option<String>,
}

async fn mail_list(
    State(state): State<AppState>,
    _owner: Owner,
    Query(q): Query<MailQuery>,
) -> AppResult<Json<Vec<quietdeck_types::mail::MailRow>>> {
    Ok(Json(crate::mail::list(&state.db, q.kind, q.account.as_deref()).await?))
}

#[derive(Deserialize, JsonSchema)]
pub struct MailSeen {
    pub kind: quietdeck_types::mail::MailKind,
}

async fn mail_seen(
    State(state): State<AppState>,
    _owner: Owner,
    Json(m): Json<MailSeen>,
) -> AppResult<Json<serde_json::Value>> {
    crate::mail::seen(&state, m.kind).await?;
    Ok(Json(serde_json::json!({})))
}

async fn mail_badges(
    State(state): State<AppState>,
    _owner: Owner,
) -> AppResult<Json<quietdeck_types::mail::MailBadges>> {
    Ok(Json(crate::mail::badges(&state.db).await?))
}

async fn presence_beat(
    State(state): State<AppState>,
    _owner: Owner,
    Json(b): Json<quietdeck_types::reading::PresenceBeat>,
) -> AppResult<Json<quietdeck_types::reading::Presence>> {
    Ok(Json(crate::reading::beat(&state, b).await?))
}

async fn presence_release(State(state): State<AppState>, _owner: Owner) -> AppResult<Json<serde_json::Value>> {
    crate::reading::release(&state).await?;
    Ok(Json(serde_json::json!({})))
}

async fn reading_rules_list(
    State(state): State<AppState>,
    Authed(_): Authed,
) -> AppResult<Json<Vec<quietdeck_types::reading::ReadingRule>>> {
    Ok(Json(crate::reading::list(&state.db).await?))
}

async fn reading_rule_create(
    State(state): State<AppState>,
    _owner: Owner,
    Json(input): Json<quietdeck_types::reading::ReadingRuleInput>,
) -> AppResult<Json<quietdeck_types::reading::ReadingRule>> {
    Ok(Json(crate::reading::create(&state, input).await?))
}

async fn reading_rule_update(
    State(state): State<AppState>,
    _owner: Owner,
    Path(Id { id }): Path<Id>,
    Json(input): Json<quietdeck_types::reading::ReadingRuleInput>,
) -> AppResult<Json<quietdeck_types::reading::ReadingRule>> {
    Ok(Json(crate::reading::update(&state, id, input).await?))
}

#[derive(Deserialize, JsonSchema)]
pub struct Enabled {
    pub enabled: bool,
}

async fn reading_rule_enabled(
    State(state): State<AppState>,
    _owner: Owner,
    Path(Id { id }): Path<Id>,
    Json(e): Json<Enabled>,
) -> AppResult<Json<quietdeck_types::reading::ReadingRule>> {
    Ok(Json(crate::reading::set_enabled(&state, id, e.enabled).await?))
}

async fn reading_rule_delete(
    State(state): State<AppState>,
    _owner: Owner,
    Path(Id { id }): Path<Id>,
) -> AppResult<Json<serde_json::Value>> {
    crate::reading::delete(&state, id).await?;
    Ok(Json(serde_json::json!({})))
}

#[derive(Deserialize, JsonSchema)]
pub struct MessagesQuery {
    pub source: Option<String>,
    /// A message ID: read after it without moving the session's mark. Left out: after the session's last read.
    pub after: Option<i64>,
    pub limit: Option<u32>,
}

async fn messages_read(
    State(state): State<AppState>,
    Agent(me): Agent,
    Query(q): Query<MessagesQuery>,
) -> AppResult<Json<quietdeck_types::message::MessagesPage>> {
    let limit = q.limit.unwrap_or(50);
    Ok(Json(crate::messages::read(&state.db, &me, q.source.as_deref(), q.after, limit).await?))
}

/// Which document a report route is about: its slug. Left out, an agent gets the document it writes, else the Inbox.
#[derive(Deserialize, JsonSchema)]
pub struct DocQuery {
    pub doc: Option<String>,
}

async fn doc_for(state: &AppState, q: &DocQuery, caller: &Caller) -> AppResult<quietdeck_types::document::Document> {
    let me = match caller {
        Caller::Session { session } => Some(session),
        _ => None,
    };
    crate::documents::resolve(&state.db, q.doc.as_deref(), me).await
}

async fn report_show(
    State(state): State<AppState>,
    Authed(caller): Authed,
    Query(q): Query<DocQuery>,
) -> AppResult<Json<Option<quietdeck_types::report::Report>>> {
    let doc = doc_for(&state, &q, &caller).await?;
    Ok(Json(crate::reports::current(&state.db, doc.id).await?))
}

async fn report_fyi_clear(
    State(state): State<AppState>,
    _owner: Owner,
    Query(q): Query<DocQuery>,
) -> AppResult<Json<Option<quietdeck_types::report::Report>>> {
    let doc = crate::documents::resolve(&state.db, q.doc.as_deref(), None).await?;
    Ok(Json(crate::reports::fyi_clear(&state, &doc).await?))
}

async fn documents_list(
    State(state): State<AppState>,
    Authed(_): Authed,
) -> AppResult<Json<Vec<quietdeck_types::document::Document>>> {
    Ok(Json(crate::documents::list(&state.db).await?))
}

async fn documents_add(
    State(state): State<AppState>,
    _owner: Owner,
    Json(req): Json<quietdeck_types::document::DocumentAdd>,
) -> AppResult<Json<quietdeck_types::document::Document>> {
    Ok(Json(crate::documents::add(&state, req).await?))
}

async fn documents_edit(
    State(state): State<AppState>,
    _owner: Owner,
    Path(Id { id }): Path<Id>,
    Json(req): Json<quietdeck_types::document::DocumentEdit>,
) -> AppResult<Json<quietdeck_types::document::Document>> {
    Ok(Json(crate::documents::edit(&state, id, req).await?))
}

async fn documents_remove(
    State(state): State<AppState>,
    _owner: Owner,
    Path(Id { id }): Path<Id>,
) -> AppResult<Json<serde_json::Value>> {
    crate::documents::remove(&state, id).await?;
    Ok(Json(serde_json::json!({})))
}

async fn session_marks_list(
    State(state): State<AppState>,
    Authed(_): Authed,
) -> AppResult<Json<Vec<quietdeck_types::document::SessionMark>>> {
    Ok(Json(crate::documents::marks(&state.db).await?))
}

async fn session_marks_set(
    State(state): State<AppState>,
    _owner: Owner,
    Json(req): Json<quietdeck_types::document::SessionMarkSet>,
) -> AppResult<Json<Vec<quietdeck_types::document::SessionMark>>> {
    Ok(Json(crate::documents::mark_set(&state, req).await?))
}

async fn session_marks_remove(
    State(state): State<AppState>,
    _owner: Owner,
    Json(req): Json<quietdeck_types::document::SessionMarkRemove>,
) -> AppResult<Json<Vec<quietdeck_types::document::SessionMark>>> {
    Ok(Json(crate::documents::mark_remove(&state, req).await?))
}

async fn hooks_list(
    State(state): State<AppState>,
    Authed(_): Authed,
) -> AppResult<Json<Vec<quietdeck_types::hooks::HookEndpoint>>> {
    Ok(Json(crate::hooks::list(&state.db).await?))
}

/// An agent proposes an endpoint (or the owner makes one); it receives nothing until the owner approves it.
async fn hooks_create(
    State(state): State<AppState>,
    Authed(caller): Authed,
    Json(req): Json<quietdeck_types::hooks::HookCreate>,
) -> AppResult<Json<quietdeck_types::hooks::HookCreated>> {
    let by = match &caller {
        quietdeck_types::api::Caller::Session { session } => Some(session.clone()),
        _ => None,
    };
    Ok(Json(crate::hooks::create(&state, by.as_ref(), req).await?))
}

async fn hooks_approve(
    State(state): State<AppState>,
    _owner: Owner,
    Path(TextId { id }): Path<TextId>,
) -> AppResult<Json<quietdeck_types::hooks::HookEndpoint>> {
    Ok(Json(crate::hooks::approve(&state, &id).await?))
}

async fn hooks_delete(
    State(state): State<AppState>,
    _owner: Owner,
    Path(TextId { id }): Path<TextId>,
) -> AppResult<axum::http::StatusCode> {
    crate::hooks::delete(&state, &id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn chat_reply(
    State(state): State<AppState>,
    Agent(me): Agent,
    Query(q): Query<DocQuery>,
    Json(req): Json<quietdeck_types::report::ChatReply>,
) -> AppResult<Json<serde_json::Value>> {
    let doc = crate::documents::resolve(&state.db, q.doc.as_deref(), Some(&me)).await?;
    crate::reports::chat_reply(&state, &doc, &req.text).await?;
    Ok(Json(serde_json::json!({})))
}

async fn report_done(
    State(state): State<AppState>,
    Agent(me): Agent,
    Query(q): Query<DocQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let doc = crate::documents::resolve(&state.db, q.doc.as_deref(), Some(&me)).await?;
    crate::reports::done(&state, &doc).await?;
    Ok(Json(serde_json::json!({})))
}

async fn report_set(
    State(state): State<AppState>,
    Agent(me): Agent,
    Query(q): Query<DocQuery>,
    Json(req): Json<quietdeck_types::report::ReportSet>,
) -> AppResult<Json<quietdeck_types::report::Report>> {
    let doc = crate::documents::resolve(&state.db, q.doc.as_deref(), Some(&me)).await?;
    Ok(Json(crate::reports::set(&state, &me, &doc, req).await?))
}

async fn handoff_add(
    State(state): State<AppState>,
    Agent(me): Agent,
    Json(req): Json<quietdeck_types::handoff::SessionHandoffAdd>,
) -> AppResult<Json<quietdeck_types::handoff::SessionHandoff>> {
    Ok(Json(crate::handoffs::add(&state.db, &me, req).await?))
}

async fn handoff_show(
    State(state): State<AppState>,
    Authed(_): Authed,
    Path(Id { id }): Path<Id>,
) -> AppResult<Json<quietdeck_types::handoff::SessionHandoff>> {
    Ok(Json(crate::handoffs::get(&state.db, id).await?))
}

async fn handoff_claim(
    State(state): State<AppState>,
    Agent(me): Agent,
    Path(Id { id }): Path<Id>,
) -> AppResult<Json<quietdeck_types::handoff::SessionHandoff>> {
    let h = crate::handoffs::claim(&state.db, id, &me).await?;
    state.changed(Changed::Cards);
    Ok(Json(h))
}

async fn report_reply(
    State(state): State<AppState>,
    Agent(me): Agent,
    Query(q): Query<DocQuery>,
    Json(req): Json<quietdeck_types::report::ReportReply>,
) -> AppResult<Json<quietdeck_types::report::Report>> {
    let doc = crate::documents::resolve(&state.db, q.doc.as_deref(), Some(&me)).await?;
    Ok(Json(crate::reports::reply(&state, &doc, req).await?))
}

async fn report_snapshot(
    State(state): State<AppState>,
    _owner: Owner,
    Path(Id { id }): Path<Id>,
) -> AppResult<Json<quietdeck_types::report::ReportSnapshot>> {
    Ok(Json(crate::reports::snapshot_get(&state.db, id).await?))
}

async fn report_answer(
    State(state): State<AppState>,
    _owner: Owner,
    Json(a): Json<quietdeck_types::report::ReportAnswerAdd>,
) -> AppResult<Json<quietdeck_types::report::Report>> {
    Ok(Json(crate::reports::answer(&state, a).await?))
}

async fn report_answers_take(
    State(state): State<AppState>,
    Agent(me): Agent,
    Query(q): Query<DocQuery>,
) -> AppResult<Json<Vec<quietdeck_types::report::ReportAnswer>>> {
    let doc = crate::documents::resolve(&state.db, q.doc.as_deref(), Some(&me)).await?;
    Ok(Json(crate::reports::take_answers(&state, &doc).await?))
}

async fn report_chat(
    State(state): State<AppState>,
    _owner: Owner,
    Query(q): Query<DocQuery>,
    Json(c): Json<quietdeck_types::report::ReportChat>,
) -> AppResult<Json<quietdeck_types::node::Delivered>> {
    let doc = crate::documents::resolve(&state.db, q.doc.as_deref(), None).await?;
    Ok(Json(crate::reports::chat(&state, &doc, c).await?))
}

async fn ping_send(
    State(state): State<AppState>,
    Agent(me): Agent,
    Json(req): Json<quietdeck_types::timeline::PingRequest>,
) -> AppResult<Json<quietdeck_types::timeline::Ping>> {
    Ok(Json(crate::pings::send(&state, &me, req).await?))
}

async fn pings_view(
    State(state): State<AppState>,
    _owner: Owner,
) -> AppResult<Json<quietdeck_types::timeline::PingsView>> {
    Ok(Json(crate::pings::view(&state).await?))
}

async fn ping_feedback(
    State(state): State<AppState>,
    _owner: Owner,
    Path(Id { id }): Path<Id>,
    Json(f): Json<quietdeck_types::timeline::PingFeedback>,
) -> AppResult<Json<quietdeck_types::timeline::Ping>> {
    Ok(Json(crate::pings::feedback(&state, id, f).await?))
}

async fn ping_feedback_take(
    State(state): State<AppState>,
    Agent(me): Agent,
) -> AppResult<Json<Vec<quietdeck_types::timeline::Ping>>> {
    Ok(Json(crate::pings::take_feedback(&state, &me).await?))
}

#[derive(Deserialize, JsonSchema)]
struct WatchesParams {
    #[serde(default)]
    all: bool,
}

async fn watches_list(
    State(state): State<AppState>,
    Authed(_): Authed,
    Query(q): Query<WatchesParams>,
) -> AppResult<Json<Vec<quietdeck_types::timeline::ThreadWatch>>> {
    Ok(Json(crate::pings::watches(&state.db, q.all).await?))
}

async fn watch_add(
    State(state): State<AppState>,
    _owner: Owner,
    Json(w): Json<quietdeck_types::timeline::WatchAdd>,
) -> AppResult<Json<quietdeck_types::timeline::ThreadWatch>> {
    Ok(Json(crate::pings::watch(&state, w).await?))
}

async fn watch_stop(State(state): State<AppState>, _owner: Owner, Path(Id { id }): Path<Id>) -> AppResult<Json<()>> {
    crate::pings::unwatch(&state, id).await?;
    Ok(Json(()))
}

async fn push_config(State(state): State<AppState>, _owner: Owner) -> Json<quietdeck_types::api::PushConfig> {
    Json(quietdeck_types::api::PushConfig { public_key: state.pusher.as_ref().map(|p| p.public_key().to_string()) })
}

async fn push_devices(
    State(state): State<AppState>,
    _owner: Owner,
) -> AppResult<Json<Vec<quietdeck_types::api::PushDevice>>> {
    Ok(Json(crate::push::devices(&state.db).await?))
}

async fn push_subscribe(
    State(state): State<AppState>,
    _owner: Owner,
    Json(s): Json<quietdeck_types::api::PushSubscribe>,
) -> AppResult<Json<Vec<quietdeck_types::api::PushDevice>>> {
    garde::Validate::validate(&s).map_err(bad)?;
    if state.pusher.is_none() {
        return Err(AppError::Conflict("web push is off on this deck: it has no VAPID keys".into()));
    }
    crate::push::check_keys(&s).map_err(AppError::BadRequest)?;
    crate::push::subscribe(&state.db, &s).await?;
    state.changed(Changed::Push);
    Ok(Json(crate::push::devices(&state.db).await?))
}

async fn push_remove(
    State(state): State<AppState>,
    _owner: Owner,
    Json(e): Json<quietdeck_types::api::PushEndpoint>,
) -> AppResult<Json<Vec<quietdeck_types::api::PushDevice>>> {
    crate::push::remove(&state.db, &e.endpoint).await?;
    state.changed(Changed::Push);
    Ok(Json(crate::push::devices(&state.db).await?))
}

async fn rules_view(State(state): State<AppState>, Authed(_): Authed) -> AppResult<Json<RulesView>> {
    Ok(Json(ops::rules_view(&state).await?))
}

async fn rule_create(
    State(state): State<AppState>,
    _owner: Owner,
    Json(input): Json<RuleInput>,
) -> AppResult<Json<Rule>> {
    Ok(Json(ops::rule_create(&state, input).await?))
}

async fn rule_update(
    State(state): State<AppState>,
    _owner: Owner,
    Path(Id { id }): Path<Id>,
    Json(input): Json<RuleInput>,
) -> AppResult<Json<Rule>> {
    Ok(Json(ops::rule_update(&state, id, input).await?))
}

async fn rule_status(
    State(state): State<AppState>,
    _owner: Owner,
    Path(Id { id }): Path<Id>,
    Json(change): Json<RuleStatusChange>,
) -> AppResult<Json<Rule>> {
    Ok(Json(ops::rule_status(&state, id, change).await?))
}

async fn leases_view(
    State(state): State<AppState>,
    Authed(_): Authed,
) -> AppResult<Json<quietdeck_types::lease::LeasesView>> {
    Ok(Json(crate::leases::view(&state.db).await?))
}

async fn lease_propose(
    State(state): State<AppState>,
    Agent(me): Agent,
    Json(req): Json<quietdeck_types::lease::LeaseRequest>,
) -> AppResult<Json<quietdeck_types::lease::LeaseAnswer>> {
    Ok(Json(crate::leases::propose(&state, &me, req).await?))
}

async fn lease_acquire(
    State(state): State<AppState>,
    Agent(me): Agent,
    Json(req): Json<quietdeck_types::lease::LeaseRequest>,
) -> AppResult<Json<quietdeck_types::lease::LeaseAnswer>> {
    Ok(Json(crate::leases::acquire(&state, &me, req).await?))
}

async fn lease_get(
    State(state): State<AppState>,
    Authed(_): Authed,
    Path(Id { id }): Path<Id>,
) -> AppResult<Json<quietdeck_types::lease::Lease>> {
    Ok(Json(crate::leases::get(&state.db, id).await?))
}

/// The holder created the filter itself (no runner): the lease becomes active.
async fn lease_register(
    State(state): State<AppState>,
    Agent(_): Agent,
    Path(Id { id }): Path<Id>,
    Json(r): Json<quietdeck_types::lease::LeaseRegister>,
) -> AppResult<Json<quietdeck_types::lease::Lease>> {
    Ok(Json(crate::leases::activate(&state, id, r.kind_state).await?))
}

async fn lease_release(
    State(state): State<AppState>,
    Authed(caller): Authed,
    Path(Id { id }): Path<Id>,
) -> AppResult<Json<quietdeck_types::lease::Lease>> {
    Ok(Json(crate::leases::release(&state, &caller, id).await?))
}

/// Whoever removed the filter (the holder, the fallback tab, the person) confirms it.
async fn lease_cleaned(
    State(state): State<AppState>,
    Authed(caller): Authed,
    Path(Id { id }): Path<Id>,
) -> AppResult<Json<quietdeck_types::lease::Lease>> {
    let lease = crate::leases::get(&state.db, id).await?;
    let by = match &caller {
        quietdeck_types::api::Caller::Session { session }
            if session.node == lease.holder.node && session.tab == lease.holder.tab =>
        {
            quietdeck_types::lease::CleanupBy::Holder
        }
        quietdeck_types::api::Caller::Node { .. } => quietdeck_types::lease::CleanupBy::Runner,
        _ => quietdeck_types::lease::CleanupBy::Fallback,
    };
    Ok(Json(crate::leases::cleaned(&state, id, by).await?))
}

async fn lease_pattern_revoke(
    State(state): State<AppState>,
    _owner: Owner,
    Path(Id { id }): Path<Id>,
) -> AppResult<Json<quietdeck_types::lease::LeasesView>> {
    Ok(Json(crate::leases::revoke_pattern(&state, id).await?))
}

async fn rules_match(
    State(state): State<AppState>,
    Agent(_): Agent,
    Json(req): Json<MatchRequest>,
) -> AppResult<Json<MatchResult>> {
    Ok(Json(ops::rules_match(&state, req).await?))
}

async fn comment_add(
    State(state): State<AppState>,
    _owner: Owner,
    Json(c): Json<CommentAdd>,
) -> AppResult<Json<Comment>> {
    Ok(Json(ops::comment_add(&state, c).await?))
}

async fn info(State(state): State<AppState>, Authed(caller): Authed) -> AppResult<Json<DeckInfo>> {
    Ok(Json(ops::info(&state, caller).await?))
}

async fn list_nodes(State(state): State<AppState>, Authed(_): Authed) -> AppResult<Json<Vec<NodeRow>>> {
    Ok(Json(nodes::list(&state.db, &state.hub).await?))
}

/// No caller to check: the one-time code is the credential.
async fn enroll(State(state): State<AppState>, Json(req): Json<EnrolRequest>) -> AppResult<Json<Enrolled>> {
    garde::Validate::validate(&req).map_err(bad)?;
    Ok(Json(nodes::enrol(&state.db, &req.code, &req.name).await?))
}

async fn new_code(State(state): State<AppState>, _owner: Owner) -> AppResult<Json<EnrolmentCode>> {
    Ok(Json(nodes::create_enrolment_code(&state.db).await?))
}

async fn revoke_node(
    State(state): State<AppState>,
    _owner: Owner,
    Path(NodeName { name }): Path<NodeName>,
) -> AppResult<()> {
    nodes::revoke(&state.db, &name).await
}

async fn add_card(State(state): State<AppState>, Agent(me): Agent, body: String) -> AppResult<Json<CardAdded>> {
    // Parsed here, not by axum's Json, so every missing and unknown field is named in one error.
    let input = CardInput::from_json(&body).map_err(AppError::BadRequest)?;
    Ok(Json(ops::add_card(&state, &me, input).await?))
}

async fn list_cards(
    State(state): State<AppState>,
    Authed(_): Authed,
    Query(f): Query<cards::ListFilter>,
) -> AppResult<Json<Vec<Card>>> {
    Ok(Json(cards::list(&state.db, &f).await?))
}

async fn get_card(
    State(state): State<AppState>,
    Authed(_): Authed,
    Path(Id { id }): Path<Id>,
) -> AppResult<Json<Card>> {
    cards::get(&state.db, id).await?.map(Json).ok_or_else(|| AppError::NotFound(format!("no card {id}")))
}

async fn resolve_card(
    State(state): State<AppState>,
    Agent(_): Agent,
    Path(Id { id }): Path<Id>,
    Json(how): Json<CardResolution>,
) -> AppResult<Json<Card>> {
    garde::Validate::validate(&how).map_err(bad)?;
    let card = cards::resolve(&state.db, id, &how).await?;
    state.changed(Changed::Cards);
    Ok(Json(card))
}

async fn list_sessions(
    State(state): State<AppState>,
    Authed(_): Authed,
    Query(f): Query<sessions::ListFilter>,
) -> AppResult<Json<Vec<SessionRow>>> {
    Ok(Json(sessions::list(&state.db, &state.hub, &f).await?))
}

#[derive(Deserialize, JsonSchema)]
pub struct SearchQuery {
    pub q: String,
    pub limit: Option<i64>,
}

async fn search_sessions(
    State(state): State<AppState>,
    Authed(_): Authed,
    Query(q): Query<SearchQuery>,
) -> AppResult<Json<Vec<SessionRow>>> {
    Ok(Json(sessions::search(&state.db, &state.hub, &q.q, q.limit.unwrap_or(20)).await?))
}

#[derive(Deserialize, JsonSchema)]
pub struct ShowQuery {
    pub session: String,
    #[serde(default)]
    pub timeline: bool,
}

async fn show_session(
    State(state): State<AppState>,
    Authed(_): Authed,
    Query(q): Query<ShowQuery>,
) -> AppResult<Json<quietdeck_types::directory::SessionDetail>> {
    Ok(Json(crate::ops::show_session(&state, &q.session, q.timeline).await?))
}

#[derive(Deserialize, JsonSchema)]
pub struct TranscriptParams {
    pub session: String,
    pub subagent: Option<String>,
    pub before: Option<u64>,
    pub after: Option<u64>,
}

async fn session_transcript(
    State(state): State<AppState>,
    _owner: Owner,
    Query(q): Query<TranscriptParams>,
) -> AppResult<Json<quietdeck_types::transcript::TranscriptPage>> {
    let query = quietdeck_types::transcript::TranscriptQuery { subagent: q.subagent, before: q.before, after: q.after };
    Ok(Json(ops::transcript(&state, &q.session, query).await?))
}

#[derive(Deserialize, JsonSchema)]
pub struct SessionParam {
    pub session: String,
}

async fn session_catchup(
    State(state): State<AppState>,
    _owner: Owner,
    Query(q): Query<SessionParam>,
) -> AppResult<Json<quietdeck_types::summary::Catchup>> {
    Ok(Json(ops::catchup(&state, &q.session).await?))
}

/// The owner, or one of the owner's sessions: starting or resuming a session is something either may do, like
/// running herdr. A node's own token can't.
fn person_or_session(caller: Caller) -> AppResult<Caller> {
    match caller {
        Caller::Node { .. } => Err(AppError::Forbidden("a node can't start sessions".into())),
        c => Ok(c),
    }
}

async fn start_session(
    State(state): State<AppState>,
    Authed(caller): Authed,
    Json(spec): Json<SessionStart>,
) -> AppResult<Json<SessionStarted>> {
    let caller = person_or_session(caller)?;
    Ok(Json(control::start(&state, &caller, spec).await?))
}

async fn resume_session(
    State(state): State<AppState>,
    Authed(caller): Authed,
    Json(target): Json<SessionTarget>,
) -> AppResult<Json<SessionStarted>> {
    let caller = person_or_session(caller)?;
    Ok(Json(control::resume(&state, &caller, target).await?))
}

/// Closing a session is the owner's click on the registry.
async fn close_session(
    State(state): State<AppState>,
    Authed(caller): Authed,
    Json(target): Json<SessionTarget>,
) -> AppResult<Json<serde_json::Value>> {
    // The owner closes any session; a session closes itself, or one that has moved (control::close checks).
    let caller = person_or_session(caller)?;
    control::close(&state, &caller, target).await?;
    Ok(Json(serde_json::json!({})))
}

/// Stop is the owner's click: Esc in the session.
async fn interrupt_session(
    State(state): State<AppState>,
    owner: Owner,
    Json(target): Json<SessionTarget>,
) -> AppResult<Json<serde_json::Value>> {
    let _ = owner;
    control::interrupt(&state, target).await?;
    Ok(Json(serde_json::json!({})))
}

async fn who_has(
    State(state): State<AppState>,
    Authed(_): Authed,
    Query(q): Query<SearchQuery>,
) -> AppResult<Json<WhoHas>> {
    Ok(Json(ops::who_has(&state, &q.q).await?))
}

async fn msg_send(
    State(state): State<AppState>,
    Agent(me): Agent,
    Json(msg): Json<MsgSend>,
) -> AppResult<Json<Delivered>> {
    Ok(Json(ops::msg_send(&state, &me, msg).await?))
}

async fn timeline_add(
    State(state): State<AppState>,
    Authed(caller): Authed,
    Json(e): Json<TimelineAdd>,
) -> AppResult<Json<TimelineEntry>> {
    Ok(Json(ops::timeline_add(&state, &caller, e).await?))
}

#[derive(Deserialize, JsonSchema)]
pub struct HistoryQuery {
    /// For an agent: `last` (since this session last asked, the default), or an RFC 3339 time. For the owner: an RFC
    /// 3339 time, a week before `until` by default.
    pub since: Option<String>,
    /// The owner's History screen only: the window's end, now by default.
    pub until: Option<String>,
    pub account: Option<String>,
}

async fn history(
    State(state): State<AppState>,
    Authed(caller): Authed,
    Query(q): Query<HistoryQuery>,
) -> AppResult<Json<History>> {
    match caller {
        Caller::Session { session } => {
            Ok(Json(ops::history(&state, &session, q.since.as_deref().unwrap_or("last")).await?))
        }
        Caller::Owner { .. } => {
            Ok(Json(ops::owner_history(&state, q.since.as_deref(), q.until.as_deref(), q.account.as_deref()).await?))
        }
        Caller::Node { .. } => Err(AppError::Forbidden("history is for sessions and the owner".into())),
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct AccountQuery {
    pub account: Option<String>,
}

/// The Inbox, or with `doc`, the same data with that document's report in it.
#[derive(Deserialize, JsonSchema)]
pub struct InboxQuery {
    pub account: Option<String>,
    pub doc: Option<String>,
}

async fn inbox(State(state): State<AppState>, _owner: Owner, Query(q): Query<InboxQuery>) -> AppResult<Json<Inbox>> {
    let doc = crate::documents::resolve(&state.db, q.doc.as_deref(), None).await?;
    Ok(Json(ops::inbox_for(&state, q.account.as_deref(), doc.id).await?))
}

async fn later(
    State(state): State<AppState>,
    _owner: Owner,
    Query(q): Query<AccountQuery>,
) -> AppResult<Json<Vec<Card>>> {
    Ok(Json(ops::later(&state, q.account.as_deref()).await?))
}

async fn get_settings(State(state): State<AppState>, Authed(_): Authed) -> AppResult<Json<Settings>> {
    Ok(Json(load_settings(&state.db).await?))
}

async fn put_settings(
    State(state): State<AppState>,
    _owner: Owner,
    Json(s): Json<Settings>,
) -> AppResult<Json<Settings>> {
    Ok(Json(ops::save_settings(&state, s).await?))
}

async fn approve_all(
    State(state): State<AppState>,
    _owner: Owner,
    Json(req): Json<ApproveAll>,
) -> AppResult<Json<Approving>> {
    Ok(Json(ops::approve_all(&state, req).await?))
}

async fn approve_report(
    State(state): State<AppState>,
    _owner: Owner,
    Json(req): Json<quietdeck_types::decision::ApproveReport>,
) -> AppResult<Json<Approving>> {
    Ok(Json(ops::approve_report(&state, req).await?))
}

async fn undo(State(state): State<AppState>, _owner: Owner, Json(u): Json<UndoBatch>) -> AppResult<()> {
    ops::undo(&state, &u.batch).await
}

async fn stop(State(state): State<AppState>, _owner: Owner) -> AppResult<Json<StopResult>> {
    Ok(Json(ops::stop(&state).await?))
}

async fn decide(
    State(state): State<AppState>,
    _owner: Owner,
    Path(Id { id }): Path<Id>,
    Json(d): Json<Decide>,
) -> AppResult<Json<Card>> {
    Ok(Json(ops::decide(&state, id, d).await?))
}

async fn retry(State(state): State<AppState>, _owner: Owner, Path(Id { id }): Path<Id>) -> AppResult<Json<Approving>> {
    Ok(Json(ops::retry(&state, id).await?))
}

async fn take(State(state): State<AppState>, Agent(me): Agent) -> AppResult<Json<Vec<Released>>> {
    Ok(Json(ops::take(&state, &me).await?))
}

async fn round_status(State(state): State<AppState>, Agent(me): Agent) -> AppResult<Json<RoundStatus>> {
    Ok(Json(crate::runner::round_status(&state, &me).await?))
}

async fn round_done(State(state): State<AppState>, Agent(me): Agent) -> AppResult<Json<RoundDone>> {
    Ok(Json(RoundDone { runner_told: crate::runner::round_done(&state, &me).await? }))
}

async fn action_start(
    State(state): State<AppState>,
    Agent(me): Agent,
    Path(Id { id }): Path<Id>,
) -> AppResult<Json<ActionRecord>> {
    Ok(Json(ops::action_start(&state, &me, id).await?))
}

async fn actions_report(
    State(state): State<AppState>,
    Agent(me): Agent,
    Json(reports): Json<Vec<ActionReport>>,
) -> AppResult<Json<Vec<ReportReceipt>>> {
    Ok(Json(ops::actions_report(&state, &me, reports).await?))
}

/// The settings, read with defaults for anything never set.
pub async fn load_settings(db: &crate::db::Db) -> anyhow::Result<Settings> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT name, value FROM settings").fetch_all(db).await?;
    let mut s = Settings::default();
    for (name, value) in rows {
        match name.as_str() {
            "undo_seconds" => s.undo_seconds = value.parse().unwrap_or(s.undo_seconds),
            "require_filter_proposal" => s.require_filter_proposal = value == "true",
            "show_pending_count" => s.show_pending_count = value == "true",
            "report" => s.report = value == "true",
            _ => {}
        }
    }
    Ok(s)
}
