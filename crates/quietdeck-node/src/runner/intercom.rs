//! A support inbox as a source (Intercom): conversations waiting on a reply wake the triage agent like new mail.
//! The runner only counts them and names their IDs; the agent reads each with its own tools, and closes one through
//! the `close_conversation` action once the person approves, like archiving an email.

use jiff::Timestamp;
use serde_json::{Value, json};

use super::config::IntercomWatchConfig;
use super::mail::MailState;
use super::tools::{Tools, render};

/// Pages read in one check, at most: enough for a busy morning, never a loop.
const MAX_PAGES: usize = 10;

/// One conversation from the list tool. Only the fields the watch needs; the agent reads the rest.
#[derive(Debug, Clone, PartialEq)]
pub struct Waiting {
    pub id: String,
    pub updated_at: i64,
}

/// Seconds since the epoch, from a number or a numeric string.
fn seconds(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn id_of(v: &Value) -> Option<String> {
    match v.get("id")? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The open conversations changed since `since` that wait on a teammate's reply: new ones, and ones the customer
/// wrote in again. A conversation the person answered or closed has no `waiting_since`, so his own work never wakes
/// the agent. A list that leaves `waiting_since` out counts every change.
pub fn waiting(listed: &Value, since: i64) -> Vec<Waiting> {
    let has_waiting = |c: &Value| c.as_object().is_some_and(|o| o.contains_key("waiting_since"));
    listed
        .get("conversations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let id = id_of(c)?;
            let updated_at = seconds(c.get("updated_at"))?;
            let open = c.get("state").and_then(Value::as_str).is_none_or(|s| s == "open");
            let waits = !has_waiting(c) || seconds(c.get("waiting_since")).is_some();
            (updated_at > since && open && waits).then_some(Waiting { id, updated_at })
        })
        .collect()
}

/// One check: list what changed since the last one, and add the conversations that wait to the next wake-up. The
/// first check only notes the time, so what was already open is the agent's own first round.
pub async fn check(watch: &IntercomWatchConfig, tools: &dyn Tools, state: &mut MailState, now: Timestamp) {
    let now_s = now.as_second();
    let Some(since) = state.intercom_since else {
        state.intercom_since = Some(now_s);
        return;
    };
    let mut page: Option<String> = None;
    let mut newest = since;
    let mut found = vec![];
    for _ in 0..MAX_PAGES {
        let ctx = json!({"updated_since": since, "starting_after": page});
        let listed = match render(&watch.conversations.input, &ctx) {
            Ok(input) => tools.call(&watch.conversations, input).await,
            Err(e) => Err(e),
        };
        let listed = match listed {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "listing the support inbox's conversations");
                state.intercom_error = Some(e);
                return;
            }
        };
        for w in waiting(&listed, since) {
            newest = newest.max(w.updated_at);
            found.push(w);
        }
        page = listed.get("next").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
        if page.is_none() {
            break;
        }
    }
    state.intercom_error = None;
    for w in found {
        if !state.intercom.contains(&w.id) {
            state.intercom.push(w.id);
        }
    }
    // Never past now: a clock that runs ahead upstream mustn't hide what comes next.
    state.intercom_since = Some(newest.min(now_s));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::config::{RunnerConfig, ToolSpec};

    /// The results still to give, and the inputs given so far.
    type Script = (Vec<Result<Value, String>>, Vec<Value>);

    /// Answers every call with the next result, and keeps the inputs it was given.
    struct Listed(std::sync::Mutex<Script>);

    #[async_trait::async_trait]
    impl Tools for Listed {
        async fn call(&self, _spec: &ToolSpec, input: Value) -> Result<Value, String> {
            let mut g = self.0.lock().unwrap();
            g.1.push(input);
            if g.0.is_empty() { Err("nothing scripted".into()) } else { g.0.remove(0) }
        }
    }

    #[tokio::test]
    async fn a_check_after_the_first_wakes_the_agent_with_the_waiting_conversations() {
        let cfg: RunnerConfig = yaml_serde::from_str(
            "agent: { tab: t }\nexecutor: { url: \"https://ex.example/mcp\", token_file: /tmp/t }\n\
             intercom_watch:\n  account: intercom\n  conversations:\n    path: tools.intercom_api.conversations.list\n\
             \x20   input: { updated_since: \"${updated_since}\", starting_after: \"${starting_after}\" }\n",
        )
        .unwrap();
        cfg.check().unwrap();
        let watch = cfg.intercom_watch.clone().unwrap();
        let tools = Listed(std::sync::Mutex::new((
            vec![
                Ok(
                    json!({"conversations": [{"id": "11", "state": "open", "updated_at": 1_000_100, "waiting_since": 1}],
                          "next": "p2"}),
                ),
                Ok(
                    json!({"conversations": [{"id": "12", "state": "open", "updated_at": 1_000_050, "waiting_since": 1}]}),
                ),
                Err("Intercom is down".into()),
            ],
            vec![],
        )));
        let mut state = MailState::default();
        let at = |s: i64| Timestamp::from_second(s).unwrap();
        check(&watch, &tools, &mut state, at(1_000_000)).await;
        assert_eq!(state.intercom_since, Some(1_000_000), "the first check only notes the time");
        assert!(state.intercom.is_empty() && tools.0.lock().unwrap().1.is_empty());

        check(&watch, &tools, &mut state, at(1_000_200)).await;
        assert_eq!(state.intercom, vec!["11", "12"]);
        assert_eq!(state.intercom_since, Some(1_000_100), "the newest change seen");
        let inputs = tools.0.lock().unwrap().1.clone();
        assert_eq!(inputs[0], json!({"updated_since": 1_000_000}), "no page on the first call");
        assert_eq!(inputs[1], json!({"updated_since": 1_000_000, "starting_after": "p2"}));
        let text = state.wakeup_text(&cfg).unwrap();
        assert!(text.starts_with("New in Intercom: 2 conversations waiting on a reply (11, 12)"), "{text}");
        assert!(text.contains("close_conversation") && text.ends_with("Run your triage round."), "{text}");

        check(&watch, &tools, &mut state, at(1_000_300)).await;
        assert_eq!(state.intercom_error.as_deref(), Some("Intercom is down"));
        assert_eq!(state.intercom_since, Some(1_000_100), "a failed check reads the same changes again next time");
        state.clear_pending();
        assert_eq!(state.wakeup_text(&cfg), None);
    }

    #[test]
    fn only_open_conversations_waiting_on_a_reply_since_the_last_check_count() {
        let listed = json!({"conversations": [
            {"id": "1", "state": "open", "updated_at": 200, "waiting_since": 190},
            {"id": "2", "state": "open", "updated_at": 200, "waiting_since": null},
            {"id": "3", "state": "closed", "updated_at": 200, "waiting_since": 190},
            {"id": "4", "state": "open", "updated_at": 100, "waiting_since": 90},
            {"id": 5, "state": "open", "updated_at": "300", "waiting_since": "250"},
        ]});
        let ids: Vec<String> = waiting(&listed, 150).into_iter().map(|w| w.id).collect();
        assert_eq!(ids, vec!["1", "5"], "answered, closed and older conversations don't wake the agent");
        let bare = json!({"conversations": [{"id": "6", "updated_at": 200}]});
        assert_eq!(waiting(&bare, 150).len(), 1, "a list without waiting_since counts every change");
    }
}
