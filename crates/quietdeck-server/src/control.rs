//! Starting, resuming and closing sessions from the deck (the registry's actions, flow 3.15). The request is the
//! approval: the owner's click, or a command run by one of the owner's sessions. Each is logged in History. The
//! target machine's node checks it and has herdr do it.

use std::time::Duration;

use garde::Validate;
use quietdeck_types::action::SessionRef;
use quietdeck_types::api::{Caller, Changed};
use quietdeck_types::directory::{SessionResume, SessionRow, SessionStart, SessionStarted, SessionTarget};
use quietdeck_types::node::DeckRequest;
use quietdeck_types::timeline::TimelineKind;

use crate::error::{AppError, AppResult, bad};
use crate::hub::HubError;
use crate::state::AppState;
use crate::store::{sessions, timeline};

/// herdr waits up to 30 seconds for an agent to take input, and a resumed session can take a while to load.
const START_TIMEOUT: Duration = Duration::from_secs(150);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(20);

fn asked_by(caller: &Caller) -> String {
    match caller {
        Caller::Session { session } => format!(" (asked by {})", session.address()),
        Caller::Node { node } => format!(" (by the runner on {node})"),
        Caller::Owner { .. } => String::new(),
    }
}

pub(crate) async fn ask(
    state: &AppState,
    node: &str,
    request: DeckRequest,
    timeout: Duration,
) -> AppResult<serde_json::Value> {
    let reply = state.hub.request(node, request, timeout).await.map_err(|e| match e {
        HubError::Offline(_) => AppError::Offline(format!("{node} isn't connected to the deck")),
        HubError::Timeout(_) => AppError::Unavailable(format!("{node} didn't answer in time")),
    })?;
    // The node's refusals (no such directory, a start-up screen in herdr) are for the person to read.
    reply.into_result::<serde_json::Value>().map_err(AppError::Conflict)
}

pub async fn start(state: &AppState, caller: &Caller, mut spec: SessionStart) -> AppResult<SessionStarted> {
    spec.validate().map_err(bad)?;
    // A start from a handoff continues the session that wrote it (the move-session skill).
    let handoff = match spec.handoff.take() {
        Some(id) => Some(crate::handoffs::unclaimed(&state.db, id).await?),
        None => None,
    };
    if let Some(h) = &handoff
        && spec.prompt.is_none()
    {
        spec.prompt = Some(crate::handoffs::first_prompt(h));
    }
    let node = spec.on.clone();
    let what = format!("{} in {}", spec.tab, spec.cwd);
    let v = ask(state, &node, DeckRequest::StartSession { spec }, START_TIMEOUT).await?;
    let started: SessionStarted = serde_json::from_value(v).map_err(|e| AppError::Internal(e.into()))?;
    let mut refs = vec![format!("{}@{node}", started.tab)];
    refs.extend(started.session_id.clone());
    let mut text = format!("Started {what} on {node}{}", asked_by(caller));
    if let Some(h) = &handoff {
        let by = SessionRef { node: node.clone(), tab: started.tab.clone(), session_id: started.session_id.clone() };
        crate::handoffs::continued(&state.db, h.id, &by).await?;
        refs.push(h.from.address());
        text.push_str(&format!(", continuing {} from handoff h_{}", h.from.address(), h.id));
    }
    timeline::log(&state.db, TimelineKind::SessionStarted, &refs, &text).await?;
    state.changed(Changed::Timeline);
    Ok(started)
}

/// The one ended session an address names, or why there isn't one.
async fn ended_session(state: &AppState, address: &str) -> AppResult<SessionRow> {
    let rows = sessions::resolve_ended(&state.db, &state.hub, address).await?;
    let mut nodes: Vec<&str> = rows.iter().map(|r| r.node.as_str()).collect();
    nodes.sort_unstable();
    nodes.dedup();
    if nodes.len() > 1 {
        let candidates: Vec<String> = rows.iter().map(|r| r.info.session_id.clone() + "@" + &r.node).collect();
        return Err(AppError::Conflict(format!(
            "{address} names sessions on more than one machine: say which, as one of {}",
            candidates.join(", ")
        )));
    }
    match rows.into_iter().next() {
        // The most recently ended one, when a tab name has had several sessions.
        Some(row) => Ok(row),
        None if !sessions::resolve_address(&state.db, &state.hub, address).await?.is_empty() => {
            Err(AppError::Conflict(format!("{address} is already running")))
        }
        None => Err(AppError::NotFound(format!("no session called {address}"))),
    }
}

