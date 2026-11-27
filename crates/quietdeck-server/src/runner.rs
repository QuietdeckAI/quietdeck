//! The deck's side of the runner (section 1, "The runner"): releasing decisions to it, answering its requests over
//! the node connection, the send-as check when a card is posted, and passing on `round done`.
//!
//! The runner speaks for its agent: it may take and run only what the agent's own cards hold, exactly as the agent
//! could, and nothing it asks for reaches another session's cards.

use std::time::Duration;

use quietdeck_types::SessionRef;
use quietdeck_types::action::Action;
use quietdeck_types::api::{Changed, RoundStatus};
use quietdeck_types::card::RunBy;
use quietdeck_types::node::{DeckRequest, NodeRequest, Reply, SendAsCheck};

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::store::{decisions, nodes};

const ASK_TIMEOUT: Duration = Duration::from_secs(10);

/// The node whose runner works for the agent `tab@node`, when it's connected now: the agent's own node, or a runner
/// kept on a node of its own.
async fn connected_runner(state: &AppState, node: &str, tab: &str) -> Option<String> {
    let (runner_node, _) = nodes::runner_for_agent(&state.db, node, tab).await.ok().flatten()?;
    state.hub.is_connected(&runner_node).await.then_some(runner_node)
}

/// Decisions on `tab@node`'s cards were released: tell the runner that works for that agent. False when there's no
/// such runner or it didn't answer, so the agent is told instead.
pub async fn notify(state: &AppState, node: &str, tab: &str) -> bool {
    let Some(runner) = connected_runner(state, node, tab).await else { return false };
    match state.hub.request(&runner, DeckRequest::DecisionsReleased, ASK_TIMEOUT).await {
        Ok(reply) => reply.into_result::<()>().is_ok(),
        Err(e) => {
            tracing::info!(%runner, error = %e, "the runner didn't take its release: telling the agent");
            false
        }
    }
}

/// `quietdeck round done` from `me`: the runner that works for `me` may send the next wake-up. Says whether a runner
/// was told.
pub async fn round_done(state: &AppState, me: &SessionRef) -> AppResult<bool> {
    let Some(runner) = connected_runner(state, &me.node, &me.tab).await else { return Ok(false) };
    let reply = state
        .hub
        .request(&runner, DeckRequest::RoundDone, ASK_TIMEOUT)
        .await
        .map_err(|e| AppError::Conflict(format!("the runner on {runner} didn't answer: {e}")))?;
    reply.into_result::<()>().map_err(AppError::Conflict)?;
    Ok(true)
}

/// `quietdeck round status` from `me`: the mail watch of the runner on its machine, when it runs for `me`.
pub async fn round_status(state: &AppState, me: &SessionRef) -> AppResult<RoundStatus> {
    let Some(runner) = connected_runner(state, &me.node, &me.tab).await else {
        return Ok(RoundStatus { runner: false, waiting: false, woke_at: None, checked_at: None, accounts: vec![] });
    };
    let reply = state
        .hub
        .request(&runner, DeckRequest::RoundStatus, ASK_TIMEOUT)
        .await
        .map_err(|e| AppError::Conflict(format!("the runner on {runner} didn't answer: {e}")))?;
    reply.into_result::<RoundStatus>().map_err(AppError::Conflict)
}

/// A draft or reply posted on a card names a `from` address. With a runner on the posting machine, it must be one
/// the account can send as, from the send-as list of the runner that works for the poster. Without one (or with it offline), the skill checks
/// through its own access, and the runner checks again before it sends anything.
/// Quietdeck solo: the actions its runner carries out (solo.yaml's `actions`). None on any other deck, or before the
/// runner has connected.
pub async fn solo_actions(state: &AppState) -> AppResult<Option<Vec<quietdeck_types::ActionType>>> {
    if !state.solo {
        return Ok(None);
    }
    Ok(nodes::any_runner_info(&state.db).await?.map(|r| r.eligible))
}

/// On a solo deck the runner is the only thing that can carry an action out, so a card proposing one it's set not to
/// (an archive under `actions: [draft]`) would only fail once approved. It's refused when posted instead, saying what
/// to do: post the card with no actions, and tell the person to do it in Gmail. Other decks hand such actions to the
/// agent, as before.
pub async fn check_solo_actions(state: &AppState, actions: &[Action]) -> AppResult<()> {
    let Some(allowed) = solo_actions(state).await? else { return Ok(()) };
    for a in actions {
        let t = a.action_type();
        if !allowed.contains(&t) {
            let list = allowed.iter().map(|x| x.as_str()).collect::<Vec<_>>().join(", ");
            let doing = match t.as_str() {
                "archive" => "archive it",
                "mark_read" => "mark it read",
                "label" => "label it",
                "draft" => "reply",
                _ => "do it",
            };
            return Err(AppError::BadRequest(format!(
                "{} is off on this deck (actions: {list}): post the card with no actions, and say in its summary to \
                 {doing} in Gmail yourself",
                t.as_str()
            )));
        }
    }
    Ok(())
}

