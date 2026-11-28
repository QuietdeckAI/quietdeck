//! A gmail-filter lease's side effects: create the filter (with the lease's label, out of the inbox and unread) and
//! move matching inbox mail; later delete the filter. Carried out for the deck's leases, or, with
//! `gmail_filters.fleetlayer`, by making the filters match Fleetlayer's held leases ([`reconcile`]). heavy-build slots
//! have no steps.

use quietdeck_types::lease::{KIND_GMAIL_FILTER, Lease, LeaseStep};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::Runner;
use super::config::{FleetlayerLeasesConfig, GmailFiltersConfig};
use super::tools;

/// The label a lease's filter puts on what it catches, so the triage agent can report it.
pub fn lease_label(lease: &Lease) -> String {
    format!("Quietdeck/Lease/{}", lease.holder.tab)
}

pub async fn step(r: &Runner, lease: &Lease, step: LeaseStep) -> Result<Value, String> {
    if lease.kind != KIND_GMAIL_FILTER {
        return Err(format!("a {} lease has no steps for the runner", lease.kind));
    }
    let cfg = r.cfg.gmail_filters.as_ref().ok_or("runner.gmail_filters isn't set in node.yaml")?;
    let acct = r
        .account_ctx(&lease.resource)
        .ok_or_else(|| format!("account {} isn't in the runner's config", lease.resource))?;
    let query = lease.params.get("query").and_then(Value::as_str).ok_or("the lease has no query")?;
    let label = lease_label(lease);
    let id = Value::from(lease.id);
    match step {
        LeaseStep::Create => {
            let (filter_id, label_id, moved) = create_filter(r, cfg, &acct, &id, query, &label).await?;
            Ok(json!({"filter_id": filter_id, "label": label, "label_id": label_id, "moved": moved}))
        }
        LeaseStep::Remove => {
            let Some(filter_id) = lease.kind_state.as_ref().and_then(|s| s.get("filter_id")).and_then(Value::as_str)
            else {
                return Ok(json!({"removed": false, "reason": "no filter was created"}));
            };
            delete_filter(r, cfg, &acct, &id, query, &label, filter_id).await
        }
    }
}

/// Make the filter (with its label, out of the inbox and unread) and move matching inbox mail under it: the filter's
/// ID, the label's ID and how many threads moved.
async fn create_filter(
    r: &Runner,
    cfg: &GmailFiltersConfig,
    acct: &Value,
    id: &Value,
    query: &str,
    label: &str,
) -> Result<(String, String, usize), String> {
    let label_id = label_id(r, cfg, acct, label).await?;
    let ctx = json!({"account": acct, "lease": {"id": id, "query": query, "label": label, "label_id": label_id}});
    let made = r.tools.call(&cfg.create, tools::render(&cfg.create.input, &ctx)?).await?;
    let filter_id = made
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("creating the filter returned no id: {}", tools::short(&made)))?
        .to_string();
    let moved = match (&cfg.inbox, &cfg.move_threads) {
        (Some(inbox), Some(mv)) => {
            let found = r.tools.call(inbox, tools::render(&inbox.input, &ctx)?).await?;
            let ids: Vec<Value> = found
                .get("threads")
                .and_then(Value::as_array)
                .map(|t| t.iter().filter_map(|t| t.get("id").cloned()).collect())
                .unwrap_or_default();
            if !ids.is_empty() {
                let mut ctx = ctx.clone();
                ctx["thread_ids"] = Value::Array(ids.clone());
                r.tools.call(mv, tools::render(&mv.input, &ctx)?).await?;
            }
            ids.len()
        }
        _ => 0,
    };
    Ok((filter_id, label_id, moved))
}

async fn delete_filter(
    r: &Runner,
    cfg: &GmailFiltersConfig,
    acct: &Value,
    id: &Value,
    query: &str,
    label: &str,
    filter_id: &str,
) -> Result<Value, String> {
    let ctx = json!({"account": acct, "lease": {"id": id, "query": query, "label": label, "filter_id": filter_id}});
    match r.tools.call(&cfg.delete, tools::render(&cfg.delete.input, &ctx)?).await {
        Ok(_) => Ok(json!({"removed": true})),
        // Deleted by hand already: the step's goal holds.
        Err(e) if e.contains("404") || e.to_lowercase().contains("not found") => {
            Ok(json!({"removed": true, "already_gone": true}))
        }
        Err(e) => Err(e),
    }
}

/// A Gmail filter the runner made for a Fleetlayer lease's payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeaseFilter {
    pub account: String,
    pub query: String,
    /// The payload's key in Fleetlayer (`payloadKey`).
    pub key: String,
    pub filter_id: String,
    pub label: String,
}

