//! The runner (section 1, "The runner"): no AI. It watches Gmail's history and wakes the triage agent when mail
//! arrives, and it carries out the person's released decisions that need no thought, so the agent spends its turns
//! only on what does.
//!
//! - It takes released decisions on its agent's cards and sorts them in code. A card goes to the agent whole, with a
//!   reason, when any action isn't eligible or has no tool, the decision carries text, the agent asked for it
//!   (`--needs-agent`), an action fails the local schema check, or a freshness check fails.
//! - It runs each action between `action start` (which Stop makes fail) and a report, and refuses an action whose
//!   payload differs from the one released.
//! - It tells the agent what it handed over and what failed, and never wakes it for a batch it carried out whole.

mod config;
mod deck;
mod gog;
mod headless;
mod intercom;
mod leases;
mod mail;
mod tools;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use quietdeck_types::action::{Action, SessionOp, SessionRef, StoryOp};
use quietdeck_types::card::{ActionRecord, ActionReport, ActionResult, ActionStatus, DecisionKind, Released};
use quietdeck_types::directory::SessionStart;
use quietdeck_types::node::{Handoff, RunnerInfo, SendAsCheck};
use quietdeck_types::reading::{HoldVerdict, ItemFacts};
use quietdeck_types::relay::DeliveryKind;
use quietdeck_types::{ActionType, AgentKind};
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc};
use tokio::time::Instant;

pub use config::RunnerConfig;
pub use deck::{AgentInbox, AgentTurns, DeckApi, LinkDeck, LocalAgent};
pub use headless::HeadlessAgent;
use mail::MailState;
pub use tools::{ConfiguredTools, Tools};

const SEND_AS_MAX_AGE: Duration = Duration::from_secs(3600);

/// What the node's link and local socket hand the runner's task.
pub enum Signal {
    /// Decisions were released, or the node just connected: take what's waiting.
    Released,
    /// The agent finished its round.
    RoundDone,
}

pub struct RunnerHandle {
    pub runner: Arc<Runner>,
    tx: mpsc::Sender<Signal>,
}

impl RunnerHandle {
    pub fn signal(&self, s: Signal) {
        // The task works through signals one at a time; a full queue already has work waiting.
        let _ = self.tx.try_send(s);
    }

    pub fn info(&self) -> RunnerInfo {
        let cfg = &self.runner.cfg;
        RunnerInfo {
            eligible: cfg.eligible.clone(),
            agent_tab: cfg.agent.tab.clone(),
            accounts: cfg.accounts.keys().cloned().collect(),
            agent_tabs: std::iter::once(cfg.agent.tab.clone()).chain(cfg.agent.tabs.iter().cloned()).collect(),
        }
    }
}

pub struct Runner {
    pub cfg: RunnerConfig,
    deck: Arc<dyn DeckApi>,
    tools: Arc<dyn Tools>,
    agent: Arc<dyn AgentInbox>,
    /// The other agents it works for (`runner.agent.tabs`), each as written there: a card one of them posted goes
    /// back to it when the runner can't carry it out.
    others: Vec<(String, Arc<dyn AgentInbox>)>,
    mail_state_path: PathBuf,
    mail: Mutex<MailState>,
    send_as: Mutex<HashMap<String, (Instant, Vec<String>)>>,
    /// The person is reading the Inbox (the deck says so): new items wait unless the deck lets them through.
    reading: std::sync::atomic::AtomicBool,
    /// "Triage now": the next wake-up goes even while the person reads.
    release: std::sync::atomic::AtomicBool,
    /// What the deck was last told the runner holds, so it's told only of changes.
    last_held: std::sync::Mutex<Option<(Option<String>, u64)>>,
    /// Drafts whose headers were read, by message ID (a draft's message ID changes with every save).
    draft_cache: std::sync::Mutex<HashMap<String, quietdeck_types::mail::MailItem>>,
}

/// One message's headers and snippet, as the message tool read them.
#[derive(Debug, Clone, Default)]
struct MessageMeta {
    thread_id: Option<String>,
    from: Option<String>,
    to: Option<String>,
    subject: Option<String>,
    snippet: Option<String>,
    at: Option<jiff::Timestamp>,
}

/// "DocSpring: 'Re: Ads' to rep@x.com (thread 1a2b)": a sent message for the next wake-up.
fn sent_line(account: &str, f: &ItemFacts, meta: Option<&MessageMeta>) -> String {
    let subject = f.subject.as_deref().map(|s| format!(" '{}'", clean_line(s))).unwrap_or_default();
    let to = meta.and_then(|m| m.to.as_deref()).map(|t| format!(" to {}", clean_line(t))).unwrap_or_default();
    let thread = f.thread_key.as_deref().map(|t| format!(" (thread {})", clean_line(t))).unwrap_or_default();
    format!("{account}:{subject}{to}{thread}")
}

fn mail_item(
    account: &str,
    kind: quietdeck_types::mail::MailKind,
    message_id: &str,
    m: MessageMeta,
) -> quietdeck_types::mail::MailItem {
    quietdeck_types::mail::MailItem {
        account: account.to_string(),
        kind,
        message_id: message_id.to_string(),
        thread_id: m.thread_id,
        draft_id: None,
        from: m.from,
        to: m.to,
        subject: m.subject,
        snippet: m.snippet,
        at: m.at.unwrap_or_else(jiff::Timestamp::now),
    }
}

trait WithDraftId {
    fn with_draft_id(self, id: Option<&str>) -> Self;
}

impl WithDraftId for quietdeck_types::mail::MailItem {
    fn with_draft_id(mut self, id: Option<&str>) -> Self {
        self.draft_id = id.map(str::to_string);
        self
    }
}

/// What a batch did, for logs and tests.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BatchOutcome {
    pub claimed: Vec<i64>,
    pub handed: Vec<(i64, String)>,
    pub done: usize,
    pub failed: usize,
    pub stopped: usize,
}

/// A thread's message IDs and labels, for the freshness checks.
#[derive(Debug, Clone, Default)]
struct ThreadMeta {
    /// Oldest first, drafts left out: each message's ID, labels, and when Gmail received it (`internalDate`, in
    /// milliseconds) when the thread tool returns it.
    messages: Vec<(String, Vec<String>, Option<i64>)>,
}

impl ThreadMeta {
    fn any_label(&self, label: &str) -> bool {
        self.messages.iter().any(|(_, l, _)| l.iter().any(|x| x == label))
    }
}

enum Sorted {
    Claim,
    Handoff { reason: String, no_handler: Option<ActionType> },
}

fn handoff(reason: impl Into<String>) -> Sorted {
    Sorted::Handoff { reason: reason.into(), no_handler: None }
}

