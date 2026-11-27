//! Fleetlayer as the node's way to deliver messages, where Fleetlayer runs (the cutover from Quietdeck's own relay and
//! delivery, 27 Nov 2026).
//!
//! Fleetlayer's node starts `quietdeck-node fleetlayer-plugin` as one of its plugins (named `runner`, granted
//! `message.send`), so what Quietdeck sends goes out as `plugin:runner@<node>`: the sender its access rules name, and
//! what the recipient sees ("Message from plugin runner (<node>) via Fleetlayer"). Fleetlayer resolves the address,
//! refuses repeats, and delivers through the harness's own channel, typing it in through the session manager (herdr)
//! when that has no way in.
//!
//! The plugin listens on a local socket (mode 0600, next to the node's own) for this node's sends. Where it isn't
//! running (no Fleetlayer on this machine, as with Quietdeck solo on a laptop) [`send`] answers None and the caller
//! uses Quietdeck's own path.
//!
//! The socket's protocol is one JSON line each way: `{"to", "text", "local"}`, answered by `{"via"}` or
//! `{"error", "code"}` (Fleetlayer's JSON-RPC error code and message, passed back as they are).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use quietdeck_types::node::Delivered;
use quietdeck_types::relay::DeliveryKind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Mutex, mpsc, oneshot};

/// Fleetlayer delivers within 5 seconds on its node, and gives a send to another node 12.
const NODE_ANSWERS_WITHIN: Duration = Duration::from_secs(14);
const CALLER_WAITS: Duration = Duration::from_secs(15);

const MESSAGE_SEND: &str = "_fleetlayer.dev/message/send";
/// Fleetlayer's error codes (its `rpc::code`).
pub mod code {
    pub const NOT_HERE: i64 = -32010;
    pub const REFUSED: i64 = -32011;
    pub const BUSY: i64 = -32012;
    pub const NO_SUCH: i64 = -32014;
}

/// The plugin's socket: `$QUIETDECK_FLEETLAYER_SOCKET`, else beside the node's local socket.
pub fn bridge_socket() -> PathBuf {
    if let Some(p) = std::env::var_os("QUIETDECK_FLEETLAYER_SOCKET") {
        return PathBuf::from(p);
    }
    crate::config::socket_path().with_file_name("fleetlayer-plugin.sock")
}

/// One message for Fleetlayer to deliver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outgoing {
    pub to: String,
    pub text: String,
    /// `to` is a tab on this machine: the plugin adds `@<node>`, Fleetlayer's name for it.
    #[serde(default)]
    pub local: bool,
    /// Outside data (an email, a webhook's payload): the harness is told it's never instructions.
    #[serde(default)]
    pub untrusted: bool,
    /// What kind of delivery it is, for the harness (`forward`, `wakeup`...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subkind: Option<String>,
}

