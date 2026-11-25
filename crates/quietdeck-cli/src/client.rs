//! The local node (for tokens) and the deck's REST API.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Nothing the CLI does may hang: every call to the node and the deck has a limit, and says which one ran out.
/// The local node answers in milliseconds; a deck call gets 3 seconds to connect and 30 in all unless the command
/// says otherwise (a message gets 5 seconds end to end, starting a session waits for the agent to come up).
pub const NODE_TIMEOUT: Duration = Duration::from_secs(10);
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
pub const DECK_TIMEOUT: Duration = Duration::from_secs(30);

/// An HTTP client with the CLI's limits.
pub fn http(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(timeout)
        .build()
        .expect("the HTTP client builds")
}

use quietdeck_types::api::ApiError;
use quietdeck_types::node::{LocalRequest, LocalResponse, NodeStatus};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("QUIETDECK_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| home().join(".config/quietdeck"))
}

pub fn tilde(p: &Path) -> String {
    match p.strip_prefix(home()) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

/// The node's socket: the same rules as the node's own `socket_path`.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("QUIETDECK_NODE_SOCKET") {
        return PathBuf::from(p);
    }
    if cfg!(target_os = "macos") {
        home().join("Library/Application Support/quietdeck/node.sock")
    } else if let Some(run) = std::env::var_os("XDG_RUNTIME_DIR") {
        PathBuf::from(run).join("quietdeck/node.sock")
    } else if let Some(sock) = user_runtime_socket() {
        // No XDG_RUNTIME_DIR (`sudo -u <user>`, a cron job): the node, a systemd user service, still listens in the
        // user's runtime directory.
        sock
    } else {
        std::env::var_os("QUIETDECK_STATE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".local/state/quietdeck"))
            .join("node.sock")
    }
}

/// `/run/user/<uid>/quietdeck/node.sock`, when it's there. The uid is this process's: /proc/self belongs to it.
fn user_runtime_socket() -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let uid = std::fs::metadata("/proc/self").ok()?.uid();
    let sock = PathBuf::from(format!("/run/user/{uid}/quietdeck/node.sock"));
    sock.exists().then_some(sock)
}

pub async fn ask_node(req: &LocalRequest) -> anyhow::Result<LocalResponse> {
    match tokio::time::timeout(NODE_TIMEOUT, ask_node_once(req)).await {
        Ok(r) => r,
        Err(_) => anyhow::bail!(
            "this machine's Quietdeck node didn't answer within {} seconds (restarting?): try again",
            NODE_TIMEOUT.as_secs()
        ),
    }
}

async fn ask_node_once(req: &LocalRequest) -> anyhow::Result<LocalResponse> {
    let path = socket_path();
    let stream = tokio::net::UnixStream::connect(&path).await.map_err(|e| {
        anyhow::anyhow!(
            "this machine's Quietdeck node isn't running ({}: {e}). Start it with quietdeck-node",
            path.display()
        )
    })?;
    let (r, mut w) = stream.into_split();
    w.write_all(format!("{}\n", serde_json::to_string(req)?).as_bytes()).await?;
    let line = BufReader::new(r).lines().next_line().await?.ok_or_else(|| anyhow::anyhow!("the node hung up"))?;
    let resp: LocalResponse = serde_json::from_str(&line)?;
    if let LocalResponse::Err { error } = resp {
        anyhow::bail!("the node: {error}");
    }
    Ok(resp)
}

pub async fn node_status() -> anyhow::Result<NodeStatus> {
    match ask_node(&LocalRequest::Status).await? {
        LocalResponse::Status(s) => Ok(s),
        other => anyhow::bail!("unexpected answer from the node: {other:?}"),
    }
}

pub struct Token {
    pub token: String,
    pub deck_url: String,
}

/// A session token for this process, naming the Claude Code session it runs in when there is one.
pub async fn session_token() -> anyhow::Result<Token> {
    let req = LocalRequest::Token {
        session_id: std::env::var("CLAUDE_CODE_SESSION_ID").ok().filter(|s| !s.is_empty()),
        claude_pid: std::env::var("CLAUDE_PID").ok().and_then(|p| p.parse().ok()),
        address: quietdeck_types::SESSION_ADDRESS_ENV
            .iter()
            .find_map(|name| std::env::var(name).ok().filter(|s| !s.is_empty())),
    };
    match ask_node(&req).await? {
        LocalResponse::Token { token, deck_url, .. } => Ok(Token { token, deck_url }),
        other => anyhow::bail!("unexpected answer from the node: {other:?}"),
    }
}

pub struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
}

/// A failed call, saying when it was a limit that ran out.
fn sent(e: reqwest::Error) -> anyhow::Error {
    if e.is_timeout() {
        anyhow::anyhow!("the deck didn't answer in time: {e}")
    } else if e.is_connect() {
        anyhow::anyhow!("couldn't reach the deck: {e}")
    } else {
        e.into()
    }
}

pub async fn read<T: DeserializeOwned>(resp: reqwest::Response) -> anyhow::Result<T> {
    let status = resp.status();
    let bytes = resp.bytes().await.map_err(sent)?;
    if status.is_success() {
        return serde_json::from_slice(&bytes).map_err(|e| anyhow::anyhow!("reading the deck's answer: {e}"));
    }
    match serde_json::from_slice::<ApiError>(&bytes) {
        Ok(e) => anyhow::bail!("{}", e.error),
        Err(_) => anyhow::bail!("the deck answered {status}: {}", String::from_utf8_lossy(&bytes).trim()),
    }
}

impl Client {
    pub async fn connect() -> anyhow::Result<Client> {
        Self::connect_with(DECK_TIMEOUT).await
    }

    /// With a limit of its own for each deck call.
    pub async fn connect_with(timeout: Duration) -> anyhow::Result<Client> {
        let t = session_token().await?;
        Ok(Client { http: http(timeout), base: t.deck_url, token: t.token })
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> anyhow::Result<T> {
        let resp = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .query(query)
            .send()
            .await
            .map_err(sent)?;
        read(resp).await
    }

    pub async fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> anyhow::Result<T> {
        let resp = self
            .http
            .post(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .map_err(sent)?;
        read(resp).await
    }
}

/// Mode 600 in a directory with mode 700.
pub fn write_secret_file(path: &Path, contents: &str) -> anyhow::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    f.write_all(contents.as_bytes())?;
    f.write_all(b"\n")?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}