/// Why a thread has a message the card wasn't prepared on, or None when it hasn't. The card's own thread has the
/// message it names (`last_message_ref`) as its newest. Any other thread the card acts on (a group of related emails)
/// is checked by time: its newest message arrived before the card was prepared, or is one the card lists in `refs`.
fn newer_message(r: &Released, account: &str, thread_id: &str, meta: &ThreadMeta) -> Option<String> {
    let card = &r.card.input;
    let when = || r.card.prepared_at.to_zoned_datetime(jiff::tz::TimeZone::system()).strftime("%H:%M").to_string();
    let newest = meta.messages.last()?;
    if account == card.account && thread_id == card.thread_key {
        let Some(prepared_on) = card.last_message_ref.as_deref() else {
            return Some("the card names no message, so the thread can't be checked for newer ones".into());
        };
        return (newest.0 != prepared_on).then(|| format!("newer message in thread since {}", when()));
    }
    if card.refs.iter().any(|r| r == &newest.0) {
        return None;
    }
    match newest.2 {
        Some(ms) if ms > r.card.prepared_at.as_millisecond() => {
            Some(format!("newer message in thread {thread_id} since {}", when()))
        }
        Some(_) => None,
        None => Some(format!(
            "couldn't tell whether thread {thread_id} has a newer message: the thread tool returns no internalDate"
        )),
    }
}

fn thread_of(a: &Action) -> Option<(&str, &str)> {
    match a {
        Action::Archive { account, thread_id }
        | Action::MarkRead { account, thread_id }
        | Action::NotSpam { account, thread_id } => Some((account, thread_id)),
        Action::Label { account, thread_id, .. } => Some((account, thread_id)),
        Action::Draft(c) => Some((&c.account, &c.thread_id)),
        _ => None,
    }
}

/// How long a forward waits for a session its card just started to reach the deck: up to 10 tries, 3 s apart.
const FORWARD_WAIT_TRIES: u32 = 10;
const FORWARD_WAIT_STEP: std::time::Duration = std::time::Duration::from_secs(3);

/// Action types the runner carries out through the deck and the orchestrator rather than a configured tool.
fn built_in(t: ActionType) -> bool {
    matches!(t, ActionType::Forward | ActionType::Resume | ActionType::NewSession | ActionType::Session)
}

fn released_actions(r: &Released) -> Vec<&ActionRecord> {
    r.actions.iter().filter(|a| a.status == ActionStatus::Released).collect()
}

fn describe(a: &Action) -> &'static str {
    match a {
        Action::Archive { .. } => "Archive",
        Action::MarkRead { .. } => "Mark read",
        Action::NotSpam { .. } => "Mark as not spam",
        Action::CloseConversation { .. } => "Close the conversation",
        Action::Label { .. } => "Label",
        Action::Forward { .. } => "Forward",
        Action::Draft(_) => "Draft",
        Action::Reply(_) => "Reply",
        Action::Story(_) => "Story",
        Action::Task { .. } => "Task",
        Action::Calendar { .. } => "Calendar event",
        Action::Resume { .. } => "Resume",
        Action::NewSession { .. } => "New session",
        Action::Lease { .. } => "Lease",
        Action::Watch { .. } => "Watch",
        Action::Session { op: SessionOp::Message, .. } => "Message session",
        Action::Session { op: SessionOp::WrapUp, .. } => "Wrap up session",
        Action::Session { op: SessionOp::Close, .. } => "Close session",
    }
}

impl Runner {
    pub fn new(
        cfg: RunnerConfig,
        deck: Arc<dyn DeckApi>,
        tools: Arc<dyn Tools>,
        agent: Arc<dyn AgentInbox>,
        state_dir: PathBuf,
    ) -> Runner {
        let mail_state_path = state_dir.join("runner-mail.json");
        let mail = Mutex::new(MailState::load(&mail_state_path));
        Runner {
            cfg,
            deck,
            tools,
            agent,
            others: vec![],
            mail_state_path,
            mail,
            send_as: Mutex::default(),
            reading: Default::default(),
            release: Default::default(),
            last_held: Default::default(),
            draft_cache: Default::default(),
        }
    }

    /// The other agents it works for, each with its inbox.
    pub fn with_others(mut self, others: Vec<(String, Arc<dyn AgentInbox>)>) -> Self {
        self.others = others;
        self
    }

    /// Where a card's hand-offs and failures go: the agent that posted it, when it's one of the others; else the main
    /// agent.
    fn agent_for(&self, posted_by: &SessionRef) -> &Arc<dyn AgentInbox> {
        self.others
            .iter()
            .find(|(spec, _)| match spec.rsplit_once('@') {
                Some((tab, node)) if !tab.is_empty() && !node.is_empty() => {
                    tab == posted_by.tab && node == posted_by.node
                }
                _ => spec == &posted_by.tab,
            })
            .map(|(_, a)| a)
            .unwrap_or(&self.agent)
    }

    fn account_ctx(&self, id: &str) -> Option<Value> {
        let a = self.cfg.accounts.get(id)?;
        let mut v = json!({"id": id, "address": a.address, "label": self.cfg.account_label(id)});
        for (k, val) in &a.vars {
            v[k] = Value::String(val.clone());
        }
        Some(v)
    }

    // --- The send-as list -----------------------------------------------------------------------------------------

