//! The published API: every route in `api.rs` and `passkeys.rs`, mounted at `/api/v1`, described by an OpenAPI 3.1
//! spec that aide builds from the handlers' own types, so the spec can't drift from the code.
//!
//! - The same routes answer at `/api` too, unversioned, for the nodes, CLIs and web apps that predate v1. The spec
//!   lists only `/api/v1`.
//! - `api/openapi.json` is the committed copy (Apache-2.0, `api/LICENSE`), rewritten by this module's test, and CI
//!   fails when it differs from what the code generates. The deck serves the same document at
//!   `/api/v1/openapi.json`.
//! - [`OPERATIONS`] gives every operation its ID, tag and summary. A test fails when a route has no entry, or an entry
//!   names no route.
//! - The compatibility policy (additive changes within v1) is in `api/README.md`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use aide::generate::GenContext;
use aide::openapi::{MediaType, OpenApi, Operation, Response, SchemaObject, StatusCode};
use aide::{OperationInput, OperationOutput};
use axum::extract::{MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use indexmap::IndexMap;
use quietdeck_types::api::ApiError;
use quietdeck_types::card::CardInput;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::{Agent, Authed, Owner};
use crate::error::AppError;
use crate::state::AppState;
use crate::{api, passkeys};

/// `info.version`: the minor version goes up with each additive change, the patch version with fixes to the
/// description alone. A breaking change is a new `/api/v2`, never a new major version of v1.
pub const API_VERSION: &str = "1.11.0";

/// The versioned prefix the spec describes.
pub const V1: &str = "/api/v1";

/// The spec's extension marking what only the default web app uses: a frontend of its own can ignore those.
pub const UI_EXTENSION: &str = "x-quietdeck-ui";

/// A numeric ID in the path (`/cards/{id}`).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct Id {
    pub id: i64,
}

/// A text ID in the path (`/hooks/{id}`).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TextId {
    pub id: String,
}

/// A node's name in the path (`/nodes/{name}`).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct NodeName {
    pub name: String,
}

/// How an operation is documented: its `operationId` (the handler's name), its tag, its one-line summary, and
/// whether only the default web app needs it (the [`UI_EXTENSION`]'s value).
pub struct Doc {
    pub method: &'static str,
    pub path: &'static str,
    pub id: &'static str,
    pub tag: &'static str,
    pub summary: &'static str,
    pub ui: Option<&'static str>,
}

const fn op(
    method: &'static str,
    path: &'static str,
    id: &'static str,
    tag: &'static str,
    summary: &'static str,
) -> Doc {
    Doc { method, path, id, tag, summary, ui: None }
}

const fn ui(
    ui: &'static str,
    method: &'static str,
    path: &'static str,
    id: &'static str,
    tag: &'static str,
    summary: &'static str,
) -> Doc {
    Doc { method, path, id, tag, summary, ui: Some(ui) }
}

/// The tags, in the reference's order, with what each covers.
pub const TAGS: &[(&str, &str)] = &[
    (
        "deck",
        "The deck itself. `GET /info` says who you are and carries the settings and accounts a frontend needs to \
         start; `GET /events` is a server-sent event stream naming what changed, so a frontend reloads only that.",
    ),
    (
        "auth",
        "Passkey sign-in for the owner's browsers. The first passkey takes a one-time setup code from the deck's \
         host; signing in sets an HttpOnly cookie, which the `owner` security scheme names.",
    ),
    (
        "nodes",
        "Machines enrolled with the deck. A node enrols once with a one-time code, then keeps a WebSocket open \
         (the node protocol, which frontends don't need) and issues session tokens to the agents on it.",
    ),
    (
        "cards",
        "A card is what an agent proposes about one email thread: a summary, why, and the actions it would take \
         (archive, label, reply, forward and more). One card per thread: posting again for the same thread makes a \
         new version. A decision names the version on screen, so the person never approves something they didn't \
         see.",
    ),
    (
        "decisions",
        "The owner's decisions. Approve All sends every card on screen with its decision; the batch waits out the \
         undo window (Settings), then the runner takes it. `undo` cancels a batch inside its window; `stop` holds \
         everything approved that hasn't started.",
    ),
    (
        "actions",
        "The runner carrying approved actions out: it marks each one started, then reports the results, which \
         finish the card. The round routes tell the woken agent where the mail watch is.",
    ),
    (
        "inbox",
        "Read views for a frontend: the Inbox (open cards by section, the unread timeline, the batch in its undo \
         window), Later, and the mail tabs' lists and badges.",
    ),
    (
        "timeline",
        "What happened without a card: automatic actions the agents carried out, History over a time range, the \
         owner's comments on either, pings (agents asking for attention, against a weekly budget) and thread \
         watches.",
    ),
    (
        "sessions",
        "Agent sessions on every node: list and search them, read a transcript or a catch-up, and start, resume, \
         interrupt or close one.",
    ),
    (
        "messages",
        "Messages between sessions by address (`tab@node`), chat messages the deck keeps for agents to read, and \
         handoffs that move a session's work to a new one.",
    ),
    (
        "rules",
        "Rules decide what may be carried out without asking: a card that matches an active rule skips the Inbox. \
         Reading rules decide what may still arrive while the owner is reading the Inbox.",
    ),
    (
        "leases",
        "Retired: leases moved to Fleetlayer (`fleetlayer lease`, its leases plugin). These routes stay within v1 \
         and answer 410 Gone with an `ApiError` saying so.",
    ),
    (
        "hooks",
        "Webhook endpoints that wake agents. An agent proposes one; it receives nothing until the owner approves \
         it.",
    ),
    ("push", "Web push: the deck's VAPID key, and the browsers subscribed to notifications."),
    (
        "report",
        "The default web app's report: the triage agent's write-up of the Inbox, its answers, chat and snapshots. \
         A frontend of its own can ignore these (they carry `x-quietdeck-ui: report`), and with the `report` \
         setting off they answer 404.",
    ),
    (
        "documents",
        "Documents are the pages agents write and the owner reads, each under a top-level tab: the Inbox (kind \
         `inbox`) on every deck, and others such as a Work page about the owner's agents (kind `work`). An agent's \
         report commands go to the document it writes; `?doc=<slug>` picks one. Session marks are the owner's \
         records about sessions: ignored, or parked until a time.",
    ),
    (
        "presence",
        "Whether the owner is reading the Inbox in the default web app, so the runner holds new items meanwhile \
         (`x-quietdeck-ui: presence`).",
    ),
];

