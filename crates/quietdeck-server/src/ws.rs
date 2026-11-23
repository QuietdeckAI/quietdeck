//! The WebSocket each node opens to the deck (`/api/nodes/ws`, node token in the Authorization header).

use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::Response;
use futures::{SinkExt, StreamExt};
use quietdeck_types::node::{
    DeckFrame, MintedToken, NodeFrame, NodeRequest, PROTOCOL_VERSION, Reply, TokenClaims, TokenSession,
};

use quietdeck_types::api::Changed;

use crate::auth::{SESSION_TOKEN_TTL_SECONDS, caller_ref, mint_session_token, node_for_token};
use crate::db::now_ms;
use crate::error::AppError;
use crate::state::AppState;
use crate::store::{nodes, sessions};

const PING_EVERY: Duration = Duration::from_secs(20);
/// A node that sends nothing (not even a pong) for this long is gone.
const SILENT_LIMIT: Duration = Duration::from_secs(60);
/// Biggest frame a node may send: a full sessions snapshot with summaries fits easily.
const MAX_FRAME: usize = 8 * 1024 * 1024;

pub async fn upgrade(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, AppError> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| AppError::Unauthorized("the node WebSocket needs the node token".into()))?;
    let node = node_for_token(&state.db, token.trim())
        .await?
        .ok_or_else(|| AppError::Unauthorized("unknown node token: enrol this machine again".into()))?;
    Ok(ws.max_message_size(MAX_FRAME).on_upgrade(move |socket| run(state, node, socket)))
}

async fn run(state: AppState, node: String, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();

    // The first frame must be hello, with our protocol version.
    let hello = match tokio::time::timeout(Duration::from_secs(10), stream.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => serde_json::from_str::<NodeFrame>(&t).ok(),
        _ => None,
    };
    let Some(NodeFrame::Hello { protocol, version, runner }) = hello else {
        tracing::warn!(%node, "node connected without a hello");
        return;
    };
    if protocol != PROTOCOL_VERSION {
        let reason = format!(
            "this deck speaks node protocol {PROTOCOL_VERSION} and the node speaks {protocol}: upgrade the {}",
            if protocol < PROTOCOL_VERSION { "node" } else { "deck" }
        );
        let _ = send(&mut sink, &DeckFrame::Refused { reason }).await;
        return;
    }
    if let Err(e) = nodes::hello(&state.db, &node, &version, runner.as_ref()).await {
        tracing::error!(%node, error = %e, "recording a node's hello");
        return;
    }
    let mut reg = state.hub.register(&node).await;
    tracing::info!(%node, %version, runner = runner.is_some(), "node connected");
    state.changed(Changed::Nodes);
    if send(&mut sink, &DeckFrame::Welcome { node: node.clone() }).await.is_err() {
        state.hub.unregister(&node, reg.conn_id).await;
        return;
    }

    let mut ping = tokio::time::interval(PING_EVERY);
    let mut last_heard = tokio::time::Instant::now();
    loop {
        tokio::select! {
            out = reg.rx.recv() => {
                let Some(frame) = out else { break };
                if send(&mut sink, &frame).await.is_err() { break; }
            }
            incoming = stream.next() => {
                last_heard = tokio::time::Instant::now();
                match incoming {
                    Some(Ok(Message::Text(text))) => match serde_json::from_str::<NodeFrame>(&text) {
                        Ok(frame) => handle(&state, &node, &reg, frame).await,
                        Err(e) => tracing::warn!(%node, error = %e, "unreadable frame from node"),
                    },
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => {}
                }
            }
            _ = ping.tick() => {
                if last_heard.elapsed() > SILENT_LIMIT {
                    tracing::info!(%node, "node went silent");
                    break;
                }
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() { break; }
                let _ = nodes::seen(&state.db, &node).await;
            }
        }
    }
    state.hub.unregister(&node, reg.conn_id).await;
    let _ = nodes::seen(&state.db, &node).await;
    tracing::info!(%node, "node disconnected");
    state.changed(Changed::Nodes);
    state.changed(Changed::Sessions);
}

async fn send(sink: &mut futures::stream::SplitSink<WebSocket, Message>, frame: &DeckFrame) -> Result<(), axum::Error> {
    let text = serde_json::to_string(frame).expect("deck frames serialise");
    sink.send(Message::Text(text.into())).await
}

async fn handle(state: &AppState, node: &str, reg: &crate::hub::Registration, frame: NodeFrame) {
    let sessions_changed = matches!(
        frame,
        NodeFrame::Sessions { .. }
            | NodeFrame::Session { .. }
            | NodeFrame::SessionClosed { .. }
            | NodeFrame::Summary { .. }
    );
    let result = match frame {
        NodeFrame::Hello { .. } => Ok(()),
        NodeFrame::Sessions { sessions } => sessions::replace_snapshot(&state.db, &state.config, node, &sessions).await,
        NodeFrame::Session { session } => sessions::upsert(&state.db, &state.config, node, &session).await,
        NodeFrame::SessionClosed { session_id, at, ended } => {
            match sessions::closed(&state.db, node, &session_id, at, ended.as_ref()).await {
                // A tab closed or an agent exited: its grants are archived. A herdr restart or a sleeping machine
                // sends no close event, so they stay.
                Ok(()) if ended.is_some() => {
                    crate::ops::session_closed(state, node, &session_id).await.map_err(|e| anyhow::anyhow!("{e}"))
                }
                other => other,
            }
        }
        NodeFrame::Summary { event } => sessions::apply_summary(&state.db, node, &event).await,
        NodeFrame::Usage { .. } => Ok(()),
        NodeFrame::Response { id, reply } => {
            reg.resolve(id, reply).await;
            Ok(())
        }
        NodeFrame::Request { id, request } => {
            // In its own task: answering can take a request back to this same node (a runner's relay delivers
            // through it), and that answer arrives on this read loop.
            let (state, node, tx) = (state.clone(), node.to_string(), reg.tx.clone());
            tokio::spawn(async move {
                let reply = answer(&state, &node, request).await;
                let _ = tx.send(DeckFrame::Response { id, reply }).await;
            });
            Ok(())
        }
    };
    match result {
        Ok(()) if sessions_changed => state.changed(Changed::Sessions),
        Ok(()) => {}
        Err(e) => tracing::error!(%node, error = %e, "handling a node frame"),
    }
}

