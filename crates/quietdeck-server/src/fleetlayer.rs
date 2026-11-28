//! Session summaries from Fleetlayer's control plane, on machines where its summaries plugin writes them instead of
//! Quietdeck's node (the move agreed with the fleetlayer tab, 26 Nov 2026).
//!
//! The deck pulls every new record into its own store (`GET /v1/summaries?since=`, every minute), so the Sessions
//! screen, search, who-has, timelines and the digest work the same whichever program wrote a summary. A machine that
//! still runs Quietdeck's summaries module keeps pushing its own; a record for the same session and period replaces
//! the older one. Catch-ups go to the session's node first and to Fleetlayer when the node can't answer.

use std::time::Duration;

use jiff::Timestamp;
use quietdeck_types::AgentKind;
use quietdeck_types::summary::{Catchup, RecordOrigin, SummaryEvent, SummaryPeriod, SummaryRecord};
use serde::Deserialize;

use crate::state::AppState;

/// How often the deck asks for new records.
const EVERY: Duration = Duration::from_secs(60);
/// Records per request; a full page is followed straight away by the next.
const PAGE: usize = 500;
/// The `settings` row that holds the feed's position.
const CURSOR: &str = "fleetlayer.summaries_cursor";

/// The control plane's address and a read-scoped token (`fleetlayer-control token add … --scope read`).
#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl Client {
    pub fn new(base: &str, token: &str) -> anyhow::Result<Client> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Client { http, base: base.trim_end_matches('/').to_string(), token: token.to_string() })
    }

    /// Records written after `cursor`, oldest first. The cursor is the feed's own `<writtenAt>:<rowid>` (exact, from
    /// `nextCursor`), or `0` to start.
    async fn since(&self, cursor: &str) -> anyhow::Result<Feed> {
        let res = self
            .http
            .get(format!("{}/v1/summaries", self.base))
            .query(&[("since", cursor.to_string()), ("limit", PAGE.to_string())])
            .bearer_auth(&self.token)
            .send()
            .await?;
        let status = res.status();
        if !status.is_success() {
            anyhow::bail!("GET /v1/summaries?since= answered {status}: {}", res.text().await.unwrap_or_default());
        }
        Ok(res.json().await?)
    }

    /// Handoffs made after `cursor`, oldest first (`GET /v1/handoffs?since=`; empty to start). `None` when the control
    /// plane has no such feed (older than Fleetlayer v0.4.0).
    pub async fn handoffs_since(
        &self,
        cursor: &str,
        limit: usize,
    ) -> anyhow::Result<Option<crate::fleetlayer_handoffs::Feed>> {
        let res = self
            .http
            .get(format!("{}/v1/handoffs", self.base))
            .query(&[("since", cursor.to_string()), ("limit", limit.to_string())])
            .bearer_auth(&self.token)
            .send()
            .await?;
        let status = res.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            anyhow::bail!("GET /v1/handoffs?since= answered {status}: {}", res.text().await.unwrap_or_default());
        }
        Ok(Some(res.json().await?))
    }

    /// Events after `after` (an event ID, or `latest` to start from now) on `channels` (comma-separated), oldest
    /// first, waiting up to `wait` seconds for one (`GET /v1/events`). `None` when the control plane answers 404.
    pub async fn events_after(
        &self,
        after: &str,
        channels: &str,
        wait: u64,
    ) -> anyhow::Result<Option<crate::fleetlayer_events::Feed>> {
        let res = self
            .http
            .get(format!("{}/v1/events", self.base))
            .query(&[("after", after), ("channel", channels), ("wait", &wait.to_string())])
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(wait + 15))
            .send()
            .await?;
        let status = res.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            anyhow::bail!("GET /v1/events answered {status}: {}", res.text().await.unwrap_or_default());
        }
        Ok(Some(res.json().await?))
    }

    /// Every reading's newest value of one kind (`GET /v1/readings?kind=`), as `{readings: [{kind, key, node, at,
    /// data}]}`.
    pub async fn readings(&self, kind: &str) -> anyhow::Result<serde_json::Value> {
        let res = self
            .http
            .get(format!("{}/v1/readings", self.base))
            .query(&[("kind", kind)])
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(10))
            .send()
            .await?;
        let status = res.status();
        if !status.is_success() {
            anyhow::bail!("GET /v1/readings answered {status}: {}", res.text().await.unwrap_or_default());
        }
        Ok(res.json().await?)
    }

    /// Every node and session in the fleet (`GET /v1/fleet`), for the Sessions screen.
    pub async fn fleet(&self) -> anyhow::Result<crate::fleet_sessions::Fleet> {
        let res = self
            .http
            .get(format!("{}/v1/fleet", self.base))
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(3))
            .send()
            .await?;
        let status = res.status();
        if !status.is_success() {
            anyhow::bail!("GET /v1/fleet answered {status}: {}", res.text().await.unwrap_or_default());
        }
        Ok(res.json().await?)
    }

    /// "Since your last message", written by the session's node through Fleetlayer. Waits while one is written.
    pub async fn catchup(&self, session_id: &str) -> anyhow::Result<Catchup> {
        let res = self
            .http
            .post(format!("{}/v1/sessions/{session_id}/catchup", self.base))
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(9 * 60))
            .send()
            .await?;
        let status = res.status();
        if !status.is_success() {
            anyhow::bail!("Fleetlayer's catch-up answered {status}: {}", res.text().await.unwrap_or_default());
        }
        Ok(res.json().await?)
    }
}