    async fn send_as_list(&self, account: &str) -> Result<Vec<String>, String> {
        if let Some((at, list)) = self.send_as.lock().await.get(account)
            && at.elapsed() < SEND_AS_MAX_AGE
        {
            return Ok(list.clone());
        }
        let spec = self.cfg.send_as.as_ref().ok_or("runner.send_as isn't set")?;
        let acct =
            self.account_ctx(account).ok_or_else(|| format!("account {account} isn't in the runner's config"))?;
        let v = self.tools.call(spec, tools::render(&spec.input, &json!({"account": acct}))?).await?;
        let list: Vec<String> = v
            .get("sendAs")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.get("sendAsEmail").and_then(Value::as_str))
                    .map(|s| s.to_lowercase())
                    .collect()
            })
            .unwrap_or_default();
        if list.is_empty() {
            return Err(format!("the send-as list for {account} came back empty"));
        }
        self.send_as.lock().await.insert(account.to_string(), (Instant::now(), list.clone()));
        Ok(list)
    }

    /// A lease's create or remove step, from the deck.
    pub async fn lease_step(
        &self,
        lease: &quietdeck_types::lease::Lease,
        step: quietdeck_types::lease::LeaseStep,
    ) -> Result<serde_json::Value, String> {
        leases::step(self, lease, step).await
    }

    /// The deck's question when a card with a draft or reply is posted.
    pub async fn send_as_check(&self, account: &str, from: &str) -> Result<SendAsCheck, String> {
        let addresses = self.send_as_list(account).await?;
        Ok(SendAsCheck { allowed: addresses.contains(&from.to_lowercase()), addresses })
    }

    // --- Freshness ------------------------------------------------------------------------------------------------

    async fn thread_meta(&self, account: &str, thread_id: &str) -> Result<ThreadMeta, String> {
        let spec = self.cfg.thread.as_ref().ok_or("runner.thread isn't set, so the thread can't be checked")?;
        let acct =
            self.account_ctx(account).ok_or_else(|| format!("account {account} isn't in the runner's config"))?;
        let input = tools::render(&spec.input, &json!({"account": acct, "action": {"thread_id": thread_id}}))?;
        let v = self.tools.call(spec, input).await?;
        let messages = v
            .get("messages")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|m| {
                        let id = m.get("id")?.as_str()?.to_string();
                        let labels: Vec<String> = m
                            .get("labelIds")
                            .and_then(Value::as_array)
                            .map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect())
                            .unwrap_or_default();
                        let received = match m.get("internalDate") {
                            Some(Value::String(s)) => s.parse::<i64>().ok(),
                            Some(Value::Number(n)) => n.as_i64(),
                            _ => None,
                        };
                        (!labels.iter().any(|l| l == "DRAFT")).then_some((id, labels, received))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(ThreadMeta { messages })
    }

    async fn label_names(&self, account: &str) -> Result<Option<HashMap<String, String>>, String> {
        let Some(spec) = &self.cfg.labels else { return Ok(None) };
        let acct =
            self.account_ctx(account).ok_or_else(|| format!("account {account} isn't in the runner's config"))?;
        let v = self.tools.call(spec, tools::render(&spec.input, &json!({"account": acct}))?).await?;
        let list = v.get("labels").and_then(Value::as_array).or_else(|| v.as_array()).cloned().unwrap_or_default();
        Ok(Some(
            list.iter()
                .filter_map(|l| {
                    let name = l.get("name")?.as_str()?.to_string();
                    let id = l.get("id").and_then(Value::as_str).unwrap_or(&name).to_string();
                    Some((name, id))
                })
                .collect(),
        ))
    }

    // --- Sorting --------------------------------------------------------------------------------------------------

    async fn sort_one(&self, r: &Released, threads: &mut HashMap<(String, String), ThreadMeta>) -> Sorted {
        if r.decision.kind == DecisionKind::Adjust || r.decision.text.is_some() {
            return handoff("adjustment text");
        }
        if r.card.input.needs_agent {
            return handoff("the agent asked to carry it out (--needs-agent)");
        }
        // A card with no actions at all (a record, a thread kept in the inbox) was approved with nothing to carry out:
        // the runner claims it, the deck marks it done, and the agent isn't woken for it (it sees the state on its next
        // round). One approved with a change or message went to the agent above.
        if r.card.input.actions.is_empty() {
            return Sorted::Claim;
        }
        let actions = released_actions(r);
        if actions.is_empty() {
            return handoff("nothing released to run");
        }
        for rec in &actions {
            let a = &rec.action;
            let t = a.action_type();
            if let Err(e) = a.check() {
                return handoff(format!("{t} failed the runner's schema check: {e}"));
            }
            if t == ActionType::Reply {
                return handoff("reply isn't runner-eligible");
            }
            if !self.cfg.eligible.contains(&t) {
                return handoff(format!("{t} isn't runner-eligible"));
            }
            if let Some(acct) = a.account()
                && !self.cfg.accounts.contains_key(acct)
            {
                return handoff(format!("account {acct} isn't in the runner's config"));
            }
            let key = config::tool_key(a);
            if !built_in(t) && !self.cfg.tools.contains_key(&key) {
                return Sorted::Handoff { reason: format!("no handler for {key}"), no_handler: Some(t) };
            }
            if let Action::Story(StoryOp::Create { board, .. }) = a
                && !self.cfg.boards.contains_key(board)
            {
                return handoff(format!("board {board} isn't in the runner's config"));
            }
        }
        // Freshness, in code.
        for rec in &actions {
            let a = &rec.action;
            if let Some((account, thread_id)) = thread_of(a) {
                let key = (account.to_string(), thread_id.to_string());
                if !threads.contains_key(&key) {
                    match self.thread_meta(account, thread_id).await {
                        Ok(m) => {
                            threads.insert(key.clone(), m);
                        }
                        Err(e) if e.contains("404") => return handoff("the thread no longer exists"),
                        Err(e) => return handoff(format!("couldn't check the thread: {e}")),
                    }
                }
                let meta = &threads[&key];
                if meta.messages.is_empty() {
                    return handoff("the thread no longer exists");
                }
                if let Some(reason) = newer_message(r, account, thread_id, meta) {
                    return handoff(reason);
                }
            }
            if let Action::Draft(c) = a {
                match self.send_as_check(&c.account, &c.from).await {
                    Ok(check) if check.allowed => {}
                    Ok(check) => {
                        return handoff(format!(
                            "{} can't send as {} (it can send as {})",
                            c.account,
                            c.from,
                            check.addresses.join(", ")
                        ));
                    }
                    Err(e) => return handoff(format!("couldn't check the send-as list: {e}")),
                }
            }
            if let Action::Label { account, add, .. } = a {
                match self.label_names(account).await {
                    Ok(Some(names)) => {
                        if let Some(missing) = add.iter().find(|l| !names.contains_key(*l) && !is_system_label(l)) {
                            return handoff(format!("label missing: {missing}"));
                        }
                    }
                    Ok(None) => {}
                    Err(e) => return handoff(format!("couldn't read the labels: {e}")),
                }
            }
        }
        Sorted::Claim
    }

    // --- Carrying out ---------------------------------------------------------------------------------------------

    /// One action in its card's order. A forward to a tab that an earlier action of the card started or resumed
    /// waits until the deck knows the new session, and isn't tried when that start failed.
    async fn run_in_card(
        &self,
        r: &Released,
        a: &Action,
        threads: &HashMap<(String, String), ThreadMeta>,
        opened: &mut HashMap<(String, String), Result<(), String>>,
    ) -> Result<Value, String> {
        match a {
            Action::NewSession { node, tab, .. } | Action::Resume { node, tab, .. } => {
                let result = self.run_action(r, a, threads).await;
                opened.insert((node.clone(), tab.clone()), result.as_ref().map(|_| ()).map_err(Clone::clone));
                result
            }
            Action::Forward { to, .. } => match opened.get(&(to.node.clone(), to.tab.clone())) {
                Some(Err(e)) => Err(format!("not sent: the session in {} didn't start ({e})", to.tab)),
                Some(Ok(())) => {
                    let mut tries = 0;
                    loop {
                        match self.run_action(r, a, threads).await {
                            Err(e) if e.starts_with("no session named") && tries < FORWARD_WAIT_TRIES => {
                                tries += 1;
                                tokio::time::sleep(FORWARD_WAIT_STEP).await;
                            }
                            result => return result,
                        }
                    }
                }
                None => self.run_action(r, a, threads).await,
            },
            _ => self.run_action(r, a, threads).await,
        }
    }

    async fn run_action(
        &self,
        r: &Released,
        a: &Action,
        threads: &HashMap<(String, String), ThreadMeta>,
    ) -> Result<Value, String> {
        let meta = thread_of(a).and_then(|(acct, t)| threads.get(&(acct.to_string(), t.to_string())));
        // Already done counts as done.
        match (a, meta) {
            (Action::Archive { .. }, Some(m)) if !m.any_label("INBOX") => return Ok(json!({"already": "archived"})),
            (Action::MarkRead { .. }, Some(m)) if !m.any_label("UNREAD") => return Ok(json!({"already": "read"})),
            (Action::NotSpam { .. }, Some(m)) if !m.any_label("SPAM") => return Ok(json!({"already": "not spam"})),
            _ => {}
        }
        match a {
            Action::Forward { to, text, note, source } => {
                let kind = DeliveryKind::Forward {
                    account: source.as_ref().map(|s| s.account.clone()).unwrap_or_else(|| r.card.input.account.clone()),
                    refs: source
                        .as_ref()
                        .map(|s| std::iter::once(s.thread_id.clone()).chain(s.message_id.clone()).collect())
                        .unwrap_or_else(|| vec![r.card.input.thread_key.clone()]),
                    rule: None,
                    sent_mail: false,
                    note: note.clone(),
                };
                let delivered = self.deck.relay(to.address(), kind, text.clone()).await?;
                Ok(json!({"forwarded_to": to.address(), "delivered": delivered}))
            }
            Action::Resume { node, tab, session_id, cwd } => {
                let s = self.deck.resume_session(node.clone(), session_id.clone(), tab.clone(), cwd.clone()).await?;
                Ok(serde_json::to_value(s).unwrap_or(Value::Null))
            }
            Action::NewSession { node, tab, cwd, prompt } => {
                let spec = SessionStart {
                    on: node.clone(),
                    tab: tab.clone(),
                    name: None,
                    agent: AgentKind::Claude,
                    model: None,
                    effort: None,
                    cwd: cwd.clone(),
                    prompt: Some(prompt.clone()),
                    handoff: None,
                };
                let s = self.deck.start_session(spec).await?;
                Ok(serde_json::to_value(s).unwrap_or(Value::Null))
            }
            Action::Session { op, to, text } => {
                let address = to.address();
                match op {
                    SessionOp::Close => {
                        self.deck.close_session(address.clone()).await?;
                        Ok(json!({"closed": address}))
                    }
                    SessionOp::Message | SessionOp::WrapUp => {
                        let body = match (op, text) {
                            (SessionOp::WrapUp, None) => quietdeck_types::action::WRAP_UP_TEXT.to_string(),
                            (_, Some(t)) => format!("From the deck's owner, approved on the Work tab: {t}"),
                            (_, None) => return Err("a message needs its text".into()),
                        };
                        let delivered = self.deck.relay(address.clone(), DeliveryKind::Deck, body).await?;
                        Ok(json!({op.as_str(): address, "delivered": delivered}))
                    }
                }
            }
            Action::Reply(_) | Action::Watch { .. } | Action::Lease { .. } => {
                Err(format!("the runner doesn't carry out {}", a.action_type()))
            }
            _ => {
                if let Action::Draft(c) = a {
                    let check = self.send_as_check(&c.account, &c.from).await?;
                    if !check.allowed {
                        return Err(format!("{} can't send as {}", c.account, c.from));
                    }
                }
                let key = config::tool_key(a);
                let spec = self.cfg.tools.get(&key).ok_or_else(|| format!("no tool for {key}"))?;
                let mut ctx = json!({
                    "action": serde_json::to_value(a).map_err(|e| e.to_string())?,
                    "card": {"id": r.card.id, "thread_key": r.card.input.thread_key,
                             "last_message_ref": r.card.input.last_message_ref, "subject": r.card.input.subject},
                });
                derived_values(a, &mut ctx["action"]);
                if let Some(acct) = a.account().and_then(|id| self.account_ctx(id)) {
                    ctx["account"] = acct;
                }
                if let Action::Story(StoryOp::Create { board, .. }) = a
                    && let Some(ids) = self.cfg.boards.get(board)
                {
                    ctx["board"] = serde_json::to_value(ids).map_err(|e| e.to_string())?;
                }
                let input = tools::render(&spec.input, &ctx)?;
                let result = self.tools.call(spec, input).await?;
                Ok(json!({"via": spec.describe(), "result": tools::short(&result)}))
            }
        }
    }

    /// A delivery from the deck for `tab`: when that's this runner's agent and the agent is headless (solo), it has no
    /// session to deliver to, so the runner hands it over, and it starts or joins a turn as mail does. None when it
    /// isn't this runner's headless agent.
    pub async fn tell_headless_agent(&self, tab: &str, kind: DeliveryKind, body: String) -> Option<Result<(), String>> {
        if self.cfg.agent.headless.is_none() || self.cfg.agent.tab != tab {
            return None;
        }
        Some(self.agent.tell(kind, body).await)
    }

    /// The headless agent as a session in this node's directory, so the deck can address it (report answers, chat,
    /// hand-offs) like any session. None for an agent in a tab, which the orchestrator reports.
    pub fn headless_session(&self) -> Option<quietdeck_types::directory::SessionInfo> {
        let h = self.cfg.agent.headless.as_ref()?;
        Some(quietdeck_types::directory::SessionInfo {
            session_id: format!("headless:{}", self.cfg.agent.tab),
            agent: AgentKind::Claude,
            workspace: None,
            tab: Some(self.cfg.agent.tab.clone()),
            address: None,
            cwd: h.cwd.clone(),
            status: quietdeck_types::AgentStatus::Idle,
            status_changed_at: jiff::Timestamp::now(),
            resume_command: None,
            running: true,
            open_url: None,
            opens_in: None,
        })
    }

    /// Take, sort and carry out everything released for the agent.
    pub async fn process(&self) -> Result<BatchOutcome, String> {
        let taken = self.deck.take().await?;
        let mut out = BatchOutcome::default();
        if taken.is_empty() {
            return Ok(out);
        }
        let mut threads: HashMap<(String, String), ThreadMeta> = HashMap::new();
        let mut claims = Vec::new();
        let mut handoffs = Vec::new();
        for r in &taken {
            match self.sort_one(r, &mut threads).await {
                Sorted::Claim => claims.push(r.decision.id),
                Sorted::Handoff { reason, no_handler } => {
                    handoffs.push(Handoff { decision_id: r.decision.id, reason, no_handler });
                }
            }
        }
        let held = self.deck.sort(claims, handoffs.clone()).await?;
        out.handed = handoffs.iter().map(|h| (h.decision_id, h.reason.clone())).collect();
        // Hand-offs go to the agent first, so it works on them while the runner carries out the rest (a batch of
        // archives can take a minute), instead of waiting for the whole batch.
        // Each agent hears about its own cards only.
        let mut by_agent: Vec<(SessionRef, Vec<String>)> = vec![];
        for h in &handoffs {
            let Some(r) = taken.iter().find(|r| r.decision.id == h.decision_id) else { continue };
            let line = format!("'{}' ({})", r.card.input.subject, h.reason);
            match by_agent.iter_mut().find(|(p, _)| p.node == r.card.posted_by.node && p.tab == r.card.posted_by.tab) {
                Some((_, lines)) => lines.push(line),
                None => by_agent.push((r.card.posted_by.clone(), vec![line])),
            }
        }
        for (posted_by, reasons) in by_agent {
            let n = reasons.len();
            let body = format!(
                "{n} card{} handed to you: {}. Take {} with `quietdeck decisions take`.",
                if n == 1 { "" } else { "s" },
                reasons.join("; "),
                if n == 1 { "it" } else { "them" }
            );
            match self.agent_for(&posted_by).tell(DeliveryKind::Handoff, body).await {
                Ok(()) => tracing::info!(cards = n, agent = %posted_by.address(), "handed cards to their agent"),
                Err(e) => tracing::info!(error = %e, "the agent will find its hand-offs with `decisions take`"),
            }
        }
        // Failures, by the agent whose card failed.
        let mut failures: Vec<(SessionRef, String)> = Vec::new();
        let mut attempted = 0usize;
        for r in taken.iter().filter(|r| held.contains(&r.decision.id)) {
            out.claimed.push(r.decision.id);
            let mut reports = Vec::new();
            // Sessions this card started or resumed, by (node, tab): a forward to one waits for it.
            let mut opened: HashMap<(String, String), Result<(), String>> = HashMap::new();
            for rec in released_actions(r) {
                let started = match self.deck.start(rec.id).await {
                    Ok(s) => s,
                    Err(e) => {
                        // Stopped, or no longer ours: don't run it.
                        tracing::info!(action = rec.id, error = %e, "the deck refused to start an action");
                        out.stopped += 1;
                        continue;
                    }
                };
                attempted += 1;
                let result = if started.action != rec.action || started.card_version != rec.card_version {
                    Err("the action's payload changed after it was released: not run".to_string())
                } else {
                    self.run_in_card(r, &rec.action, &threads, &mut opened).await
                };
                match result {
                    Ok(outcome) => {
                        out.done += 1;
                        reports.push(ActionReport {
                            action_id: rec.id,
                            result: ActionResult::Done { outcome: Some(outcome) },
                        });
                    }
                    Err(error) => {
                        out.failed += 1;
                        failures.push((
                            r.card.posted_by.clone(),
                            format!(
                                "{} of '{}' ({}): {error}. Card c_{} is waiting with Retry.",
                                describe(&rec.action),
                                r.card.input.subject,
                                self.cfg.account_label(&r.card.input.account),
                                r.card.id
                            ),
                        ));
                        reports.push(ActionReport { action_id: rec.id, result: ActionResult::Failed { error } });
                    }
                }
            }
            if !reports.is_empty()
                && let Err(e) = self.deck.report(reports).await
            {
                tracing::warn!(error = %e, card = r.card.id, "reporting results to the deck");
            }
        }
        let mut posters: Vec<SessionRef> = vec![];
        for (p, _) in &failures {
            if !posters.iter().any(|x| x.node == p.node && x.tab == p.tab) {
                posters.push(p.clone());
            }
        }
        for p in posters {
            let mine: Vec<&str> =
                failures.iter().filter(|(x, _)| x.node == p.node && x.tab == p.tab).map(|(_, f)| f.as_str()).collect();
            let body = format!("Runner: {} of {attempted} failed. {}", mine.len(), mine.join(" "));
            if let Err(e) = self.agent_for(&p).tell(DeliveryKind::RunnerReport, body).await {
                tracing::info!(error = %e, "the agent will see the failures on the deck");
            }
        }
        Ok(out)
    }

    // --- The mail watch -------------------------------------------------------------------------------------------

    /// One check of every account, then a wake-up if one is due.
    pub async fn check_mail(&self) {
        let Some(watch) = &self.cfg.mail_watch else { return };
        let mut state = self.mail.lock().await;
        // Mail that didn't fit in the last wake-up goes in this one, once that one has gone.
        if watch.message_text.is_some() && state.mail_text.is_empty() && !state.mail_backlog.is_empty() {
            let n = state.mail_backlog.len().min(MAIL_TEXT_MAX);
            let batch: Vec<mail::Waiting> = state.mail_backlog.drain(..n).collect();
            for w in batch {
                let m = mail::NewMail {
                    account: w.account.clone(),
                    id: w.id,
                    thread_id: w.thread_id,
                    labels: vec!["INBOX".into()],
                };
                state.mail_text.push(self.mail_block(watch, &m).await);
                state.add(&w.account, Some(1));
            }
        }
        let checked = mail::check(&self.cfg, watch, self.tools.as_ref(), &mut state).await;
        let new = checked.new;
        self.threads_moved(&new, checked.left).await;
        if !new.is_empty() {
            let mut facts = Vec::with_capacity(new.len());
            let mut sent = vec![];
            for m in &new {
                // Filed as spam: counted for the wake-up, read by the agent itself; never held, never a card here.
                if m.labels.iter().any(|l| l == "SPAM") {
                    *state.spam.entry(m.account.clone()).or_default() += 1;
                    continue;
                }
                // A message that's already gone (Gmail took back a follow-up nudge) isn't mail.
                if let Some((f, meta)) = self.facts(watch, m).await {
                    let is_sent = m.labels.iter().any(|l| l == "SENT");
                    let in_inbox = m.labels.iter().any(|l| l == "INBOX");
                    if is_sent && !in_inbox {
                        // The person's own mail: not new mail to triage, and no wake-up of its own (Nathan, 25 Nov).
                        state.sent.push(sent_line(&self.cfg.account_label(&m.account), &f, meta.as_ref()));
                    } else if watch.message_text.is_some() && state.mail_text.len() >= MAIL_TEXT_MAX {
                        // A full wake-up already: this one waits for a later one, text and all.
                        state.mail_backlog.push(mail::Waiting {
                            account: m.account.clone(),
                            id: m.id.clone(),
                            thread_id: m.thread_id.clone(),
                        });
                    } else {
                        facts.push((m.account.clone(), f));
                        if watch.message_text.is_some() {
                            state.mail_text.push(self.mail_block(watch, m).await);
                        }
                    }
                    if is_sent && let Some(meta) = meta {
                        sent.push(mail_item(&m.account, quietdeck_types::mail::MailKind::Sent, &m.id, meta));
                    }
                }
            }
            if !sent.is_empty()
                && let Err(e) = self.deck.mail_sent(sent).await
            {
                tracing::debug!(error = %e, "sending the sent messages for the Sent tab");
            }
            self.sort_new(&mut state, facts).await;
        }
        self.check_intercom(&mut state).await;
        self.check_lease_filters(&mut state).await;
        self.wake_if_due(&mut state).await;
        state.save(&self.mail_state_path);
        let turns = self.agent.turns();
        let mail_error = [
            state.error.clone(),
            state.intercom_error.as_ref().map(|i| format!("Intercom: {i}")),
            state.lease_error.as_ref().map(|l| format!("Lease filters: {l}")),
        ]
        .into_iter()
        .flatten()
        .reduce(|a, b| format!("{a}; {b}"));
        let health = quietdeck_types::node::RunnerHealth {
            mail_ok_at: state.ok_at,
            mail_checked_at: state.checked_at,
            mail_error,
            agent_ok_at: turns.as_ref().and_then(|t| t.ok_at),
            agent_failed_at: turns.as_ref().and_then(|t| t.failed.as_ref().map(|f| f.0)),
            agent_error: turns.and_then(|t| t.failed.map(|f| f.1)),
        };
        drop(state);
        self.deck.health(health).await;
    }

    /// The support inbox, on its own interval (the mail watch's tick drives it).
    async fn check_intercom(&self, state: &mut MailState) {
        let Some(watch) = &self.cfg.intercom_watch else { return };
        let every = config::parse_duration(&watch.every).unwrap_or(Duration::from_secs(120));
        let now = jiff::Timestamp::now();
        if let Some(last) = state.intercom_checked_at
            && now.duration_since(last).unsigned_abs() < every
        {
            return;
        }
        state.intercom_checked_at = Some(now);
        intercom::check(watch, self.tools.as_ref(), state, now).await;
    }

    /// The Gmail filters for Fleetlayer's leases, on their own interval (the mail watch's tick drives it).
    async fn check_lease_filters(&self, state: &mut MailState) {
        let Some(fl) = self.cfg.gmail_filters.as_ref().and_then(|g| g.fleetlayer.as_ref()) else { return };
        let every = config::parse_duration(&fl.every).unwrap_or(Duration::from_secs(60));
        let now = jiff::Timestamp::now();
        if let Some(last) = state.lease_checked_at
            && now.duration_since(last).unsigned_abs() < every
        {
            return;
        }
        state.lease_checked_at = Some(now);
        state.lease_error = leases::reconcile(self, fl, &mut state.lease_filters).await.err();
        if let Some(e) = &state.lease_error {
            tracing::warn!(error = %e, "runner: making the Gmail filters match Fleetlayer's leases");
        }
    }

    /// Threads the person archived or deleted in Gmail themselves, and threads that came back with new mail: each
    /// candidate is checked (every message gone from the inbox, or the thread deleted) before the deck hears of it.
    /// The deck resolves the cards whose threads have all gone, and the agent is told, so it takes them off the
    /// report (or closes it when nothing's left).
    async fn threads_moved(&self, new: &[mail::NewMail], candidates: Vec<(String, String)>) {
        use quietdeck_types::mail::LeftThread;
        let back: Vec<LeftThread> = new
            .iter()
            .filter(|m| m.labels.iter().any(|l| l == "INBOX"))
            .filter_map(|m| Some(LeftThread { account: m.account.clone(), thread_id: m.thread_id.clone()? }))
            .collect();
        let mut left = vec![];
        for (account, thread_id) in candidates {
            if back.iter().any(|b| b.account == account && b.thread_id == thread_id) {
                continue;
            }
            let gone = match self.thread_meta(&account, &thread_id).await {
                Ok(t) => !t.any_label("INBOX"),
                Err(e) if e.contains("404") => true,
                Err(e) => {
                    tracing::debug!(%account, error = %e, "checking whether a thread left the inbox");
                    false
                }
            };
            if gone {
                left.push(LeftThread { account, thread_id });
            }
        }
        if left.is_empty() && back.is_empty() {
            return;
        }
        let resolved = match self.deck.threads_left(left, back).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "telling the deck which threads left the inbox");
                return;
            }
        };
        if resolved.is_empty() {
            return;
        }
        let n = resolved.len();
        let list: Vec<String> = resolved.iter().map(|c| format!("c_{} '{}'", c.id, c.subject)).collect();
        let body = format!(
            "{} left the inbox outside Quietdeck (archived or deleted in Gmail, by the person or by an agent working \
             directly), so the deck resolved {}: {}. Take {} off the report (`quietdeck report show --markdown`, \
             then `report set`). If nothing on it waits for the person any more, mark it done with \
             `quietdeck report done` so the Inbox says \"All done.\"",
            if n == 1 { "An email" } else { "Emails" },
            if n == 1 { "its card" } else { "their cards" },
            list.join(", "),
            if n == 1 { "it" } else { "them" }
        );
        tracing::info!(cards = n, "threads left the inbox outside Quietdeck: telling the agent");
        if let Err(e) = self.agent.tell(DeliveryKind::RunnerReport, body).await {
            tracing::info!(error = %e, "the agent will see the resolved cards on the deck");
        }
    }

    /// One message's headers, labels and snippet (never its body), through the configured message tool. `Ok(None)`:
    /// the message no longer exists. `Err`: no tool, or it failed.
    async fn message_meta(
        &self,
        watch: &config::MailWatchConfig,
        account: &str,
        message_id: &str,
    ) -> Result<Option<MessageMeta>, String> {
        let spec = watch.message.as_ref().ok_or("runner.mail_watch.message isn't set")?;
        let acct =
            self.account_ctx(account).ok_or_else(|| format!("account {account} isn't in the runner's config"))?;
        let input = tools::render(&spec.input, &json!({"account": acct, "message_id": message_id}))?;
        match self.tools.call(spec, input).await {
            Ok(v) => {
                let header = |name: &str| {
                    v.pointer("/payload/headers").and_then(Value::as_array).and_then(|hs| {
                        hs.iter().find_map(|h| {
                            (h.get("name")?.as_str()?.eq_ignore_ascii_case(name))
                                .then(|| h.get("value")?.as_str().map(str::to_string))
                                .flatten()
                        })
                    })
                };
                let internal = match v.get("internalDate") {
                    Some(Value::String(s)) => s.parse::<i64>().ok(),
                    Some(Value::Number(n)) => n.as_i64(),
                    _ => None,
                };
                Ok(Some(MessageMeta {
                    thread_id: v.get("threadId").and_then(Value::as_str).map(str::to_string),
                    from: header("From"),
                    to: header("To"),
                    subject: header("Subject"),
                    snippet: v.get("snippet").and_then(Value::as_str).map(str::to_string),
                    at: internal.and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok()),
                }))
            }
            Err(e) if e.contains("404") => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// One new message as the agent reads it: who, what and when, then its text (sanitised by the tool, cut at
    /// [`MAIL_TEXT_CHARS`]), between `<email>` tags the text can't close.
    async fn mail_block(&self, watch: &config::MailWatchConfig, m: &mail::NewMail) -> String {
        let label = self.cfg.account_label(&m.account);
        let thread = m.thread_id.as_deref().unwrap_or("");
        let address = self.cfg.accounts.get(&m.account).map(|a| a.address.as_str()).unwrap_or("");
        // The account's ID is what cards take; the label and address are for reading it with gog.
        let open = format!(
            "<email account=\"{}\" label=\"{label}\" address=\"{address}\" thread=\"{thread}\" message=\"{}\">",
            m.account, m.id
        );
        let Some(spec) = &watch.message_text else { return format!("{open}\n</email>") };
        let read = match self.account_ctx(&m.account) {
            Some(acct) => match tools::render(&spec.input, &json!({"account": acct, "message_id": m.id})) {
                Ok(input) => self.tools.call(spec, input).await,
                Err(e) => Err(e),
            },
            None => Err(format!("account {} isn't in the runner's config", m.account)),
        };
        let v = match read {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(account = %m.account, error = %e, "reading a new message's text");
                return format!("{open}\n(The runner couldn't read this message: {e})\n</email>");
            }
        };
        let field = |k: &str| v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(clean_line);
        let mut out = open;
        for (name, key) in [("From", "from"), ("To", "to"), ("Cc", "cc"), ("Subject", "subject"), ("Date", "date")] {
            if let Some(val) = field(key) {
                out.push_str(&format!("\n{name}: {val}"));
            }
        }
        // A tool that gives the text (`body`) puts it in, cut short. gog's gives only Gmail's snippet, a hint: the agent
        // reads the message itself.
        let text = match v.get("body").and_then(Value::as_str).map(str::trim).filter(|b| !b.is_empty()) {
            Some(body) => {
                let mut t: String = body.chars().take(MAIL_TEXT_CHARS).collect();
                if body.chars().count() > MAIL_TEXT_CHARS {
                    t.push_str(&format!("\n[… cut at {MAIL_TEXT_CHARS} characters: read the rest yourself]"));
                }
                t
            }
            None => {
                let snippet = v.get("snippet").and_then(Value::as_str).map(clean_line).unwrap_or_default();
                format!("Snippet (Gmail's preview of the first lines, not the email): {snippet}")
            }
        };
        out.push_str("\n\n");
        // No tag survives, in any case or spelling: the text can't close <email> or the delivery's own wrapper.
        out.push_str(&untagged(&text));
        out.push_str("\n</email>");
        out
    }

    /// What's known about a new message without reading its body. None when it no longer exists.
    async fn facts(
        &self,
        watch: &config::MailWatchConfig,
        m: &mail::NewMail,
    ) -> Option<(ItemFacts, Option<MessageMeta>)> {
        let mut f = ItemFacts {
            source: "gmail".into(),
            account: m.account.clone(),
            thread_key: m.thread_id.clone(),
            labels: m.labels.clone(),
            ..ItemFacts::default()
        };
        if watch.message.is_none() {
            return Some((f, None));
        }
        match self.message_meta(watch, &m.account, &m.id).await {
            Ok(Some(meta)) => {
                f.from = meta.from.clone();
                f.subject = meta.subject.clone();
                f.text = meta.snippet.clone();
                if let Some(t) = &meta.thread_id {
                    f.thread_key = Some(t.clone());
                }
                Some((f, Some(meta)))
            }
            Ok(None) => {
                tracing::info!(account = %m.account, message = %m.id, "a new message was already gone: not waking for it");
                None
            }
            Err(e) => {
                tracing::warn!(account = %m.account, error = %e, "reading a new message's headers");
                Some((f, None))
            }
        }
    }

    /// Every draft on each account, for the Drafts tab: listed with the drafts tool, each new one's headers read once
    /// (a draft gets a new message ID with every save), the whole list sent so drafts that were sent or deleted go.
    pub async fn sync_drafts(&self) {
        let Some(watch) = &self.cfg.mail_watch else { return };
        let Some(spec) = &watch.drafts else { return };
        for account in self.cfg.accounts.keys() {
            let Some(acct) = self.account_ctx(account) else { continue };
            let listed = match tools::render(&spec.input, &json!({"account": acct})) {
                Ok(input) => self.tools.call(spec, input).await,
                Err(e) => Err(e),
            };
            let listed = match listed {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(%account, error = %e, "listing drafts");
                    continue;
                }
            };
            let mut drafts = vec![];
            let mut complete = true;
            for d in listed.get("drafts").and_then(Value::as_array).into_iter().flatten() {
                let Some(mid) = d.pointer("/message/id").and_then(Value::as_str) else { continue };
                let cached = self.draft_cache.lock().expect("draft cache").get(mid).cloned();
                let item = match cached {
                    Some(item) => item,
                    None => match self.message_meta(watch, account, mid).await {
                        Ok(Some(meta)) => {
                            let item = mail_item(account, quietdeck_types::mail::MailKind::Draft, mid, meta)
                                .with_draft_id(d.get("id").and_then(Value::as_str));
                            self.draft_cache.lock().expect("draft cache").insert(mid.to_string(), item.clone());
                            item
                        }
                        Ok(None) => continue,
                        Err(e) => {
                            tracing::warn!(%account, error = %e, "reading a draft's headers");
                            complete = false;
                            continue;
                        }
                    },
                };
                drafts.push(item);
            }
            // A partial list would drop drafts that are still there: wait for a complete one.
            if !complete {
                continue;
            }
            if let Err(e) = self.deck.mail_drafts(account.clone(), drafts).await {
                tracing::debug!(%account, error = %e, "sending the drafts list");
            }
        }
    }

    /// Count new items, or while the person reads, ask the deck which may get through now.
    async fn sort_new(&self, state: &mut MailState, items: Vec<(String, ItemFacts)>) {
        let reading = self.reading.load(std::sync::atomic::Ordering::SeqCst);
        let verdicts = if reading {
            match self.deck.hold_check(items.iter().map(|(_, f)| f.clone()).collect()).await {
                Ok(v) if v.len() == items.len() => v,
                Ok(_) | Err(_) => {
                    tracing::info!("the deck didn't say what may get through: holding the new mail");
                    items.iter().map(|_| HoldVerdict::Hold).collect()
                }
            }
        } else {
            items.iter().map(|_| HoldVerdict::Hold).collect()
        };
        for ((account, f), v) in items.into_iter().zip(verdicts) {
            match v {
                HoldVerdict::Let { reason } if reading && !reason.is_empty() => {
                    let what = f.subject.as_deref().map(|s| format!(" \"{}\"", clean_line(s))).unwrap_or_default();
                    state.urgent.push(format!("{reason}: new mail on {}{what}", self.cfg.account_label(&account)));
                }
                _ => state.add(&account, Some(1)),
            }
        }
    }

    /// The deck says whether the person is reading the Inbox. Going idle lets the held wake-up go.
    pub async fn set_reading(&self, active: bool) {
        let was = self.reading.swap(active, std::sync::atomic::Ordering::SeqCst);
        if was == active {
            return;
        }
        tracing::info!(active, "the person's reading the Inbox changed");
        let mut state = self.mail.lock().await;
        self.wake_if_due(&mut state).await;
        state.save(&self.mail_state_path);
    }

    /// "Triage now": wake the agent for what's held, even while the person reads.
    pub async fn release_held(&self) {
        self.release.store(true, std::sync::atomic::Ordering::SeqCst);
        let mut state = self.mail.lock().await;
        self.wake_if_due(&mut state).await;
        state.save(&self.mail_state_path);
    }

    pub async fn round_done(&self) {
        let mut state = self.mail.lock().await;
        tracing::info!("the triage agent finished its round");
        state.waiting = false;
        self.wake_if_due(&mut state).await;
        state.save(&self.mail_state_path);
    }

    /// The deck says new chat messages arrived: count them into the next wake-up, and wake the agent now if it's
    /// between rounds.
    pub async fn messages_arrived(&self, channels: &[quietdeck_types::message::ChannelCount]) {
        let mut state = self.mail.lock().await;
        let reading = self.reading.load(std::sync::atomic::Ordering::SeqCst);
        let mut held = channels.to_vec();
        for c in &mut held {
            if reading && !c.let_through.is_empty() {
                for why in &c.let_through {
                    state.urgent.push(format!(
                        "{}: a new message in {} ({})",
                        clean_line(why),
                        clean_line(&c.channel_name),
                        clean_line(&c.source)
                    ));
                }
                c.count = c.count.saturating_sub(c.let_through.len() as u32);
            }
        }
        state.add_messages(&held);
        self.wake_if_due(&mut state).await;
        state.save(&self.mail_state_path);
    }

    /// Tell the deck what waits for the person's idle moment (nothing, when they aren't reading).
    async fn report_held(&self, state: &MailState, reading: bool) {
        let now = if reading { state.held(&self.cfg) } else { (None, 0) };
        if self.last_held.lock().expect("held lock").as_ref() == Some(&now) {
            return;
        }
        match self.deck.held(now.0.clone(), now.1).await {
            Ok(()) => *self.last_held.lock().expect("held lock") = Some(now),
            Err(e) => tracing::debug!(error = %e, "telling the deck what the runner holds"),
        }
    }

    pub async fn round_status(&self) -> quietdeck_types::api::RoundStatus {
        self.mail.lock().await.status(&self.cfg)
    }

    async fn wake_if_due(&self, state: &mut MailState) {
        let reading = self.reading.load(std::sync::atomic::Ordering::SeqCst)
            && !self.release.load(std::sync::atomic::Ordering::SeqCst);
        self.report_held(state, reading).await;
        // Overnight nothing wakes the agent: it all waits, and the first check after quiet hours sends one wake-up.
        if self.cfg.quiet_hours.as_ref().is_some_and(|q| q.contains(jiff::Timestamp::now())) {
            return;
        }
        // No waiting for `round done`: an agent that's busy reads the wake-up on its next turn, so every check with
        // something new wakes it (Nathan, 25 November: "why would anything block a wake up ever? just send the
        // message!"). Only the person reading the Inbox holds new items back, and only from the Inbox.
        if reading {
            // Only what the deck let through goes now; the rest waits for the person's idle moment.
            let Some(text) = state.urgent_text(&self.cfg) else { return };
            match self.agent.tell(DeliveryKind::Wakeup, text).await {
                Ok(()) => {
                    tracing::info!(
                        items = state.urgent.len(),
                        "woke the triage agent for what got through while the person reads"
                    );
                    state.woke_at = Some(jiff::Timestamp::now());
                    state.urgent.clear();
                }
                Err(e) => tracing::info!(error = %e, "the triage agent isn't reachable for its wake-up"),
            }
            return;
        }
        let Some(mut text) = state
            .wakeup_text(&self.cfg)
            .or_else(|| (!state.urgent.is_empty()).then(|| "Run your triage round.".to_string()))
        else {
            return;
        };
        if !state.urgent.is_empty() {
            text = format!("Also: {}. {text}", state.urgent.join("; "));
        }
        let mail = state.mail_counts(&self.cfg).unwrap_or_default();
        let messages = state.message_counts().unwrap_or_default();
        match self.agent.tell(DeliveryKind::Wakeup, text).await {
            Ok(()) => {
                tracing::info!(%mail, %messages, "woke the triage agent");
                state.woke_at = Some(jiff::Timestamp::now());
                state.clear_pending();
                self.release.store(false, std::sync::atomic::Ordering::SeqCst);
                self.report_held(state, false).await;
            }
            // Kept: the next check tries again, with anything newer added.
            Err(e) => tracing::info!(error = %e, "the triage agent isn't reachable for its wake-up"),
        }
    }
}

