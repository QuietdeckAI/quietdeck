//! The mail watch: Gmail's history for each account, message IDs and labels only, and one wake-up at a time to the
//! triage agent. Nothing here reads a subject, a header or a body.
//!
//! The last history ID per account is kept on this machine, so after sleep the next check picks up where it left
//! off. After a wake-up, no other goes until the agent says its round is done; new mail meanwhile is counted and goes
//! in the next one.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::config::{MailWatchConfig, RunnerConfig};
use super::tools::{Tools, render};

/// Pages read per account per check: a long sleep catches up over a few checks.
const MAX_PAGES: usize = 10;

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct MailState {
    /// The last history ID read, per account.
    pub history: BTreeMap<String, String>,
    /// A wake-up went and the agent hasn't said `round done` yet.
    pub waiting: bool,
    /// New messages since the last wake-up, per account. `None`: some, but the history ran out, so how many isn't
    /// known.
    pub pending: BTreeMap<String, Option<u64>>,
}

impl MailState {
    pub fn load(path: &PathBuf) -> MailState {
        std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &PathBuf) {
        let tmp = path.with_extension("tmp");
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&tmp, serde_json::to_vec_pretty(self).expect("mail state serialises"))?;
            std::fs::rename(&tmp, path)
        };
        if let Err(e) = write() {
            tracing::warn!(error = %e, path = %path.display(), "saving the mail watch's history IDs");
        }
    }

    fn add(&mut self, account: &str, n: Option<u64>) {
        let entry = self.pending.entry(account.to_string()).or_insert(Some(0));
        *entry = match (*entry, n) {
            (Some(a), Some(b)) => Some(a + b),
            _ => None,
        };
    }

    /// "New mail: 2 Personal, 1 Work." in config order, or None when there's nothing to say.
    pub fn wakeup_text(&self, cfg: &RunnerConfig) -> Option<String> {
        let parts: Vec<String> = cfg
            .accounts
            .keys()
            .filter_map(|id| match self.pending.get(id) {
                Some(Some(0)) | None => None,
                Some(Some(n)) => Some(format!("{n} {}", cfg.account_label(id))),
                Some(None) => Some(format!("{} (some while the runner couldn't follow)", cfg.account_label(id))),
            })
            .collect();
        (!parts.is_empty()).then(|| {
            format!(
                "New mail: {}. Run your triage round, then `quietdeck round done` so the next wake-up can come.",
                parts.join(", ")
            )
        })
    }
}

fn history_id(v: &Value) -> Option<String> {
    match v.get("historyId")? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Gmail answers an expired history ID with HTTP 404.
fn expired(error: &str) -> bool {
    error.contains("404")
}

/// One check of every account. Updates the history IDs and pending counts in `state`.
pub async fn check(cfg: &RunnerConfig, watch: &MailWatchConfig, tools: &dyn Tools, state: &mut MailState) {
    for (id, account) in &cfg.accounts {
        let acct = json!({"id": id, "address": account.address, "label": cfg.account_label(id)});
        let mut acct = acct;
        for (k, v) in &account.vars {
            acct[k] = Value::String(v.clone());
        }
        let Some(start) = state.history.get(id).cloned() else {
            // First run: start from now. Mail from before the runner is the agent's own first round.
            match profile(watch, tools, &acct).await {
                Ok(h) => {
                    state.history.insert(id.clone(), h);
                }
                Err(e) => tracing::warn!(account = %id, error = %e, "reading the mailbox's history ID"),
            }
            continue;
        };
        let mut seen: HashSet<String> = HashSet::new();
        let mut page: Option<String> = None;
        let mut latest = start.clone();
        let mut failed = false;
        for _ in 0..MAX_PAGES {
            let ctx = json!({"account": acct, "history_id": start, "page_token": page});
            let input = match render(&watch.history.input, &ctx) {
                Ok(i) => i,
                Err(e) => {
                    tracing::warn!(account = %id, error = %e, "runner.mail_watch.history input");
                    failed = true;
                    break;
                }
            };
            match tools.call(&watch.history, input).await {
                Ok(v) => {
                    for h in v.get("history").and_then(Value::as_array).into_iter().flatten() {
                        for added in h.get("messagesAdded").and_then(Value::as_array).into_iter().flatten() {
                            let msg = &added["message"];
                            let labels: Vec<&str> = msg
                                .get("labelIds")
                                .and_then(Value::as_array)
                                .map(|a| a.iter().filter_map(Value::as_str).collect())
                                .unwrap_or_default();
                            if (labels.contains(&"INBOX") || labels.contains(&"SENT"))
                                && let Some(mid) = msg.get("id").and_then(Value::as_str)
                            {
                                seen.insert(mid.to_string());
                            }
                        }
                    }
                    if let Some(h) = history_id(&v) {
                        latest = h;
                    }
                    page = v.get("nextPageToken").and_then(Value::as_str).map(str::to_string);
                    if page.is_none() {
                        break;
                    }
                }
                Err(e) if expired(&e) => {
                    // The saved ID is too old (a long sleep): start again from now, and say mail came.
                    match profile(watch, tools, &acct).await {
                        Ok(h) => {
                            state.history.insert(id.clone(), h);
                            state.add(id, None);
                        }
                        Err(e) => tracing::warn!(account = %id, error = %e, "reading the mailbox's history ID"),
                    }
                    failed = true;
                    break;
                }
                Err(e) => {
                    tracing::warn!(account = %id, error = %e, "reading the mailbox's history");
                    failed = true;
                    break;
                }
            }
        }
        if failed {
            continue;
        }
        state.history.insert(id.clone(), latest);
        if !seen.is_empty() {
            state.add(id, Some(seen.len() as u64));
        }
    }
}

async fn profile(watch: &MailWatchConfig, tools: &dyn Tools, acct: &Value) -> Result<String, String> {
    let input = render(&watch.profile.input, &json!({"account": acct}))?;
    let v = tools.call(&watch.profile, input).await?;
    history_id(&v).ok_or_else(|| "the profile has no historyId".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_add_up_and_unknowns_stay_unknown() {
        let cfg: RunnerConfig = yaml_serde::from_str(
            "agent: { tab: t }\naccounts:\n  personal: { address: a@example.com }\n  \
             work: { address: b@example.com, label: Work }\n",
        )
        .unwrap();
        let mut s = MailState::default();
        assert_eq!(s.wakeup_text(&cfg), None);
        s.add("personal", Some(1));
        s.add("personal", Some(1));
        s.add("work", Some(1));
        assert!(s.wakeup_text(&cfg).unwrap().starts_with("New mail: 2 Personal, 1 Work."));
        s.add("personal", None);
        assert!(s.wakeup_text(&cfg).unwrap().contains("Personal (some while"));
    }
}
