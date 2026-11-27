//! Fleetlayer's session registry as the node's list of sessions (the cutover's step 5, agreed with the quietdeck and
//! fleetlayer tabs, 27 Nov 2026).
//!
//! Where this machine runs a Fleetlayer node, the sessions come from its registry (`node/sessions` on its local
//! socket): the same session IDs (the harness's own: Claude Code's or Codex's), tabs and addresses herdr reports,
//! since Fleetlayer's herdr plugin passes them through. Everything else, opening, closing, interrupting and typing
//! into a session, goes to the orchestrator inside (herdr) exactly as before, so no delivery changes path here: the
//! deck's words and the runner's never reach an agent through Fleetlayer by this route.
//!
//! Without Fleetlayer's node (not installed, or stopped), the orchestrator inside answers for the sessions too.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use jiff::Timestamp;
use quietdeck_types::directory::SessionInfo;
use quietdeck_types::node::Delivered;
use quietdeck_types::{AgentKind, AgentStatus};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::{EventStream, OpenTab, Opened, Orchestrator, StreamError};

const ASK_WITHIN: Duration = Duration::from_secs(5);

pub struct Fleetlayer {
    pub socket: PathBuf,
    /// Acts on sessions, and lists them when Fleetlayer's node doesn't answer.
    pub inner: Box<dyn Orchestrator>,
}

impl Fleetlayer {
    pub fn new(inner: Box<dyn Orchestrator>) -> Fleetlayer {
        Fleetlayer { socket: crate::fleetlayer::node_socket(), inner }
    }

    /// Fleetlayer's running sessions on this node, or None when its node doesn't answer.
    async fn registry(&self) -> Option<anyhow::Result<Vec<SessionInfo>>> {
        let stream = tokio::net::UnixStream::connect(&self.socket).await.ok()?;
        let ask = async move {
            let (r, mut w) = stream.into_split();
            w.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"node/sessions\",\"params\":{}}\n").await?;
            let mut lines = BufReader::new(r).lines();
            while let Some(line) = lines.next_line().await? {
                let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
                if msg.get("id") != Some(&json!(1)) || msg.get("method").is_some() {
                    continue;
                }
                if let Some(e) = msg.get("error") {
                    anyhow::bail!("Fleetlayer's node/sessions: {}", e["message"].as_str().unwrap_or("failed"));
                }
                let listed: Listed = serde_json::from_value(msg["result"].clone())?;
                return Ok(listed.sessions.into_iter().filter_map(Listed::info).collect());
            }
            anyhow::bail!("Fleetlayer's node hung up before answering")
        };
        Some(match tokio::time::timeout(ASK_WITHIN, ask).await {
            Ok(r) => r,
            Err(_) => Err(anyhow::anyhow!("Fleetlayer's node didn't list its sessions within 5 seconds")),
        })
    }
}