/// Gmail's own labels, which a labels list may leave out.
/// Values tools need in another shape than the action has: `story_number` (the number in "sc-1234" or "1234", for
/// trackers whose tools take a number), and `agent_labels` (["agent:<tab>"] when the task names the tab that works on
/// it) with `agent_label_objects` ([{"name": "agent:<tab>"}], for tools that take label objects); both are left out
/// without a tab, so the tool's key goes too.
fn derived_values(a: &Action, v: &mut Value) {
    let story_id = match a {
        Action::Story(StoryOp::Comment { story_id, .. }) => Some(story_id.as_str()),
        Action::Task { story_id, .. } => story_id.as_deref(),
        _ => None,
    };
    if let Some(n) = story_id.and_then(story_number) {
        v["story_number"] = json!(n);
    }
    if let Action::Task { agent_tab: Some(tab), .. } = a {
        v["agent_labels"] = json!([format!("agent:{tab}")]);
        v["agent_label_objects"] = json!([{"name": format!("agent:{tab}")}]);
    }
}

/// New messages whose text one wake-up carries; more are named as waiting.
const MAIL_TEXT_MAX: usize = 25;
/// Each message's text is cut here: enough to triage, never a whole newsletter.
const MAIL_TEXT_CHARS: usize = 3000;

/// Untrusted text with every angle bracket swapped for a look-alike, so nothing in it reads as a tag.
fn untagged(s: &str) -> String {
    s.replace('<', "‹").replace('>', "›")
}