/// Every operation, by method and path (without the `/api/v1` prefix). One line each, so it reads as a table.
#[rustfmt::skip]
pub const OPERATIONS: &[Doc] = &[
    op("get", "/info", "info", "deck", "The deck's version and release, and who the caller is"),
    op("get", "/settings", "get_settings", "deck", "The deck's settings"),
    op("put", "/settings", "put_settings", "deck", "Change the deck's settings"),
    op("get", "/events", "events", "deck", "Server-sent events naming what changed, so a frontend knows what to reload"),
    op("get", "/auth/state", "auth_state", "auth", "Whether this browser is signed in, and whether a passkey exists yet"),
    op("post", "/auth/register/start", "register_start", "auth", "Start registering a passkey"),
    op("post", "/auth/register/finish", "register_finish", "auth", "Finish registering a passkey, which signs this browser in"),
    op("post", "/auth/login/start", "login_start", "auth", "Start signing in with a passkey"),
    op("post", "/auth/login/finish", "login_finish", "auth", "Finish signing in with a passkey: sets the session cookie"),
    op("post", "/auth/logout", "logout", "auth", "Sign this browser out"),
    op("get", "/passkeys", "list_passkeys", "auth", "The owner's passkeys"),
    op("patch", "/passkeys/{id}", "rename_passkey", "auth", "Rename a passkey"),
    op("delete", "/passkeys/{id}", "remove_passkey", "auth", "Remove a passkey"),
    op("get", "/nodes", "list_nodes", "nodes", "The enrolled nodes"),
    op("post", "/nodes/enroll", "enroll", "nodes", "Enrol a node with a one-time enrolment code"),
    op("post", "/nodes/enrolment-codes", "new_code", "nodes", "Make a one-time enrolment code for a new node"),
    op("get", "/nodes/ws", "node_socket", "nodes", "A node's WebSocket to the deck (the node protocol, not for frontends)"),
    op("delete", "/nodes/{name}", "revoke_node", "nodes", "Revoke a node"),
    op("get", "/cards", "list_cards", "cards", "Cards, filtered by account, section or state"),
    op("post", "/cards", "add_card", "cards", "Post a card, or a new version of the open card for the same thread"),
    op("get", "/cards/{id}", "get_card", "cards", "One card"),
    op("post", "/cards/{id}/resolve", "resolve_card", "cards", "The agent's recheck found the card needs nothing more"),
    op("post", "/cards/{id}/decide", "decide", "decisions", "Decide one card: approve, adjust, reject or later"),
    op("post", "/cards/{id}/retry", "retry", "decisions", "Retry a card whose actions failed"),
    op("post", "/decisions/approve-all", "approve_all", "decisions", "Approve a list of cards, each at the version on screen"),
    op("post", "/decisions/undo", "undo", "decisions", "Undo a batch of decisions while it's in its undo window"),
    op("post", "/decisions/stop", "stop", "decisions", "Stop: hold every approved action that hasn't started"),
    op("post", "/decisions/take", "take", "decisions", "The runner takes the decisions released from the undo window"),
    op("post", "/actions/{id}/start", "action_start", "actions", "The runner, just before carrying an action out"),
    op("post", "/actions/report", "actions_report", "actions", "The runner's results for actions it carried out"),
    op("get", "/round", "round_status", "actions", "Where the runner's mail watch is, for the agent it wakes"),
    op("post", "/round/done", "round_done", "actions", "The woken agent finished its round"),
    op("get", "/inbox", "inbox", "inbox", "The Inbox, for one account or all of them"),
    op("get", "/later", "later", "inbox", "Cards put off with Later"),
    op("get", "/mail", "mail_list", "inbox", "One of the mail tabs' lists"),
    op("post", "/mail/seen", "mail_seen", "inbox", "Mark a mail tab seen, which clears its badge"),
    op("get", "/mail/badges", "mail_badges", "inbox", "The counts on the mail tabs"),
    op("post", "/timeline", "timeline_add", "timeline", "An agent records an automatic action it carried out"),
    op("get", "/history", "history", "timeline", "What happened since the calling session last looked"),
    op("post", "/comments", "comment_add", "timeline", "Comment on a timeline entry or a History row"),
    op("get", "/pings", "pings_view", "timeline", "The last seven days' pings, against the weekly budget"),
    op("post", "/pings", "ping_send", "timeline", "An agent pings the owner"),
    op("post", "/pings/{id}/feedback", "ping_feedback", "timeline", "The owner's feedback on a ping"),
    op("post", "/pings/feedback/take", "ping_feedback_take", "timeline", "An agent takes the feedback on its pings"),
    op("get", "/watches", "watches_list", "timeline", "Thread watches"),
    op("post", "/watches", "watch_add", "timeline", "Watch a thread"),
    op("post", "/watches/{id}/stop", "watch_stop", "timeline", "Stop watching a thread"),
    op("get", "/sessions", "list_sessions", "sessions", "Sessions on every node"),
    op("get", "/sessions/search", "search_sessions", "sessions", "Search sessions"),
    op("get", "/sessions/show", "show_session", "sessions", "One session, optionally with its timeline"),
    op("get", "/sessions/transcript", "session_transcript", "sessions", "A page of a session's transcript"),
    op("get", "/sessions/catchup", "session_catchup", "sessions", "What happened in a session since the owner's last message"),
    op("post", "/sessions/start", "start_session", "sessions", "Start a session on a node"),
    op("post", "/sessions/resume", "resume_session", "sessions", "Resume a session"),
    op("post", "/sessions/close", "close_session", "sessions", "Close a session"),
    op("post", "/sessions/interrupt", "interrupt_session", "sessions", "Interrupt a session"),
    op("post", "/sessions/answer", "answer_prompt", "sessions", "Answer a session's blocked prompt (its own menu), through Fleetlayer"),
    op("get", "/who-has", "who_has", "sessions", "Which session has a thread, account or topic"),
    op("post", "/msg", "msg_send", "messages", "Send a message to a session by address"),
    op("get", "/messages", "messages_read", "messages", "Read messages after the caller's last read, or after a message ID"),
    op("post", "/handoffs", "handoff_add", "messages", "A session writes a handoff about itself"),
    op("get", "/handoffs/{id}", "handoff_show", "messages", "One handoff"),
    op("post", "/handoffs/{id}/claim", "handoff_claim", "messages", "The new session claims a handoff"),
    op("get", "/rules", "rules_view", "rules", "The rules"),
    op("post", "/rules", "rule_create", "rules", "Create a rule"),
    op("put", "/rules/{id}", "rule_update", "rules", "Change a rule"),
    op("post", "/rules/{id}/status", "rule_status", "rules", "Archive, restore or move a rule"),
    op("post", "/rules/match", "rules_match", "rules", "May this card be carried out without the owner?"),
    op("get", "/reading-rules", "reading_rules_list", "rules", "Reading rules: what may arrive while the owner reads"),
    op("post", "/reading-rules", "reading_rule_create", "rules", "Create a reading rule"),
    op("put", "/reading-rules/{id}", "reading_rule_update", "rules", "Change a reading rule"),
    op("delete", "/reading-rules/{id}", "reading_rule_delete", "rules", "Delete a reading rule"),
    op("post", "/reading-rules/{id}/enabled", "reading_rule_enabled", "rules", "Turn a reading rule on or off"),
    op("get", "/leases", "leases_view", "leases", "Retired: answers 410 Gone (leases moved to Fleetlayer)"),
    op("post", "/leases/propose", "lease_propose", "leases", "Retired: answers 410 Gone (leases moved to Fleetlayer)"),
    op("post", "/leases/acquire", "lease_acquire", "leases", "Retired: answers 410 Gone (leases moved to Fleetlayer)"),
    op("get", "/leases/{id}", "lease_get", "leases", "Retired: answers 410 Gone (leases moved to Fleetlayer)"),
    op("post", "/leases/{id}/register", "lease_register", "leases", "Retired: answers 410 Gone (leases moved to Fleetlayer)"),
    op("post", "/leases/{id}/release", "lease_release", "leases", "Retired: answers 410 Gone (leases moved to Fleetlayer)"),
    op("post", "/leases/{id}/cleaned", "lease_cleaned", "leases", "Retired: answers 410 Gone (leases moved to Fleetlayer)"),
    op("post", "/lease-patterns/{id}/revoke", "lease_pattern_revoke", "leases", "Retired: answers 410 Gone (leases moved to Fleetlayer)"),
    op("get", "/hooks", "hooks_list", "hooks", "Webhook endpoints"),
    op("post", "/hooks", "hooks_create", "hooks", "Propose a webhook endpoint; it receives nothing until approved"),
    op("post", "/hooks/{id}/approve", "hooks_approve", "hooks", "Approve a webhook endpoint"),
    op("delete", "/hooks/{id}", "hooks_delete", "hooks", "Delete a webhook endpoint"),
    op("get", "/push", "push_config", "push", "Web push on this deck: the VAPID public key, or none when push is off"),
    op("get", "/push/devices", "push_devices", "push", "The devices that get push notifications"),
    op("post", "/push/devices", "push_subscribe", "push", "Subscribe this browser to push notifications"),
    op("post", "/push/devices/remove", "push_remove", "push", "Remove a push device"),
    ui("report", "get", "/report", "report_show", "report", "The triage agent's current report"),
    ui("report", "post", "/report", "report_set", "report", "The triage agent writes a new version of the report"),
    ui("report", "post", "/report/done", "report_done", "report", "The triage agent marks the report done"),
    ui(
        "report",
        "post",
        "/report/approve",
        "approve_report",
        "report",
        "Approve All by the report's Defaults: every default not changed on the version on screen",
    ),
    ui("report", "post", "/report/answers", "report_answer", "report", "The owner answers a question in the report"),
    ui("report", "post", "/report/answers/take", "report_answers_take", "report", "The triage agent takes the answers"),
    ui("report", "post", "/report/chat", "report_chat", "report", "The owner writes to the triage agent from the report"),
    ui("report", "post", "/report/replies", "report_reply", "report", "The triage agent replies in the report"),
    ui("report", "post", "/chat/reply", "chat_reply", "report", "The triage agent replies in the report's chat"),
    ui("report", "get", "/report/snapshots/{id}", "report_snapshot", "report", "An earlier version of the report"),
    ui("report", "post", "/report/fyi/clear", "report_fyi_clear", "report", "Clear a Work document's FYI list"),
    ui("report", "get", "/fleetlayer/approvals", "fleet_approvals", "report", "Fleetlayer's pending access requests and open admin windows"),
    ui("report", "post", "/fleetlayer/approvals/{id}", "fleet_decide", "report", "Approve or refuse a Fleetlayer request (a window needs the passkey page first)"),
    ui("report", "post", "/fleetlayer/windows/{id}/close", "fleet_close", "report", "Close a Fleetlayer admin window now (id is its role)"),
    op("get", "/documents", "documents_list", "documents", "The documents, in order, with the tab each sits under"),
    op("post", "/documents", "documents_add", "documents", "Add a document"),
    op("patch", "/documents/{id}", "documents_edit", "documents", "Rename a document, move it, or change who writes it"),
    op("post", "/documents/{id}/remove", "documents_remove", "documents", "Remove a document (not the Inbox)"),
    op("get", "/session-marks", "session_marks_list", "documents", "The owner's marks on sessions: ignored, parked"),
    op("post", "/session-marks", "session_marks_set", "documents", "Ignore a session, or park it (until a time)"),
    op("post", "/session-marks/remove", "session_marks_remove", "documents", "Take a mark off a session"),
    ui("presence", "post", "/presence", "presence_beat", "presence", "The Inbox page's heartbeat while the owner reads"),
    ui("presence", "post", "/presence/release", "presence_release", "presence", "The owner stopped reading"),
];

