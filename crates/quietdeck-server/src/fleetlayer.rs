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

    /// Records written after `cursor` (a `writtenAt` in ms), oldest first.
    async fn since(&self, cursor: i64) -> anyhow::Result<Vec<Wire>> {
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

impl Wire {
    /// Quietdeck's record, or None for one it can't place (an agent or period kind it doesn't know).
    fn record(self) -> Option<(String, SummaryRecord)> {
        let period = match self.period.kind.as_str() {
            "hour" => SummaryPeriod::Hour(self.period.key),
            "day" => SummaryPeriod::Day(self.period.key),
            "week" => SummaryPeriod::Week(self.period.key),
            _ => return None,
        };
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

async fn cursor(state: &AppState) -> anyhow::Result<i64> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM settings WHERE name = ?").bind(CURSOR).fetch_optional(&state.db).await?;
    Ok(row.and_then(|r| r.0.parse().ok()).unwrap_or(0))
}

async fn set_cursor(state: &AppState, value: i64) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO settings (name, value) VALUES (?, ?) \
         ON CONFLICT (owner_type, owner_id, name) DO UPDATE SET value = excluded.value",
    )
    .bind(CURSOR)
    .bind(value.to_string())
    .execute(&state.db)
    .await?;
    Ok(())
}

/// Stores one page of records. Returns how many there were and the newest `writtenAt`.
async fn store(state: &AppState, page: Vec<Wire>) -> anyhow::Result<(usize, Option<i64>)> {
    let n = page.len();
    let newest = page.iter().map(|w| w.written_at).max();
    for wire in page {
        if let Some((node, record)) = wire.record() {
            crate::store::sessions::apply_summary(&state.db, &node, &SummaryEvent::Summary(record)).await?;
        }
    }
    Ok((n, newest))
}

/// Pulls everything new once, page after page. Returns how many records it stored.
pub async fn pull(state: &AppState, client: &Client) -> anyhow::Result<usize> {
    let mut total = 0;
    loop {
        let from = cursor(state).await?;
        let page = client.since(from).await?;
        let (n, newest) = store(state, page).await?;
        total += n;
        let moved = newest.filter(|t| *t > from);
        if let Some(newest) = moved {
            set_cursor(state, newest).await?;
        }
        // A short page is the end; so is a full one that didn't move the cursor (every record shared one writtenAt,
        // with a feed that answers >=), or the same page would come back forever.
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

    fn wire(session: &str, kind: &str, key: &str, to: i64, written: i64, sentence: &str) -> Wire {
        serde_json::from_value(serde_json::json!({
            "node": "agents", "sessionId": session, "period": {"kind": kind, "key": key}, "agent": "claude",
            "activityFrom": to - 60_000, "activityTo": to, "writtenAt": written, "sentence": sentence,
            "paragraph": "p", "page": "pg", "tab": "ci", "dir": "/home/nathan/code/app", "origin": "node"
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn records_land_in_the_deck_store_and_newer_ones_replace_older() {
        let state = AppState::for_tests().await;
        let t = 1_795_600_000_000;
        let (n, newest) = store(
            &state,
            vec![
                wire("s1", "hour", "2026-11-26T01", t, t + 1, "first"),
                wire("s1", "hour", "2026-11-26T02", t + 3_600_000, t + 3_600_001, "second"),
                wire("s2", "fortnight", "x", t, t + 2, "unknown period"),
            ],
        )
        .await
        .unwrap();
        assert_eq!((n, newest), (3, Some(t + 3_600_001)));
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

    #[tokio::test]
    async fn the_cursor_is_kept_in_settings() {
        let state = AppState::for_tests().await;
        assert_eq!(cursor(&state).await.unwrap(), 0);
        set_cursor(&state, 42).await.unwrap();
        set_cursor(&state, 43).await.unwrap();
        assert_eq!(cursor(&state).await.unwrap(), 43);
        assert_eq!(crate::api::load_settings(&state.db).await.unwrap(), Default::default());
    }
}
