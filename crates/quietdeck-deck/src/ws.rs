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
}

async fn send(sink: &mut futures::stream::SplitSink<WebSocket, Message>, frame: &DeckFrame) -> Result<(), axum::Error> {
    let text = serde_json::to_string(frame).expect("deck frames serialise");
    sink.send(Message::Text(text.into())).await
}

async fn handle(state: &AppState, node: &str, reg: &crate::hub::Registration, frame: NodeFrame) {
    let result = match frame {
        NodeFrame::Hello { .. } => Ok(()),
        NodeFrame::Sessions { sessions } => sessions::replace_snapshot(&state.db, &state.config, node, &sessions).await,
        NodeFrame::Session { session } => sessions::upsert(&state.db, &state.config, node, &session).await,
        NodeFrame::SessionClosed { session_id, at } => sessions::closed(&state.db, node, &session_id, at).await,
        NodeFrame::Summary { event } => sessions::apply_summary(&state.db, node, &event).await,
        NodeFrame::Usage { .. } => Ok(()),
        NodeFrame::Response { id, reply } => {
            reg.resolve(id, reply).await;
            Ok(())
        }
        NodeFrame::Request { id, request } => {
            let reply = answer(state, node, request).await;
            let _ = reg.tx.send(DeckFrame::Response { id, reply }).await;
            Ok(())
        }
    };
    if let Err(e) = result {
        tracing::error!(%node, error = %e, "handling a node frame");
    }
}

async fn answer(state: &AppState, node: &str, request: NodeRequest) -> Reply {
    match request {
        NodeRequest::MintToken { session } => match mint_for(state, node, session).await {
            Ok(t) => Reply::ok(&t),
            Err(e) => Reply::Err { error: e.to_string() },
        },
    }
}

async fn mint_for(state: &AppState, node: &str, session: Option<TokenSession>) -> anyhow::Result<MintedToken> {
    let (session_id, mut tab, pane) = match session {
        Some(s) => (s.session_id, s.tab, s.pane),
        None => (None, None, None),
    };
    if tab.is_none()
        && let Some(id) = &session_id
    {
        tab = sessions::tab_of(&state.db, node, id).await?;
    }
    let exp = now_ms() / 1000 + SESSION_TOKEN_TTL_SECONDS;
    let claims = TokenClaims { node: node.to_string(), session_id, tab, pane, exp };
    Ok(MintedToken {
        token: mint_session_token(&state.signing_key, &claims),
        expires_at: jiff::Timestamp::from_second(exp)?,
        caller: caller_ref(&claims),
    })
}