/// The deck sending to sessions through the control plane (`POST /v1/messages`), as `service:<token name>` with a
/// send-scoped token: decided by Fleetlayer's access rules, delivered from "Quietdeck" and not marked as outside data.
#[derive(Debug, Clone)]
pub struct Sender {
    http: reqwest::Client,
    base: String,
    token: String,
}

/// How a send through the control plane ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sent {
    Delivered(quietdeck_types::node::Delivered),
    /// Fleetlayer doesn't know the session (a machine without Fleetlayer): the caller tries the node.
    Unknown,
    /// Refused (403), offline or busy (503), or anything else: Fleetlayer's own words, with the HTTP status.
    Failed {
        status: u16,
        message: String,
    },
}

impl Sender {
    pub fn new(base: &str, token: &str) -> anyhow::Result<Sender> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(SEND_WITHIN_SECS))
            .build()?;
        Ok(Sender { http, base: base.trim_end_matches('/').to_string(), token: token.to_string() })
    }

    /// Send to `<tab>@<node>`. A connection that fails before the request is sent counts as Unknown (the node path
    /// is safe then); a timeout after it's sent is Failed, since Fleetlayer may still have delivered it.
    pub async fn send(&self, to: &str, text: &str, subkind: &str) -> Sent {
        let res = self
            .http
            .post(format!("{}/v1/messages", self.base))
            .bearer_auth(&self.token)
            .json(&serde_json::json!({"to": to, "text": text, "label": "Quietdeck", "subkind": subkind}))
            .send()
            .await;
        let res = match res {
            Ok(r) => r,
            Err(e) if e.is_connect() => {
                tracing::warn!(error = %e, "Fleetlayer's control plane is unreachable: delivering through the node");
                return Sent::Unknown;
            }
            Err(e) => return Sent::Failed { status: 0, message: format!("Fleetlayer didn't answer: {e}") },
        };
        let status = res.status().as_u16();
        let body: serde_json::Value = res.json().await.unwrap_or_default();
        match status {
            200 => Sent::Delivered(match body["via"].as_str() {
                Some("harness_channel") => quietdeck_types::node::Delivered::Socket,
                _ => quietdeck_types::node::Delivered::Orchestrator,
            }),
            404 => Sent::Unknown,
            _ => Sent::Failed {
                status,
                message: body["message"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("Fleetlayer answered {status}")),
            },
        }
    }
}