/// The API's routes, at `/api/v1` and at `/api`, with `/api/v1/openapi.json`; and the spec they serve.
pub fn routes(state: AppState) -> (Router<AppState>, Arc<Value>) {
    let (v1, legacy, spec) = build(None);
    let spec = Arc::new(spec);
    let served = spec.clone();
    let router = v1
        .merge(legacy)
        .route_layer(axum::middleware::from_fn_with_state(state, report_gate))
        .route_layer(axum::middleware::map_response(json_errors))
        .route(&format!("{V1}/openapi.json"), get(move || std::future::ready(Json(served.as_ref().clone()))));
    (router, spec)
}

/// Whether a matched route (at either prefix) is one of the report's: `OPERATIONS` with `ui: report`.
fn is_report(method: &axum::http::Method, matched: &str) -> bool {
    let path = matched.strip_prefix(V1).or_else(|| matched.strip_prefix("/api"));
    OPERATIONS
        .iter()
        .any(|d| d.ui == Some("report") && Some(d.path) == path && method.as_str().eq_ignore_ascii_case(d.method))
}

/// The report's routes answer 404 while the deck has no report (Settings: `report` off).
async fn report_gate(
    State(state): State<AppState>,
    matched: Option<MatchedPath>,
    req: Request,
    next: Next,
) -> axum::response::Response {
    if matched.is_some_and(|m| is_report(req.method(), m.as_str())) {
        match crate::reports::enabled(&state.db).await {
            Ok(true) => {}
            Ok(false) => return AppError::NotFound(crate::reports::OFF.into()).into_response(),
            Err(e) => return AppError::Internal(e).into_response(),
        }
    }
    next.run(req).await
}