pub async fn check_send_as(state: &AppState, me: &SessionRef, actions: &[Action]) -> AppResult<()> {
    let composes = actions.iter().filter_map(|a| match a {
        Action::Draft(c) | Action::Reply(c) => Some(c),
        _ => None,
    });
    for c in composes {
        let Some(runner) = connected_runner(state, &me.node, &me.tab).await else { return Ok(()) };
        let req = DeckRequest::SendAs { account: c.account.clone(), from: c.from.clone() };
        let check = match state.hub.request(&runner, req, ASK_TIMEOUT).await {
            Ok(reply) => match reply.into_result::<SendAsCheck>() {
                Ok(check) => check,
                Err(e) => {
                    tracing::info!(%runner, error = %e, "send-as check unavailable: the runner checks again");
                    return Ok(());
                }
            },
            Err(_) => return Ok(()),
        };
        if !check.allowed {
            return Err(AppError::BadRequest(format!(
                "{} can't send as {}: it can send as {}",
                c.account,
                c.from,
                check.addresses.join(", ")
            )));
        }
    }
    Ok(())
}

/// Tell every connected runner something. One that's offline or doesn't answer is logged and skipped.
pub async fn tell_runners(state: &AppState, req: DeckRequest) -> u32 {
    let rows: Vec<(String,)> =
        match sqlx::query_as("SELECT name FROM nodes WHERE runner_info IS NOT NULL AND revoked_at IS NULL")
            .fetch_all(&state.db)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "finding the runners to tell");
                return 0;
            }
        };
    let mut told = 0;
    for (node,) in rows {
        if !state.hub.is_connected(&node).await {
            continue;
        }
        match state.hub.request(&node, req.clone(), ASK_TIMEOUT).await {
            Ok(reply) => match reply.into_result::<()>() {
                Ok(()) => told += 1,
                Err(e) => tracing::warn!(%node, error = %e, "the runner refused"),
            },
            Err(e) => tracing::warn!(%node, error = %e, "the runner didn't answer"),
        }
    }
    told
}

/// A request from a node's runner. Refused unless the node has a runner, which acts for its agent's tab only.
pub async fn answer(state: &AppState, node: &str, request: NodeRequest) -> Reply {
    match answer_inner(state, node, request).await {
        Ok(reply) => reply,
        Err(e) => Reply::Err { error: e.to_string() },
    }
}