/// A payload some held lease asks for, with its first holder (`<tab>@<node>`).
#[derive(Debug, Clone, PartialEq)]
pub struct Held {
    pub account: String,
    pub query: String,
    pub key: String,
    pub holder: String,
}

/// The payloads the held leases of `kind` ask for, once each, from `fleetlayer lease list --json`
/// (`{kinds: [{name, held: [lease]}], unlisted: [lease]}`: a held lease of a kind since removed from the settings is
/// in `unlisted` until it ends), in the order Fleetlayer lists them. None when the answer doesn't know the kind at all:
/// then nothing changes, rather than every filter going at once.
pub fn held(list: &Value, kind: &str) -> Option<Vec<Held>> {
    let mut out: Vec<Held> = vec![];
    let listed: Vec<&Value> =
        list["kinds"].as_array().into_iter().flatten().filter(|k| k["name"].as_str() == Some(kind)).collect();
    let unlisted: Vec<Value> = list["unlisted"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|l| l["kind"].as_str() == Some(kind))
        .cloned()
        .collect();
    if listed.is_empty() && unlisted.is_empty() {
        return None;
    }
    let leases = listed.into_iter().flat_map(|k| k["held"].as_array().cloned().unwrap_or_default()).chain(unlisted);
    for l in leases {
        let (Some(account), Some(query)) = (l["payload"]["account"].as_str(), l["payload"]["query"].as_str()) else {
            continue;
        };
        if out.iter().any(|h| h.account == account && h.query == query) {
            continue;
        }
        out.push(Held {
            account: account.to_string(),
            query: query.to_string(),
            key: l["payloadKey"].as_str().unwrap_or_default().to_string(),
            holder: l["holder"].as_str().unwrap_or("-").to_string(),
        });
    }
    Some(out)
}

/// What to make and what to take away so the filters match the held payloads exactly.
pub fn plan<'a>(held: &'a [Held], made: &'a [LeaseFilter]) -> (Vec<&'a Held>, Vec<&'a LeaseFilter>) {
    let missing = held.iter().filter(|h| !made.iter().any(|m| m.account == h.account && m.query == h.query)).collect();
    let stale = made.iter().filter(|m| !held.iter().any(|h| h.account == m.account && h.query == m.query)).collect();
    (missing, stale)
}

/// The label a held lease's filter puts on what it catches: its first holder's tab.
pub fn holder_label(holder: &str) -> String {
    let tab = holder.rsplit_once('@').map_or(holder, |(tab, _)| tab);
    let tab = if tab.is_empty() || tab == "-" { "script" } else { tab };
    format!("Quietdeck/Lease/{tab}")
}

/// List the held leases and make the filters match: create what's missing, remove what no held lease covers. Each
/// change is kept in `made` as it happens, so a failure part-way leaves it true. The problems, if any, together.
pub async fn reconcile(r: &Runner, fl: &FleetlayerLeasesConfig, made: &mut Vec<LeaseFilter>) -> Result<(), String> {
    let cfg = r.cfg.gmail_filters.as_ref().ok_or("runner.gmail_filters isn't set in node.yaml")?;
    let list = lease_list(fl).await?;
    let held = held(&list, &fl.kind)
        .ok_or_else(|| format!("Fleetlayer's leases list no {} kind: the filters stay as they are", fl.kind))?;
    let (missing, stale) = plan(&held, made);
    let (missing, stale): (Vec<Held>, Vec<LeaseFilter>) =
        (missing.into_iter().cloned().collect(), stale.into_iter().cloned().collect());
    let mut problems = vec![];
    for h in missing {
        let Some(acct) = r.account_ctx(&h.account) else {
            problems.push(format!("account {} (lease of {}) isn't in the runner's config", h.account, h.holder));
            continue;
        };
        let label = holder_label(&h.holder);
        match create_filter(r, cfg, &acct, &Value::from(h.key.clone()), &h.query, &label).await {
            Ok((filter_id, _, moved)) => {
                tracing::info!(account = %h.account, query = %h.query, holder = %h.holder, moved, "lease filter made");
                made.push(LeaseFilter { account: h.account, query: h.query, key: h.key, filter_id, label });
            }
            Err(e) => problems.push(format!("making the filter for {} ({}): {e}", h.query, h.account)),
        }
    }
    for m in stale {
        let Some(acct) = r.account_ctx(&m.account) else {
            problems.push(format!("account {} isn't in the runner's config any more", m.account));
            continue;
        };
        match delete_filter(r, cfg, &acct, &Value::from(m.key.clone()), &m.query, &m.label, &m.filter_id).await {
            Ok(_) => {
                tracing::info!(account = %m.account, query = %m.query, "lease filter removed");
                made.retain(|f| f != &m);
            }
            Err(e) => problems.push(format!("removing the filter for {} ({}): {e}", m.query, m.account)),
        }
    }
    if problems.is_empty() { Ok(()) } else { Err(problems.join("; ")) }
}

