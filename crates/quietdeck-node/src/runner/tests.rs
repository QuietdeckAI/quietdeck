//! The runner contract test (section 8): a fake deck, a fake agent, and a fake Executor that is a real MCP server
//! (rmcp over streamable HTTP) recording every call.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use quietdeck_types::ActionType;
use quietdeck_types::card::{ActionRecord, ActionReport, ActionResult, ActionStatus, Released};
use quietdeck_types::decision::ReportReceipt;
use quietdeck_types::directory::{SessionStart, SessionStarted};
use quietdeck_types::node::{Delivered, Handoff};
use quietdeck_types::reading::{HoldVerdict, ItemFacts};
use quietdeck_types::relay::DeliveryKind;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ServerCapabilities, ServerConfig};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, ServerHandler, tool, tool_handler, tool_router};
use serde_json::{Value, json};

use super::config::RunnerConfig;
use super::*;

// --- The fake Executor ----------------------------------------------------------------------------------------------

type Script = Arc<dyn Fn(&str, &Value) -> Result<Value, String> + Send + Sync>;

#[derive(Clone)]
struct FakeExecutor {
    calls: Arc<StdMutex<Vec<(String, Value)>>>,
    script: Script,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct CallArgs {
    calls: Value,
}

#[tool_router]
impl FakeExecutor {
    #[tool(description = "Run one app tool by path.")]
    async fn call(&self, Parameters(args): Parameters<CallArgs>) -> Result<CallToolResult, ErrorData> {
        let path = args.calls["path"].as_str().unwrap_or_default().to_string();
        let input = args.calls["input"].clone();
        self.calls.lock().unwrap().push((path.clone(), input.clone()));
        let item = match (self.script)(&path, &input) {
            Ok(value) => json!({"path": path, "ok": true, "value": value}),
            Err(message) => json!({"path": path, "ok": false, "error": {"message": message}}),
        };
        // As Executor answers one call (not an array of them): `value` is the call's own result.
        Ok(CallToolResult::structured(json!({"status": "completed", "execution": {"ok": true, "value": item}})))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for FakeExecutor {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
}

/// Start a fake Executor; returns its URL and its call log.
async fn executor(script: Script) -> (String, Arc<StdMutex<Vec<(String, Value)>>>) {
    let calls: Arc<StdMutex<Vec<(String, Value)>>> = Arc::default();
    let fake = FakeExecutor { calls: calls.clone(), script, tool_router: FakeExecutor::tool_router() };
    let config = StreamableHttpServerConfig::default().disable_allowed_hosts();
    let service: StreamableHttpService<FakeExecutor, LocalSessionManager> =
        StreamableHttpService::new(move || Ok(fake.clone()), Default::default(), config);
    let app = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/mcp"), calls)
}

const THREAD: &str = "tools.google.api.gmail.users.gmailUsersThreadsGet";
const SEND_AS: &str = "tools.google.api.gmail.users.gmailUsersSettingsSendAsList";
const PROFILE: &str = "tools.google.api.gmail.users.gmailUsersGetProfile";
const HISTORY: &str = "tools.google.api.gmail.users.gmailUsersHistoryList";

/// Cards are prepared at 09:12 UTC on 23 November (1795425120000 ms). Threads: `t_old` (newest message m2, a newer one
/// than the card's m1, arrived after it), `t_undated` (Gmail gave no dates), and anything else (newest m1, in the
/// inbox, from before the card).
const BEFORE_CARD: i64 = 1795421520000;
const AFTER_CARD: i64 = 1795425720000;
fn gmail(path: &str, input: &Value) -> Result<Value, String> {
    match path {
        THREAD => {
            let id = input.pointer("/input/path/id").and_then(Value::as_str).unwrap_or_default();
            match id {
                "t_old" => Ok(json!({"id": id, "messages": [
                    {"id": "m1", "labelIds": ["INBOX"], "internalDate": BEFORE_CARD.to_string()},
                    {"id": "m2", "labelIds": ["INBOX"], "internalDate": AFTER_CARD.to_string()}]})),
                "t_undated" => Ok(json!({"id": id, "messages": [{"id": "u1", "labelIds": ["INBOX"]}]})),
                _ => Ok(json!({"id": id, "messages": [
                    {"id": "m1", "labelIds": ["INBOX", "UNREAD"], "internalDate": BEFORE_CARD},
                    {"id": "d1", "labelIds": ["DRAFT"]}]})),
            }
        }
        SEND_AS => Ok(json!({"sendAs": [{"sendAsEmail": "me@example.com"}, {"sendAsEmail": "Alias@Example.com"}]})),
        "tools.google.gmail.modify" if input["threadIds"] == json!(["t_fail"]) => Err("Executor error 401".into()),
        _ => Ok(json!({"ok": true})),
    }
}

fn config(url: &str, extra: &str) -> RunnerConfig {
    let yaml = format!(
        r#"
enabled: true
agent: {{ tab: email triage }}
executor: {{ url: "{url}", token_file: /dev/null }}
accounts:
  personal: {{ address: me@example.com, vars: {{ executor_account: acc_1 }} }}
thread: {{ path: {THREAD}, input: {{ accountId: "${{account.executor_account}}", input: {{ path: {{ userId: me, id: "${{action.thread_id}}" }}, query: {{ format: minimal }} }} }} }}
send_as: {{ path: {SEND_AS}, input: {{ accountId: "${{account.executor_account}}", input: {{ path: {{ userId: me }} }} }} }}
mail_watch:
  every: 30s
  profile: {{ path: {PROFILE}, input: {{ accountId: "${{account.executor_account}}", input: {{ path: {{ userId: me }} }} }} }}
  history: {{ path: {HISTORY}, input: {{ accountId: "${{account.executor_account}}", input: {{ path: {{ userId: me }}, query: {{ startHistoryId: "${{history_id}}", pageToken: "${{page_token}}", historyTypes: [messageAdded] }} }} }} }}
tools:
  archive: {{ path: tools.google.gmail.modify, input: {{ account: "${{account.address}}", threadIds: ["${{action.thread_id}}"], archive: true }} }}
  draft: {{ path: tools.google.gmail.draft, input: {{ account: "${{account.address}}", from: "${{action.from}}", to: "${{action.to}}", subject: "${{action.subject}}", body: "${{action.body}}", replyToMessageId: "${{action.reply_to_message_id}}" }} }}
  story.create: {{ path: tools.shortcut.stories.create, input: {{ name: "${{action.title}}", workflow_state_id: "${{board.workflow_state_id}}" }} }}
boards:
  engineering: {{ workflow_state_id: 500000008 }}
{extra}"#
    );
    let c: RunnerConfig = yaml_serde::from_str(&yaml).unwrap();
    c.check().unwrap();
    c
}

// --- The fake deck and agent ----------------------------------------------------------------------------------------

#[derive(Default)]
struct FakeDeck {
    released: StdMutex<Vec<Released>>,
    sorted: StdMutex<Vec<(Vec<i64>, Vec<Handoff>)>>,
    /// What `action start` answers, by action ID: the record as the deck has it.
    records: StdMutex<HashMap<i64, ActionRecord>>,
    stopped: StdMutex<HashSet<i64>>,
    reports: StdMutex<Vec<ActionReport>>,
    relayed: StdMutex<Vec<(String, String)>>,
    /// While the person reads: subjects the deck lets through, with why. Everything else is held.
    lets: StdMutex<HashMap<String, String>>,
    held: StdMutex<Vec<(Option<String>, u64)>>,
    /// Threads the runner said left the inbox; the deck "resolves" a card for each.
    left: StdMutex<Vec<quietdeck_types::mail::LeftThread>>,
}

#[async_trait]
impl DeckApi for FakeDeck {
    async fn hold_check(&self, items: Vec<ItemFacts>) -> Result<Vec<HoldVerdict>, String> {
        let lets = self.lets.lock().unwrap();
        Ok(items
            .iter()
            .map(|i| match i.thread_key.as_deref().and_then(|t| lets.get(t)) {
                Some(reason) => HoldVerdict::Let { reason: reason.clone() },
                None => HoldVerdict::Hold,
            })
            .collect())
    }
    async fn held(&self, summary: Option<String>, count: u64) -> Result<(), String> {
        self.held.lock().unwrap().push((summary, count));
        Ok(())
    }
    async fn threads_left(
        &self,
        left: Vec<quietdeck_types::mail::LeftThread>,
        _back: Vec<quietdeck_types::mail::LeftThread>,
    ) -> Result<Vec<quietdeck_types::mail::LeftCard>, String> {
        self.left.lock().unwrap().extend(left.clone());
        Ok(left
            .iter()
            .enumerate()
            .map(|(i, t)| quietdeck_types::mail::LeftCard {
                id: 10 + i as i64,
                subject: format!("About {}", t.thread_id),
            })
            .collect())
    }
    async fn take(&self) -> Result<Vec<Released>, String> {
        Ok(self.released.lock().unwrap().clone())
    }
    async fn sort(&self, claims: Vec<i64>, handoffs: Vec<Handoff>) -> Result<Vec<i64>, String> {
        self.sorted.lock().unwrap().push((claims.clone(), handoffs));
        Ok(claims)
    }
    async fn start(&self, id: i64) -> Result<ActionRecord, String> {
        if self.stopped.lock().unwrap().contains(&id) {
            return Err(format!("action {id} was stopped: don't run it"));
        }
        self.records.lock().unwrap().get(&id).cloned().ok_or_else(|| format!("no action {id}"))
    }
    async fn report(&self, reports: Vec<ActionReport>) -> Result<Vec<ReportReceipt>, String> {
        let receipts = reports.iter().map(|r| ReportReceipt { action_id: r.action_id, error: None }).collect();
        self.reports.lock().unwrap().extend(reports);
        Ok(receipts)
    }
    async fn relay(&self, to: String, _: DeliveryKind, body: String) -> Result<Delivered, String> {
        self.relayed.lock().unwrap().push((to, body));
        Ok(Delivered::Orchestrator)
    }
    async fn start_session(&self, _: SessionStart) -> Result<SessionStarted, String> {
        Err("not in this test".into())
    }
    async fn resume_session(&self, _: String, _: String, _: String, _: String) -> Result<SessionStarted, String> {
        Err("not in this test".into())
    }
}

#[derive(Default)]
struct FakeAgent {
    told: StdMutex<Vec<(DeliveryKind, String)>>,
    offline: StdMutex<bool>,
}

#[async_trait]
impl AgentInbox for FakeAgent {
    async fn tell(&self, kind: DeliveryKind, body: String) -> Result<(), String> {
        if *self.offline.lock().unwrap() {
            return Err("email triage isn't running".into());
        }
        self.told.lock().unwrap().push((kind, body));
        Ok(())
    }
}

/// A released decision on card `id` with these actions, prepared on message m1 of its thread.
fn released(id: i64, actions: Value, extra: Value) -> Released {
    let actions_arr = actions.as_array().unwrap().clone();
    let records: Vec<Value> = actions_arr
        .iter()
        .enumerate()
        .map(|(i, a)| {
            json!({"id": id * 10 + i as i64, "card_id": id, "card_version": 1, "seq": i, "action": a,
                "status": "released", "run_by": null, "outcome": null, "error": null, "started_at": null,
                "finished_at": null})
        })
        .collect();
    let mut card = json!({"id": id, "version": 1,
        "posted_by": {"node": "server", "tab": "email triage", "session_id": null},
        "state": "approved", "handoff_reason": null, "grant_id": null, "prepared_at": "2026-11-23T09:12:00Z",
        "rechecked_at": null, "recheck_note": null, "updated_at": "2026-11-23T09:12:00Z",
        "account": "personal", "thread_key": format!("t{id}"), "last_message_ref": "m1",
        "subject": format!("Subject {id}"), "summary": "s", "section": "actions", "reason": "r",
        "actions": actions, "action_records": records});
    let mut decision = json!({"id": id + 100, "card_id": id, "card_version": 1, "kind": "approve", "fields": null,
        "text": null, "released_at": "2026-11-23T09:20:00Z", "claimed_by": null, "picked_up_at": null,
        "at": "2026-11-23T09:19:00Z"});
    for (k, v) in extra.as_object().unwrap() {
        if k == "text" || k == "kind" {
            decision[k] = v.clone();
        } else {
            card[k] = v.clone();
        }
    }
    let records = card["action_records"].clone();
    serde_json::from_value(json!({"decision": decision, "card": card, "actions": records, "handoff_reason": null}))
        .unwrap()
}

fn archive(thread: &str) -> Value {
    json!({"type": "archive", "account": "personal", "thread_id": thread})
}

fn draft(thread: &str, from: &str) -> Value {
    json!({"type": "draft", "account": "personal", "thread_id": thread, "reply_to_message_id": "m1",
        "from": from, "to": ["you@example.com"], "subject": "Re: hi", "body": "Thanks!"})
}

struct Harness {
    runner: Runner,
    deck: Arc<FakeDeck>,
    agent: Arc<FakeAgent>,
    calls: Arc<StdMutex<Vec<(String, Value)>>>,
    _dir: tempfile::TempDir,
}

async fn harness(cards: Vec<Released>, extra_config: &str) -> Harness {
    let (url, calls) = executor(Arc::new(gmail)).await;
    let cfg = config(&url, extra_config);
    let deck = Arc::new(FakeDeck::default());
    for r in &cards {
        for a in &r.actions {
            deck.records.lock().unwrap().insert(a.id, a.clone());
        }
    }
    *deck.released.lock().unwrap() = cards;
    let agent = Arc::new(FakeAgent::default());
    let dir = tempfile::tempdir().unwrap();
    let tools = ConfiguredTools::new(Some((cfg.executor.clone().unwrap(), "test-token".into())));
    let runner = Runner::new(cfg, deck.clone(), Arc::new(tools), agent.clone(), dir.path().to_path_buf());
    Harness { runner, deck, agent, calls, _dir: dir }
}

impl Harness {
    fn paths(&self) -> Vec<String> {
        self.calls.lock().unwrap().iter().map(|(p, _)| p.clone()).collect()
    }
    fn calls_to(&self, path: &str) -> Vec<Value> {
        self.calls.lock().unwrap().iter().filter(|(p, _)| p == path).map(|(_, i)| i.clone()).collect()
    }
    fn handoffs(&self) -> Vec<Handoff> {
        self.deck.sorted.lock().unwrap().iter().flat_map(|(_, h)| h.clone()).collect()
    }
    fn reason(&self, decision: i64) -> String {
        self.handoffs().into_iter().find(|h| h.decision_id == decision).map(|h| h.reason).unwrap_or_default()
    }
    fn told(&self, kind: fn(&DeliveryKind) -> bool) -> Vec<String> {
        self.agent.told.lock().unwrap().iter().filter(|(k, _)| kind(k)).map(|(_, b)| b.clone()).collect()
    }
}

fn is_handoff(k: &DeliveryKind) -> bool {
    matches!(k, DeliveryKind::Handoff)
}
fn is_report(k: &DeliveryKind) -> bool {
    matches!(k, DeliveryKind::RunnerReport)
}
fn is_wakeup(k: &DeliveryKind) -> bool {
    matches!(k, DeliveryKind::Wakeup)
}

// --- The contract ---------------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_mixed_batch_is_claimed_and_handed_over_with_reasons() {
    let story = json!({"type": "story", "op": "create", "board": "engineering", "story_type": "bug",
        "title": "Seal error", "description": "d"});
    let reply = json!({"type": "reply", "account": "personal", "thread_id": "t3", "reply_to_message_id": "m1",
        "from": "me@example.com", "to": ["you@example.com"], "subject": "Re: hi", "body": "No thanks"});
    let watch = json!({"type": "watch", "account": "personal", "thread_id": "t7", "on": true});
    let h = harness(
        vec![
            released(1, json!([archive("t1")]), json!({})),
            released(2, json!([draft("t2", "alias@example.com")]), json!({})),
            released(3, json!([reply]), json!({})),
            released(4, json!([archive("t4")]), json!({"kind": "adjust", "text": "forward it to api instead"})),
            released(5, json!([archive("t5")]), json!({"needs_agent": true})),
            released(6, json!([archive("t_old")]), json!({"thread_key": "t_old"})),
            released(7, json!([watch]), json!({})),
            released(8, json!([story]), json!({})),
        ],
        "",
    )
    .await;
    let out = h.runner.process().await.unwrap();
    assert_eq!(out.claimed, vec![101, 102, 108]);
    assert_eq!(out.done, 3);
    assert_eq!(h.reason(103), "reply isn't runner-eligible");
    assert_eq!(h.reason(104), "adjustment text");
    assert_eq!(h.reason(105), "the agent asked to carry it out (--needs-agent)");
    let prepared: jiff::Timestamp = "2026-11-23T09:12:00Z".parse().unwrap();
    let local = prepared.to_zoned_datetime(jiff::tz::TimeZone::system()).strftime("%H:%M").to_string();
    assert_eq!(h.reason(106), format!("newer message in thread since {local}"), "in this machine's time");
    assert_eq!(h.reason(107), "no handler for watch");
    let counted: Vec<Option<ActionType>> = h.handoffs().iter().map(|h| h.no_handler).collect();
    assert_eq!(counted.iter().flatten().collect::<Vec<_>>(), vec![&ActionType::Watch], "counted by type only");

    // Each claimed action went out once, through its configured tool and input.
    let modify = h.calls_to("tools.google.gmail.modify");
    assert_eq!(modify, vec![json!({"account": "me@example.com", "threadIds": ["t1"], "archive": true})]);
    let drafts = h.calls_to("tools.google.gmail.draft");
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0]["from"], "alias@example.com");
    assert_eq!(drafts[0]["replyToMessageId"], "m1");
    assert_eq!(
        h.calls_to("tools.shortcut.stories.create"),
        vec![json!({"name": "Seal error", "workflow_state_id": 500000008})],
        "the board's IDs come from the runner's config"
    );
    // Nothing handed over reached a tool that changes anything.
    for p in h.paths() {
        assert!(
            [THREAD, SEND_AS, "tools.google.gmail.modify", "tools.google.gmail.draft", "tools.shortcut.stories.create"]
                .contains(&p.as_str()),
            "unexpected call to {p}"
        );
    }
    assert_eq!(h.deck.reports.lock().unwrap().len(), 3);
    let handed = h.told(is_handoff);
    assert_eq!(handed.len(), 1, "one message for the whole batch");
    assert!(handed[0].starts_with("5 cards handed to you:"), "{}", handed[0]);
    assert!(h.told(is_report).is_empty());
}

