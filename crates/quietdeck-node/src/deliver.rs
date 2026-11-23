//! Putting a delivery in front of a session, the way `docs/claude-code-messaging.md` describes:
//! - a Claude Code session with a messaging socket gets one `user` frame on it, with an envelope that names the
//!   sender (`from-name`) and claims no permission mode;
//! - a Codex session, or a Claude Code session with no socket, gets `herdr agent prompt`.
//!
//! The only failure the node reports as "offline" is a session that isn't running. Nothing is queued or retried.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;

use quietdeck_types::AgentKind;
use quietdeck_types::node::Delivered;
use quietdeck_types::relay::Delivery;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

/// The deck reads this prefix as "the session is offline".
pub const OFFLINE_PREFIX: &str = "offline:";

/// The Claude Code messaging protocol this node speaks.
pub const PEER_PROTOCOL: u64 = 1;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registration {
    pub pid: u32,
    pub session_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub peer_protocol: Option<u64>,
    #[serde(default)]
    pub messaging_socket_path: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KeyFile {
    peer_token: String,
}

pub struct Deliverer {
    pub claude_sessions_dir: PathBuf,
    pub herdr: String,
    /// IDs of recent deliveries: a repeat is refused, so nothing reaches a session twice.
    seen: Mutex<VecDeque<String>>,
}

const SEEN_LIMIT: usize = 512;

impl Deliverer {
    pub fn new(home: &Path, herdr: String) -> Deliverer {
        Deliverer { claude_sessions_dir: home.join(".claude/sessions"), herdr, seen: Mutex::default() }
    }

    pub async fn deliver(&self, d: &Delivery) -> Result<Delivered, String> {
        {
            let mut seen = self.seen.lock().await;
            if seen.contains(&d.id) {
                return Err(format!("delivery {} was already handed over", d.id));
            }
            seen.push_back(d.id.clone());
            if seen.len() > SEEN_LIMIT {
                seen.pop_front();
            }
        }
        let text = d.render();
        if d.target.agent == AgentKind::Claude
            && let Some(session_id) = &d.target.session_id
        {
            match find_registration(&self.claude_sessions_dir, session_id).await {
                Some(reg) if usable(&reg) => {
                    return send_socket(&self.claude_sessions_dir, &reg, &d.from_name(), &text).await;
                }
                Some(_) => {} // no socket, or another protocol: herdr below
                None if d.target.pane.is_none() => {
                    return Err(format!("{OFFLINE_PREFIX} no running Claude Code session {session_id}"));
                }
                None => {}
            }
        }
        let pane = d
            .target
            .pane
            .as_deref()
            .ok_or_else(|| format!("{OFFLINE_PREFIX} the session has no herdr pane and no messaging socket"))?;
        herdr_prompt(&self.herdr, pane, &text).await
    }
}

fn usable(reg: &Registration) -> bool {
    reg.peer_protocol == Some(PEER_PROTOCOL) && reg.messaging_socket_path.is_some() && pid_alive(reg.pid)
}

pub fn pid_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else { return false };
    // Signal 0 checks the process exists; EPERM means it exists but isn't ours.
    match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None) {
        Ok(()) => true,
        Err(e) => e == nix::errno::Errno::EPERM,
    }
}

/// Every interactive session's registry file.
pub async fn registrations(dir: &Path) -> Vec<Registration> {
    let mut out = Vec::new();
    let Ok(mut rd) = tokio::fs::read_dir(dir).await else { return out };
    while let Ok(Some(entry)) = rd.next_entry().await {
        let p = entry.path();
        if p.extension().is_some_and(|e| e == "json")
            && let Ok(bytes) = tokio::fs::read(&p).await
            && let Ok(reg) = serde_json::from_slice::<Registration>(&bytes)
        {
            out.push(reg);
        }
    }
    out
}

pub async fn find_registration(dir: &Path, session_id: &str) -> Option<Registration> {
    let mut found: Vec<Registration> =
        registrations(dir).await.into_iter().filter(|r| r.session_id == session_id).collect();
    // A resumed session can leave a stale file from its old process: the live one wins.
    found.sort_by_key(|r| !pid_alive(r.pid));
    found.into_iter().next()
}

pub async fn registration_for_pid(dir: &Path, pid: u32) -> Option<Registration> {
    let bytes = tokio::fs::read(dir.join(format!("{pid}.json"))).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Claude Code's envelope: the receiver parses it and re-serialises it, so attributes must be escaped as it would.
pub fn envelope(from_name: &str, body: &str) -> String {
    let name = from_name.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;");
    format!("<cross-session-message from-name=\"{name}\">\n{body}\n</cross-session-message>")
}

fn key_path(dir: &Path, reg: &Registration, socket: &str) -> PathBuf {
    let hash = hex::encode(Sha256::digest(socket.as_bytes()));
    dir.join(format!("{}.{hash}.key", reg.pid))
}

async fn send_socket(dir: &Path, reg: &Registration, from_name: &str, text: &str) -> Result<Delivered, String> {
    let socket = reg.messaging_socket_path.as_deref().expect("checked by usable()");
    let key: KeyFile = tokio::fs::read(key_path(dir, reg, socket))
        .await
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| "the session's messaging key isn't readable".to_string())?;
    let mut stream = match tokio::time::timeout(Duration::from_secs(3), tokio::net::UnixStream::connect(socket)).await {
        Ok(Ok(s)) => s,
        // The socket refuses: the session is gone.
        _ => return Err(format!("{OFFLINE_PREFIX} the session's messaging socket refused the connection")),
    };
    let auth = serde_json::json!({"type": "auth", "token": key.peer_token});
    let frame = serde_json::json!({
        "type": "user",
        "message": {"role": "user", "content": envelope(from_name, text)},
        "msg_id": uuid_like(),
        "priority": "next",
        "session_id": reg.session_id,
    });
    let payload = format!("{auth}\n{frame}\n");
    stream.write_all(payload.as_bytes()).await.map_err(|e| format!("writing to the session's socket: {e}"))?;
    // An accepted message gets no answer. A refused one may: read briefly for an error line.
    let mut buf = vec![0u8; 4096];
    match tokio::time::timeout(Duration::from_millis(300), stream.read(&mut buf)).await {
        Ok(Ok(n)) if n > 0 => {
            let reply = String::from_utf8_lossy(&buf[..n]);
            if reply.contains("error") || reply.contains("denied") || reply.contains("unauthor") {
                return Err(format!("the session refused the message: {}", reply.trim()));
            }
            Ok(Delivered::Socket)
        }
        _ => Ok(Delivered::Socket),
    }
}

