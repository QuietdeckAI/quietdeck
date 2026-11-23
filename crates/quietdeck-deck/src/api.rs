//! The REST API. Agents call it with the `quietdeck` CLI (session tokens from their node); the web app with the
//! owner's sign-in. The MCP endpoint calls the same functions (`ops`).

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::routing::{delete, get, post};
use quietdeck_types::api::{DeckInfo, EnrolRequest, Enrolled, EnrolmentCode, Settings};
use quietdeck_types::card::{Card, CardAdded, CardInput, CardResolution};
use quietdeck_types::directory::{NodeRow, SessionRow, WhoHas};
use quietdeck_types::node::Delivered;
use quietdeck_types::relay::MsgSend;
use quietdeck_types::timeline::{History, TimelineAdd, TimelineEntry};
use serde::Deserialize;

use crate::auth::{Agent, Authed, Owner};
use crate::error::{AppError, AppResult, bad};
use crate::ops;
use crate::state::AppState;
use crate::store::{cards, nodes, sessions};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/info", get(info))
        .route("/api/nodes", get(list_nodes))
        .route("/api/nodes/enroll", post(enroll))
        .route("/api/nodes/enrolment-codes", post(new_code))
        .route("/api/nodes/ws", get(crate::ws::upgrade))
        .route("/api/nodes/{name}", delete(revoke_node))
        .route("/api/cards", get(list_cards).post(add_card))
        .route("/api/cards/{id}", get(get_card))
        .route("/api/cards/{id}/resolve", post(resolve_card))
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/search", get(search_sessions))
        .route("/api/who-has", get(who_has))
        .route("/api/msg", post(msg_send))
        .route("/api/timeline", post(timeline_add))
        .route("/api/history", get(history))
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

async fn new_code(State(state): State<AppState>, Owner(_): Owner) -> AppResult<Json<EnrolmentCode>> {
    Ok(Json(nodes::create_enrolment_code(&state.db).await?))
}

async fn revoke_node(State(state): State<AppState>, Owner(_): Owner, Path(name): Path<String>) -> AppResult<()> {
    nodes::revoke(&state.db, &name).await
}

async fn add_card(
    State(state): State<AppState>,
    Agent(me): Agent,
    Json(input): Json<CardInput>,
) -> AppResult<Json<CardAdded>> {
    Ok(Json(ops::add_card(&state, &me, input).await?))
}

async fn list_cards(
    State(state): State<AppState>,
    Authed(_): Authed,
    Query(f): Query<cards::ListFilter>,
) -> AppResult<Json<Vec<Card>>> {
    Ok(Json(cards::list(&state.db, &f).await?))
}

async fn get_card(State(state): State<AppState>, Authed(_): Authed, Path(id): Path<i64>) -> AppResult<Json<Card>> {
    cards::get(&state.db, id).await?.map(Json).ok_or_else(|| AppError::NotFound(format!("no card {id}")))
}

async fn resolve_card(
    State(state): State<AppState>,
    Agent(_): Agent,
    Path(id): Path<i64>,
    Json(how): Json<CardResolution>,
) -> AppResult<Json<Card>> {
    garde::Validate::validate(&how).map_err(bad)?;
    Ok(Json(cards::resolve(&state.db, id, &how).await?))
}

async fn list_sessions(
    State(state): State<AppState>,
    Authed(_): Authed,
    Query(f): Query<sessions::ListFilter>,
) -> AppResult<Json<Vec<SessionRow>>> {
    Ok(Json(sessions::list(&state.db, &state.hub, &f).await?))
}

#[derive(Deserialize)]
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

#[derive(Deserialize)]
pub struct HistoryQuery {
    /// `last` (since this session last asked), or an RFC 3339 time.
    pub since: Option<String>,
}

async fn history(
    State(state): State<AppState>,
    Agent(me): Agent,
    Query(q): Query<HistoryQuery>,
) -> AppResult<Json<History>> {
    Ok(Json(ops::history(&state, &me, q.since.as_deref().unwrap_or("last")).await?))
}

/// The settings, read with defaults for anything never set.
pub async fn load_settings(db: &crate::db::Db) -> anyhow::Result<Settings> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT name, value FROM settings").fetch_all(db).await?;
    let mut s = Settings::default();
    for (name, value) in rows {
        match name.as_str() {
            "undo_seconds" => s.undo_seconds = value.parse().unwrap_or(s.undo_seconds),
            "require_filter_proposal" => s.require_filter_proposal = value == "true",
            _ => {}
        }
    }
    Ok(s)
}
