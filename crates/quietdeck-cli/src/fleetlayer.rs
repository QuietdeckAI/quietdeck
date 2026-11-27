//! `quietdeck msg send` through Fleetlayer, where this machine runs a Fleetlayer node: the same request the
//! `fleetlayer msg send` CLI makes on the node's local socket, with the calling session named the same way, so the
//! recipient sees who sent it and replies go back to them. Without a node to talk to, the deck's relay carries it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use quietdeck_types::node::Delivered;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Fleetlayer answers within 5 seconds on its node, and 12 for a session on another node.
pub const TIMEOUT: Duration = Duration::from_secs(15);

/// `$FLEETLAYER_SOCKET`, else the node's default: under `$XDG_RUNTIME_DIR`, else `~/.local/state`.
pub fn node_socket() -> PathBuf {
    if let Some(p) = std::env::var_os("FLEETLAYER_SOCKET") {
        return PathBuf::from(p);
    }
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(d) => PathBuf::from(d).join("fleetlayer/node.sock"),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state/fleetlayer/node.sock"),
    }
}

/// The calling session, as Fleetlayer's CLI names it.
fn caller() -> Value {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    json!({
        "sessionId": env("CLAUDE_CODE_SESSION_ID"),
        "address": env("FLEETLAYER_SESSION_ADDRESS").or_else(|| env("HERDR_PANE_ID")),
    })
}

/// None when no Fleetlayer node answers here. A refusal from Fleetlayer is an answer: its message says why ("…; did
/// you mean 'x@n'?"), and the caller doesn't try the deck after it.
pub async fn send(socket: &Path, to: &str, text: &str) -> Option<anyhow::Result<Delivered>> {
    let stream = tokio::net::UnixStream::connect(socket).await.ok()?;
    let ask = async move {
        let (r, mut w) = stream.into_split();
        let req = json!({"jsonrpc": "2.0", "id": 1, "method": "_fleetlayer.dev/message/send",
                         "params": {"to": to, "text": text, "caller": caller()}});
        w.write_all(format!("{req}\n").as_bytes()).await?;
        let mut lines = BufReader::new(r).lines();
        while let Some(line) = lines.next_line().await? {
            let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
            if msg.get("id") != Some(&json!(1)) || msg.get("method").is_some() {
                continue;
            }
            if let Some(e) = msg.get("error") {
                anyhow::bail!("{}", e["message"].as_str().unwrap_or("Fleetlayer refused the message"));
            }
            return Ok(match msg["result"]["via"].as_str() {
                Some("harness_channel") => Delivered::Socket,
                _ => Delivered::Orchestrator,
            });
        }
        anyhow::bail!("the Fleetlayer node hung up before answering")
    };
    Some(match tokio::time::timeout(TIMEOUT, ask).await {
        Ok(r) => r,
        Err(_) => Err(anyhow::anyhow!(
            "not confirmed within {} seconds: the message may not have been delivered to {to}. Try again",
            TIMEOUT.as_secs()
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn no_node_means_the_deck_carries_it() {
        let dir = tempfile::tempdir().unwrap();
        assert!(send(&dir.path().join("node.sock"), "triage", "hi").await.is_none());
    }

    /// A fake Fleetlayer node that answers one request with `answer` (its `id` filled in) and returns the request.
    async fn node(socket: &Path, answer: Value) -> tokio::task::JoinHandle<Value> {
        let listener = tokio::net::UnixListener::bind(socket).unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (r, mut w) = stream.into_split();
            let req: Value =
                serde_json::from_str(&BufReader::new(r).lines().next_line().await.unwrap().unwrap()).unwrap();
            let mut answer = answer;
            answer["id"] = req["id"].clone();
            w.write_all(format!("{answer}\n").as_bytes()).await.unwrap();
            req
        })
    }

    #[tokio::test]
    async fn a_message_goes_to_the_node_with_its_caller() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("node.sock");
        let seen = node(&socket, json!({"jsonrpc": "2.0", "result": {"via": "harness_channel"}})).await;
        let r = send(&socket, "planning@nathans-mac", "hello").await.unwrap().unwrap();
        assert_eq!(r, Delivered::Socket);
        let req = seen.await.unwrap();
        assert_eq!(req["method"], "_fleetlayer.dev/message/send");
        assert_eq!(req["params"]["to"], "planning@nathans-mac");
        assert_eq!(req["params"]["text"], "hello");
        assert!(req["params"]["caller"].is_object());
    }

    #[tokio::test]
    async fn fleetlayers_refusal_is_the_answer() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("node.sock");
        let _seen = node(
            &socket,
            json!({"jsonrpc": "2.0", "error": {"code": -32014,
            "message": "no session named 'planing' here or on any other node; did you mean 'planning@nathans-mac'?"}}),
        )
        .await;
        let e = send(&socket, "planing", "hello").await.unwrap().unwrap_err();
        assert!(e.to_string().contains("did you mean 'planning@nathans-mac'"), "{e}");
    }
}