#[tokio::test]
async fn an_all_simple_batch_never_wakes_the_agent() {
    let h = harness(vec![released(1, json!([archive("t1")]), json!({}))], "").await;
    let out = h.runner.process().await.unwrap();
    assert_eq!(out.done, 1);
    assert!(h.agent.told.lock().unwrap().is_empty());
}

#[tokio::test]
async fn forged_unreleased_and_unknown_items_never_reach_executor() {
    let mut forged = released(1, json!([archive("t1")]), json!({}));
    let mut unreleased = released(2, json!([archive("t2")]), json!({}));
    unreleased.actions[0].status = ActionStatus::Pending;
    let other_account = released(3, json!([{"type": "archive", "account": "work", "thread_id": "t3"}]), json!({}));
    let bad_schema = released(4, json!([{"type": "archive", "account": "personal", "thread_id": ""}]), json!({}));
    forged.card.input.subject = "Forged".into();
    let h = harness(vec![forged, unreleased, other_account, bad_schema], "").await;
    // The deck now answers `start` with a different payload than the one released.
    h.deck.records.lock().unwrap().get_mut(&10).unwrap().action =
        serde_json::from_value(archive("t_someone_elses")).unwrap();
    let out = h.runner.process().await.unwrap();
    assert_eq!(out.failed, 1);
    assert!(h.calls_to("tools.google.gmail.modify").is_empty(), "nothing was archived");
    assert_eq!(h.reason(102), "nothing released to run");
    assert_eq!(h.reason(103), "account work isn't in the runner's config");
    assert!(h.reason(104).contains("failed the runner's schema check"));
    let reports = h.deck.reports.lock().unwrap().clone();
    assert!(matches!(&reports[0].result, ActionResult::Failed { error } if error.contains("payload changed")));
    let report = h.told(is_report);
    assert_eq!(report.len(), 1);
    assert!(report[0].contains("Archive of 'Forged' (Personal)"), "{}", report[0]);
}