/// A header value on one line, short, with nothing that could look like a tag.
fn clean_line(s: &str) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let one = untagged(&one);
    if one.chars().count() > 300 { one.chars().take(300).collect::<String>() + "…" } else { one }
}

fn story_number(id: &str) -> Option<u64> {
    let id = id.trim();
    let digits = id.get(..3).filter(|p| p.eq_ignore_ascii_case("sc-")).map_or(id, |_| &id[3..]);
    digits.parse().ok()
}

fn is_system_label(l: &str) -> bool {
    matches!(l, "INBOX" | "UNREAD" | "STARRED" | "IMPORTANT" | "SPAM" | "TRASH")
}

/// Start the runner's task: released decisions and round-done signals as they come, and the mail watch on its
/// interval. One thing at a time.
pub fn spawn(runner: Arc<Runner>) -> RunnerHandle {
    let (tx, mut rx) = mpsc::channel::<Signal>(16);
    let task = runner.clone();
    tokio::spawn(async move {
        let every = task
            .cfg
            .mail_watch
            .as_ref()
            .and_then(|m| config::parse_duration(&m.every).ok())
            .unwrap_or(Duration::from_secs(30));
        let mut tick = tokio::time::interval(every);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // The Drafts tab's list, every 5 minutes (and at start).
        let mut drafts_tick = tokio::time::interval(Duration::from_secs(300));
        drafts_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                s = rx.recv() => match s {
                    Some(Signal::Released) => match task.process().await {
                        Ok(o) if o != BatchOutcome::default() => tracing::info!(
                            claimed = o.claimed.len(), handed = o.handed.len(), done = o.done, failed = o.failed,
                            stopped = o.stopped, "runner batch"),
                        Ok(_) => {}
                        Err(e) => tracing::warn!(error = %e, "runner: taking released decisions"),
                    },
                    Some(Signal::RoundDone) => task.round_done().await,
                    None => return,
                },
                _ = drafts_tick.tick() => task.sync_drafts().await,
                _ = tick.tick() => {
                    if !task.deck.connected() {
                        tracing::debug!("not connected to the deck yet: mail waits for the next check");
                        continue;
                    }
                    task.check_mail().await;
                    // Also take what's waiting on every tick: a release nudge the deck couldn't deliver (the node was
                    // reconnecting, a timeout) then costs at most one tick, never a stranded card.
                    match task.process().await {
                        Ok(o) if o != BatchOutcome::default() => tracing::info!(
                            claimed = o.claimed.len(), handed = o.handed.len(), done = o.done, failed = o.failed,
                            stopped = o.stopped, "runner batch"),
                        Ok(_) => {}
                        Err(e) => tracing::debug!(error = %e, "runner: taking released decisions on the tick"),
                    }
                }
            }
        }
    });
    RunnerHandle { runner, tx }
}

/// The runner's token for Executor, from wherever node.yaml says.
pub fn executor_token(cfg: &config::ExecutorConfig) -> anyhow::Result<String> {
    if let Some(var) = &cfg.token_env {
        let t = std::env::var(var).map(|t| t.trim().to_string()).unwrap_or_default();
        if t.is_empty() {
            anyhow::bail!("runner.executor.token_env names {var}, which isn't set");
        }
        return Ok(t);
    }
    let path =
        cfg.token_file.as_deref().ok_or_else(|| anyhow::anyhow!("runner.executor needs token_file or token_env"))?;
    crate::config::read_secret_file(&crate::config::expand(path))
}
