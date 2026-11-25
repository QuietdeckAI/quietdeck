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
mod leases;
mod mail;
mod tools;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use quietdeck_types::action::{Action, StoryOp};
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
pub use deck::{AgentInbox, DeckApi, LinkDeck, LocalAgent};
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
        }
    }
}

pub struct Runner {
    pub cfg: RunnerConfig,
    deck: Arc<dyn DeckApi>,
    tools: Arc<dyn Tools>,
    agent: Arc<dyn AgentInbox>,
    mail_state_path: PathBuf,
    mail: Mutex<MailState>,
    send_as: Mutex<HashMap<String, (Instant, Vec<String>)>>,
    /// The person is reading the Inbox (the deck says so): new items wait unless the deck lets them through.
    reading: std::sync::atomic::AtomicBool,
    /// "Triage now": the next wake-up goes even while the person reads.
    release: std::sync::atomic::AtomicBool,
    /// What the deck was last told the runner holds, so it's told only of changes.
    last_held: std::sync::Mutex<Option<(Option<String>, u64)>>,
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
    /// Oldest first, drafts left out.
    messages: Vec<(String, Vec<String>)>,
}

impl ThreadMeta {
    fn newest(&self) -> Option<&str> {
        self.messages.last().map(|(id, _)| id.as_str())
    }
    fn any_label(&self, label: &str) -> bool {
        self.messages.iter().any(|(_, l)| l.iter().any(|x| x == label))
    }
}

enum Sorted {
    Claim,
    Handoff { reason: String, no_handler: Option<ActionType> },
}

fn handoff(reason: impl Into<String>) -> Sorted {
    Sorted::Handoff { reason: reason.into(), no_handler: None }
}

fn thread_of(a: &Action) -> Option<(&str, &str)> {
    match a {
        Action::Archive { account, thread_id } | Action::MarkRead { account, thread_id } => Some((account, thread_id)),
        Action::Label { account, thread_id, .. } => Some((account, thread_id)),
        Action::Draft(c) => Some((&c.account, &c.thread_id)),
        _ => None,
    }
}

/// Action types the runner carries out through the deck and the orchestrator rather than a configured tool.
fn built_in(t: ActionType) -> bool {
    matches!(t, ActionType::Forward | ActionType::Resume | ActionType::NewSession)
}

fn released_actions(r: &Released) -> Vec<&ActionRecord> {
    r.actions.iter().filter(|a| a.status == ActionStatus::Released).collect()
}