#[tokio::test]
async fn a_card_for_several_emails_checks_each_by_when_its_newest_message_arrived() {
    let when = {
        let prepared: jiff::Timestamp = "2026-11-23T09:12:00Z".parse().unwrap();
        prepared.to_zoned_datetime(jiff::tz::TimeZone::system()).strftime("%H:%M").to_string()
    };
    let h = harness(
        vec![
            // Its own thread (newest m1, as the card says) and two more from before it: carried out.
            released(1, json!([archive("t1"), archive("t_a"), archive("t_b")]), json!({})),
            // Another thread got a message after the card was prepared: the agent looks again.
            released(2, json!([archive("t2"), archive("t_old")]), json!({})),
            // Gmail gave no dates and the card doesn't list the message: can't tell, so the agent checks.
            released(3, json!([archive("t3"), archive("t_undated")]), json!({})),
            // ...unless the card lists that message in its refs.
            released(4, json!([archive("t4"), archive("t_undated")]), json!({"refs": ["u1"]})),
        ],
        "",
    )
    .await;
    let out = h.runner.process().await.unwrap();
    assert_eq!(out.claimed, vec![101, 104]);
    assert_eq!(h.reason(102), format!("newer message in thread t_old since {when}"));
    assert_eq!(
        h.reason(103),
        "couldn't tell whether thread t_undated has a newer message: the thread tool returns no internalDate"
    );
}