pub async fn resume(state: &AppState, caller: &Caller, target: SessionTarget) -> AppResult<SessionStarted> {
    target.validate().map_err(bad)?;
    let row = ended_session(state, &target.session).await?;
    let cwd = row.info.cwd.clone().ok_or_else(|| {
        AppError::Conflict(format!("{} has no working directory on record, so it can't be resumed here", row.address()))
    })?;
    let spec = SessionResume {
        session_id: row.info.session_id.clone(),
        agent: row.info.agent,
        tab: row.info.tab.clone(),
        cwd,
    };
    let v = ask(state, &row.node, DeckRequest::ResumeSession { spec }, START_TIMEOUT).await?;
    let started: SessionStarted = serde_json::from_value(v).map_err(|e| AppError::Internal(e.into()))?;
    let refs = vec![format!("{}@{}", started.tab, row.node), row.info.session_id.clone()];
    let text = format!("Resumed {} on {}{}", started.tab, row.node, asked_by(caller));
    timeline::log(&state.db, TimelineKind::SessionResumed, &refs, &text).await?;
    state.changed(Changed::Timeline);
    Ok(started)
}

/// The runner's `resume`: one session by its ID, on the machine it ran on, in the tab the card names.
pub async fn resume_by_id(
    state: &AppState,
    caller: &Caller,
    node: &str,
    session_id: &str,
    tab: &str,
    cwd: &str,
) -> AppResult<SessionStarted> {
    let row = sessions::get(&state.db, &state.hub, node, session_id).await?;
    if row.as_ref().is_some_and(|r| r.info.running) {
        return Err(AppError::Conflict(format!("{tab}@{node} is already running")));
    }
    let agent = row.map(|r| r.info.agent).unwrap_or(quietdeck_types::AgentKind::Claude);
    let spec = SessionResume { session_id: session_id.into(), agent, tab: Some(tab.into()), cwd: cwd.into() };
    let v = ask(state, node, DeckRequest::ResumeSession { spec }, START_TIMEOUT).await?;
    let started: SessionStarted = serde_json::from_value(v).map_err(|e| AppError::Internal(e.into()))?;
    let refs = vec![format!("{}@{node}", started.tab), session_id.to_string()];
    let text = format!("Resumed {} on {node}{}", started.tab, asked_by(caller));
    timeline::log(&state.db, TimelineKind::SessionResumed, &refs, &text).await?;
    state.changed(Changed::Timeline);
    Ok(started)
}

/// The one running session a target names.
async fn running_one(state: &AppState, target: &SessionTarget) -> AppResult<quietdeck_types::directory::SessionRow> {
    target.validate().map_err(bad)?;
    let mut rows = sessions::resolve_address(&state.db, &state.hub, &target.session).await?;
    match rows.len() {
        0 => Err(AppError::NotFound(format!("no running session called {}", target.session))),
        1 => Ok(rows.remove(0)),
        _ => {
            let candidates: Vec<String> = rows.iter().map(|r| r.info.session_id.clone() + "@" + &r.node).collect();
            Err(AppError::Conflict(format!(
                "{} names more than one running session: say which, as one of {}",
                target.session,
                candidates.join(", ")
            )))
        }
    }
}

/// Stop: Esc in the session, so it ends its turn. The person's click on the report, while its agent works.
pub async fn interrupt(state: &AppState, target: SessionTarget) -> AppResult<()> {
    let row = running_one(state, &target).await?;
    ask(state, &row.node, DeckRequest::InterruptSession { session_id: row.info.session_id.clone() }, CLOSE_TIMEOUT)
        .await?;
    Ok(())
}