/// axum answers a request it can't read (bad JSON, the wrong content type, a bad query or path) in plain text. Every
/// error the API answers is `ApiError` JSON instead, so a client handles one shape.
async fn json_errors(res: axum::response::Response) -> axum::response::Response {
    let plain = res
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/plain"));
    if !(plain && res.status().is_client_error()) {
        return res;
    }
    let (parts, body) = res.into_parts();
    let text = match axum::body::to_bytes(body, 16 * 1024).await {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => "the request couldn't be read".to_string(),
    };
    (parts.status, Json(ApiError { error: text })).into_response()
}

/// The spec alone, as the committed `api/openapi.json` has it.
pub fn spec() -> Value {
    build(None).2
}

/// What went wrong generating the spec, which the test reports.
pub fn spec_errors() -> Vec<String> {
    let errors = Rc::new(RefCell::new(Vec::new()));
    let _ = build(Some(errors.clone()));
    errors.take()
}

fn build(errors: Option<Rc<RefCell<Vec<String>>>>) -> (Router<AppState>, Router<AppState>, Value) {
    aide::generate::reset_context();
    if let Some(errors) = errors {
        aide::generate::on_error(move |e| errors.borrow_mut().push(e.to_string()));
    }
    let core = api::router().merge(passkeys::router());
    let legacy = Router::new().nest("/api", Router::from(core.clone()));
    // `POST /cards` reads its body as text, to name the field a bad card got wrong: its schema is `CardInput`'s.
    let card_input = aide::generate::in_context(|ctx| ctx.schema.subschema_for::<CardInput>());
    // The event stream's payload, which no handler's type names.
    let changed = aide::generate::in_context(|ctx| ctx.schema.subschema_for::<quietdeck_types::api::Changed>());
    let mut api = OpenApi::default();
    let v1 = aide::axum::ApiRouter::new().nest(V1, core).finish_api(&mut api);
    let mut spec = serde_json::to_value(&api).expect("the spec serialises");
    describe(&mut spec, card_input.to_value(), changed.to_value());
    let mut spec = sorted(spec);
    prune_unused_schemas(&mut spec);
    (v1, legacy, spec)
}

