//! The node's local socket (mode 0600): one JSON line in, one JSON line out. The CLI and the MCP headers helper ask
//! for a session token here; the menu bar app reads status and usage.
//!
//! A caller claiming to be a Claude Code session is checked: the claimed `CLAUDE_PID` must be an ancestor of the
//! connecting process, and that process's registry entry must carry the claimed session ID. A caller that doesn't
//! pass gets a token for a plain shell on this machine.

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
            version: crate::link::VERSION.to_string(),
            runner: false,
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
        LocalRequest::Token { session_id, claude_pid, pane } => {
            let session = verify_session(node, peer_pid, session_id, claude_pid, pane).await;
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
    pane: Option<String>,
) -> Option<TokenSession> {
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
    Some(TokenSession { session_id: Some(reg.session_id), tab, pane })
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
        return rest.split_whitespace().nth(1)?.parse().ok();
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
    async fn this_process_descends_from_its_parent() {
        let me = std::process::id();
        let parent = parent_of(me).await.unwrap();
        assert!(is_ancestor(parent, me).await);
        assert!(is_ancestor(me, me).await);
        assert!(!is_ancestor(me, parent).await);
    }
}