async fn answer_inner(state: &AppState, node: &str, request: NodeRequest) -> AppResult<Reply> {
    let Some(info) = nodes::runner_info(&state.db, node).await? else {
        return Err(AppError::Forbidden(format!("{node} has no runner switched on")));
    };
    let (agent_node, agent_tab) = info.agent(node);
    let agent = SessionRef { node: agent_node, tab: agent_tab, session_id: None };
    let caller = quietdeck_types::api::Caller::Node { node: node.to_string() };
    Ok(match request {
        NodeRequest::MintToken { .. } | NodeRequest::MessagesIngest { .. } => {
            unreachable!("answered by the node connection")
        }
        NodeRequest::RunnerTake => Reply::ok(&decisions::runner_take(&state.db, &agent).await?),
        NodeRequest::RunnerSort { claims, handoffs } => {
            let held = decisions::runner_sort(&state.db, &agent, &claims, &handoffs).await?;
            state.changed(Changed::Cards);
            Reply::ok(&held)
        }
        NodeRequest::ActionStart { action_id } => {
            let a = decisions::start(&state.db, &agent, RunBy::Runner, action_id).await?;
            state.changed(Changed::Cards);
            Reply::ok(&a)
        }
        NodeRequest::ActionsReport { reports } => {
            if reports.is_empty() || reports.len() > 500 {
                return Err(AppError::BadRequest("report 1 to 500 actions at a time".into()));
            }
            let r = decisions::report(&state.db, &agent, RunBy::Runner, &reports).await?;
            state.changed(Changed::Cards);
            Reply::ok(&r)
        }
        NodeRequest::Relay { to, kind, body } => {
            match crate::relay::send(&state.db, &state.hub, &to, kind, body).await {
                Ok(d) => Reply::ok(&d),
                Err(e) => Reply::Err { error: e.to_string() },
            }
        }
        NodeRequest::StartSession { spec } => Reply::ok(&crate::control::start(state, &caller, spec).await?),
        NodeRequest::HoldCheck { items } => Reply::ok(&crate::reading::hold_check(&state.db, &items).await?),
        NodeRequest::MailSent { items } => {
            crate::mail::sent(state, &items).await?;
            Reply::ok(&())
        }
        NodeRequest::MailDrafts { account, drafts } => {
            crate::mail::drafts(state, &account, &drafts).await?;
            Reply::ok(&())
        }
        NodeRequest::ThreadsLeftInbox { left, back } => {
            Reply::ok(&crate::mail::threads_left(state, &left, &back).await?)
        }
        NodeRequest::RunnerHeld { summary, count } => {
            crate::reading::held(state, node, summary, count).await?;
            Reply::ok(&())
        }
        NodeRequest::ResumeSession { node: on, session_id, tab, cwd } => {
            Reply::ok(&crate::control::resume_by_id(state, &caller, &on, &session_id, &tab, &cwd).await?)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use quietdeck_types::ActionType;
    use quietdeck_types::card::{ActionRecord, ActionReport, ActionResult, CardInput, CardState, Released};
    use quietdeck_types::decision::{ApproveAll, CardDecision};
    use quietdeck_types::node::{Handoff, RunnerInfo};
    use serde_json::json;

    async fn with_runner(state: &AppState, node: &str, tab: &str) {
        sqlx::query("INSERT INTO nodes (name, token_hash, created_at) VALUES (?, ?, 0)")
            .bind(node)
            .bind(format!("hash-{node}"))
            .execute(&state.db)
            .await
            .unwrap();
        let info = RunnerInfo { eligible: vec![ActionType::Archive], agent_tab: tab.into(), accounts: vec![] };
        nodes::hello(&state.db, node, "test", Some(&info)).await.unwrap();
    }

    async fn approved_card(state: &AppState, tab: &str) -> i64 {
        let me = SessionRef { node: "server".into(), tab: tab.into(), session_id: None };
        let input: CardInput = serde_json::from_value(json!({"account": "personal", "thread_key": "t1",
            "subject": "Invoice", "summary": "s", "section": "actions", "reason": "r",
            "actions": [{"type": "archive", "account": "personal", "thread_id": "t1"}]}))
        .unwrap();
        let card = crate::store::cards::add(&state.db, &me, &input, CardState::Open, None).await.unwrap().card;
        let d = CardDecision {
            card_id: card.id,
            version: 1,
            kind: quietdeck_types::card::DecisionKind::Approve,
            fields: None,
            text: None,
        };
        decisions::approve_all(&state.db, &ApproveAll { cards: vec![d], timeline: vec![], single: None }, 0)
            .await
            .unwrap();
        decisions::release_due(&state.db).await.unwrap();
        card.id
    }

    /// On a drafts-only solo deck, an archive card is refused when posted with a hint, and the same card with no
    /// actions is taken and closes once approved, so nothing approved fails.
    #[tokio::test]
    async fn a_solo_deck_refuses_actions_its_runner_wont_carry_out_and_says_what_to_do() {
        let mut state = AppState::for_tests().await;
        state.solo = true;
        sqlx::query("INSERT INTO nodes (name, token_hash, created_at) VALUES ('solo', 'h', 0)")
            .execute(&state.db)
            .await
            .unwrap();
        let info = RunnerInfo { eligible: vec![ActionType::Draft], agent_tab: "email triage".into(), accounts: vec![] };
        nodes::hello(&state.db, "solo", "test", Some(&info)).await.unwrap();
        let me = SessionRef { node: "solo".into(), tab: "email triage".into(), session_id: None };
        let with_archive: CardInput = serde_json::from_value(json!({"account": "personal", "thread_key": "t1",
            "subject": "Receipt", "summary": "Receipt", "section": "archive_list", "reason": "r",
            "actions": [{"type": "archive", "account": "personal", "thread_id": "t1"}]}))
        .unwrap();
        let e = crate::ops::add_card(&state, &me, with_archive).await.unwrap_err().to_string();
        assert!(e.contains("archive is off on this deck (actions: draft): post the card with no actions"), "{e}");
        assert!(e.contains("archive it in Gmail yourself"), "{e}");
        assert_eq!(solo_actions(&state).await.unwrap(), Some(vec![ActionType::Draft]));

        let without: CardInput = serde_json::from_value(json!({"account": "personal", "thread_key": "t1",
            "subject": "Receipt", "summary": "Receipt: archive it in Gmail yourself", "section": "archive_list",
            "reason": "r", "actions": []}))
        .unwrap();
        let card = crate::ops::add_card(&state, &me, without).await.unwrap().card;
        let d = CardDecision {
            card_id: card.id,
            version: card.version,
            kind: quietdeck_types::card::DecisionKind::Approve,
            fields: None,
            text: None,
        };
        decisions::approve_all(&state.db, &ApproveAll { cards: vec![d], timeline: vec![], single: None }, 0)
            .await
            .unwrap();
        decisions::release_due(&state.db).await.unwrap();
        let after = crate::store::cards::get(&state.db, card.id).await.unwrap().unwrap();
        assert_ne!(after.state, CardState::Open, "it closes once approved");

        // Other decks hand such actions to the agent, as before.
        state.solo = false;
        assert_eq!(solo_actions(&state).await.unwrap(), None);
    }

    fn ok<T: serde::de::DeserializeOwned>(r: Reply) -> T {
        r.into_result().unwrap()
    }

    #[tokio::test]
    async fn a_runner_takes_claims_starts_and_reports_through_the_node_connection() {
        let state = AppState::for_tests().await;
        with_runner(&state, "server", "email triage").await;
        let card = approved_card(&state, "email triage").await;
        let taken: Vec<Released> = ok(answer(&state, "server", NodeRequest::RunnerTake).await);
        assert_eq!(taken.len(), 1);
        let held: Vec<i64> = ok(answer(
            &state,
            "server",
            NodeRequest::RunnerSort { claims: vec![taken[0].decision.id], handoffs: vec![] },
        )
        .await);
        assert_eq!(held.len(), 1);
        let id = taken[0].actions[0].id;
        let rec: ActionRecord = ok(answer(&state, "server", NodeRequest::ActionStart { action_id: id }).await);
        assert_eq!(rec.action, taken[0].actions[0].action);
        let reports =
            vec![ActionReport { action_id: id, result: ActionResult::Done { outcome: Some(json!({"via": "x"})) } }];
        let _: serde_json::Value = ok(answer(&state, "server", NodeRequest::ActionsReport { reports }).await);
        let c = crate::store::cards::get(&state.db, card).await.unwrap().unwrap();
        assert_eq!(c.state, CardState::Done);
        assert_eq!(c.action_records[0].run_by, Some(quietdeck_types::card::RunBy::Runner));
    }

    #[tokio::test]
    async fn a_runner_on_a_node_of_its_own_works_for_its_agent_elsewhere() {
        let state = AppState::for_tests().await;
        // The agent posts from "server"; its runner lives on "server-runner", under a user the agent can't read.
        with_runner(&state, "server-runner", "email triage@server").await;
        let card = approved_card(&state, "email triage").await;
        let taken: Vec<Released> = ok(answer(&state, "server-runner", NodeRequest::RunnerTake).await);
        assert_eq!(taken.len(), 1, "it takes the agent's cards");
        assert_eq!(taken[0].card.id, card);
        let found = nodes::runner_for_agent(&state.db, "server", "email triage").await.unwrap();
        assert_eq!(found.map(|(n, _)| n).as_deref(), Some("server-runner"));
        // A plain tab still means the runner's own node.
        with_runner(&state, "laptop", "email triage").await;
        let taken: Vec<Released> = ok(answer(&state, "laptop", NodeRequest::RunnerTake).await);
        assert!(taken.is_empty(), "laptop's runner works for email triage@laptop, not @server");
    }

    #[test]
    fn the_agent_is_on_the_runners_node_unless_named() {
        let info = |t: &str| RunnerInfo { eligible: vec![], agent_tab: t.into(), accounts: vec![] };
        assert_eq!(info("triage").agent("n1"), ("n1".to_string(), "triage".to_string()));
        assert_eq!(info("email triage@n2").agent("n1"), ("n2".to_string(), "email triage".to_string()));
        assert_eq!(info("odd@").agent("n1"), ("n1".to_string(), "odd@".to_string()));
    }

    #[tokio::test]
    async fn a_node_without_a_runner_or_another_tabs_cards_get_nothing() {
        let state = AppState::for_tests().await;
        approved_card(&state, "email triage").await;
        let r = answer(&state, "laptop", NodeRequest::RunnerTake).await;
        assert!(r.into_result::<Vec<Released>>().unwrap_err().contains("no runner"));
        with_runner(&state, "server", "other tab").await;
        let taken: Vec<Released> = ok(answer(&state, "server", NodeRequest::RunnerTake).await);
        assert!(taken.is_empty(), "the runner acts for its own agent's cards only");
    }

    #[tokio::test]
    async fn hand_offs_reach_the_agents_take() {
        let state = AppState::for_tests().await;
        with_runner(&state, "server", "email triage").await;
        approved_card(&state, "email triage").await;
        let taken: Vec<Released> = ok(answer(&state, "server", NodeRequest::RunnerTake).await);
        let h = Handoff {
            decision_id: taken[0].decision.id,
            reason: "newer message in thread since 09:12".into(),
            no_handler: None,
        };
        let _: Vec<i64> =
            ok(answer(&state, "server", NodeRequest::RunnerSort { claims: vec![], handoffs: vec![h] }).await);
        let me = SessionRef { node: "server".into(), tab: "email triage".into(), session_id: None };
        let mine = decisions::take(&state.db, &me).await.unwrap();
        assert_eq!(mine[0].handoff_reason.as_deref(), Some("newer message in thread since 09:12"));
    }
}
