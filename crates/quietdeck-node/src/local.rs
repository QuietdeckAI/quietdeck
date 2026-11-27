//! The node's local socket (mode 0600): one JSON line in, one JSON line out. The CLI and the MCP headers helper ask
//! for a session token here; the menu bar app reads status and usage.
//!
//! A caller claiming to be a Claude Code session is checked: the claimed `CLAUDE_PID` must be an ancestor of the
//! connecting process, and that process's registry entry must carry the claimed session ID. A caller that doesn't
//! pass gets a token for a plain shell on this machine.
//!
//! A headless turn of the runner's agent (`claude -p`, see `runner::headless`) has no tab and no registry entry that
//! names one, so the node keeps its own record of the turns running now ([`HeadlessTurns`]): a caller descended from
//! one is that turn, with the agent's tab and the turn's session ID. Only the runner inside this process fills it.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use quietdeck_types::node::{LocalRequest, LocalResponse, MintedToken, NodeRequest, NodeStatus, TokenSession};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;

use crate::Node;

/// A cached token is handed out until it has this long left.
const REFRESH_BEFORE: i64 = 5 * 60;

#[derive(Default)]
pub struct TokenCache {
    by_caller: Mutex<HashMap<String, MintedToken>>,
}

/// The headless agent turns running now, by the pid of each turn's `claude` process.
#[derive(Debug, Default)]
pub struct HeadlessTurns(std::sync::Mutex<HashMap<u32, (u64, TokenSession)>>);

/// A turn's place in [`HeadlessTurns`], given up when it's dropped: when the child exits, is stopped on a timeout
/// or the turn's future is dropped, so a pid used again later never inherits the identity.
pub struct TurnRegistration {
    turns: Arc<HeadlessTurns>,
    pid: u32,
    seq: u64,
}

impl Drop for TurnRegistration {
    fn drop(&mut self) {
        let mut map = self.turns.0.lock().expect("headless turns");
        if map.get(&self.pid).is_some_and(|(seq, _)| *seq == self.seq) {
            map.remove(&self.pid);
        }
    }
}

impl HeadlessTurns {
    /// Record a turn that has just started, for as long as the returned registration lives.
    pub fn begin(self: &Arc<Self>, pid: u32, tab: String, session_id: String) -> TurnRegistration {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let session = TokenSession { session_id: Some(session_id), tab: Some(tab), address: None };
        self.0.lock().expect("headless turns").insert(pid, (seq, session));
        TurnRegistration { turns: self.clone(), pid, seq }
    }

    /// The turns running now.
    #[cfg(test)]
    pub fn running(&self) -> Vec<TokenSession> {
        self.0.lock().expect("headless turns").values().map(|(_, s)| s.clone()).collect()
    }

    fn get(&self, pid: u32) -> Option<TokenSession> {
        self.0.lock().expect("headless turns").get(&pid).map(|(_, s)| s.clone())
    }

    /// The running turn `pid` is or descends from, if any.
    pub async fn caller(&self, pid: u32) -> Option<TokenSession> {
        if self.0.lock().expect("headless turns").is_empty() {
            return None;
        }
        let mut current = pid;
        for _ in 0..64 {
            if let Some(s) = self.get(current) {
                return Some(s);
            }
            match parent_of(current).await {
                Some(p) if p > 1 && p != current => current = p,
                _ => return None,
            }
        }
        None
    }
}

pub async fn serve(node: Arc<Node>, path: &Path) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    tracing::info!(socket = %path.display(), "local socket listening");
    loop {
        let (stream, _) = listener.accept().await?;
        let node = node.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(node, stream).await {
                tracing::debug!(error = %e, "local socket client");
            }
        });
    }
}

async fn handle(node: Arc<Node>, stream: UnixStream) -> anyhow::Result<()> {
    let peer_pid = stream.peer_cred().ok().and_then(|c| c.pid()).and_then(|p| u32::try_from(p).ok());
    let (r, mut w) = stream.into_split();
    let mut lines = BufReader::new(r).lines();
    while let Some(line) = tokio::time::timeout(Duration::from_secs(30), lines.next_line()).await?? {
        let resp = match serde_json::from_str::<LocalRequest>(&line) {
            Ok(req) => respond(&node, req, peer_pid).await,
            Err(e) => LocalResponse::Err { error: format!("unreadable request: {e}") },
        };
        let mut out = serde_json::to_string(&resp)?;
        out.push('\n');
        w.write_all(out.as_bytes()).await?;
    }
    Ok(())
}

async fn respond(node: &Node, req: LocalRequest, peer_pid: Option<u32>) -> LocalResponse {
    match req {
        LocalRequest::Status => LocalResponse::Status(NodeStatus {
            name: node.config.node.name.clone(),
            deck_url: node.config.deck_url(),
            connected: node.link.is_connected(),
            version: crate::link::version(),
            runner: node.runner.get().is_some(),
        }),
        LocalRequest::Usage => match &node.usage {
            Some(u) => LocalResponse::Usage { accounts: u.snapshot().await },
            None => LocalResponse::Usage { accounts: vec![] },
        },
        LocalRequest::ImportSummaries { dir } => match &node.summaries {
            Some(s) => match s.import_claude_summarizer(dir.into()).await {
                Ok(records) => LocalResponse::Imported { records },
                Err(e) => LocalResponse::Err { error: e.to_string() },
            },
            None => LocalResponse::Err { error: "summaries are off in node.yaml".into() },
        },
        LocalRequest::Token { session_id, claude_pid, address } => {
            let session = verify_session(node, peer_pid, session_id, claude_pid, address).await;
            match token_for(node, session).await {
                Ok(t) => LocalResponse::Token {
                    token: t.token,
                    deck_url: node.config.deck_url(),
                    expires_at: t.expires_at,
                    caller: t.caller,
                },
                Err(error) => LocalResponse::Err { error },
            }
        }
    }
}