/// Routes that take no sign-in: the sign-in ceremony itself, and enrolment (which takes a one-time code).
const OPEN: &[&str] = &[
    "/auth/state",
    "/auth/register/start",
    "/auth/register/finish",
    "/auth/login/start",
    "/auth/login/finish",
    "/auth/logout",
    "/nodes/enroll",
];

/// Info, tags, security schemes, each operation's ID, tag and summary, the event stream, and the card body.
fn describe(spec: &mut Value, card_input: Value, changed: Value) {
    spec["info"] = json!({
        "title": "Quietdeck API",
        "version": API_VERSION,
        "summary": "Quietdeck's core: cards, decisions, actions, rules, sessions and the timeline.",
        "description": "Build your own frontend for a Quietdeck deck. Every route is under `/api/v1`. Within v1, \
            changes are additive only: new routes, new optional fields in requests, new fields in responses (ignore \
            the ones you don't know) and new enum values (handle the ones you don't know). Anything that would break \
            a client goes in `/api/v2`, and v1 keeps working alongside it. Operations tagged `report` or `presence` \
            (with `x-quietdeck-ui`) belong to the default web app's own screens: a frontend of its own can ignore \
            them, and with the `report` setting off the report's routes answer 404 and the rest of the API works \
            on cards alone. Errors answer `ApiError` with a 4xx or 5xx status.",
        "license": { "name": "Apache-2.0", "identifier": "Apache-2.0" },
    });
    spec["tags"] = TAGS.iter().map(|(name, description)| json!({ "name": name, "description": description })).collect();
    spec["components"]["securitySchemes"] = json!({
        "owner": {
            "type": "apiKey",
            "in": "cookie",
            "name": crate::passkeys::COOKIE_NAME,
            "description": "The owner in a browser, signed in with a passkey (`/auth/login/*`). Requests that change \
                something must come from the deck's own origin.",
        },
        "session": {
            "type": "http",
            "scheme": "bearer",
            "bearerFormat": "qds_…",
            "description": "An agent's session token, from the node it runs on (the `quietdeck` CLI).",
        },
        "node": {
            "type": "http",
            "scheme": "bearer",
            "bearerFormat": "qdn_…",
            "description": "A node's own token, from enrolment.",
        },
    });
    spec["servers"] = json!([{
        "url": "https://{deck}",
        "description": "Your deck",
        "variables": { "deck": { "default": "deck.example.com", "description": "The deck's host name" } },
    }]);
    let paths = &mut spec["paths"];
    for path in OPEN {
        for (_, operation) in paths[format!("{V1}{path}")].as_object_mut().unwrap() {
            operation["security"] = json!([{}]);
        }
    }
    paths[format!("{V1}/nodes/ws")]["get"]["security"] = json!([{ "node": [] }]);
    paths[format!("{V1}/events")] = json!({
        "get": {
            "security": [{ "owner": [] }],
            "description": "Server-sent events. Every event is named `changed` (listen with \
                `addEventListener(\"changed\", …)`: `onmessage` sees only unnamed events), and its `data` is a JSON \
                string naming what changed (`Changed`), never the data itself: refetch that. `all` means the stream \
                fell behind: refetch everything. A comment line every 20 seconds keeps the connection open. The \
                stream ends when the deck restarts: reconnect (EventSource does) and refetch.",
            "responses": {
                "200": {
                    "description": "The event stream.",
                    "content": { "text/event-stream": {
                        "schema": {
                            "type": "string",
                            "description": "`event: changed` lines, each with a `data:` line holding a `Changed`.",
                            "contentSchema": changed.clone(),
                        },
                        "example": "event: changed\ndata: \"cards\"\n\n",
                    } },
                },
                "default": error_response_json(),
            },
            "x-quietdeck-events": { "changed": changed },
        },
    });
    paths[format!("{V1}/cards")]["post"]["requestBody"] =
        json!({ "required": true, "content": { "application/json": { "schema": card_input } } });
    for (path, method, example) in examples() {
        paths[format!("{V1}{path}")][method]["requestBody"]["content"]["application/json"]["example"] = example;
    }
    // axum's plain-text rejections answer `ApiError` JSON too (`json_errors`): the `default` response covers them.
    for item in paths.as_object_mut().unwrap().values_mut() {
        for operation in item.as_object_mut().unwrap().values_mut() {
            if let Some(responses) = operation.get_mut("responses").and_then(Value::as_object_mut) {
                responses.retain(|status, r| {
                    !(status.starts_with('4') && r["content"].as_object().is_some_and(|c| c.contains_key("text/plain")))
                });
            }
        }
    }
    for doc in OPERATIONS {
        let Some(operation) = paths.get_mut(format!("{V1}{}", doc.path)).and_then(|p| p.get_mut(doc.method)) else {
            continue; // the test names it
        };
        operation["operationId"] = json!(doc.id);
        operation["tags"] = json!([doc.tag]);
        operation["summary"] = json!(doc.summary);
        if let Some(ui) = doc.ui {
            operation[UI_EXTENSION] = json!(ui);
        }
        if doc.tag == "leases" {
            operation["deprecated"] = json!(true);
        }
    }
}