/// `node/sessions`' answer. Unknown fields are ignored: Fleetlayer only adds fields.
#[derive(Debug, Deserialize)]
struct Listed {
    sessions: Vec<FlSession>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FlSession {
    session_id: String,
    cwd: Option<String>,
    #[serde(default)]
    meta: FlMeta,
    ended: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FlMeta {
    harness: Option<String>,
    address: Option<String>,
    #[serde(default)]
    named: bool,
    workspace: Option<String>,
    tab: Option<String>,
    status: Option<String>,
    status_changed_at: Option<Timestamp>,
    #[serde(default)]
    running: bool,
    open_url: Option<String>,
    opens_in: Option<String>,
}

impl Listed {
    /// Quietdeck's record of a running session. Left out: ended sessions, and ones without their real ID yet
    /// (`@<pane>`, `named: false`: an agent still starting), as herdr's own list leaves them out.
    fn info(s: FlSession) -> Option<SessionInfo> {
        if !s.meta.running || s.ended.as_ref().is_some_and(|e| !e.is_null()) || !s.meta.named {
            return None;
        }
        let agent = match s.meta.harness.as_deref() {
            Some("claude") => AgentKind::Claude,
            Some("codex") => AgentKind::Codex,
            _ => return None,
        };
        Some(SessionInfo {
            session_id: s.session_id,
            agent,
            workspace: s.meta.workspace,
            tab: s.meta.tab,
            address: s.meta.address,
            cwd: s.cwd,
            status: match s.meta.status.as_deref() {
                Some("idle") => AgentStatus::Idle,
                Some("done") => AgentStatus::Done,
                Some("working") => AgentStatus::Working,
                Some("blocked") => AgentStatus::Blocked,
                _ => AgentStatus::Unknown,
            },
            status_changed_at: s.meta.status_changed_at.unwrap_or_else(Timestamp::now),
            resume_command: None,
            running: true,
            open_url: s.meta.open_url,
            opens_in: s.meta.opens_in,
        })
    }
}

#[async_trait]
impl Orchestrator for Fleetlayer {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    async fn sessions(&self) -> anyhow::Result<Vec<SessionInfo>> {
        match self.registry().await {
            Some(listed) => listed,
            None => self.inner.sessions().await,
        }
    }

    /// The inner orchestrator's events, for prompt status changes (the list itself comes from Fleetlayer). Where it
    /// isn't reachable but Fleetlayer's node is, refused: the node reads the list every few seconds.
    async fn subscribe(&self) -> Result<Box<dyn EventStream>, StreamError> {
        match self.inner.subscribe().await {
            Err(StreamError::Io(e)) if tokio::net::UnixStream::connect(&self.socket).await.is_ok() => Err(
                StreamError::Refused(format!("{} isn't reachable ({e}); reading Fleetlayer's list", self.inner.name())),
            ),
            other => other,
        }
    }

    async fn open(&self, tab: &OpenTab) -> Result<Opened, String> {
        self.inner.open(tab).await
    }

    async fn close(&self, session_id: &str) -> Result<(), String> {
        self.inner.close(session_id).await
    }

    async fn interrupt(&self, session_id: &str) -> Result<(), String> {
        self.inner.interrupt(session_id).await
    }

    async fn deliver(&self, address: &str, text: &str) -> Result<Delivered, String> {
        self.inner.deliver(address, text).await
    }

    fn own_messages(&self) -> Vec<String> {
        self.inner.own_messages()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// An orchestrator that counts deliveries and lists one session of its own.
    #[derive(Default)]
    struct Inner {
        delivered: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Orchestrator for Inner {
        fn name(&self) -> &'static str {
            "herdr"
        }
        async fn sessions(&self) -> anyhow::Result<Vec<SessionInfo>> {
            Ok(vec![SessionInfo {
                session_id: "from-herdr".into(),
                agent: AgentKind::Claude,
                workspace: None,
                tab: Some("ci".into()),
                address: Some("w1:p1".into()),
                cwd: None,
                status: AgentStatus::Idle,
                status_changed_at: Timestamp::now(),
                resume_command: None,
                running: true,
                open_url: None,
                opens_in: None,
            }])
        }
        async fn subscribe(&self) -> Result<Box<dyn EventStream>, StreamError> {
            Err(StreamError::Io(std::io::Error::other("herdr isn't running")))
        }
        async fn open(&self, _: &OpenTab) -> Result<Opened, String> {
            Err("no".into())
        }
        async fn close(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        async fn interrupt(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        async fn deliver(&self, _: &str, _: &str) -> Result<Delivered, String> {
            self.delivered.fetch_add(1, Ordering::SeqCst);
            Ok(Delivered::Orchestrator)
        }
        fn own_messages(&self) -> Vec<String> {
            vec![]
        }
    }

    /// A fake Fleetlayer node answering `node/sessions` with `sessions`, and failing the test on anything else.
    fn node(socket: &std::path::Path, sessions: Value) {
        let listener = tokio::net::UnixListener::bind(socket).unwrap();
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let sessions = sessions.clone();
                tokio::spawn(async move {
                    let (r, mut w) = stream.into_split();
                    let mut lines = BufReader::new(r).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let req: Value = serde_json::from_str(&line).unwrap();
                        assert_eq!(req["method"], "node/sessions", "only the list goes to Fleetlayer");
                        let answer = json!({"jsonrpc": "2.0", "id": req["id"],
                                            "result": {"node": "agents", "sessions": sessions}});
                        w.write_all(format!("{answer}\n").as_bytes()).await.unwrap();
                    }
                });
            }
        });
    }

    /// Two running sessions as Fleetlayer 0.3.7 lists them (copied from `fleetlayer --json sessions` on the agents
    /// VM, 27 Nov 2026), one ended, and one still starting.
    fn listed() -> Value {
        json!([
            {"sessionId": "1b0d551b-273b-4173-9f27-86f9ef2e0603", "manager": "herdr", "harnessPlugin": "claude-code",
             "cwd": "/home/nathan/code/quietdeck-cutover", "title": "quietdeck cutover",
             "updatedAt": "2026-11-27T06:10:17.221513032Z",
             "meta": {"harness": "claude", "address": "w1:pJ", "named": true, "workspace": "w1",
                      "tab": "quietdeck cutover", "status": "working",
                      "statusChangedAt": "2026-11-27T06:10:17.221512161Z", "running": true,
                      "openUrl": "herdr://open?machine=agents&tab=quietdeck%20cutover", "opensIn": "herdr",
                      "deliverable": true, "somethingNew": 1},
             "ended": null},
            {"sessionId": "019a4f0c-codex", "manager": "herdr", "cwd": "/home/nathan/code/app",
             "meta": {"harness": "codex", "address": "w1:p9", "named": true, "tab": "api", "status": "idle",
                      "running": true}, "ended": null},
            {"sessionId": "old", "meta": {"harness": "claude", "named": true, "tab": "gone", "running": false},
             "ended": {"at": "2026-11-27T05:00:00Z", "reason": "exited"}},
            {"sessionId": "@w1:pZ", "meta": {"harness": "claude", "address": "w1:pZ", "named": false,
             "tab": "starting", "running": true}, "ended": null}
        ])
    }

    #[tokio::test]
    async fn sessions_keep_the_ids_tabs_and_addresses_herdr_gives_them() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("node.sock");
        node(&socket, listed());
        let fl = Fleetlayer { socket, inner: Box::new(Inner::default()) };
        let sessions = fl.sessions().await.unwrap();
        assert_eq!(sessions.len(), 2, "ended and still-starting sessions are left out: {sessions:?}");
        let me = &sessions[0];
        assert_eq!(me.session_id, "1b0d551b-273b-4173-9f27-86f9ef2e0603");
        assert_eq!(me.tab.as_deref(), Some("quietdeck cutover"));
        assert_eq!(me.address.as_deref(), Some("w1:pJ"));
        assert_eq!(me.workspace.as_deref(), Some("w1"));
        assert_eq!(me.agent, AgentKind::Claude);
        assert_eq!(me.status, AgentStatus::Working);
        assert_eq!(me.opens_in.as_deref(), Some("herdr"));
        assert!(me.running);
        let codex = &sessions[1];
        assert_eq!(
            (codex.session_id.as_str(), codex.agent, codex.tab.as_deref()),
            ("019a4f0c-codex", AgentKind::Codex, Some("api"))
        );
    }