impl Sender {
    /// Type an allowed slash command into an idle session (`POST /v1/sessions/act`, Fleetlayer v0.4.1), as
    /// service:quietdeck. Busy (working, blocked, someone typing) is a conflict to try again later.
    pub async fn act(&self, to: &str, command: &str, why: &str) -> crate::error::AppResult<()> {
        use crate::error::AppError;
        let res = self
            .http
            .post(format!("{}/v1/sessions/act", self.base))
            .bearer_auth(&self.token)
            .json(&serde_json::json!({"to": to, "action": "command", "command": command, "why": why}))
            .send()
            .await
            .map_err(|e| AppError::Unavailable(format!("Fleetlayer didn't answer: {e}")))?;
        let status = res.status().as_u16();
        if status == 200 {
            return Ok(());
        }
        let body: serde_json::Value = res.json().await.unwrap_or_default();
        let message =
            body["message"].as_str().map(str::to_string).unwrap_or_else(|| format!("Fleetlayer answered {status}"));
        Err(match status {
            409 => AppError::Conflict(format!(
                "{to} is busy (working, waiting on a prompt, or someone's typing): {message}"
            )),
            403 => AppError::Forbidden(message),
            404 => {
                AppError::Conflict(format!("this Fleetlayer can't type commands into sessions yet (v0.4.1): {message}"))
            }
            503 => AppError::Offline(message),
            _ => AppError::Unavailable(message),
        })
    }
}

/// The deck's delegate client: the signed-in person's own words into a session (`POST /v1/sessions/say`).
pub static SAYER: std::sync::OnceLock<Sayer> = std::sync::OnceLock::new();

pub struct Sayer {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl Sayer {
    pub fn new(base: &str, token: &str) -> anyhow::Result<Sayer> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(SEND_WITHIN_SECS))
            .build()?;
        Ok(Sayer { http, base: base.trim_end_matches('/').to_string(), token: token.to_string() })
    }

    /// Type one line of the person's into an idle session with an empty input line, as them; it arrives as "From the
    /// Work tab: <text>". Fleetlayer's message on any refusal (busy, not allowed, offline, too old a Fleetlayer).
    pub async fn say(&self, person: &str, to: &str, text: &str) -> Result<(), String> {
        let res = self
            .http
            .post(format!("{}/v1/sessions/say", self.base))
            .bearer_auth(&self.token)
            .header("Fleetlayer-Person", person)
            .json(&serde_json::json!({"to": to, "text": text, "from": "the Work tab"}))
            .send()
            .await
            .map_err(|e| format!("Fleetlayer didn't answer: {e}"))?;
        let status = res.status().as_u16();
        if status == 200 {
            return Ok(());
        }
        let body: serde_json::Value = res.json().await.unwrap_or_default();
        Err(format!("{status}: {}", body["message"].as_str().unwrap_or("refused")))
    }
}

/// One line the person typed or picked, as `say` takes it: on one line, and never a slash command or shell escape.
pub fn say_line(value: &str) -> Option<String> {
    let line = value.split_whitespace().collect::<Vec<_>>().join(" ");
    (!line.is_empty() && !line.starts_with('/') && !line.starts_with('!')).then_some(line)
}

/// The relay waits 4 seconds for a node; Fleetlayer's own delivery is cut off at 5, and a cross-node one at 12.
const SEND_WITHIN_SECS: u64 = 13;

/// A page of `/v1/summaries?since=`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Feed {
    /// Records and tombstones, in cursor order.
    summaries: Vec<serde_json::Value>,
    /// Pass back as `since` for the records strictly after this page.
    next_cursor: Option<String>,
}

/// One record as `/v1/summaries` serves it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Wire {
    node: String,
    session_id: String,
    period: WirePeriod,
    agent: String,
    activity_from: i64,
    activity_to: i64,
    written_at: i64,
    sentence: String,
    paragraph: String,
    page: String,
    tab: Option<String>,
    dir: Option<String>,
    origin: String,
}

#[derive(Debug, Deserialize)]
struct WirePeriod {
    kind: String,
    key: String,
}

impl WirePeriod {
    fn period(self) -> Option<SummaryPeriod> {
        match self.kind.as_str() {
            "hour" => Some(SummaryPeriod::Hour(self.key)),
            "day" => Some(SummaryPeriod::Day(self.key)),
            "week" => Some(SummaryPeriod::Week(self.key)),
            _ => None,
        }
    }
}

/// Fleetlayer deleted summaries: one period's record, or (`period: null`) every record of the session on that node.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Tombstone {
    node: String,
    session_id: String,
    period: Option<WirePeriod>,
}