impl Outgoing {
    /// A Quietdeck delivery, rendered as the session sees it, with its kind and whether it's outside data.
    pub fn of(to: &str, kind: &DeliveryKind, body: &str, local: bool) -> Outgoing {
        let (subkind, untrusted) = match kind {
            DeliveryKind::Message { .. } => ("message", false),
            DeliveryKind::Forward { .. } => ("forward", true),
            DeliveryKind::Webhook { .. } => ("webhook", true),
            DeliveryKind::Wakeup => ("wakeup", false),
            DeliveryKind::Handoff => ("handoff", false),
            DeliveryKind::RunnerReport => ("runner_report", false),
            DeliveryKind::Deck => ("deck", false),
        };
        Outgoing { to: to.into(), text: kind.render(body), local, untrusted, subkind: Some(subkind.into()) }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct BridgeAnswer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    via: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    code: Option<i64>,
}

/// A send Fleetlayer turned down: its message is written for a person ("…; did you mean 'x@n'?").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendError {
    pub code: Option<i64>,
    pub message: String,
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Send through Fleetlayer. None when it doesn't run Quietdeck's plugin here, so the caller uses its own path; a
/// refusal or failure from Fleetlayer is an answer, never a reason to try another way.
pub async fn send(msg: &Outgoing) -> Option<Result<Delivered, SendError>> {
    send_via(&bridge_socket(), msg).await
}

pub async fn send_via(socket: &Path, msg: &Outgoing) -> Option<Result<Delivered, SendError>> {
    let stream = UnixStream::connect(socket).await.ok()?;
    let ask = async {
        let (r, mut w) = stream.into_split();
        let line = serde_json::to_string(msg).map_err(|e| e.to_string())?;
        w.write_all(format!("{line}\n").as_bytes()).await.map_err(|e| e.to_string())?;
        let answer = BufReader::new(r)
            .lines()
            .next_line()
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Quietdeck's Fleetlayer plugin hung up".to_string())?;
        serde_json::from_str::<BridgeAnswer>(&answer).map_err(|e| e.to_string())
    };
    let answer = match tokio::time::timeout(CALLER_WAITS, ask).await {
        Err(_) => {
            return Some(Err(SendError {
                code: None,
                message: format!("Fleetlayer didn't answer within {} seconds", CALLER_WAITS.as_secs()),
            }));
        }
        Ok(Err(e)) => return Some(Err(SendError { code: None, message: e })),
        Ok(Ok(a)) => a,
    };
    Some(match (answer.via.as_deref(), answer.error) {
        (_, Some(message)) => Err(SendError { code: answer.code, message }),
        (Some("harness_channel"), None) => Ok(Delivered::Socket),
        (Some(_), None) => Ok(Delivered::Orchestrator),
        (None, None) => Err(SendError { code: None, message: "Fleetlayer's answer said neither how nor why".into() }),
    })
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, (i64, String)>>>>>;

/// `quietdeck-node fleetlayer-plugin`: run as a Fleetlayer plugin on stdio until the node closes it.
pub async fn run_plugin() -> anyhow::Result<()> {
    serve_plugin(tokio::io::stdin(), tokio::io::stdout(), bridge_socket()).await
}

pub async fn serve_plugin<R, W>(input: R, output: W, socket: PathBuf) -> anyhow::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (tx, mut rx) = mpsc::channel::<String>(64);
    let writer = tokio::spawn(async move {
        let mut output = output;
        while let Some(line) = rx.recv().await {
            if output.write_all(line.as_bytes()).await.is_err()
                || output.write_all(b"\n").await.is_err()
                || output.flush().await.is_err()
            {
                break;
            }
        }
    });
    let pending: Pending = Arc::default();
    let ids = Arc::new(AtomicU64::new(1));
    let mut listening: Option<tokio::task::JoinHandle<()>> = None;

    let mut lines = BufReader::new(input).lines();
    while let Some(line) = lines.next_line().await? {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            tracing::warn!("a line from the Fleetlayer node that isn't JSON");
            continue;
        };
        let id = msg.get("id").cloned();
        match (msg.get("method").and_then(Value::as_str), id) {
            (Some(method), Some(id)) => {
                let reply = match method {
                    "initialize" => {
                        let node = msg
                            .pointer("/params/clientCapabilities/_meta/fleetlayer.dev~1node/node")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        if listening.is_none() {
                            listening = Some(listen(&socket, node.clone(), tx.clone(), pending.clone(), ids.clone())?);
                            tracing::info!(%node, socket = %socket.display(), "Quietdeck's sends go through Fleetlayer");
                        }
                        json!({"jsonrpc": "2.0", "id": id, "result": {
                            "protocolVersion": 1,
                            "agentCapabilities": {"_meta": {"fleetlayer.dev/plugin": {"protocol": 1, "roles": {}}}},
                            "agentInfo": {"name": "quietdeck-runner", "version": env!("CARGO_PKG_VERSION")},
                            "authMethods": [],
                        }})
                    }
                    "_fleetlayer.dev/ping" => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
                    _ => json!({"jsonrpc": "2.0", "id": id,
                                "error": {"code": -32601, "message": format!("{method} isn't something this plugin does")}}),
                };
                let _ = tx.send(reply.to_string()).await;
            }
            (Some(_), None) => {} // a notification: nothing this plugin follows
            (None, Some(id)) => {
                let Some(n) = id.as_u64() else { continue };
                if let Some(waiting) = pending.lock().await.remove(&n) {
                    let result = match msg.get("error") {
                        Some(e) => Err((
                            e["code"].as_i64().unwrap_or(-32603),
                            e["message"].as_str().unwrap_or("Fleetlayer failed").to_string(),
                        )),
                        None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                    };
                    let _ = waiting.send(result);
                }
            }
            (None, None) => {}
        }
    }
    // The node closed stdin: it's stopping this plugin. Sends from now on take Quietdeck's own path.
    if let Some(l) = listening {
        l.abort();
    }
    let _ = std::fs::remove_file(&socket);
    drop(tx);
    let _ = writer.await;
    Ok(())
}