    #[tokio::test]
    async fn without_fleetlayers_node_herdr_lists_them() {
        let dir = tempfile::tempdir().unwrap();
        let fl = Fleetlayer { socket: dir.path().join("none.sock"), inner: Box::new(Inner::default()) };
        assert_eq!(fl.sessions().await.unwrap()[0].session_id, "from-herdr");
        assert!(matches!(fl.subscribe().await, Err(StreamError::Io(_))), "herdr down and no Fleetlayer: as before");
    }

    #[tokio::test]
    async fn typing_into_a_session_never_goes_through_fleetlayer() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("node.sock");
        node(&socket, listed());
        let delivered = Arc::new(AtomicUsize::new(0));
        let fl = Fleetlayer { socket, inner: Box::new(Inner { delivered: delivered.clone() }) };
        // A wake-up the deliverer falls back to typing: the inner orchestrator types it, and the fake node fails the
        // test if anything but node/sessions reaches it.
        let text = quietdeck_types::relay::DeliveryKind::Wakeup.render("New mail: 1 Personal.");
        assert_eq!(fl.deliver("w1:pJ", &text).await, Ok(Delivered::Orchestrator));
        assert_eq!(delivered.load(Ordering::SeqCst), 1);
        assert!(matches!(fl.subscribe().await, Err(StreamError::Refused(_))), "herdr down, Fleetlayer up: poll it");
    }
}
