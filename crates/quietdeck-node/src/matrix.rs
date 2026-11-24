//! The Matrix reader (sc-7779): plain code, no AI. It signs in as the node's own Matrix user (`@quietdeck`), syncs
//! the rooms node.yaml names, and hands each new message to the deck (`messages_ingest`), which keeps it and tells
//! the runner. The agent reads the messages with `quietdeck messages` and never holds a Matrix token.
//!
//! - The password is read from a file only this user can read, once, to sign in. The device, its access token and
//!   its crypto store (rooms are end-to-end encrypted) stay in the state directory, mode 700, and never leave this
//!   machine.
//! - The first sign-in starts from now: what the room already held isn't handed over. After a restart, the stored
//!   sync token picks up where the reader stopped, and the deck drops any message it already has.
//! - Messages the deck hasn't taken yet wait in `outbox.json`, so a deck that's down or a restart loses none.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use matrix_sdk::authentication::matrix::MatrixSession;
use matrix_sdk::config::SyncSettings;
use matrix_sdk::ruma::OwnedRoomId;
use matrix_sdk::ruma::api::error::ErrorKind;
use matrix_sdk::ruma::events::room::encrypted::OriginalSyncRoomEncryptedEvent;
use matrix_sdk::ruma::events::room::message::OriginalSyncRoomMessageEvent;
use matrix_sdk::{Client, Room};
use quietdeck_types::message::{IncomingMessage, MAX_INGEST, MessagesStored};
use quietdeck_types::node::NodeRequest;
use serde::Deserialize;
use tokio::sync::{Mutex, Notify};

use crate::Node;
use crate::config::{expand, read_secret_file, state_dir};

const DECK_TIMEOUT: Duration = Duration::from_secs(30);
const RETRY: Duration = Duration::from_secs(30);
const SYNC_TIMEOUT: Duration = Duration::from_secs(30);
/// Messages kept for the deck at most; past this the oldest go, with a warning.
const OUTBOX_MAX: usize = 5_000;

/// node.yaml's `matrix:` section.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatrixConfig {
    /// The homeserver's URL (`https://matrix.example.com`).
    pub homeserver: String,
    /// The node's own Matrix user (`@quietdeck:example.com`).
    pub user: String,
    /// A file only this user can read, holding the user's password and nothing else. Read to sign in.
    pub password_file: String,
    /// The rooms to read, by ID (`!abc:example.com`). The user must already be a member.
    pub rooms: Vec<String>,
    /// Where the device, its token and its crypto store live. Default: `<state dir>/matrix`.
    #[serde(default)]
    pub store: Option<String>,
    /// The device's name, which other members see. Default: "Quietdeck (<node name>)".
    #[serde(default)]
    pub device_name: Option<String>,
}

impl MatrixConfig {
    pub fn check(&self) -> Result<(), String> {
        if !self.homeserver.starts_with("https://") && !self.homeserver.starts_with("http://") {
            return Err("matrix.homeserver must be an http(s) URL".into());
        }
        if !self.user.starts_with('@') || !self.user.contains(':') {
            return Err("matrix.user must be a full Matrix ID such as @quietdeck:example.com".into());
        }
        if self.rooms.is_empty() {
            return Err("matrix.rooms names no room".into());
        }
        for r in &self.rooms {
            OwnedRoomId::try_from(r.as_str()).map_err(|e| format!("matrix.rooms: {r:?} isn't a room ID: {e}"))?;
        }
        Ok(())
    }

    fn store_dir(&self) -> PathBuf {
        self.store.as_deref().map(expand).unwrap_or_else(|| state_dir().join("matrix"))
    }
}

// --- The outbox ------------------------------------------------------------------------------------------------------

/// Messages read but not yet taken by the deck, kept on disk.
pub struct Outbox {
    path: PathBuf,
    queue: Mutex<Vec<IncomingMessage>>,
    pub ready: Notify,
}

impl Outbox {
    pub fn load(path: PathBuf) -> Outbox {
        let queue: Vec<IncomingMessage> =
            std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Outbox { path, queue: Mutex::new(queue), ready: Notify::new() }
    }

