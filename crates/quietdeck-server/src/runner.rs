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
use quietdeck_types::node::{DeckRequest, NodeRequest, Reply, RunnerInfo, SendAsCheck};

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::store::{decisions, nodes};

const ASK_TIMEOUT: Duration = Duration::from_secs(10);

/// The node's runner, if it has one and it's connected now.
async fn connected_runner(state: &AppState, node: &str) -> Option<RunnerInfo> {
    if !state.hub.is_connected(node).await {
        return None;
    }
    nodes::runner_info(&state.db, node).await.ok().flatten()
}

/// Decisions on `tab@node`'s cards were released: tell that node's runner, when it runs for that tab. False when
/// there's no such runner or it didn't answer, so the agent is told instead.
pub async fn notify(state: &AppState, node: &str, tab: &str) -> bool {
    match connected_runner(state, node).await {
        Some(r) if r.agent_tab == tab => {
            match state.hub.request(node, DeckRequest::DecisionsReleased, ASK_TIMEOUT).await {
                Ok(reply) => reply.into_result::<()>().is_ok(),
                Err(e) => {
                    tracing::info!(%node, error = %e, "the runner didn't take its release: telling the agent");
                    false
                }
            }
        }
        _ => false,
    }
}

/// `quietdeck round done` from `me`: the runner on its machine may send the next wake-up. Says whether a runner
/// was told.
pub async fn round_done(state: &AppState, me: &SessionRef) -> AppResult<bool> {
    match connected_runner(state, &me.node).await {
        Some(r) if r.agent_tab == me.tab => {
            let reply = state
                .hub
                .request(&me.node, DeckRequest::RoundDone, ASK_TIMEOUT)
                .await
                .map_err(|e| AppError::Conflict(format!("the runner on {} didn't answer: {e}", me.node)))?;
            reply.into_result::<()>().map_err(AppError::Conflict)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// `quietdeck round status` from `me`: the mail watch of the runner on its machine, when it runs for `me`.
pub async fn round_status(state: &AppState, me: &SessionRef) -> AppResult<RoundStatus> {
    match connected_runner(state, &me.node).await {
        Some(r) if r.agent_tab == me.tab => {
            let reply = state
                .hub
                .request(&me.node, DeckRequest::RoundStatus, ASK_TIMEOUT)
                .await
                .map_err(|e| AppError::Conflict(format!("the runner on {} didn't answer: {e}", me.node)))?;
            reply.into_result::<RoundStatus>().map_err(AppError::Conflict)
        }
        _ => Ok(RoundStatus { runner: false, waiting: false, woke_at: None, checked_at: None, accounts: vec![] }),
    }
}

/// A draft or reply posted on a card names a `from` address. With a runner on the posting machine, it must be one
/// the account can send as, from the runner's send-as list. Without one (or with it offline), the skill checks
/// through its own access, and the runner checks again before it sends anything.
pub async fn check_send_as(state: &AppState, me: &SessionRef, actions: &[Action]) -> AppResult<()> {
    let composes = actions.iter().filter_map(|a| match a {
        Action::Draft(c) | Action::Reply(c) => Some(c),
        _ => None,
    });
    for c in composes {
        if connected_runner(state, &me.node).await.is_none() {
            return Ok(());
        }
        let req = DeckRequest::SendAs { account: c.account.clone(), from: c.from.clone() };
        let check = match state.hub.request(&me.node, req, ASK_TIMEOUT).await {
            Ok(reply) => match reply.into_result::<SendAsCheck>() {
                Ok(check) => check,
                Err(e) => {
                    tracing::info!(node = %me.node, error = %e, "send-as check unavailable: the runner checks again");
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
    let agent = SessionRef { node: node.to_string(), tab: info.agent_tab.clone(), session_id: None };
    let caller = quietdeck_types::api::Caller::Node { node: node.to_string() };
    Ok(match request {
        NodeRequest::MintToken { .. } => unreachable!("answered by the node connection"),
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
    use quietdeck_types::node::Handoff;
    use serde_json::json;

    async fn with_runner(state: &AppState, node: &str, tab: &str) {
        sqlx::query("INSERT INTO nodes (name, token_hash, created_at) VALUES (?, 'x', 0)")
            .bind(node)
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
        decisions::approve_all(&state.db, &ApproveAll { cards: vec![d], timeline: vec![] }, 0).await.unwrap();
        decisions::release_due(&state.db).await.unwrap();
        card.id
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