/// `fleetlayer lease list --json`, within 30 seconds.
async fn lease_list(fl: &FleetlayerLeasesConfig) -> Result<Value, String> {
    let mut cmd = tokio::process::Command::new(&fl.command);
    cmd.args(["lease", "list", "--json"]).kill_on_drop(true);
    let out = tokio::time::timeout(std::time::Duration::from_secs(30), cmd.output())
        .await
        .map_err(|_| format!("{} lease list took over 30 seconds", fl.command))?
        .map_err(|e| format!("running {}: {e}", fl.command))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("{} lease list failed: {}", fl.command, err.trim()));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("{} lease list --json: {e}", fl.command))
}

/// The lease label's ID, made first when it doesn't exist.
async fn label_id(r: &Runner, cfg: &GmailFiltersConfig, acct: &Value, label: &str) -> Result<String, String> {
    let list = r.cfg.labels.as_ref().ok_or("runner.labels isn't set, so the runner can't find the lease's label")?;
    let ctx = json!({"account": acct, "lease": {"label": label}});
    let found = r.tools.call(list, tools::render(&list.input, &ctx)?).await?;
    let existing = found.get("labels").and_then(Value::as_array).and_then(|ls| {
        ls.iter()
            .find(|l| l.get("name").and_then(Value::as_str) == Some(label))
            .and_then(|l| l.get("id"))
            .and_then(Value::as_str)
    });
    if let Some(id) = existing {
        return Ok(id.to_string());
    }
    let made = r.tools.call(&cfg.create_label, tools::render(&cfg.create_label.input, &ctx)?).await?;
    made.get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("creating label {label} returned no id: {}", tools::short(&made)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_follow_the_held_payloads() {
        let list = json!({"kinds": [
            {"name": "build-server", "held": [{"id": 1, "holder": "a@x", "payload": null}]},
            {"name": "gmail-filter", "held": [
                {"id": 2, "holder": "ci watch@agents", "payloadKey": "k1",
                 "payload": {"account": "docspring", "query": "from:builds@circleci.com"}},
                {"id": 3, "holder": "other@agents", "payloadKey": "k1",
                 "payload": {"account": "docspring", "query": "from:builds@circleci.com"}},
                {"id": 4, "holder": "-@agents", "payloadKey": "k2",
                 "payload": {"account": "personal", "query": "from:x@y.com"}}
            ], "asking": [{"id": 5, "holder": "z@agents", "payload": {"account": "a", "query": "from:q"}}]}
        ], "ended": []});
        let held = held(&list, "gmail-filter").unwrap();
        assert_eq!(held.len(), 2, "one per payload, held leases only");
        assert_eq!(super::held(&json!({"kinds": [], "ended": []}), "gmail-filter"), None, "unknown kind: no change");
        let removed = json!({"kinds": [], "unlisted": [
            {"kind": "gmail-filter", "holder": "a@b", "payload": {"account": "x", "query": "from:y"}}
        ]});
        assert_eq!(super::held(&removed, "gmail-filter").unwrap().len(), 1, "a removed kind's held lease counts");
        assert_eq!(held[0].holder, "ci watch@agents");
        assert_eq!(holder_label(&held[0].holder), "Quietdeck/Lease/ci watch");
        assert_eq!(holder_label(&held[1].holder), "Quietdeck/Lease/script");
        let made = vec![
            LeaseFilter {
                account: "docspring".into(),
                query: "from:builds@circleci.com".into(),
                key: "k1".into(),
                filter_id: "f1".into(),
                label: "Quietdeck/Lease/ci watch".into(),
            },
            LeaseFilter {
                account: "docspring".into(),
                query: "from:gone@x.com".into(),
                key: "k9".into(),
                filter_id: "f9".into(),
                label: "Quietdeck/Lease/old".into(),
            },
        ];
        let (missing, stale) = plan(&held, &made);
        assert_eq!(missing.iter().map(|h| h.key.as_str()).collect::<Vec<_>>(), vec!["k2"]);
        assert_eq!(stale.iter().map(|m| m.filter_id.as_str()).collect::<Vec<_>>(), vec!["f9"]);
    }
}