#[tokio::test]
async fn an_executor_error_fails_the_action_and_sends_one_report() {
    let h = harness(
        vec![released(1, json!([archive("t_fail")]), json!({})), released(2, json!([archive("t2")]), json!({}))],
        "",
    )
    .await;
    let out = h.runner.process().await.unwrap();
    assert_eq!((out.done, out.failed), (1, 1));
    let report = h.told(is_report);
    assert_eq!(report.len(), 1);
    assert_eq!(
        report[0],
        "Runner: 1 of 2 failed. Archive of 'Subject 1' (Personal): Executor error 401. Card c_1 is waiting with Retry."
    );
    let failed =
        h.deck.reports.lock().unwrap().iter().filter(|r| matches!(r.result, ActionResult::Failed { .. })).count();
    assert_eq!(failed, 1);
}

#[tokio::test]
async fn stop_mid_batch_leaves_the_rest_unrun() {
    let h = harness(
        vec![released(1, json!([archive("t1")]), json!({})), released(2, json!([archive("t2")]), json!({}))],
        "",
    )
    .await;
    h.deck.stopped.lock().unwrap().insert(20);
    let out = h.runner.process().await.unwrap();
    assert_eq!((out.done, out.stopped), (1, 1));
    assert_eq!(h.calls_to("tools.google.gmail.modify").len(), 1);
}