/// The session the caller really is, or None for a plain shell.
async fn verify_session(
    node: &Node,
    peer_pid: Option<u32>,
    session_id: Option<String>,
    claude_pid: Option<u32>,
    address: Option<String>,
) -> Option<TokenSession> {
    if let Some(peer) = peer_pid
        && let Some(turn) = node.headless_turns.caller(peer).await
    {
        return Some(turn);
    }
    let (peer, claude_pid) = (peer_pid?, claude_pid?);
    if !is_ancestor(claude_pid, peer).await {
        tracing::warn!(peer, claude_pid, "a caller claimed a Claude Code process that isn't its ancestor");
        return None;
    }
    let reg = crate::deliver::registration_for_pid(&node.deliverer.claude_sessions_dir, claude_pid).await?;
    if session_id.as_deref().is_some_and(|s| s != reg.session_id) {
        // Claude Code changes the session ID on /clear and resume; the registry is the truth.
        tracing::debug!("caller's CLAUDE_CODE_SESSION_ID differs from the registry; using the registry's");
    }
    let tab = node.sessions.lock().await.iter().find(|s| s.session_id == reg.session_id).and_then(|s| s.tab.clone());
    Some(TokenSession { session_id: Some(reg.session_id), tab, address })
}

async fn token_for(node: &Node, session: Option<TokenSession>) -> Result<MintedToken, String> {
    let key = match &session {
        Some(s) => format!("{}|{}", s.session_id.as_deref().unwrap_or(""), s.tab.as_deref().unwrap_or("")),
        None => "shell".to_string(),
    };
    let now = Timestamp::now().as_second();
    if let Some(t) = node.tokens.by_caller.lock().await.get(&key)
        && t.expires_at.as_second() - now > REFRESH_BEFORE
    {
        return Ok(t.clone());
    }
    let reply = node.link.request(NodeRequest::MintToken { session }, Duration::from_secs(10)).await?;
    let minted: MintedToken = reply.into_result()?;
    node.tokens.by_caller.lock().await.insert(key, minted.clone());
    Ok(minted)
}

/// Whether `ancestor` is `pid` or one of its parents.
pub async fn is_ancestor(ancestor: u32, pid: u32) -> bool {
    let mut current = pid;
    for _ in 0..64 {
        if current == ancestor {
            return true;
        }
        match parent_of(current).await {
            Some(p) if p > 1 && p != current => current = p,
            _ => return false,
        }
    }
    false
}

async fn parent_of(pid: u32) -> Option<u32> {
    #[cfg(target_os = "linux")]
    {
        let stat = tokio::fs::read_to_string(format!("/proc/{pid}/stat")).await.ok()?;
        // pid (comm) state ppid ...: comm can hold spaces and parentheses, so read after the last ')'.
        let rest = &stat[stat.rfind(')')? + 1..];
        rest.split_whitespace().nth(1)?.parse().ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let out =
            tokio::process::Command::new("ps").args(["-o", "ppid=", "-p", &pid.to_string()]).output().await.ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_a_running_turn_and_what_it_starts_get_its_identity() {
        let turns = Arc::new(HeadlessTurns::default());
        let me = std::process::id();
        let parent = parent_of(me).await.unwrap();
        let mut child = tokio::process::Command::new("sleep").arg("5").spawn().unwrap();
        let child_pid = child.id().unwrap();

        // Nothing registered: nobody is a turn.
        assert_eq!(turns.caller(child_pid).await, None);

        let reg = turns.begin(me, "email triage".into(), "s-1".into());
        let got = turns.caller(child_pid).await.expect("a process the turn started is the turn");
        assert_eq!(got.tab.as_deref(), Some("email triage"));
        assert_eq!(got.session_id.as_deref(), Some("s-1"));
        assert_eq!(turns.caller(me).await.unwrap().tab.as_deref(), Some("email triage"));
        // A process that doesn't descend from the turn (its parent) gets no tab.
        assert_eq!(turns.caller(parent).await, None);

        drop(reg);
        assert_eq!(turns.caller(child_pid).await, None, "the identity ends with the turn");

        // A later turn on the same pid isn't removed by an earlier registration going away.
        let old = turns.begin(me, "a".into(), "s-old".into());
        let new = turns.begin(me, "b".into(), "s-new".into());
        drop(old);
        assert_eq!(turns.caller(me).await.unwrap().session_id.as_deref(), Some("s-new"));
        drop(new);
        let _ = child.kill().await;
    }

    #[tokio::test]
    async fn this_process_descends_from_its_parent() {
        let me = std::process::id();
        let parent = parent_of(me).await.unwrap();
        assert!(is_ancestor(parent, me).await);
        assert!(is_ancestor(me, me).await);
        assert!(!is_ancestor(me, parent).await);
    }
}
