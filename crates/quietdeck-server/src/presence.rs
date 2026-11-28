//! Whether the person is asleep, from Fleetlayer's presence authority, for the triage runner: while they are, its agent
//! isn't woken, and the first check after they wake sends one catch-up (in place of the fixed `runner.quiet_hours`,
//! which the runner still uses while this is unknown).
//!
//! The deck reads `GET /v1/readings?kind=presence` with its read token every minute and tells the runners on each
//! change. A reading counts only from the authority's own node (any node with `reading.publish` could publish one of
//! that kind), and one older than 15 minutes means the authority has stopped: unknown.

use std::sync::Mutex;
use std::time::Duration;

use jiff::Timestamp;
use quietdeck_types::node::DeckRequest;
use serde_json::Value;

use crate::fleetlayer::Client;
use crate::state::AppState;

const EVERY: Duration = Duration::from_secs(60);
/// A reading older than this means the authority has stopped (Fleetlayer's presence docs: it publishes at least every
/// 5 minutes).
const STALE_SECONDS: i64 = 15 * 60;
/// A failing watch is a problem on /healthz/runners after this long (until then the last state stands).
pub const PROBLEM_AFTER_MS: i64 = 15 * 60_000;

/// What the runners were last told: Some(asleep), or None while unknown.
static ASLEEP: Mutex<Option<bool>> = Mutex::new(None);
/// Since when (epoch ms) and why the watch has failed, cleared by a good read.
static PROBLEM: Mutex<Option<(i64, String)>> = Mutex::new(None);

/// Whether the person is asleep, as far as the deck knows.
pub fn asleep() -> Option<bool> {
    *ASLEEP.lock().expect("presence lock")
}

/// The watch's problem once it has lasted [`PROBLEM_AFTER_MS`] at `now` (epoch ms).
pub fn problem(now: i64) -> Option<String> {
    let p = PROBLEM.lock().expect("presence lock").clone()?;
    (now - p.0 >= PROBLEM_AFTER_MS).then(|| format!("Fleetlayer presence: {} (for {} min)", p.1, (now - p.0) / 60_000))
}

/// `nathan@agents-presence` → (person, node).
pub fn target(s: &str) -> Option<(String, String)> {
    let (person, node) = s.trim().split_once('@')?;
    (!person.is_empty() && !node.is_empty()).then(|| (person.to_string(), node.to_string()))
}

/// The person's state from a `/v1/readings` page: Some(asleep) from a fresh reading by the authority's node, None from
/// a stale one, an error when there's none from that node.
pub fn judge(page: &Value, person: &str, node: &str, now: Timestamp) -> Result<Option<bool>, String> {
    let reading = page["readings"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|r| r["kind"] == "presence" && r["key"] == person && r["node"] == node)
        .ok_or_else(|| format!("no presence reading for {person} from {node}"))?;
    let at = reading["data"]["at"]
        .as_str()
        .or(reading["at"].as_str())
        .and_then(|a| a.parse::<Timestamp>().ok())
        .ok_or("the presence reading has no time")?;
    if now.duration_since(at).as_secs() > STALE_SECONDS {
        return Ok(None);
    }
    match reading["data"]["state"].as_str() {
        Some("asleep") => Ok(Some(true)),
        Some(_) => Ok(Some(false)),
        None => Err("the presence reading has no state".into()),
    }
}

async fn check(state: &AppState, client: &Client, person: &str, node: &str) {
    let now = Timestamp::now();
    let judged = match client.readings("presence").await {
        Ok(page) => judge(&page, person, node, now),
        Err(e) => Err(e.to_string()),
    };
    let asleep = match judged {
        Ok(a) => {
            *PROBLEM.lock().expect("presence lock") = None;
            a
        }
        Err(e) => {
            tracing::warn!(error = %e, "reading the person's presence from Fleetlayer");
            let mut p = PROBLEM.lock().expect("presence lock");
            let since = p.as_ref().map_or(now.as_millisecond(), |p| p.0);
            *p = Some((since, e));
            // A failed read leaves what the runners were told until it has failed for a while.
            if now.as_millisecond() - since < PROBLEM_AFTER_MS {
                return;
            }
            None
        }
    };
    let changed = {
        let mut told = ASLEEP.lock().expect("presence lock");
        let changed = *told != asleep;
        *told = asleep;
        changed
    };
    if changed {
        tracing::info!(?asleep, "the person's presence changed");
        let active = crate::reading::is_active(&state.db).await.unwrap_or(false);
        crate::runner::tell_runners(state, DeckRequest::Presence { active, asleep }).await;
    }
}

/// Reads the person's presence every minute, when `QUIETDECK_FLEETLAYER_PRESENCE` and the read token are set.
pub async fn ticker(state: AppState, target: Option<(String, String)>) {
    let (Some((person, node)), Some(client)) = (target, state.fleetlayer.clone()) else { return };
    tracing::info!(%person, %node, "following the person's presence from Fleetlayer");
    loop {
        check(&state, &client, &person, &node).await;
        tokio::time::sleep(EVERY).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn page(node: &str, state: &str, at: &str) -> Value {
        json!({"readings": [{"kind": "presence", "key": "nathan", "node": node, "at": at,
            "data": {"person": "nathan", "state": state, "at": at}}]})
    }

    #[test]
    fn asleep_counts_only_from_the_authority_and_only_while_fresh() {
        let now: Timestamp = "2026-11-29T12:00:00Z".parse().unwrap();
        let fresh = "2026-11-29T11:57:00Z";
        assert_eq!(judge(&page("agents-presence", "asleep", fresh), "nathan", "agents-presence", now), Ok(Some(true)));
        assert_eq!(judge(&page("agents-presence", "away", fresh), "nathan", "agents-presence", now), Ok(Some(false)));
        assert_eq!(
            judge(&page("agents-presence", "present", fresh), "nathan", "agents-presence", now),
            Ok(Some(false))
        );
        // Another node's reading of the same kind isn't the authority's.
        assert!(judge(&page("laptop", "asleep", fresh), "nathan", "agents-presence", now).is_err());
        // An authority that stopped publishing: unknown, so the runner's own quiet hours apply.
        let old = "2026-11-29T11:40:00Z";
        assert_eq!(judge(&page("agents-presence", "asleep", old), "nathan", "agents-presence", now), Ok(None));
        assert!(judge(&json!({"readings": []}), "nathan", "agents-presence", now).is_err());
    }

    #[test]
    fn the_target_is_person_at_node() {
        assert_eq!(target("nathan@agents-presence"), Some(("nathan".into(), "agents-presence".into())));
        assert_eq!(target("agents-presence"), None);
        assert_eq!(target("@x"), None);
    }
}