#[tokio::test]
async fn eligibility_is_config() {
    let h = harness(vec![released(1, json!([archive("t1")]), json!({}))], "eligible: [draft, story]\n").await;
    h.runner.process().await.unwrap();
    assert_eq!(h.reason(101), "archive isn't runner-eligible");
    assert!(h.calls_to("tools.google.gmail.modify").is_empty());
}

#[tokio::test]
async fn a_draft_from_an_address_the_account_cant_send_as_is_refused_before_executor() {
    let h = harness(vec![released(1, json!([draft("t1", "ceo@example.com")]), json!({}))], "").await;
    h.runner.process().await.unwrap();
    assert!(h.reason(101).contains("can't send as ceo@example.com"), "{}", h.reason(101));
    assert!(h.calls_to("tools.google.gmail.draft").is_empty());
    let check = h.runner.send_as_check("personal", "ALIAS@example.com").await.unwrap();
    assert!(check.allowed, "addresses compare without case");
}

#[tokio::test]
async fn forwards_go_through_the_relay() {
    let fwd = json!({"type": "forward", "to": {"node": "laptop", "tab": "api", "session_id": null},
        "text": "Customer reports a seal error", "source": {"account": "personal", "thread_id": "t1"}});
    let h = harness(vec![released(1, json!([fwd]), json!({}))], "").await;
    let out = h.runner.process().await.unwrap();
    assert_eq!(out.done, 1);
    assert_eq!(h.deck.relayed.lock().unwrap()[0].0, "api@laptop");
    assert!(h.paths().is_empty(), "a forward needs no Executor call");
}

// --- The mail watch -------------------------------------------------------------------------------------------------

/// Messages added to a mailbox: (history ID, message ID, label).
type Added = Arc<StdMutex<Vec<(u64, &'static str, &'static str)>>>;

/// A history feed: the profile says `head`, and history since an ID lists the messages added after it.
fn feed(head: Arc<StdMutex<u64>>, added: Added) -> Script {
    Arc::new(move |path: &str, input: &Value| match path {
        PROFILE => Ok(json!({"emailAddress": "me@example.com", "historyId": head.lock().unwrap().to_string()})),
        HISTORY => {
            let since: u64 =
                input.pointer("/input/query/startHistoryId").and_then(Value::as_str).unwrap().parse().unwrap();
            let history: Vec<Value> = added
                .lock()
                .unwrap()
                .iter()
                .filter(|(h, _, _)| *h > since)
                .map(|(_, id, label)| json!({"messagesAdded": [{"message": {"id": id, "threadId": id, "labelIds": [label]}}]}))
                .collect();
            Ok(json!({"history": history, "historyId": head.lock().unwrap().to_string()}))
        }
        other => panic!("the mail watch asked Executor for {other}: it may read history and the profile only"),
    })
}

