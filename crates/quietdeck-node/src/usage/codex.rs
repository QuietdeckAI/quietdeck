//! Codex plan usage: the ChatGPT subscription Codex is signed in to (`~/.codex/auth.json`), read the way Codex reads
//! it. The node polls `https://chatgpt.com/backend-api/wham/usage` at most every `poll_minutes`, and between polls
//! takes the `rate_limits` snapshot Codex writes into its session logs after each response
//! (`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`). The token never leaves this module.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

/// Epoch milliseconds.
pub type Ms = i64;

/// An ISO 8601 time as epoch ms; one with no offset is read as local time, as JavaScript does.
pub fn parse_iso(s: &str) -> Option<Ms> {
    if let Ok(ts) = s.parse::<jiff::Timestamp>() {
        return Some(ts.as_millisecond());
    }
    s.parse::<jiff::civil::DateTime>()
        .ok()
        .and_then(|dt| dt.to_zoned_datetime(jiff::tz::TimeZone::system()).ok())
        .map(|z| z.timestamp().as_millisecond())
}

/// How long a `Retry-After` header asks to wait: seconds, or an HTTP date.
pub fn retry_after(header: Option<&str>, now: Ms) -> Option<Ms> {
    let header = header?.trim();
    if let Ok(seconds) = header.parse::<f64>() {
        return Some((seconds * 1000.0).max(0.0) as Ms);
    }
    let at = jiff::fmt::rfc2822::parse(header).ok().map(|z| z.timestamp().as_millisecond())?;
    Some((at - now).max(0))
}

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
/// How often the session logs are looked at for a newer snapshot.
pub const SESSION_SCAN_MS: Ms = 60_000;
/// The longest a failed poll waits before the next.
const MAX_BACKOFF_MS: Ms = 15 * 60_000;

/// What `auth.json` holds that the poll needs.
#[derive(Clone, PartialEq)]
pub struct CodexAuth {
    pub access_token: String,
    pub account_id: String,
}

impl std::fmt::Debug for CodexAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexAuth").field("account_id", &self.account_id).finish_non_exhaustive()
    }
}