pub async fn close(state: &AppState, caller: &Caller, target: SessionTarget) -> AppResult<()> {
    let row = &running_one(state, &target).await?;
    if let Caller::Session { session } = caller {
        let own = session.node == row.node && session.session_id.as_deref() == Some(row.info.session_id.as_str());
        let (_, moved_to) = crate::handoffs::links(&state.db, &row.node, &row.info.session_id).await?;
        if !own && moved_to.is_none() {
            return Err(AppError::Forbidden(format!(
                "a session can close only itself, or a session that has moved (a session started from its handoff \
                 continues it): {} hasn't moved",
                row.address()
            )));
        }
    }
    let request = DeckRequest::CloseSession { session_id: row.info.session_id.clone() };
    ask(state, &row.node, request, CLOSE_TIMEOUT).await?;
    let refs = vec![row.address(), row.info.session_id.clone()];
    let text = format!("Closed {}{}", row.address(), asked_by(caller));
    timeline::log(&state.db, TimelineKind::SessionClosed, &refs, &text).await?;
    state.changed(Changed::Timeline);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::Timestamp;
    use quietdeck_types::directory::{SessionEnd, SessionInfo};
    use quietdeck_types::node::{DeckFrame, Reply};
    use quietdeck_types::{AgentKind, AgentStatus};

    fn info(id: &str, tab: &str, running: bool) -> SessionInfo {
        SessionInfo {
            session_id: id.into(),
            agent: AgentKind::Claude,
            workspace: None,
            tab: Some(tab.into()),
            address: Some("w1:p1".into()),
            cwd: Some("/Users/sam/code/app".into()),
            status: AgentStatus::Idle,
            status_changed_at: Timestamp::now(),
            resume_command: None,
            running,
            open_url: None,
            opens_in: None,
        }
    }

    fn owner() -> Caller {
        Caller::Owner { device: "laptop".into() }
    }

    /// Answer the next request the node gets, and return it.
    async fn answer_next(reg: &mut crate::hub::Registration, reply: Reply) -> DeckRequest {
        let DeckFrame::Request { id, request } = reg.rx.recv().await.unwrap() else { panic!("expected a request") };
        reg.resolve(id, reply).await;
        request
    }

    #[tokio::test]
    async fn resume_sends_the_ended_sessions_own_directory_and_tab_and_logs_it() {
        let state = AppState::for_tests().await;
        sessions::upsert(&state.db, &state.config, "laptop", &info("s1", "billing", true)).await.unwrap();
        sessions::closed(&state.db, "laptop", "s1", Timestamp::now(), Some(&SessionEnd::TabClosed)).await.unwrap();
        let mut reg = state.hub.register("laptop").await;
        let s = state.clone();
        let call =
            tokio::spawn(async move { resume(&s, &owner(), SessionTarget { session: "billing@laptop".into() }).await });
        let started = SessionStarted {
            node: "laptop".into(),
            tab: "billing".into(),
            address: "w1:p9".into(),
            session_id: Some("s1".into()),
            prompt_error: None,
        };
        let req = answer_next(&mut reg, Reply::ok(&started)).await;
        assert_eq!(
            req,
            DeckRequest::ResumeSession {
                spec: SessionResume {
                    session_id: "s1".into(),
                    agent: AgentKind::Claude,
                    tab: Some("billing".into()),
                    cwd: "/Users/sam/code/app".into()
                }
            }
        );
        assert_eq!(call.await.unwrap().unwrap(), started);
        let logged = timeline::between(&state.db, 0, i64::MAX).await.unwrap();
        assert_eq!(logged.len(), 1);
        assert_eq!(logged[0].kind, TimelineKind::SessionResumed);
        assert!(logged[0].read_at.is_some(), "History only: it never waits on the Inbox");
        assert_eq!(logged[0].text, "Resumed billing on laptop");
    }

    #[tokio::test]
    async fn a_running_session_isnt_resumed_and_an_unknown_one_isnt_found() {
        let state = AppState::for_tests().await;
        sessions::upsert(&state.db, &state.config, "laptop", &info("s1", "billing", true)).await.unwrap();
        let running = resume(&state, &owner(), SessionTarget { session: "billing".into() }).await.unwrap_err();
        assert!(matches!(running, AppError::Conflict(ref m) if m.contains("already running")), "{running}");
        let unknown = resume(&state, &owner(), SessionTarget { session: "nope".into() }).await.unwrap_err();
        assert!(matches!(unknown, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn the_nodes_refusal_reaches_the_person_and_nothing_is_logged() {
        let state = AppState::for_tests().await;
        let mut reg = state.hub.register("laptop").await;
        let s = state.clone();
        let spec = SessionStart {
            on: "laptop".into(),
            tab: "esign cert".into(),
            name: None,
            agent: AgentKind::Claude,
            model: None,
            effort: None,
            cwd: "~/code/app".into(),
            prompt: Some("Pick up the certificate order.".into()),
            handoff: None,
        };
        let call = tokio::spawn(async move { start(&s, &owner(), spec).await });
        let msg = "esign cert needs you in herdr: the agent stopped at the folder trust prompt while starting.";
        let req = answer_next(&mut reg, Reply::Err { error: msg.into() }).await;
        assert!(matches!(req, DeckRequest::StartSession { .. }));
        let err = call.await.unwrap().unwrap_err();
        assert!(matches!(err, AppError::Conflict(ref m) if m == msg), "{err}");
        assert!(timeline::between(&state.db, 0, i64::MAX).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn starts_check_their_fields_and_need_the_machine_online() {
        let state = AppState::for_tests().await;
        let mut spec = SessionStart {
            on: "laptop".into(),
            tab: "b".into(),
            name: None,
            agent: AgentKind::Claude,
            model: Some("--dangerously-skip-permissions".into()),
            effort: None,
            cwd: "~/code/app".into(),
            prompt: None,
            handoff: None,
        };
        let flag = start(&state, &owner(), spec.clone()).await.unwrap_err();
        assert!(matches!(flag, AppError::BadRequest(_)), "a model can't smuggle in a flag: {flag}");
        spec.model = Some("opus".into());
        spec.tab = "bad\u{1b}[2Jtab".into();
        assert!(matches!(start(&state, &owner(), spec.clone()).await.unwrap_err(), AppError::BadRequest(_)));
        spec.tab = "b".into();
        let offline = start(&state, &owner(), spec).await.unwrap_err();
        assert!(matches!(offline, AppError::Offline(_)), "{offline}");
    }

    #[tokio::test]
    async fn close_asks_the_node_and_the_close_event_records_how_it_ended() {
        let state = AppState::for_tests().await;
        sessions::upsert(&state.db, &state.config, "laptop", &info("s1", "billing", true)).await.unwrap();
        let mut reg = state.hub.register("laptop").await;
        let s = state.clone();
        let call =
            tokio::spawn(async move { close(&s, &owner(), SessionTarget { session: "billing@laptop".into() }).await });
        let req = answer_next(&mut reg, Reply::ok(&serde_json::json!({}))).await;
        assert_eq!(req, DeckRequest::CloseSession { session_id: "s1".into() });
        call.await.unwrap().unwrap();
        // The node then forwards herdr's tab_closed.
        sessions::closed(&state.db, "laptop", "s1", Timestamp::now(), Some(&SessionEnd::TabClosed)).await.unwrap();
        let row = sessions::get(&state.db, &state.hub, "laptop", "s1").await.unwrap().unwrap();
        assert!(!row.info.running);
        assert_eq!(row.ended, Some(SessionEnd::TabClosed));
        assert!(row.closed_at.is_some());
        // Resumed later: running again, and no longer ended.
        sessions::upsert(&state.db, &state.config, "laptop", &info("s1", "billing", true)).await.unwrap();
        let row = sessions::get(&state.db, &state.hub, "laptop", "s1").await.unwrap().unwrap();
        assert_eq!((row.info.running, row.ended, row.closed_at), (true, None, None));
    }

    #[tokio::test]
    async fn stop_presses_esc_in_the_one_running_session_it_names() {
        let state = AppState::for_tests().await;
        sessions::upsert(&state.db, &state.config, "laptop", &info("s1", "email triage", true)).await.unwrap();
        let mut reg = state.hub.register("laptop").await;
        let s = state.clone();
        let call = tokio::spawn(async move { interrupt(&s, SessionTarget { session: "s1@laptop".into() }).await });
        let req = answer_next(&mut reg, Reply::ok(&serde_json::json!({}))).await;
        assert_eq!(req, DeckRequest::InterruptSession { session_id: "s1".into() });
        call.await.unwrap().unwrap();
        let none = interrupt(&state, SessionTarget { session: "nobody@laptop".into() }).await;
        assert!(matches!(none, Err(AppError::NotFound(_))), "{none:?}");
    }
}