#[tokio::test]
async fn the_mail_watch_wakes_for_each_new_batch_and_resumes_after_a_restart() {
    let head = Arc::new(StdMutex::new(100u64));
    let added: Added = Arc::default();
    let (url, calls) = executor(feed(head.clone(), added.clone())).await;
    let dir = tempfile::tempdir().unwrap();
    let agent = Arc::new(FakeAgent::default());
    let make = |agent: Arc<FakeAgent>| {
        let cfg = config(&url, "");
        let tools = ConfiguredTools::new(Some((cfg.executor.clone().unwrap(), "t".into())));
        Runner::new(cfg, Arc::new(FakeDeck::default()), Arc::new(tools), agent, dir.path().to_path_buf())
    };
    let runner = make(agent.clone());

    runner.check_mail().await; // first run: starts from now
    assert!(agent.told.lock().unwrap().is_empty());

    added.lock().unwrap().extend([(101, "a", "INBOX"), (102, "b", "SENT"), (103, "spam", "SPAM")]);
    *head.lock().unwrap() = 103;
    runner.check_mail().await;
    let wakes = agent.told.lock().unwrap().iter().filter(|(k, _)| is_wakeup(k)).count();
    assert_eq!(wakes, 1);
    assert!(agent.told.lock().unwrap()[0].1.starts_with("New mail: 2 Personal."));

    added.lock().unwrap().push((104, "c", "INBOX"));
    *head.lock().unwrap() = 104;
    runner.check_mail().await;
    // No waiting for `round done`: new mail wakes the agent again at once (a busy agent reads it on its next turn).
    assert_eq!(agent.told.lock().unwrap().len(), 2);
    assert!(agent.told.lock().unwrap()[1].1.starts_with("New mail: 1 Personal."));
    runner.round_done().await;
    assert_eq!(agent.told.lock().unwrap().len(), 2, "round done has nothing left to send");

    // A restart picks up from the saved history ID.
    drop(runner);
    let runner = make(agent.clone());
    runner.check_mail().await;
    let last_history = calls.lock().unwrap().iter().rev().find(|(p, _)| p == HISTORY).unwrap().1.clone();
    assert_eq!(last_history.pointer("/input/query/startHistoryId"), Some(&json!("104")));
    assert_eq!(agent.told.lock().unwrap().len(), 2);
    let paths: HashSet<String> = calls.lock().unwrap().iter().map(|(p, _)| p.clone()).collect();
    assert_eq!(paths, HashSet::from([PROFILE.to_string(), HISTORY.to_string()]));
}

#[tokio::test]
async fn while_the_person_reads_only_what_the_deck_lets_through_wakes_the_agent() {
    let head = Arc::new(StdMutex::new(100u64));
    let added: Added = Arc::default();
    let (url, _) = executor(feed(head.clone(), added.clone())).await;
    let dir = tempfile::tempdir().unwrap();
    let agent = Arc::new(FakeAgent::default());
    let deck = Arc::new(FakeDeck::default());
    let cfg = config(&url, "");
    let tools = ConfiguredTools::new(Some((cfg.executor.clone().unwrap(), "t".into())));
    let runner = Runner::new(cfg, deck.clone(), Arc::new(tools), agent.clone(), dir.path().to_path_buf());
    runner.check_mail().await; // first run: starts from now
    runner.set_reading(true).await;

    added.lock().unwrap().push((101, "a", "INBOX"));
    *head.lock().unwrap() = 101;
    runner.check_mail().await;
    assert!(agent.told.lock().unwrap().is_empty(), "held while the person reads");
    assert_eq!(deck.held.lock().unwrap().last(), Some(&(Some("1 Personal".to_string()), 1)));

    // An update the deck lets through wakes the agent for that alone.
    deck.lets.lock().unwrap().insert("b".into(), "an update on c_4's thread".into());
    added.lock().unwrap().push((102, "b", "INBOX"));
    *head.lock().unwrap() = 102;
    runner.check_mail().await;
    let told = agent.told.lock().unwrap().clone();
    assert_eq!(told.len(), 1);
    assert!(
        told[0]
            .1
            .starts_with("The person is reading the Inbox, so only these get through now: an update on c_4's thread"),
        "{}",
        told[0].1
    );
    assert!(told[0].1.contains("Everything else new (1 Personal) waits"), "{}", told[0].1);

    // Going idle after the round lets the held mail go.
    runner.round_done().await;
    assert_eq!(agent.told.lock().unwrap().len(), 1, "still reading");
    runner.set_reading(false).await;
    let told = agent.told.lock().unwrap().clone();
    assert_eq!(told.len(), 2);
    assert!(told[1].1.starts_with("New mail: 1 Personal."), "{}", told[1].1);
    assert_eq!(deck.held.lock().unwrap().last(), Some(&(None, 0)));

    // "Triage now" wakes for what's held even while the person reads.
    runner.round_done().await;
    runner.set_reading(true).await;
    added.lock().unwrap().push((103, "c", "INBOX"));
    *head.lock().unwrap() = 103;
    runner.check_mail().await;
    assert_eq!(agent.told.lock().unwrap().len(), 2);
    runner.release_held().await;
    assert!(agent.told.lock().unwrap()[2].1.starts_with("New mail: 1 Personal."));
}

#[tokio::test]
async fn a_wakeup_for_an_agent_thats_offline_waits_for_the_next_check() {
    let head = Arc::new(StdMutex::new(100u64));
    let added: Added = Arc::default();
    let (url, _) = executor(feed(head.clone(), added.clone())).await;
    let dir = tempfile::tempdir().unwrap();
    let agent = Arc::new(FakeAgent::default());
    let cfg = config(&url, "");
    let tools = ConfiguredTools::new(Some((cfg.executor.clone().unwrap(), "t".into())));
    let runner = Runner::new(cfg, Arc::new(FakeDeck::default()), Arc::new(tools), agent.clone(), dir.path().into());
    runner.check_mail().await;
    *agent.offline.lock().unwrap() = true;
    added.lock().unwrap().push((101, "a", "INBOX"));
    *head.lock().unwrap() = 101;
    runner.check_mail().await;
    *agent.offline.lock().unwrap() = false;
    runner.check_mail().await;
    let told = agent.told.lock().unwrap().clone();
    assert_eq!(told.len(), 1);
    assert!(told[0].1.starts_with("New mail: 1 Personal."));
}