fn describe(a: &Action) -> &'static str {
    match a {
        Action::Archive { .. } => "Archive",
        Action::MarkRead { .. } => "Mark read",
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
            mail_state_path,
            mail,
            send_as: Mutex::default(),
            reading: Default::default(),
            release: Default::default(),
            last_held: Default::default(),
        }
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
                        (!labels.iter().any(|l| l == "DRAFT")).then_some((id, labels))
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
                let Some(prepared_on) = r.card.input.last_message_ref.as_deref() else {
                    return handoff("the card names no message, so the thread can't be checked for newer ones");
                };
                if meta.messages.is_empty() {
                    return handoff("the thread no longer exists");
                }
                if meta.newest() != Some(prepared_on) {
                    let when = r.card.prepared_at.to_zoned_datetime(jiff::tz::TimeZone::system()).strftime("%H:%M");
                    return handoff(format!("newer message in thread since {when}"));
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
        let mut failures: Vec<String> = Vec::new();
        let mut attempted = 0usize;
        for r in taken.iter().filter(|r| held.contains(&r.decision.id)) {
            out.claimed.push(r.decision.id);
            let mut reports = Vec::new();
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
                    self.run_action(r, &rec.action, &threads).await
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
                        failures.push(format!(
                            "{} of '{}' ({}): {error}. Card c_{} is waiting with Retry.",
                            describe(&rec.action),
                            r.card.input.subject,
                            self.cfg.account_label(&r.card.input.account),
                            r.card.id
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
        if !handoffs.is_empty() {
            let n = handoffs.len();
            let reasons: Vec<String> = handoffs
                .iter()
                .filter_map(|h| {
                    let r = taken.iter().find(|r| r.decision.id == h.decision_id)?;
                    Some(format!("'{}' ({})", r.card.input.subject, h.reason))
                })
                .collect();
            let body = format!(
                "{n} card{} handed to you: {}. Take {} with `quietdeck decisions take`.",
                if n == 1 { "" } else { "s" },
                reasons.join("; "),
                if n == 1 { "it" } else { "them" }
            );
            match self.agent.tell(DeliveryKind::Handoff, body).await {
                Ok(()) => tracing::info!(cards = n, "handed cards to the triage agent"),
                Err(e) => tracing::info!(error = %e, "the agent will find its hand-offs with `decisions take`"),
            }
        }
        if !failures.is_empty() {
            let body = format!("Runner: {} of {attempted} failed. {}", failures.len(), failures.join(" "));
            if let Err(e) = self.agent.tell(DeliveryKind::RunnerReport, body).await {
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
        let new = mail::check(&self.cfg, watch, self.tools.as_ref(), &mut state).await;
        if !new.is_empty() {
            let mut facts = Vec::with_capacity(new.len());
            for m in &new {
                // A message that's already gone (Gmail took back a follow-up nudge) isn't mail.
                if let Some(f) = self.facts(watch, m).await {
                    facts.push((m.account.clone(), f));
                }
            }
            self.sort_new(&mut state, facts).await;
        }
        self.wake_if_due(&mut state).await;
        state.save(&self.mail_state_path);
    }

    /// What's known about a new message without reading its body. None when it no longer exists.
    async fn facts(&self, watch: &config::MailWatchConfig, m: &mail::NewMail) -> Option<ItemFacts> {
        let mut f = ItemFacts {
            source: "gmail".into(),
            account: m.account.clone(),
            thread_key: m.thread_id.clone(),
            labels: m.labels.clone(),
            ..ItemFacts::default()
        };
        let Some(spec) = &watch.message else { return Some(f) };
        let acct = self.account_ctx(&m.account)?;
        let input = match tools::render(&spec.input, &json!({"account": acct, "message_id": m.id})) {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!(error = %e, "runner.mail_watch.message input");
                return Some(f);
            }
        };
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
                f.from = header("From");
                f.subject = header("Subject");
                f.text = v.get("snippet").and_then(Value::as_str).map(str::to_string);
                if let Some(t) = v.get("threadId").and_then(Value::as_str) {
                    f.thread_key = Some(t.to_string());
                }
                Some(f)
            }
            Err(e) if e.contains("404") => {
                tracing::info!(account = %m.account, message = %m.id, "a new message was already gone: not waking for it");
                None
            }
            Err(e) => {
                tracing::warn!(account = %m.account, error = %e, "reading a new message's headers");
                Some(f)
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
                    let what = f.subject.as_deref().map(|s| format!(" \"{s}\"")).unwrap_or_default();
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
                    state.urgent.push(format!("{why}: a new message in {} ({})", c.channel_name, c.source));
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
        if state.waiting {
            return;
        }
        if reading {
            // Only what the deck let through goes now; the rest waits for the person's idle moment.
            let Some(text) = state.urgent_text(&self.cfg) else { return };
            match self.agent.tell(DeliveryKind::Wakeup, text).await {
                Ok(()) => {
                    tracing::info!(
                        items = state.urgent.len(),
                        "woke the triage agent for what got through while the person reads"
                    );
                    state.waiting = true;
                    state.woke_at = Some(jiff::Timestamp::now());
                    state.urgent.clear();
                }
                Err(e) => tracing::info!(error = %e, "the triage agent isn't reachable for its wake-up"),
            }
            return;
        }
        let Some(mut text) = state.wakeup_text(&self.cfg).or_else(|| {
            (!state.urgent.is_empty())
                .then(|| "Run your triage round, then `quietdeck round done` so the next wake-up can come.".to_string())
        }) else {
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
                state.waiting = true;
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
                _ = tick.tick() => {
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