impl Wire {
    /// Quietdeck's record, or None for one it can't place (an agent or period kind it doesn't know).
    fn record(self) -> Option<(String, SummaryRecord)> {
        let period = self.period.period()?;
        let agent = match self.agent.as_str() {
            "claude" => AgentKind::Claude,
            "codex" => AgentKind::Codex,
            _ => return None,
        };
        let origin = match self.origin.as_str() {
            "claude_summarizer" => RecordOrigin::ClaudeSummarizer,
            _ => RecordOrigin::Node,
        };
        let ms = |v: i64| Timestamp::from_millisecond(v).ok();
        let record = SummaryRecord {
            session_id: self.session_id,
            agent,
            period,
            activity_from: ms(self.activity_from)?,
            activity_to: ms(self.activity_to)?,
            written_at: ms(self.written_at)?,
            sentence: self.sentence,
            paragraph: self.paragraph,
            page: self.page,
            tab: self.tab,
            dir: self.dir,
            origin,
        };
        Some((self.node, record))
    }
}

async fn cursor(state: &AppState) -> anyhow::Result<String> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM settings WHERE name = ?").bind(CURSOR).fetch_optional(&state.db).await?;
    Ok(row.map(|r| r.0).unwrap_or_else(|| "0".into()))
}

async fn set_cursor(state: &AppState, value: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO settings (name, value) VALUES (?, ?) \
         ON CONFLICT (owner_type, owner_id, name) DO UPDATE SET value = excluded.value",
    )
    .bind(CURSOR)
    .bind(value)
    .execute(&state.db)
    .await?;
    Ok(())
}

/// Stores one page of records and applies its tombstones, in order. Returns how many items there were.
async fn store(state: &AppState, page: Vec<serde_json::Value>) -> anyhow::Result<usize> {
    let n = page.len();
    for item in page {
        if item.get("deleted").and_then(serde_json::Value::as_bool) == Some(true) {
            match serde_json::from_value::<Tombstone>(item) {
                Ok(t) => {
                    let period = match t.period {
                        Some(p) => match p.period() {
                            Some(p) => Some(p),
                            None => continue,
                        },
                        None => None,
                    };
                    crate::store::sessions::forget_summaries(&state.db, &t.node, &t.session_id, period.as_ref())
                        .await?;
                }
                Err(e) => tracing::warn!(error = %e, "a Fleetlayer tombstone the deck can't read"),
            }
            continue;
        }
        match serde_json::from_value::<Wire>(item) {
            Ok(wire) => {
                if let Some((node, record)) = wire.record() {
                    crate::store::sessions::apply_summary(&state.db, &node, &SummaryEvent::Summary(record)).await?;
                }
            }
            Err(e) => tracing::warn!(error = %e, "a Fleetlayer summary the deck can't read"),
        }
    }
    Ok(n)
}

/// Pulls everything new once, page after page. Returns how many records it stored.
pub async fn pull(state: &AppState, client: &Client) -> anyhow::Result<usize> {
    let mut total = 0;
    loop {
        let from = cursor(state).await?;
        let feed = client.since(&from).await?;
        let n = store(state, feed.summaries).await?;
        total += n;
        let moved = feed.next_cursor.filter(|c| *c != from);
        if let Some(next) = &moved {
            set_cursor(state, next).await?;
        }
        // A short page is the end; so is a full one that didn't move the cursor, or the same page would come back
        // forever.
        if n < PAGE || moved.is_none() {
            break;
        }
    }
    if total > 0 {
        state.changed(crate::events::Changed::Sessions);
    }
    Ok(total)
}