async fn answer(state: &AppState, node: &str, request: NodeRequest) -> Reply {
    match request {
        NodeRequest::MintToken { session } => match mint_for(state, node, session).await {
            Ok(t) => Reply::ok(&t),
            Err(e) => Reply::Err { error: e.to_string() },
        },
        other => crate::runner::answer(state, node, other).await,
    }
}

async fn mint_for(state: &AppState, node: &str, session: Option<TokenSession>) -> anyhow::Result<MintedToken> {
    let (session_id, mut tab, address) = match session {
        Some(s) => (s.session_id, s.tab, s.address),
        None => (None, None, None),
    };
    if tab.is_none()
        && let Some(id) = &session_id
    {
        tab = sessions::tab_of(&state.db, node, id).await?;
    }
    let exp = now_ms() / 1000 + SESSION_TOKEN_TTL_SECONDS;
    let claims = TokenClaims { node: node.to_string(), session_id, tab, address, exp };
    Ok(MintedToken {
        token: mint_session_token(&state.signing_key, &claims),
        expires_at: jiff::Timestamp::from_second(exp)?,
        caller: caller_ref(&claims),
    })
}

#[cfg(test)]
mod tests {
    use futures::{SinkExt, StreamExt};
    use quietdeck_types::directory::SessionInfo;
    use quietdeck_types::node::{
        DeckFrame, DeckRequest, Delivered, NodeFrame, NodeRequest, PROTOCOL_VERSION, Reply, RunnerInfo,
    };
    use quietdeck_types::relay::DeliveryKind;
    use quietdeck_types::{ActionType, AgentKind, AgentStatus};
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    use crate::state::AppState;
    use crate::store::nodes;

    fn text(frame: &NodeFrame) -> Message {
        Message::Text(serde_json::to_string(frame).unwrap().into())
    }

    /// A runner's relay to a session on its own node: the deck asks that node to deliver while the node's own
    /// request is still open. Both must be answered.
    #[tokio::test]
    async fn a_runner_relays_to_a_session_on_its_own_node() {
        let state = AppState::for_tests().await;
        let code = nodes::create_enrolment_code(&state.db).await.unwrap();
        let enrolled = nodes::enrol(&state.db, &code.code, "server").await.unwrap();
        let app =
            axum::Router::new().route("/api/nodes/ws", axum::routing::get(super::upgrade)).with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let mut req = format!("ws://{addr}/api/nodes/ws").into_client_request().unwrap();
        req.headers_mut().insert("Authorization", format!("Bearer {}", enrolled.token).parse().unwrap());
        let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        let (mut sink, mut stream) = ws.split();
        let runner = RunnerInfo { eligible: vec![ActionType::Forward], agent_tab: "triage".into(), accounts: vec![] };
        sink.send(text(&NodeFrame::Hello { protocol: PROTOCOL_VERSION, version: "t".into(), runner: Some(runner) }))
            .await
            .unwrap();
        let session = SessionInfo {
            session_id: "s-target".into(),
            agent: AgentKind::Claude,
            workspace: None,
            tab: Some("target".into()),
            address: Some("p1".into()),
            cwd: None,
            status: AgentStatus::Idle,
            status_changed_at: jiff::Timestamp::now(),
            resume_command: None,
            running: true,
            open_url: None,
            opens_in: None,
        };
        sink.send(text(&NodeFrame::Sessions { sessions: vec![session] })).await.unwrap();
        let relay = NodeRequest::Relay { to: "target@server".into(), kind: DeliveryKind::Deck, body: "hello".into() };
        sink.send(text(&NodeFrame::Request { id: 7, request: relay })).await.unwrap();

        let mut delivered = false;
        let answered = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while let Some(msg) = stream.next().await {
                let Message::Text(t) = msg.unwrap() else { continue };
                match serde_json::from_str::<DeckFrame>(&t).unwrap() {
                    DeckFrame::Request { id, request: DeckRequest::Deliver { delivery } } => {
                        assert_eq!(delivery.target.session_id.as_deref(), Some("s-target"));
                        delivered = true;
                        let reply = Reply::ok(&Delivered::Socket);
                        sink.send(text(&NodeFrame::Response { id, reply })).await.unwrap();
                    }
                    DeckFrame::Response { id: 7, reply } => return reply,
                    DeckFrame::Refused { reason } => panic!("refused: {reason}"),
                    _ => {}
                }
            }
            panic!("the deck closed the connection");
        })
        .await
        .expect("the deck answers the relay while it waits on this node's delivery");
        assert!(delivered);
        assert!(answered.into_result::<serde_json::Value>().is_ok());
    }
}
