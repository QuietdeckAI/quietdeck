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
use axum::Json;
use axum::Router;
use axum::routing::get;
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
pub const API_VERSION: &str = "1.0.0";

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
    ("deck", "The deck itself: its version, settings and the live event stream."),
    ("auth", "Passkey sign-in for the owner's browsers."),
    ("nodes", "Machines enrolled with the deck, and the WebSocket their runners keep open."),
    ("cards", "Cards: what an agent proposes, one per thread, each with its versions and actions."),
    ("decisions", "The owner's decisions on cards, the undo window, Stop, and the runner taking approved work."),
    ("actions", "The runner carrying approved actions out and reporting the results."),
    ("inbox", "The Inbox and Later views, and the mail tabs' lists."),
    ("timeline", "The timeline of automatic actions, History, comments, pings and thread watches."),
    ("sessions", "Agent sessions across every node: listing, transcripts, starting, resuming and closing them."),
    ("messages", "Messages between sessions, chat messages the deck keeps, and handoffs between sessions."),
    ("rules", "Rules: what may be done without asking, and reading rules for mail held while the owner reads."),
    ("leases", "Leases: Gmail filters and heavy-build slots that agents ask for and hand back."),
    ("hooks", "Webhook endpoints that wake agents."),
    ("push", "Web push devices."),
    (
        "report",
        "The default web app's report: the triage agent's write-up of the Inbox, its answers, chat and snapshots. \
         A frontend of its own can ignore these (they carry `x-quietdeck-ui: report`).",
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
    op("get", "/leases", "leases_view", "leases", "What's held, queued, waiting or overdue, and the approved filters"),
    op("post", "/leases/propose", "lease_propose", "leases", "Propose a lease (a Gmail filter) for the owner to approve"),
    op("post", "/leases/acquire", "lease_acquire", "leases", "Acquire a lease (a heavy-build slot), optionally waiting"),
    op("get", "/leases/{id}", "lease_get", "leases", "One lease"),
    op("post", "/leases/{id}/register", "lease_register", "leases", "Register what a lease made (the filter's ID)"),
    op("post", "/leases/{id}/release", "lease_release", "leases", "Release a lease"),
    op("post", "/leases/{id}/cleaned", "lease_cleaned", "leases", "Confirm a released lease's filter is gone"),
    op("post", "/lease-patterns/{id}/revoke", "lease_pattern_revoke", "leases", "Revoke an approved lease pattern"),
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
    ui("presence", "post", "/presence", "presence_beat", "presence", "The Inbox page's heartbeat while the owner reads"),
    ui("presence", "post", "/presence/release", "presence_release", "presence", "The owner stopped reading"),
];

/// The API's routes, at `/api/v1` and at `/api`, with `/api/v1/openapi.json`; and the spec they serve.
pub fn routes() -> (Router<AppState>, Arc<Value>) {
    let (v1, legacy, spec) = build(None);
    let spec = Arc::new(spec);
    let served = spec.clone();
    let router = v1
        .merge(legacy)
        .route(&format!("{V1}/openapi.json"), get(move || std::future::ready(Json(served.as_ref().clone()))));
    (router, spec)
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
    let mut api = OpenApi::default();
    let v1 = aide::axum::ApiRouter::new().nest(V1, core).finish_api(&mut api);
    let mut spec = serde_json::to_value(&api).expect("the spec serialises");
    describe(&mut spec, card_input.to_value());
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
fn describe(spec: &mut Value, card_input: Value) {
    spec["info"] = json!({
        "title": "Quietdeck API",
        "version": API_VERSION,
        "summary": "Quietdeck's core: cards, decisions, actions, rules, leases, sessions and the timeline.",
        "description": "Build your own frontend for a Quietdeck deck. Every route is under `/api/v1`. Within v1, \
            changes are additive only: new routes, new optional fields in requests, new fields in responses (ignore \
            the ones you don't know) and new enum values (handle the ones you don't know). Anything that would break \
            a client goes in `/api/v2`, and v1 keeps working alongside it. Operations tagged `report` or `presence` \
            (with `x-quietdeck-ui`) belong to the default web app's own screens: a frontend of its own can ignore \
            them. Errors answer `ApiError` with a 4xx or 5xx status.",
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
            "responses": {
                "200": {
                    "description": "A stream of events, each naming what changed (`Changed`), as `data:` lines.",
                    "content": { "text/event-stream": { "schema": { "type": "string" } } },
                },
                "default": error_response_json(),
            },
        },
    });
    paths[format!("{V1}/cards")]["post"]["requestBody"] =
        json!({ "required": true, "content": { "application/json": { "schema": card_input } } });
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
    }
}

fn error_response_json() -> Value {
    json!({
        "description": "An error, with its message.",
        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ApiError" } } },
    })
}

/// Every object's keys in order, so the document is the same whichever features serde_json was built with. A schema
/// of `true` (any JSON, from `serde_json::Value`) becomes `{}`, which means the same and which more tools read.
fn sorted(v: Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(k, v)| {
                        let v = if k == "schema" && v == Value::Bool(true) { json!({}) } else { v };
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

        let (router, spec) = routes();
        let state = crate::state::AppState::for_tests().await;
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
}