#[tokio::test]
async fn threads_the_person_archived_in_gmail_go_to_the_deck_and_the_agent_hears() {
    // Since history 100: the Inbox label came off a message in ta (its whole thread is out now) and one in tb (another
    // tb message is still in the inbox), and tc was deleted.
    let script: Script = Arc::new(|path: &str, input: &Value| match path {
        PROFILE => Ok(json!({"historyId": "100"})),
        HISTORY => Ok(json!({"historyId": "101", "history": [
            {"labelsRemoved": [{"message": {"id": "a1", "threadId": "ta", "labelIds": []}, "labelIds": ["INBOX"]}]},
            {"labelsRemoved": [{"message": {"id": "b1", "threadId": "tb", "labelIds": []}, "labelIds": ["INBOX"]}]},
            {"labelsRemoved": [{"message": {"id": "x1", "threadId": "tx", "labelIds": ["INBOX"]}, "labelIds": ["UNREAD"]}]},
            {"messagesDeleted": [{"message": {"id": "c1", "threadId": "tc"}}]}
        ]})),
        THREAD => match input.pointer("/input/path/id").and_then(Value::as_str).unwrap_or_default() {
            "ta" => Ok(json!({"messages": [{"id": "a0", "labelIds": ["SENT"]}, {"id": "a1", "labelIds": []}]})),
            "tb" => Ok(json!({"messages": [{"id": "b0", "labelIds": ["INBOX"]}, {"id": "b1", "labelIds": []}]})),
            "tc" => Err("HTTP 404 Not Found".into()),
            other => panic!("the runner checked thread {other}: only threads that lost the Inbox label"),
        },
        other => panic!("unexpected tool {other}"),
    });
    let (url, _) = executor(script).await;
    let dir = tempfile::tempdir().unwrap();
    let agent = Arc::new(FakeAgent::default());
    let deck = Arc::new(FakeDeck::default());
    let cfg = config(&url, "");
    let tools = ConfiguredTools::new(Some((cfg.executor.clone().unwrap(), "t".into())));
    let runner = Runner::new(cfg, deck.clone(), Arc::new(tools), agent.clone(), dir.path().into());
    runner.check_mail().await; // the first check starts the watch at 100
    runner.check_mail().await;
    let left: Vec<String> = deck.left.lock().unwrap().iter().map(|t| t.thread_id.clone()).collect();
    assert_eq!(left, ["ta", "tc"]);
    let told = agent.told.lock().unwrap().clone();
    assert_eq!(told.len(), 1, "{told:?}");
    assert!(matches!(told[0].0, DeliveryKind::RunnerReport));
    assert!(told[0].1.starts_with("The person archived emails in Gmail themselves"), "{}", told[0].1);
    assert!(told[0].1.contains("c_10 'About ta'") && told[0].1.contains("quietdeck report done"), "{}", told[0].1);
}

/// Against a real Executor: `QD_EXECUTOR_URL`, `QD_EXECUTOR_TOKEN` and `QD_EXECUTOR_ACCOUNT` (an Executor Google
/// account ID). Reads the mailbox's profile and one page of history: message IDs and labels only.
#[tokio::test]
#[ignore = "needs a real Executor"]
async fn live_executor_reads_history_metadata() {
    let url = std::env::var("QD_EXECUTOR_URL").unwrap();
    let token = std::env::var("QD_EXECUTOR_TOKEN").unwrap();
    let account = std::env::var("QD_EXECUTOR_ACCOUNT").unwrap();
    let mut cfg = config(&url, "");
    cfg.accounts.get_mut("personal").unwrap().vars.insert("executor_account".into(), account);
    let tools = ConfiguredTools::new(Some((cfg.executor.clone().unwrap(), token)));
    let mut state = mail::MailState::default();
    let watch = cfg.mail_watch.clone().unwrap();
    let acct = json!({"executor_account": cfg.accounts["personal"].vars["executor_account"]});
    let input = tools::render(&watch.profile.input, &json!({"account": acct})).unwrap();
    let profile = tools.call(&watch.profile, input).await;
    assert!(profile.as_ref().is_ok_and(|p| p.get("historyId").is_some()), "the profile: {profile:?}");
    mail::check(&cfg, &watch, &tools, &mut state).await;
    let start = state.history.get("personal").cloned().expect("the profile's history ID");
    let back: u64 = start.parse::<u64>().unwrap() - 2000;
    state.history.insert("personal".into(), back.to_string());
    mail::check(&cfg, &watch, &tools, &mut state).await;
    let after: u64 = state.history["personal"].parse().unwrap();
    assert!(after >= start.parse::<u64>().unwrap(), "history moved forward to {after}");
    println!("history {back} -> {after}, pending {:?}", state.pending);
}

#[tokio::test]
async fn a_story_for_a_board_the_runner_doesnt_know_goes_to_the_agent() {
    let story = json!({"type": "story", "op": "create", "board": "marketing", "story_type": "chore",
        "title": "Reply to the reseller", "description": "d"});
    let h = harness(vec![released(1, json!([story]), json!({}))], "").await;
    h.runner.process().await.unwrap();
    assert_eq!(h.reason(101), "board marketing isn't in the runner's config");
    assert!(h.paths().is_empty());
}

// --- Lease steps ----------------------------------------------------------------------------------------------------

