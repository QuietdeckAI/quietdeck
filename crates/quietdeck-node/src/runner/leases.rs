//! A gmail-filter lease's side effects, carried out for the deck: create the filter (with the lease's label, out of
//! the inbox and unread) and move matching inbox mail; later delete the filter. heavy-build slots have no steps.

use quietdeck_types::lease::{KIND_GMAIL_FILTER, Lease, LeaseStep};
use serde_json::{Value, json};

use super::Runner;
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
    match step {
        LeaseStep::Create => {
            let label_id = label_id(r, cfg, &acct, &label).await?;
            let ctx = json!({"account": acct, "lease": {"id": lease.id, "query": query, "label": label, "label_id": label_id}});
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
            Ok(json!({"filter_id": filter_id, "label": label, "label_id": label_id, "moved": moved}))
        }
        LeaseStep::Remove => {
            let Some(filter_id) = lease.kind_state.as_ref().and_then(|s| s.get("filter_id")).and_then(Value::as_str)
            else {
                return Ok(json!({"removed": false, "reason": "no filter was created"}));
            };
            let ctx = json!({"account": acct, "lease": {"id": lease.id, "query": query, "label": label, "filter_id": filter_id}});
            match r.tools.call(&cfg.delete, tools::render(&cfg.delete.input, &ctx)?).await {
                Ok(_) => Ok(json!({"removed": true})),
                // Deleted by hand already: the step's goal holds.
                Err(e) if e.contains("404") || e.to_lowercase().contains("not found") => {
                    Ok(json!({"removed": true, "already_gone": true}))
                }
                Err(e) => Err(e),
            }
        }
    }
}

/// The lease label's ID, made first when it doesn't exist.
async fn label_id(
    r: &Runner,
    cfg: &super::config::GmailFiltersConfig,
    acct: &Value,
    label: &str,
) -> Result<String, String> {
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