/// Request examples for the main writes, which the reference shows as samples. The test parses each into its type.
pub fn examples() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        (
            "/cards",
            "post",
            json!({
                "account": "work",
                "thread_key": "18c2f0a9d4e7b311",
                "subject": "Invoice 2041 from Acme Hosting",
                "summary": "Acme's November invoice, $240, paid by card on file.",
                "section": "archive_list",
                "reason": "A receipt: nothing to do.",
                "sender": { "name": "Acme Billing", "address": "billing@acme.example" },
                "actions": [
                    { "type": "label", "account": "work", "thread_id": "18c2f0a9d4e7b311", "add": ["Receipts"] },
                    { "type": "archive", "account": "work", "thread_id": "18c2f0a9d4e7b311" }
                ]
            }),
        ),
        (
            "/documents",
            "post",
            json!({
                "slug": "work",
                "name": "Work",
                "tab": "Work",
                "kind": "work",
                "writer_node": "laptop",
                "writer_tab": "overseer"
            }),
        ),
        (
            "/cards/{id}/decide",
            "post",
            json!({ "version": 2, "kind": "adjust", "text": "Forward it to accounts instead" }),
        ),
        (
            "/decisions/approve-all",
            "post",
            json!({
                "cards": [
                    { "card_id": 41, "version": 1, "kind": "approve" },
                    { "card_id": 42, "version": 3, "kind": "adjust", "text": "Say we'll pay on Friday" }
                ],
                "timeline": [118, 119]
            }),
        ),
        ("/decisions/undo", "post", json!({ "batch": "b_7f3a" })),
    ]
}

fn error_response_json() -> Value {
    json!({
        "description": "An error, with its message.",
        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ApiError" } } },
    })
}

/// Every object's keys in order, so the document is the same whichever features serde_json was built with. A schema
/// of `true` (any JSON, from `serde_json::Value`) becomes `{}`, which means the same and which more renderers read.
fn sorted(v: Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(k, v)| {
                        let v = match (k.as_str(), v) {
                            ("schema" | "items", Value::Bool(true)) => json!({}),
                            ("properties", Value::Object(props)) => Value::Object(
                                props
                                    .into_iter()
                                    .map(|(name, p)| (name, if p == Value::Bool(true) { json!({}) } else { p }))
                                    .collect(),
                            ),
                            (_, v) => v,
                        };
                        (k, sorted(v))
                    })
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

/// Drops schemas nothing refers to: the query and path parameter structs, whose fields aide lists as parameters.
fn prune_unused_schemas(spec: &mut Value) {
    fn refs(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(map) => {
                for (k, v) in map {
                    match (k.as_str(), v) {
                        ("$ref", Value::String(r)) => {
                            if let Some(name) = r.strip_prefix("#/components/schemas/") {
                                out.push(name.to_string());
                            }
                        }
                        _ => refs(v, out),
                    }
                }
            }
            Value::Array(items) => items.iter().for_each(|v| refs(v, out)),
            _ => {}
        }
    }
    let schemas = spec["components"]["schemas"].take();
    let mut outside = spec.clone();
    outside["components"]["schemas"] = json!({});
    let mut wanted = Vec::new();
    refs(&outside, &mut wanted);
    let mut kept = serde_json::Map::new();
    while let Some(name) = wanted.pop() {
        if kept.contains_key(&name) {
            continue;
        }
        if let Some(schema) = schemas.get(&name) {
            refs(schema, &mut wanted);
            kept.insert(name, schema.clone());
        }
    }
    spec["components"]["schemas"] = sorted(Value::Object(kept));
}

fn require(operation: &mut Operation, schemes: &[&str]) {
    for scheme in schemes {
        let requirement = IndexMap::from_iter([(scheme.to_string(), Vec::new())]);
        if !operation.security.contains(&requirement) {
            operation.security.push(requirement);
        }
    }
}

impl OperationInput for Owner {
    fn operation_input(_ctx: &mut GenContext, operation: &mut Operation) {
        require(operation, &["owner"]);
    }
}

impl OperationInput for Agent {
    fn operation_input(_ctx: &mut GenContext, operation: &mut Operation) {
        require(operation, &["session"]);
    }
}