fn uuid_like() -> String {
    let b: [u8; 16] = rand::random();
    let h = hex::encode(b);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

pub async fn herdr_prompt(herdr: &str, pane: &str, text: &str) -> Result<Delivered, String> {
    let out = tokio::process::Command::new(herdr)
        .args(["agent", "prompt", pane, text])
        .output()
        .await
        .map_err(|e| format!("running {herdr}: {e}"))?;
    if out.status.success() {
        return Ok(Delivered::Herdr);
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let err = if err.is_empty() { String::from_utf8_lossy(&out.stdout).trim().to_string() } else { err };
    if err.contains("not found") || err.contains("no such pane") || err.contains("unknown pane") {
        return Err(format!("{OFFLINE_PREFIX} {err}"));
    }
    Err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use quietdeck_types::relay::{DeliveryKind, DeliveryTarget};

    #[test]
    fn the_envelope_escapes_the_name_and_claims_no_mode() {
        let e = envelope("a \"b\" <c>", "hello");
        assert_eq!(e, "<cross-session-message from-name=\"a &quot;b&quot; &lt;c&gt;\">\nhello\n</cross-session-message>");
        assert!(!e.contains("from-mode"));
    }

    #[test]
    fn the_key_file_hashes_the_socket_path_as_written() {
        let reg = Registration {
            pid: 42,
            session_id: "s".into(),
            cwd: None,
            peer_protocol: Some(1),
            messaging_socket_path: Some("/tmp/cc-socks/42.sock".into()),
            name: None,
            status: None,
            kind: None,
        };
        let p = key_path(Path::new("/x"), &reg, "/tmp/cc-socks/42.sock");
        let hash = hex::encode(Sha256::digest(b"/tmp/cc-socks/42.sock"));
        assert_eq!(p, PathBuf::from(format!("/x/42.{hash}.key")));
    }

    fn delivery(id: &str, session_id: &str, pane: Option<&str>) -> Delivery {
        Delivery {
            id: id.into(),
            target: DeliveryTarget {
                tab: "t".into(),
                session_id: Some(session_id.into()),
                pane: pane.map(Into::into),
                agent: AgentKind::Claude,
            },
            kind: DeliveryKind::Deck,
            body: "hi".into(),
        }
    }

    #[tokio::test]
    async fn a_session_that_isnt_running_is_offline() {
        let dir = tempfile::tempdir().unwrap();
        let d = Deliverer::new(dir.path(), "herdr-that-does-not-exist".into());
        let err = d.deliver(&delivery("d1", "nope", None)).await.unwrap_err();
        assert!(err.starts_with(OFFLINE_PREFIX), "{err}");
    }

    #[tokio::test]
    async fn a_repeat_delivery_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let d = Deliverer::new(dir.path(), "herdr-that-does-not-exist".into());
        let _ = d.deliver(&delivery("d1", "nope", None)).await;
        assert!(d.deliver(&delivery("d1", "nope", None)).await.unwrap_err().contains("already"));
    }

    /// A fake Claude Code session: a registry file, a key file and a socket that records what it gets.
    #[tokio::test]
    async fn a_claude_session_gets_one_frame_on_its_socket() {
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join(".claude/sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let sock = home.path().join("s.sock");
        let sock_s = sock.to_str().unwrap().to_string();
        let pid = std::process::id();
        std::fs::write(
            sessions.join(format!("{pid}.json")),
            serde_json::json!({"pid": pid, "sessionId": "s1", "peerProtocol": 1, "messagingSocketPath": sock_s}).to_string(),
        )
        .unwrap();
        let reg = registration_for_pid(&sessions, pid).await.unwrap();
        std::fs::write(key_path(&sessions, &reg, &sock_s), r#"{"peerToken":"abc"}"#).unwrap();
        let listener = tokio::net::UnixListener::bind(&sock).unwrap();
        let server = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut got = String::new();
            s.read_to_string(&mut got).await.unwrap();
            got
        });
        let d = Deliverer::new(home.path(), "herdr-that-does-not-exist".into());
        let mut del = delivery("d2", "s1", None);
        del.kind = DeliveryKind::Wakeup;
        del.body = "New mail: 1 Personal".into();
        // The fake session closes after reading; the deliverer waits briefly for an error line, then closes.
        let r = d.deliver(&del).await.unwrap();
        assert_eq!(r, Delivered::Socket);
        let got = server.await.unwrap();
        let mut lines = got.lines();
        assert_eq!(lines.next().unwrap(), serde_json::json!({"type": "auth", "token": "abc"}).to_string());
        let frame: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert_eq!(frame["type"], "user");
        assert_eq!(frame["priority"], "next");
        let content = frame["message"]["content"].as_str().unwrap();
        assert!(content.starts_with("<cross-session-message from-name=\"Quietdeck runner\">\nNew mail: 1 Personal"));
    }
}