    fn save(&self, queue: &[IncomingMessage]) {
        let tmp = self.path.with_extension("tmp");
        let write = || -> std::io::Result<()> {
            std::fs::write(&tmp, serde_json::to_vec(queue).expect("messages serialise"))?;
            std::fs::rename(&tmp, &self.path)
        };
        if let Err(e) = write() {
            tracing::warn!(error = %e, path = %self.path.display(), "saving the Matrix outbox");
        }
    }

    pub async fn push(&self, m: IncomingMessage) {
        let mut q = self.queue.lock().await;
        if q.iter().any(|x| x.channel == m.channel && x.external_id == m.external_id) {
            return;
        }
        q.push(m);
        if q.len() > OUTBOX_MAX {
            let drop = q.len() - OUTBOX_MAX;
            tracing::warn!(dropped = drop, "the Matrix outbox is full: dropping the oldest messages");
            q.drain(..drop);
        }
        self.save(&q);
        drop(q);
        self.ready.notify_one();
    }

    pub async fn len(&self) -> usize {
        self.queue.lock().await.len()
    }

    /// Hand everything waiting to `send`, a batch at a time, removing each batch it takes. Stops at the first failure.
    pub async fn flush<F, Fut>(&self, send: F) -> Result<u32, String>
    where
        F: Fn(Vec<IncomingMessage>) -> Fut,
        Fut: std::future::Future<Output = Result<MessagesStored, String>>,
    {
        let mut new = 0;
        loop {
            let batch: Vec<IncomingMessage> = {
                let q = self.queue.lock().await;
                q.iter().take(MAX_INGEST).cloned().collect()
            };
            if batch.is_empty() {
                return Ok(new);
            }
            let stored = send(batch.clone()).await?;
            new += stored.new;
            let mut q = self.queue.lock().await;
            q.retain(|m| !batch.iter().any(|b| b.channel == m.channel && b.external_id == m.external_id));
            self.save(&q);
        }
    }
}

/// Hands the outbox to the deck whenever something's waiting, retrying while the deck is away.
async fn forward(node: Arc<Node>, outbox: Arc<Outbox>) {
    loop {
        if outbox.len().await > 0 {
            let r = outbox
                .flush(|messages| {
                    let node = node.clone();
                    async move {
                        node.link
                            .request(NodeRequest::MessagesIngest { messages }, DECK_TIMEOUT)
                            .await?
                            .into_result::<MessagesStored>()
                    }
                })
                .await;
            match r {
                Ok(new) => tracing::info!(new, "handed Matrix messages to the deck"),
                Err(e) => {
                    let waiting = outbox.len().await;
                    tracing::info!(error = %e, waiting, "the deck didn't take the Matrix messages yet");
                    tokio::time::sleep(RETRY).await;
                    continue;
                }
            }
        }
        // A missed notification costs at most one retry interval.
        let _ = tokio::time::timeout(RETRY, outbox.ready.notified()).await;
    }
}

// --- The reader ------------------------------------------------------------------------------------------------------

fn private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)
}

/// Start the reader and its forwarder. Runs until the node stops; errors are logged and retried.
pub fn spawn(node: Arc<Node>, cfg: MatrixConfig) {
    let dir = cfg.store_dir();
    if let Err(e) = private_dir(&dir) {
        tracing::error!(error = %e, dir = %dir.display(), "the Matrix reader can't use its state directory: it's off");
        return;
    }
    let outbox = Arc::new(Outbox::load(dir.join("outbox.json")));
    tokio::spawn(forward(node.clone(), outbox.clone()));
    tokio::spawn(async move {
        loop {
            match read(&node.config.node.name, &cfg, &dir, outbox.clone()).await {
                Ok(()) => tracing::warn!("the Matrix sync stopped: starting again"),
                Err(Restart::Fresh(why)) => {
                    // The device is gone (signed out elsewhere, or deleted): a new one needs a new crypto store.
                    tracing::warn!(%why, "the Matrix device is no longer valid: signing in again with a new device");
                    let _ = std::fs::remove_file(dir.join("session.json"));
                    let _ = std::fs::remove_dir_all(dir.join("sdk"));
                }
                Err(Restart::Later(why)) => tracing::warn!(%why, "the Matrix reader failed: trying again"),
            }
            tokio::time::sleep(RETRY).await;
        }
    });
}