/// A ChatGPT sign-in in `auth.json`; None for an API key sign-in or a file that isn't one.
pub fn auth_from(text: &str) -> Option<CodexAuth> {
    let v: Value = serde_json::from_str(text).ok()?;
    let tokens = v.get("tokens")?;
    let access_token = tokens.get("access_token")?.as_str().filter(|t| !t.is_empty())?.to_string();
    let account_id = tokens.get("account_id")?.as_str().filter(|t| !t.is_empty())?.to_string();
    Some(CodexAuth { access_token, account_id })
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodexWindow {
    pub pct: f64,
    pub window_minutes: Option<u32>,
    pub resets_at: Option<Ms>,
}

/// One reading of the subscription's limits.
#[derive(Debug, Clone, PartialEq)]
pub struct CodexReading {
    pub primary: Option<CodexWindow>,
    pub secondary: Option<CodexWindow>,
    pub plan: Option<String>,
    pub email: Option<String>,
    pub at: Ms,
}

fn window(v: Option<&Value>, now: Ms) -> Option<CodexWindow> {
    let v = v.filter(|v| v.is_object())?;
    let pct = v.get("used_percent")?.as_f64()?;
    let window_minutes = v
        .get("window_minutes")
        .and_then(Value::as_u64)
        .or_else(|| v.get("limit_window_seconds").and_then(Value::as_u64).map(|s| s / 60))
        .and_then(|m| u32::try_from(m).ok());
    // Snapshots say `resets_at`, the usage endpoint `reset_at`, both in epoch seconds; the endpoint also has
    // `reset_after_seconds`.
    let resets_at = v
        .get("resets_at")
        .or_else(|| v.get("reset_at"))
        .and_then(Value::as_f64)
        .map(|s| (s * 1000.0) as Ms)
        .or_else(|| v.get("reset_after_seconds").and_then(Value::as_f64).map(|s| now + (s * 1000.0) as Ms));
    Some(CodexWindow { pct: pct.clamp(0.0, 100.0), window_minutes, resets_at })
}

/// The usage endpoint's answer.
pub fn reading_from_usage(body: &Value, now: Ms) -> Option<CodexReading> {
    let limits = body.get("rate_limit")?;
    Some(CodexReading {
        primary: window(limits.get("primary_window"), now),
        secondary: window(limits.get("secondary_window"), now),
        plan: body.get("plan_type").and_then(Value::as_str).map(str::to_string),
        email: body.get("email").and_then(Value::as_str).map(str::to_string),
        at: now,
    })
}

/// A session log line with a `rate_limits` snapshot, at the time Codex wrote it.
pub fn reading_from_log_line(line: &str) -> Option<CodexReading> {
    if !line.contains("\"rate_limits\"") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    let at = parse_iso(v.get("timestamp")?.as_str()?)?;
    let limits = v.get("payload")?.get("rate_limits").filter(|l| l.is_object())?;
    let primary = window(limits.get("primary"), at);
    let secondary = window(limits.get("secondary"), at);
    if primary.is_none() && secondary.is_none() {
        return None;
    }
    Some(CodexReading {
        primary,
        secondary,
        plan: limits.get("plan_type").and_then(Value::as_str).map(str::to_string),
        email: None,
        at,
    })
}

/// The newest snapshot in the session logs of the last two days, or None.
pub fn latest_session_reading(sessions: &Path, now: Ms) -> Option<CodexReading> {
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for days_ago in 0..2 {
        let day = jiff::Timestamp::from_millisecond(now - days_ago * 86_400_000)
            .ok()?
            .to_zoned_datetime(jiff::tz::TimeZone::system())
            .date();
        let dir = sessions
            .join(format!("{:04}", day.year()))
            .join(format!("{:02}", day.month()))
            .join(format!("{:02}", day.day()));
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"))
                && let Ok(modified) = e.metadata().and_then(|m| m.modified())
            {
                files.push((modified, path));
            }
        }
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    // The newest few files: a session that just answered wrote the newest snapshot.
    files.iter().take(5).filter_map(|(_, path)| last_reading_in(path)).max_by_key(|r| r.at)
}

/// The last snapshot in one log, read from its end (logs grow to many megabytes).
fn last_reading_in(path: &Path) -> Option<CodexReading> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut start = len;
    let mut tail: Vec<u8> = Vec::new();
    const CHUNK: u64 = 256 * 1024;
    // At most 4 MB back.
    while start > 0 && len - start < 16 * CHUNK {
        let from = start.saturating_sub(CHUNK);
        let mut chunk = vec![0; (start - from) as usize];
        f.seek(SeekFrom::Start(from)).ok()?;
        f.read_exact(&mut chunk).ok()?;
        chunk.extend_from_slice(&tail);
        tail = chunk;
        start = from;
        // The first line of a chunk that doesn't start the file may be cut: it doesn't parse, and is read whole
        // with the next chunk.
        let found = String::from_utf8_lossy(&tail).lines().rev().find_map(reading_from_log_line);
        if found.is_some() {
            return found;
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq)]
pub enum CodexHealth {
    Ok,
    SigninNeeded(String),
}

/// The Codex sign-in as last read.
#[derive(Debug, Clone, PartialEq)]
pub struct CodexState {
    pub account_id: String,
    pub health: CodexHealth,
    pub reading: Option<CodexReading>,
    /// Where `reading` came from: true for a session log, false for a poll.
    pub is_from_session: bool,
}

pub enum Answer {
    Ok(Value),
    Rejected,
    Failed(Option<Ms>),
}

async fn fetch(http: &reqwest::Client, auth: &CodexAuth, now: Ms) -> Answer {
    let res = http
        .get(USAGE_URL)
        .bearer_auth(&auth.access_token)
        .header("chatgpt-account-id", &auth.account_id)
        .header("User-Agent", "codex_cli_rs")
        .send()
        .await;
    match res {
        Ok(r) if r.status().as_u16() == 401 || r.status().as_u16() == 403 => Answer::Rejected,
        Ok(r) if r.status().is_success() => match r.json::<Value>().await {
            Ok(v) => Answer::Ok(v),
            Err(_) => Answer::Failed(None),
        },
        Ok(r) => Answer::Failed(retry_after(r.headers().get("retry-after").and_then(|v| v.to_str().ok()), now)),
        Err(_) => Answer::Failed(None),
    }
}

/// Watches the Codex sign-in until `stopped`, telling `changed` each time what it shows changes.
pub async fn watch(
    home: PathBuf,
    poll_ms: Ms,
    now: impl Fn() -> Ms + Send,
    log: impl Fn(&str) + Send,
    mut changed: impl FnMut(Option<CodexState>) + Send,
    asked: tokio::sync::watch::Receiver<u64>,
) {
    let http = reqwest::Client::builder().timeout(Duration::from_secs(20)).build().unwrap_or_default();
    let auth_file = home.join(".codex").join("auth.json");
    let sessions = home.join(".codex").join("sessions");
    let mut asked = asked;
    let mut state: Option<CodexState> = None;
    let mut polled_at: Ms = 0;
    let mut next_poll: Ms = 0;
    let mut backoff: Ms = 0;
    loop {
        let t = now();
        let auth = tokio::fs::read_to_string(&auth_file).await.ok().and_then(|t| auth_from(&t));
        let mut next = match &auth {
            None => None,
            Some(auth) => {
                let mut s = state.clone().filter(|s| s.account_id == auth.account_id).unwrap_or(CodexState {
                    account_id: auth.account_id.clone(),
                    health: CodexHealth::Ok,
                    reading: None,
                    is_from_session: false,
                });
                if t >= next_poll && t - polled_at >= 30_000 {
                    polled_at = t;
                    match fetch(&http, auth, t).await {
                        Answer::Ok(body) => {
                            backoff = 0;
                            next_poll = t + poll_ms;
                            s.health = CodexHealth::Ok;
                            if let Some(r) = reading_from_usage(&body, now()) {
                                let email =
                                    r.email.clone().or_else(|| s.reading.as_ref().and_then(|x| x.email.clone()));
                                s.reading = Some(CodexReading { email, ..r });
                                s.is_from_session = false;
                            }
                        }
                        Answer::Rejected => {
                            next_poll = t + poll_ms;
                            s.health = CodexHealth::SigninNeeded("sign-in rejected: run codex login".into());
                        }
                        Answer::Failed(retry) => {
                            backoff = (backoff * 2).clamp(2 * 60_000, MAX_BACKOFF_MS);
                            next_poll = t + retry.unwrap_or(0).max(backoff);
                            log(&format!("codex usage poll failed; next try in {} s", (next_poll - t) / 1000));
                        }
                    }
                }
                let sessions = sessions.clone();
                let newer =
                    tokio::task::spawn_blocking(move || latest_session_reading(&sessions, t)).await.ok().flatten();
                if let Some(r) = newer.filter(|r| s.reading.as_ref().is_none_or(|x| r.at > x.at)) {
                    let email = s.reading.as_ref().and_then(|x| x.email.clone());
                    let plan = r.plan.clone().or_else(|| s.reading.as_ref().and_then(|x| x.plan.clone()));
                    s.reading = Some(CodexReading { email, plan, ..r });
                    s.is_from_session = true;
                }
                Some(s)
            }
        };
        if next.is_none() && state.is_some() {
            log("codex is signed out");
        }
        if next != state {
            std::mem::swap(&mut state, &mut next);
            changed(state.clone());
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(SESSION_SCAN_MS as u64)) => {}
            r = asked.changed() => {
                if r.is_err() {
                    return;
                }
                next_poll = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_the_usage_endpoint() {
        let now = 1_795_000_000_000;
        let body = json!({
            "plan_type": "pro", "email": "a@example.com",
            "rate_limit": {
                "primary_window": { "used_percent": 12.5, "limit_window_seconds": 18000, "reset_after_seconds": 600, "reset_at": 1795000600 },
                "secondary_window": { "used_percent": 40, "limit_window_seconds": 604800, "reset_at": 1795400000 }
            }
        });
        let r = reading_from_usage(&body, now).expect("a reading");
        assert_eq!(r.plan.as_deref(), Some("pro"));
        assert_eq!(
            r.primary,
            Some(CodexWindow { pct: 12.5, window_minutes: Some(300), resets_at: Some(1_795_000_600_000) })
        );
        assert_eq!(r.secondary.as_ref().map(|w| w.window_minutes), Some(Some(10080)));
    }

    #[test]
    fn reads_a_session_log_snapshot_and_skips_other_lines() {
        let line = r#"{"timestamp":"2026-11-23T00:35:21.661Z","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"limit_id":"codex","primary":{"used_percent":12.0,"window_minutes":10080,"resets_at":1795761031},"secondary":null,"plan_type":"prolite"}}}"#;
        let r = reading_from_log_line(line).expect("a reading");
        assert_eq!(r.at, parse_iso("2026-11-23T00:35:21.661Z").unwrap_or_default());
        assert_eq!(
            r.primary,
            Some(CodexWindow { pct: 12.0, window_minutes: Some(10080), resets_at: Some(1_795_761_031_000) })
        );
        assert_eq!(r.secondary, None);
        assert_eq!(r.plan.as_deref(), Some("prolite"));
        assert_eq!(reading_from_log_line(r#"{"timestamp":"2026-11-23T00:35:21Z","payload":{"type":"message"}}"#), None);
        assert_eq!(
            reading_from_log_line(r#"{"timestamp":"2026-11-23T00:35:21Z","payload":{"rate_limits":null}}"#),
            None
        );
    }

    #[test]
    fn reads_the_last_snapshot_in_a_log() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("rollout-x.jsonl");
        let line = |pct: u32, at: &str| {
            format!(
                r#"{{"timestamp":"{at}","payload":{{"rate_limits":{{"primary":{{"used_percent":{pct},"window_minutes":300,"resets_at":1795761031}}}}}}}}"#
            )
        };
        let filler =
            format!(r#"{{"timestamp":"2026-11-23T00:00:00Z","payload":{{"text":"{}"}}}}"#, "x".repeat(300_000));
        std::fs::write(&file, [line(5, "2026-11-23T00:00:01Z"), line(9, "2026-11-23T00:00:02Z"), filler].join("\n"))
            .expect("write");
        assert_eq!(last_reading_in(&file).and_then(|r| r.primary).map(|w| w.pct), Some(9.0));
    }

    #[test]
    fn auth_needs_a_chatgpt_sign_in() {
        assert_eq!(
            auth_from(r#"{"tokens":{"access_token":"t","account_id":"acc"}}"#).map(|a| a.account_id),
            Some("acc".into())
        );
        assert_eq!(auth_from(r#"{"OPENAI_API_KEY":"sk-x","tokens":null}"#), None);
        assert!(
            !format!("{:?}", auth_from(r#"{"tokens":{"access_token":"secret","account_id":"acc"}}"#))
                .contains("secret")
        );
    }
}