const FILTERS: &str = r#"
labels: { path: gm.labels.list, input: { accountId: "${account.executor_account}" } }
gmail_filters:
  create_label: { path: gm.labels.create, input: { accountId: "${account.executor_account}", body: { name: "${lease.label}" } } }
  create: { path: gm.filters.create, input: { accountId: "${account.executor_account}", body: { criteria: { query: "${lease.query}" }, action: { addLabelIds: ["${lease.label_id}"], removeLabelIds: [INBOX, UNREAD] } } } }
  delete: { path: gm.filters.delete, input: { accountId: "${account.executor_account}", id: "${lease.filter_id}" } }
  inbox: { path: gm.threads.list, input: { q: "${lease.query} in:inbox" } }
  move: { path: tools.google.gmail.modify, input: { account: "${account.address}", threadIds: "${thread_ids}", addLabels: ["${lease.label}"], archive: true, read: true } }
"#;

fn filter_lease(kind_state: Option<Value>) -> quietdeck_types::lease::Lease {
    serde_json::from_value(json!({
        "id": 9, "kind": "gmail-filter", "resource": "personal", "params": {"query": "from:ci.example.com"},
        "holder": {"node": "laptop", "tab": "api", "session_id": null}, "reason": "watching CI", "state": "approved",
        "ttl_seconds": 86400, "idle_minutes": 30, "created_at": "2026-11-23T08:00:00Z", "activated_at": null,
        "last_renewed_at": null, "expires_at": null, "cleanup_requested_at": null, "cleanup_by": null, "ended_at": null,
        "end_reason": null, "pattern_id": 1, "kind_state": kind_state, "card_id": null, "queue_position": null
    }))
    .unwrap()
}

#[tokio::test]
async fn a_filter_lease_is_created_with_its_label_moves_inbox_mail_and_is_removed() {
    let script: Script = Arc::new(|path: &str, _input: &Value| match path {
        "gm.labels.list" => Ok(json!({"labels": [{"id": "INBOX", "name": "INBOX"}]})),
        "gm.labels.create" => Ok(json!({"id": "Label_7"})),
        "gm.filters.create" => Ok(json!({"id": "filter_1"})),
        "gm.threads.list" => Ok(json!({"threads": [{"id": "t1"}, {"id": "t2"}]})),
        "gm.filters.delete" => Err("Executor error 404: Filter not found".into()),
        _ => Ok(json!({})),
    });
    let (url, calls) = executor(script).await;
    let cfg = config(&url, FILTERS);
    let dir = tempfile::tempdir().unwrap();
    let tools = ConfiguredTools::new(Some((cfg.executor.clone().unwrap(), "test-token".into())));
    let runner = Runner::new(
        cfg,
        Arc::new(FakeDeck::default()),
        Arc::new(tools),
        Arc::new(FakeAgent::default()),
        dir.path().into(),
    );

    let state = runner.lease_step(&filter_lease(None), quietdeck_types::lease::LeaseStep::Create).await.unwrap();
    assert_eq!(state["filter_id"], "filter_1");
    assert_eq!(state["moved"], 2);
    let log = calls.lock().unwrap().clone();
    let paths: Vec<&str> = log.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        paths,
        ["gm.labels.list", "gm.labels.create", "gm.filters.create", "gm.threads.list", "tools.google.gmail.modify"]
    );
    assert_eq!(log[1].1["body"]["name"], "Quietdeck/Lease/api");
    assert_eq!(log[2].1["body"]["action"]["addLabelIds"], json!(["Label_7"]));
    assert_eq!(log[2].1["body"]["criteria"]["query"], "from:ci.example.com");
    assert_eq!(log[3].1["q"], "from:ci.example.com in:inbox");
    assert_eq!(log[4].1["threadIds"], json!(["t1", "t2"]));

    // Deleted by hand already: removal still counts as done.
    let removed = runner
        .lease_step(&filter_lease(Some(json!({"filter_id": "filter_1"}))), quietdeck_types::lease::LeaseStep::Remove)
        .await
        .unwrap();
    assert_eq!(removed["already_gone"], true);
    assert_eq!(calls.lock().unwrap().last().unwrap().1["id"], "filter_1");
}

#[test]
fn tools_get_the_story_number_and_the_agent_label() {
    let comment: Action =
        serde_json::from_value(json!({"type": "story", "op": "comment", "story_id": "sc-1234", "text": "x"})).unwrap();
    let mut v = serde_json::to_value(&comment).unwrap();
    super::derived_values(&comment, &mut v);
    assert_eq!(v["story_number"], json!(1234));

    let done: Action = serde_json::from_value(json!({"type": "task", "op": "done", "story_id": "77"})).unwrap();
    let mut v = serde_json::to_value(&done).unwrap();
    super::derived_values(&done, &mut v);
    assert_eq!(v["story_number"], json!(77));
    assert!(v.get("agent_labels").is_none(), "no tab, no label key");

    let task: Action =
        serde_json::from_value(json!({"type": "task", "op": "create", "title": "Sign", "agent_tab": "finances"}))
            .unwrap();
    let mut v = serde_json::to_value(&task).unwrap();
    super::derived_values(&task, &mut v);
    assert_eq!(v["agent_labels"], json!(["agent:finances"]));
    assert_eq!(v["agent_label_objects"], json!([{"name": "agent:finances"}]));
    assert!(v.get("story_number").is_none());

    assert_eq!(super::story_number("SC-9"), Some(9));
    assert_eq!(super::story_number("PROJ-9"), None);
}