/// Runs for as long as the deck does, when it has a Fleetlayer address and token.
pub async fn ticker(state: AppState) {
    let Some(client) = state.fleetlayer.clone() else { return };
    let mut failing = false;
    loop {
        match pull(&state, &client).await {
            Ok(n) => {
                if failing || n > 0 {
                    tracing::info!(records = n, "summaries from Fleetlayer");
                }
                failing = false;
            }
            Err(e) if !failing => {
                tracing::warn!(error = %e, "can't read summaries from Fleetlayer; trying again every minute");
                failing = true;
            }
            Err(_) => {}
        }
        tokio::select! {
            _ = tokio::time::sleep(EVERY) => {}
            _ = crate::state::until_stopping(state.stopping.clone()) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_person_says_is_one_plain_line() {
        assert_eq!(say_line("Publish it").as_deref(), Some("Publish it"));
        assert_eq!(say_line("  Use FCM\nbut not\tyet ").as_deref(), Some("Use FCM but not yet"));
        assert_eq!(say_line("/clear"), None);
        assert_eq!(say_line("!rm -rf"), None);
        assert_eq!(say_line("   "), None);
    }

    /// A fake control plane: answers POST /v1/messages by the `to` it gets, and keeps what it got.
    async fn control_plane() -> (String, std::sync::Arc<tokio::sync::Mutex<Vec<serde_json::Value>>>) {
        use axum::http::StatusCode;
        let seen: std::sync::Arc<tokio::sync::Mutex<Vec<serde_json::Value>>> = Default::default();
        let kept = seen.clone();
        let app = axum::Router::new().route(
            "/v1/messages",
            axum::routing::post(move |headers: axum::http::HeaderMap, axum::Json(m): axum::Json<serde_json::Value>| {
                let kept = kept.clone();
                async move {
                    assert_eq!(headers["authorization"], "Bearer send-token");
                    kept.lock().await.push(m.clone());
                    let (status, body) = match m["to"].as_str().unwrap() {
                        "email triage@agents-triage" => (StatusCode::OK, serde_json::json!({"via": "harness_channel"})),
                        "laptop tab@laptop" => (StatusCode::NOT_FOUND, serde_json::json!({"code": -32014, "message": "no such node"})),
                        "ci@agents" => (StatusCode::SERVICE_UNAVAILABLE, serde_json::json!({"code": -32010, "message": "ci on agents isn't running"})),
                        _ => (StatusCode::FORBIDDEN, serde_json::json!({"code": -32011, "message": "service:quietdeck may not message that session"})),
                    };
                    (status, axum::Json(body))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, seen)
    }

    #[tokio::test]
    async fn the_deck_sends_as_quietdeck_and_an_unknown_session_falls_back_to_the_node() {
        let (url, seen) = control_plane().await;
        let fl = Sender::new(&url, "send-token").unwrap();
        assert_eq!(
            fl.send("email triage@agents-triage", "Nathan wrote: hi", "deck").await,
            Sent::Delivered(quietdeck_types::node::Delivered::Socket)
        );
        let got = seen.lock().await[0].clone();
        assert_eq!(got["label"], "Quietdeck");
        assert_eq!(got["subkind"], "deck");
        assert_eq!(got["text"], "Nathan wrote: hi");
        assert_eq!(fl.send("laptop tab@laptop", "x", "deck").await, Sent::Unknown);
        assert_eq!(
            fl.send("ci@agents", "x", "deck").await,
            Sent::Failed { status: 503, message: "ci on agents isn't running".into() }
        );
        assert_eq!(
            fl.send("api@agents", "x", "deck").await,
            Sent::Failed { status: 403, message: "service:quietdeck may not message that session".into() }
        );
    }

    #[tokio::test]
    async fn an_unreachable_control_plane_means_the_node_delivers() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let fl = Sender::new(&url, "send-token").unwrap();
        assert_eq!(fl.send("email triage@agents-triage", "x", "deck").await, Sent::Unknown);
    }

    #[test]
    fn only_the_decks_own_words_go_as_the_deck() {
        use quietdeck_types::relay::DeliveryKind;
        assert!(DeliveryKind::Deck.is_deck_words());
        assert!(DeliveryKind::Handoff.is_deck_words());
        let forward =
            DeliveryKind::Forward { account: "a".into(), refs: vec![], rule: None, sent_mail: false, note: None };
        assert!(!forward.is_deck_words());
        let from = quietdeck_types::action::SessionRef { node: "agents".into(), tab: "ci".into(), session_id: None };
        assert!(!DeliveryKind::Message { from }.is_deck_words());
    }

    fn wire(session: &str, kind: &str, key: &str, to: i64, written: i64, sentence: &str) -> serde_json::Value {
        serde_json::json!({
            "node": "agents", "sessionId": session, "period": {"kind": kind, "key": key}, "agent": "claude",
            "activityFrom": to - 60_000, "activityTo": to, "writtenAt": written, "sentence": sentence,
            "paragraph": "p", "page": "pg", "tab": "ci", "dir": "/home/nathan/code/app", "origin": "node"
        })
    }

    #[tokio::test]
    async fn records_land_in_the_deck_store_and_newer_ones_replace_older() {
        let state = AppState::for_tests().await;
        let t = 1_795_600_000_000;
        let n = store(
            &state,
            vec![
                wire("s1", "hour", "2026-11-26T01", t, t + 1, "first"),
                wire("s1", "hour", "2026-11-26T02", t + 3_600_000, t + 3_600_001, "second"),
                wire("s2", "fortnight", "x", t, t + 2, "unknown period"),
            ],
        )
        .await
        .unwrap();
        assert_eq!(n, 3);
        let timeline = crate::store::sessions::timeline(&state.db, "agents", "s1").await.unwrap();
        assert_eq!(timeline.iter().map(|r| r.sentence.as_str()).collect::<Vec<_>>(), ["first", "second"]);
        assert!(crate::store::sessions::timeline(&state.db, "agents", "s2").await.unwrap().is_empty());
        let row = crate::store::sessions::get(&state.db, &state.hub, "agents", "s1").await.unwrap().unwrap();
        assert_eq!(row.sentence.as_deref(), Some("second"));
        // The same period again, rewritten: it replaces the stored one.
        store(&state, vec![wire("s1", "hour", "2026-11-26T02", t + 3_600_000, t + 3_700_000, "second, again")])
            .await
            .unwrap();
        let timeline = crate::store::sessions::timeline(&state.db, "agents", "s1").await.unwrap();
        assert_eq!(timeline.last().map(|r| r.sentence.as_str()), Some("second, again"));
        assert_eq!(timeline.len(), 2);
    }

    #[test]
    fn a_feed_page_parses_with_its_cursors() {
        let feed: Feed = serde_json::from_value(serde_json::json!({"summaries": [{
            "node": "agents", "sessionId": "s1", "period": {"kind": "day", "key": "2026-11-26"}, "agent": "codex",
            "activityFrom": 1, "activityTo": 2, "writtenAt": 3, "sentence": "s", "paragraph": "p", "page": "pg",
            "tab": null, "dir": null, "origin": "claude_summarizer", "cursor": "3:17"
        }], "nextCursor": "3:17"}))
        .unwrap();
        assert_eq!(feed.next_cursor.as_deref(), Some("3:17"));
        let wire: Wire = serde_json::from_value(feed.summaries.into_iter().next().unwrap()).unwrap();
        let (node, r) = wire.record().unwrap();
        assert_eq!((node.as_str(), r.agent, r.origin), ("agents", AgentKind::Codex, RecordOrigin::ClaudeSummarizer));
        assert_eq!(r.period, SummaryPeriod::Day("2026-11-26".into()));
    }

    #[tokio::test]
    async fn tombstones_drop_a_period_or_the_whole_session() {
        let state = AppState::for_tests().await;
        let t = 1_795_600_000_000;
        store(
            &state,
            vec![
                wire("s1", "hour", "2026-11-26T01", t, t + 1, "first"),
                wire("s1", "hour", "2026-11-26T02", t + 3_600_000, t + 3_600_001, "second"),
            ],
        )
        .await
        .unwrap();
        let gone = |kind: Option<&str>| {
            let period = kind.map(|k| serde_json::json!({"kind": k, "key": "2026-11-26T02"}));
            serde_json::json!({"deleted": true, "node": "agents", "sessionId": "s1", "period": period, "cursor": "x"})
        };
        store(&state, vec![gone(Some("hour"))]).await.unwrap();
        let row = crate::store::sessions::get(&state.db, &state.hub, "agents", "s1").await.unwrap().unwrap();
        assert_eq!(row.sentence.as_deref(), Some("first"), "the newest record left is the latest again");
        store(&state, vec![gone(None)]).await.unwrap();
        assert!(crate::store::sessions::timeline(&state.db, "agents", "s1").await.unwrap().is_empty());
        assert!(
            crate::store::sessions::get(&state.db, &state.hub, "agents", "s1").await.unwrap().is_none(),
            "a session that only existed for its summaries goes"
        );
    }

    #[tokio::test]
    async fn the_cursor_is_kept_in_settings() {
        let state = AppState::for_tests().await;
        assert_eq!(cursor(&state).await.unwrap(), "0");
        set_cursor(&state, "1795600000000:41").await.unwrap();
        set_cursor(&state, "1795600000000:42").await.unwrap();
        assert_eq!(cursor(&state).await.unwrap(), "1795600000000:42");
        assert_eq!(crate::api::load_settings(&state.db).await.unwrap(), Default::default());
    }
}