impl OperationInput for Authed {
    fn operation_input(_ctx: &mut GenContext, operation: &mut Operation) {
        require(operation, &["owner", "session", "node"]);
    }
}

impl OperationOutput for AppError {
    type Inner = ApiError;

    fn operation_response(ctx: &mut GenContext, _operation: &mut Operation) -> Option<Response> {
        Some(Response {
            description: "An error, with its message.".into(),
            content: IndexMap::from_iter([(
                "application/json".into(),
                MediaType {
                    schema: Some(SchemaObject {
                        json_schema: ctx.schema.subschema_for::<ApiError>(),
                        example: None,
                        external_docs: None,
                    }),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        })
    }

    fn inferred_responses(ctx: &mut GenContext, operation: &mut Operation) -> Vec<(Option<StatusCode>, Response)> {
        Self::operation_response(ctx, operation).map(|r| vec![(None, r)]).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    const METHODS: &[&str] = &["get", "put", "post", "delete", "patch"];

    fn operations(spec: &Value) -> BTreeSet<(String, String)> {
        let mut found = BTreeSet::new();
        for (path, item) in spec["paths"].as_object().unwrap() {
            for method in METHODS {
                if item.get(*method).is_some() {
                    found.insert((method.to_string(), path.clone()));
                }
            }
        }
        found
    }

    /// Writes `api/openapi.json`, as ts-rs writes `web/src/types`: CI fails when the committed copy differs.
    #[test]
    fn committed_spec_is_current() {
        let errors = spec_errors();
        assert!(errors.is_empty(), "generating the spec: {errors:#?}");
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../api/openapi.json");
        let text = serde_json::to_string_pretty(&spec()).unwrap() + "\n";
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
            std::fs::write(&path, text).unwrap();
        }
    }

    #[test]
    fn every_operation_is_documented_and_every_doc_has_an_operation() {
        let spec = spec();
        let found = operations(&spec);
        let documented: BTreeSet<(String, String)> =
            OPERATIONS.iter().map(|d| (d.method.to_string(), format!("{V1}{}", d.path))).collect();
        let undocumented: Vec<_> = found.difference(&documented).collect();
        let stale: Vec<_> = documented.difference(&found).collect();
        assert!(undocumented.is_empty(), "add these to OPERATIONS: {undocumented:#?}");
        assert!(stale.is_empty(), "these OPERATIONS name no route: {stale:#?}");
        let mut ids = BTreeSet::new();
        for doc in OPERATIONS {
            assert!(ids.insert(doc.id), "operationId {} twice", doc.id);
            assert!(TAGS.iter().any(|(t, _)| *t == doc.tag), "{} has an unknown tag {}", doc.id, doc.tag);
        }
    }

    /// Every route the routers register is in the spec: `api_route` documents a route, plain `route` doesn't, so
    /// the only `route` allowed is the event stream, which `describe` writes by hand.
    #[test]
    fn every_route_is_in_the_spec() {
        let spec = spec();
        let paths = spec["paths"].as_object().unwrap();
        for (file, source) in [("api.rs", include_str!("api.rs")), ("passkeys.rs", include_str!("passkeys.rs"))] {
            let router = source.split("pub fn router()").nth(1).unwrap();
            let router = &router[..router.find("\n}\n").unwrap()];
            for line in router.lines().map(str::trim) {
                if let Some(rest) = line.strip_prefix(".route(\"") {
                    assert!(rest.starts_with("/events\""), "{file}: `{line}` isn't documented: use api_route");
                }
                if let Some(rest) = line.strip_prefix(".api_route(\"") {
                    let path = &rest[..rest.find('"').unwrap()];
                    assert!(paths.contains_key(&format!("{V1}{path}")), "{file}: {path} isn't in the spec");
                }
            }
        }
    }

    /// Path templates have their parameters, and every operation names its security unless it's sign-in itself.
    #[test]
    fn operations_are_complete() {
        let spec = spec();
        let open: Vec<String> = OPEN.iter().map(|p| format!("{V1}{p}")).collect();
        for (path, item) in spec["paths"].as_object().unwrap() {
            let wanted: Vec<&str> =
                path.split('/').filter_map(|s| s.strip_prefix('{').and_then(|s| s.strip_suffix('}'))).collect();
            for method in METHODS {
                let Some(op) = item.get(*method) else { continue };
                let params: Vec<&str> = op["parameters"]
                    .as_array()
                    .map(|ps| ps.iter().filter(|p| p["in"] == "path").filter_map(|p| p["name"].as_str()).collect())
                    .unwrap_or_default();
                assert_eq!(params, wanted, "{method} {path}: path parameters");
                let security = op["security"].as_array().cloned().unwrap_or_default();
                let none = security.is_empty() || security.iter().any(|s| s == &json!({}));
                assert_eq!(none, open.contains(path), "{method} {path}: security {security:?}");
            }
        }
    }

    /// `/api/v1` and the unversioned `/api` both reach the routes, and the deck serves the spec.
    #[tokio::test]
    async fn both_prefixes_route_and_the_spec_is_served() {
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        let state = crate::state::AppState::for_tests().await;
        let (router, spec) = routes(state.clone());
        let app = router.with_state(state);
        for uri in ["/api/v1/info", "/api/info"] {
            let res = app.clone().oneshot(axum::http::Request::get(uri).body(axum::body::Body::empty()).unwrap()).await;
            // Unauthenticated, but routed: 401, not the 404 an unknown path gets.
            assert_eq!(res.unwrap().status(), axum::http::StatusCode::UNAUTHORIZED, "{uri}");
        }
        let res = app
            .oneshot(axum::http::Request::get("/api/v1/openapi.json").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), axum::http::StatusCode::OK);
        let body: Value = serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(&body, spec.as_ref());
        assert_eq!(body["info"]["version"], API_VERSION);
    }

    /// With the report off, its routes answer 404 at both prefixes, and the core forgets a report kept from before.
    #[tokio::test]
    async fn the_retired_lease_routes_answer_gone() {
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        let state = crate::state::AppState::for_tests().await;
        let code = crate::store::nodes::create_enrolment_code(&state.db).await.unwrap();
        let node = crate::store::nodes::enrol(&state.db, &code.code, "laptop").await.unwrap();
        let (router, _) = routes(state.clone());
        let app = router.with_state(state);
        let get = |uri: &str| axum::http::Request::get(uri).body(axum::body::Body::empty()).unwrap();
        assert_eq!(app.clone().oneshot(get("/api/v1/leases")).await.unwrap().status(), 401, "still signed in first");
        for uri in ["/api/v1/leases", "/api/leases/3"] {
            let req = axum::http::Request::get(uri)
                .header("authorization", format!("Bearer {}", node.token))
                .body(axum::body::Body::empty())
                .unwrap();
            let res = app.clone().oneshot(req).await.unwrap();
            assert_eq!(res.status(), 410, "{uri}");
            let body: Value = serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
            assert!(body["error"].as_str().unwrap().contains("fleetlayer lease acquire gmail-filter"), "{body}");
        }
    }

    #[tokio::test]
    async fn the_report_can_be_off() {
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        let state = crate::state::AppState::for_tests().await;
        let session =
            quietdeck_types::SessionRef { node: "laptop".into(), tab: "triage".into(), session_id: Some("s1".into()) };
        let report =
            quietdeck_types::report::ReportSet { markdown: "# Inbox".into(), base: None, new: true, change: None };
        crate::reports::set(&state, &session, &crate::reports::inbox(&state.db).await.unwrap(), report).await.unwrap();
        let (router, _) = routes(state.clone());
        let app = router.with_state(state.clone());
        let get = |uri: &str| axum::http::Request::get(uri).body(axum::body::Body::empty()).unwrap();
        // On: routed to the handler, which wants a sign-in.
        assert_eq!(app.clone().oneshot(get("/api/v1/report")).await.unwrap().status(), 401);

        let off = quietdeck_types::api::Settings { report: false, ..Default::default() };
        crate::ops::save_settings(&state, off).await.unwrap();
        for uri in ["/api/v1/report", "/api/report", "/api/v1/report/snapshots/1"] {
            let res = app.clone().oneshot(get(uri)).await.unwrap();
            assert_eq!(res.status(), 404, "{uri}");
            let body: Value = serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
            assert_eq!(body["error"], crate::reports::OFF);
        }
        // Not a report route: still routed.
        assert_eq!(app.oneshot(get("/api/v1/inbox")).await.unwrap().status(), 401);
        assert!(crate::reports::current(&state.db, quietdeck_types::document::INBOX_ID).await.unwrap().is_none());
        let inbox = crate::ops::inbox(&state, None).await.unwrap();
        assert!(inbox.report.is_none() && inbox.finished_cards.is_empty());
    }

    /// The examples are requests the deck accepts: each parses into its type and passes its checks.
    #[test]
    fn examples_are_valid_requests() {
        use quietdeck_types::decision::{ApproveAll, Decide, UndoBatch};
        for (path, _, example) in examples() {
            match path {
                "/cards" => {
                    let card: CardInput = serde_json::from_value(example).unwrap();
                    card.check().unwrap();
                }
                "/cards/{id}/decide" => drop(serde_json::from_value::<Decide>(example).unwrap()),
                "/decisions/approve-all" => {
                    let all: ApproveAll = serde_json::from_value(example).unwrap();
                    garde::Validate::validate(&all).unwrap();
                }
                "/decisions/undo" => drop(serde_json::from_value::<UndoBatch>(example).unwrap()),
                "/documents" => {
                    let add: quietdeck_types::document::DocumentAdd = serde_json::from_value(example).unwrap();
                    garde::Validate::validate(&add).unwrap();
                }
                other => panic!("no check for the {other} example"),
            }
        }
    }

    /// A body the deck can't read answers `ApiError` JSON, as every other error does, and the spec says so.
    #[tokio::test]
    async fn unreadable_requests_answer_json() {
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        let state = crate::state::AppState::for_tests().await;
        let (router, spec) = routes(state.clone());
        let app = router.with_state(state);
        let req = axum::http::Request::post("/api/v1/nodes/enroll")
            .header("content-type", "application/json")
            .body(axum::body::Body::from("{not json"))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert!(res.status().is_client_error());
        let body: Value = serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert!(body["error"].as_str().is_some_and(|e| !e.is_empty()), "{body}");
        let text = serde_json::to_string(spec.as_ref()).unwrap();
        assert!(!text.contains("text/plain"), "the spec still lists a plain-text response");
    }
}