/// `quietdeck-node matrix-check`: sign in (or reuse the device), sync for a while and print what would go to the deck,
/// which also waits in the outbox for the node to hand over. Stop the node first: two readers mustn't share a store.
pub async fn check(node_name: &str, cfg: &MatrixConfig, seconds: u64) -> anyhow::Result<()> {
    cfg.check().map_err(anyhow::Error::msg)?;
    let dir = cfg.store_dir();
    private_dir(&dir)?;
    let outbox = Arc::new(Outbox::load(dir.join("outbox.json")));
    let before = outbox.len().await;
    let r = tokio::time::timeout(Duration::from_secs(seconds), read(node_name, cfg, &dir, outbox.clone())).await;
    match r {
        Err(_) => {}
        Ok(Ok(())) => anyhow::bail!("the sync stopped early"),
        Ok(Err(Restart::Fresh(why))) => anyhow::bail!("the device is no longer valid: {why}"),
        Ok(Err(Restart::Later(why))) => anyhow::bail!("{why}"),
    }
    let q = outbox.queue.lock().await;
    println!("Synced for {seconds}s as {}. {} new message(s) wait for the deck:", cfg.user, q.len() - before);
    for m in &q[before..] {
        let line = m.body.lines().next().unwrap_or_default();
        println!("  {} {} in {}: {}", m.sent_at, m.sender, m.channel_name, line.chars().take(100).collect::<String>());
    }
    Ok(())
}

enum Restart {
    /// Start over with a new device.
    Fresh(String),
    /// Try again with the same device.
    Later(String),
}

fn later(e: impl std::fmt::Display) -> Restart {
    Restart::Later(e.to_string())
}

fn invalid_token(e: &matrix_sdk::Error) -> bool {
    matches!(e.client_api_error_kind(), Some(ErrorKind::UnknownToken(_)))
}

async fn read(node_name: &str, cfg: &MatrixConfig, dir: &Path, outbox: Arc<Outbox>) -> Result<(), Restart> {
    let client = Client::builder()
        .homeserver_url(&cfg.homeserver)
        .sqlite_store(dir.join("sdk"), None)
        .build()
        .await
        .map_err(later)?;
    let session_path = dir.join("session.json");
    let saved: Option<MatrixSession> = std::fs::read(&session_path).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let fresh = saved.is_none();
    match saved {
        Some(session) => client.restore_session(session).await.map_err(later)?,
        None => {
            let password = read_secret_file(&expand(&cfg.password_file)).map_err(later)?;
            let name = cfg.device_name.clone().unwrap_or_else(|| format!("Quietdeck ({node_name})"));
            client
                .matrix_auth()
                .login_username(&cfg.user, &password)
                .initial_device_display_name(&name)
                .send()
                .await
                .map_err(later)?;
            let session = client.matrix_auth().session().ok_or_else(|| later("signed in, but there's no session"))?;
            write_private(&session_path, &serde_json::to_vec(&session).expect("sessions serialise")).map_err(later)?;
            tracing::info!(user = %cfg.user, device = %session.meta.device_id, "signed in to Matrix with a new device");
        }
    }

    let settings = SyncSettings::default().timeout(SYNC_TIMEOUT);
    if fresh {
        // Start from now: what the rooms held before the first sign-in isn't handed over.
        if let Err(e) = client.sync_once(settings.clone()).await {
            return Err(if invalid_token(&e) { Restart::Fresh(e.to_string()) } else { later(e) });
        }
    }

    let rooms: Vec<OwnedRoomId> = cfg.rooms.iter().filter_map(|r| OwnedRoomId::try_from(r.as_str()).ok()).collect();
    for r in &rooms {
        if client.get_room(r).is_none() {
            tracing::warn!(room = %r, "the Matrix user isn't in this room yet: its messages will come once it joins");
        }
    }
    let rooms = Arc::new(rooms);
    {
        let rooms = rooms.clone();
        let outbox = outbox.clone();
        let me = client.user_id().map(|u| u.to_owned());
        client.add_event_handler(move |ev: OriginalSyncRoomMessageEvent, room: Room| {
            let rooms = rooms.clone();
            let outbox = outbox.clone();
            let mine = me.as_deref() == Some(&*ev.sender);
            async move {
                if mine || !rooms.iter().any(|r| r == room.room_id()) {
                    return;
                }
                outbox.push(incoming(&room, &ev)).await;
            }
        });
    }
    {
        let rooms = rooms.clone();
        client.add_event_handler(move |ev: OriginalSyncRoomEncryptedEvent, room: Room| {
            let rooms = rooms.clone();
            async move {
                if rooms.iter().any(|r| r == room.room_id()) {
                    // The sender didn't share the key with this device (yet). The message isn't handed over.
                    tracing::warn!(room = %room.room_id(), event = %ev.event_id, sender = %ev.sender,
                        "a Matrix message this device can't decrypt");
                }
            }
        });
    }
    tracing::info!(rooms = rooms.len(), "reading Matrix");
    match client.sync(settings).await {
        Ok(()) => Ok(()),
        Err(e) if invalid_token(&e) => Err(Restart::Fresh(e.to_string())),
        Err(e) => Err(later(e)),
    }
}