fn listen(
    socket: &Path,
    node: String,
    tx: mpsc::Sender<String>,
    pending: Pending,
    ids: Arc<AtomicU64>,
) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(dir) = socket.parent() {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    Ok(tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else { continue };
            let (node, tx, pending, ids) = (node.clone(), tx.clone(), pending.clone(), ids.clone());
            tokio::spawn(async move {
                let (r, mut w) = stream.into_split();
                let Ok(Some(line)) = BufReader::new(r).lines().next_line().await else { return };
                let answer = match serde_json::from_str::<Outgoing>(&line) {
                    Err(e) => BridgeAnswer { error: Some(format!("not a send: {e}")), ..Default::default() },
                    Ok(s) => relay(&node, s, &tx, &pending, &ids).await,
                };
                let line = serde_json::to_string(&answer).unwrap_or_default();
                let _ = w.write_all(format!("{line}\n").as_bytes()).await;
            });
        }
    }))
}

async fn relay(node: &str, s: Outgoing, tx: &mpsc::Sender<String>, pending: &Pending, ids: &AtomicU64) -> BridgeAnswer {
    let to = if s.local && !node.is_empty() { format!("{}@{node}", s.to) } else { s.to };
    let id = ids.fetch_add(1, Ordering::Relaxed);
    let (done, wait) = oneshot::channel();
    pending.lock().await.insert(id, done);
    let req = json!({"jsonrpc": "2.0", "id": id, "method": MESSAGE_SEND, "params": {
        "to": to, "text": s.text, "untrusted": s.untrusted, "subkind": s.subkind}});
    if tx.send(req.to_string()).await.is_err() {
        pending.lock().await.remove(&id);
        return BridgeAnswer { error: Some("the Fleetlayer node has gone".into()), ..Default::default() };
    }
    match tokio::time::timeout(NODE_ANSWERS_WITHIN, wait).await {
        Ok(Ok(Ok(v))) => {
            BridgeAnswer { via: Some(v["via"].as_str().unwrap_or("typed").to_string()), ..Default::default() }
        }
        Ok(Ok(Err((code, message)))) => BridgeAnswer { error: Some(message), code: Some(code), ..Default::default() },
        Ok(Err(_)) => BridgeAnswer { error: Some("the Fleetlayer node has gone".into()), ..Default::default() },
        Err(_) => {
            pending.lock().await.remove(&id);
            BridgeAnswer {
                error: Some(format!(
                    "the Fleetlayer node didn't answer within {} seconds; it may still deliver",
                    NODE_ANSWERS_WITHIN.as_secs()
                )),
                ..Default::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn line<R: AsyncRead + Unpin>(r: &mut tokio::io::Lines<BufReader<R>>) -> Value {
        serde_json::from_str(&r.next_line().await.unwrap().unwrap()).unwrap()
    }

    /// The plugin with a fake Fleetlayer node on its stdio, initialized as node `agents-triage`.
    async fn plugin(
        socket: PathBuf,
    ) -> (
        tokio::io::WriteHalf<tokio::io::DuplexStream>,
        tokio::io::Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
    ) {
        let (node_side, plugin_side) = tokio::io::duplex(1 << 16);
        let (pr, pw) = tokio::io::split(plugin_side);
        tokio::spawn(serve_plugin(pr, pw, socket));
        let (nr, mut nw) = tokio::io::split(node_side);
        let mut nr = BufReader::new(nr).lines();
        let init = json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
            "protocolVersion": 1,
            "clientCapabilities": {"_meta": {"fleetlayer.dev/node": {"protocol": 1, "node": "agents-triage"}}}}});
        nw.write_all(format!("{init}\n").as_bytes()).await.unwrap();
        let answer = line(&mut nr).await;
        assert_eq!(answer["id"], 0);
        assert_eq!(answer["result"]["agentCapabilities"]["_meta"]["fleetlayer.dev/plugin"]["protocol"], 1);
        (nw, nr)
    }

    fn forward() -> DeliveryKind {
        DeliveryKind::Forward {
            account: "personal".into(),
            refs: vec!["t1".into()],
            rule: None,
            sent_mail: false,
            note: None,
        }
    }

    #[tokio::test]
    async fn no_plugin_means_quietdecks_own_path() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            send_via(&dir.path().join("none.sock"), &Outgoing::of("triage", &DeliveryKind::Wakeup, "hi", true))
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_local_send_names_the_node_and_comes_back_delivered() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("q/fleetlayer-plugin.sock");
        let (mut nw, mut nr) = plugin(socket.clone()).await;
        let sent = tokio::spawn({
            let socket = socket.clone();
            async move {
                send_via(&socket, &Outgoing::of("email triage", &DeliveryKind::Wakeup, "New mail: 1 Personal.", true))
                    .await
            }
        });
        let req = line(&mut nr).await;
        assert_eq!(req["method"], MESSAGE_SEND);
        assert_eq!(req["params"]["to"], "email triage@agents-triage");
        assert_eq!(req["params"]["text"], "New mail: 1 Personal.");
        assert_eq!(req["params"]["untrusted"], false);
        assert_eq!(req["params"]["subkind"], "wakeup");
        let ok = json!({"jsonrpc": "2.0", "id": req["id"], "result": {"via": "harness_channel"}});
        nw.write_all(format!("{ok}\n").as_bytes()).await.unwrap();
        assert_eq!(sent.await.unwrap(), Some(Ok(Delivered::Socket)));
    }

    #[tokio::test]
    async fn fleetlayers_refusal_comes_back_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("fleetlayer-plugin.sock");
        let (mut nw, mut nr) = plugin(socket.clone()).await;
        let sent = tokio::spawn({
            let socket = socket.clone();
            async move { send_via(&socket, &Outgoing::of("tirage@agents-triage", &forward(), "the email", false)).await }
        });
        let req = line(&mut nr).await;
        assert_eq!(req["params"]["to"], "tirage@agents-triage");
        assert_eq!(req["params"]["untrusted"], true);
        assert_eq!(req["params"]["subkind"], "forward");
        let text = req["params"]["text"].as_str().unwrap();
        assert!(text.starts_with("Forwarded by Quietdeck: an email (personal)."), "{text}");
        assert!(text.ends_with("the email"), "{text}");
        let no = json!({"jsonrpc": "2.0", "id": req["id"], "error": {"code": code::NO_SUCH,
            "message": "no session named 'tirage' on agents-triage; did you mean 'triage@agents-triage'?",
            "data": {"suggestions": ["triage@agents-triage"]}}});
        nw.write_all(format!("{no}\n").as_bytes()).await.unwrap();
        let e = sent.await.unwrap().unwrap().unwrap_err();
        assert_eq!(e.code, Some(code::NO_SUCH));
        assert!(e.message.contains("did you mean 'triage@agents-triage'"), "{e}");
    }

    #[tokio::test]
    async fn pings_are_answered_and_the_socket_goes_when_the_node_stops_the_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("fleetlayer-plugin.sock");
        let (mut nw, mut nr) = plugin(socket.clone()).await;
        nw.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"_fleetlayer.dev/ping\",\"params\":{}}\n")
            .await
            .unwrap();
        assert_eq!(line(&mut nr).await["id"], 7);
        assert!(socket.exists());
        nw.shutdown().await.unwrap();
        drop(nw);
        for _ in 0..50 {
            if !socket.exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the socket outlived the plugin");
    }
}