fn room_name(room: &Room) -> String {
    room.cached_display_name()
        .map(|n| n.to_string())
        .or_else(|| room.name())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| room.room_id().to_string())
}

fn incoming(room: &Room, ev: &OriginalSyncRoomMessageEvent) -> IncomingMessage {
    let ms = i64::try_from(u64::from(ev.origin_server_ts.0)).unwrap_or(0);
    IncomingMessage {
        source: "matrix".into(),
        channel: room.room_id().to_string(),
        channel_name: room_name(room),
        external_id: ev.event_id.to_string(),
        sender: ev.sender.to_string(),
        body: ev.content.body().to_string(),
        sent_at: jiff::Timestamp::from_millisecond(ms).unwrap_or_else(|_| jiff::Timestamp::now()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str) -> IncomingMessage {
        IncomingMessage {
            source: "matrix".into(),
            channel: "!r:example.com".into(),
            channel_name: "Alerts".into(),
            external_id: id.into(),
            sender: "@a:example.com".into(),
            body: "disk full".into(),
            sent_at: jiff::Timestamp::now(),
        }
    }

    #[tokio::test]
    async fn the_outbox_keeps_what_the_deck_didnt_take_across_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("outbox.json");
        let outbox = Outbox::load(path.clone());
        outbox.push(msg("$1")).await;
        outbox.push(msg("$1")).await;
        outbox.push(msg("$2")).await;
        assert_eq!(outbox.len().await, 2);

        let down = outbox.flush(|_| async { Err::<MessagesStored, _>("not connected".to_string()) }).await;
        assert!(down.is_err());
        let outbox = Outbox::load(path.clone());
        assert_eq!(outbox.len().await, 2, "kept on disk");

        let sent = Arc::new(std::sync::Mutex::new(vec![]));
        let new = outbox
            .flush(|batch| {
                let sent = sent.clone();
                async move {
                    let n = batch.len() as u32;
                    sent.lock().unwrap().extend(batch.into_iter().map(|m| m.external_id));
                    Ok(MessagesStored { new: n })
                }
            })
            .await
            .unwrap();
        assert_eq!(new, 2);
        assert_eq!(*sent.lock().unwrap(), ["$1", "$2"]);
        assert_eq!(Outbox::load(path).len().await, 0);
    }

    #[test]
    fn the_config_is_checked() {
        let cfg: MatrixConfig = yaml_serde::from_str(
            "homeserver: https://matrix.example.com\nuser: '@quietdeck:example.com'\n\
             password_file: ~/.config/quietdeck/matrix-password\nrooms: ['!abc:example.com']\n",
        )
        .unwrap();
        assert_eq!(cfg.check(), Ok(()));
        let mut bad = cfg.clone();
        bad.rooms = vec!["#alias:example.com".into()];
        assert!(bad.check().unwrap_err().contains("isn't a room ID"));
        let mut bad = cfg;
        bad.user = "quietdeck".into();
        assert!(bad.check().is_err());
    }
}
