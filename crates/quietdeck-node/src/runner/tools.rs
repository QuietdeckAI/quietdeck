//! Running a configured tool: an Executor app tool over MCP, or a command. Inputs are templates filled from the
//! action, the card and the account (see [`super::config::ToolSpec`]).
//!
//! Nothing is retried. A call that fails may still have happened, so the action is marked failed and the person
//! decides on Retry.

use std::time::Duration;

use async_trait::async_trait;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService, ServiceExt};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Map, Value};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use super::config::{ExecutorConfig, ToolSpec};

const CALL_TIMEOUT: Duration = Duration::from_secs(60);

#[async_trait]
pub trait Tools: Send + Sync {
    /// Run one tool with an input already filled in. Returns what the tool returned.
    async fn call(&self, spec: &ToolSpec, input: Value) -> Result<Value, String>;
}

enum Rendered {
    Value(Value),
    Missing,
}

fn lookup<'a>(ctx: &'a Value, path: &str) -> Option<&'a Value> {
    let mut v = ctx;
    for part in path.split('.') {
        v = v.get(part)?;
    }
    (!v.is_null()).then_some(v)
}

fn placeholder(s: &str) -> Option<&str> {
    let inner = s.strip_prefix("${")?.strip_suffix('}')?;
    (!inner.is_empty() && inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')).then_some(inner)
}

fn render_str(s: &str, ctx: &Value) -> Result<Rendered, String> {
    if let Some(name) = placeholder(s) {
        return Ok(lookup(ctx, name).map_or(Rendered::Missing, |v| Rendered::Value(v.clone())));
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after.find('}').ok_or_else(|| format!("unclosed ${{ in {s:?}"))?;
        let name = &after[..end];
        let value = lookup(ctx, name).ok_or_else(|| format!("{s:?} names ${{{name}}}, which has no value here"))?;
        match value {
            Value::String(v) => out.push_str(v),
            Value::Number(n) => out.push_str(&n.to_string()),
            Value::Bool(b) => out.push_str(&b.to_string()),
            _ => return Err(format!("${{{name}}} isn't text, so it can't go inside {s:?}")),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(Rendered::Value(Value::String(out)))
}

fn render_value(t: &Value, ctx: &Value) -> Result<Rendered, String> {
    Ok(match t {
        Value::String(s) => render_str(s, ctx)?,
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for i in items {
                if let Rendered::Value(v) = render_value(i, ctx)? {
                    out.push(v);
                }
            }
            Rendered::Value(Value::Array(out))
        }
        Value::Object(map) => {
            let mut out = Map::new();
            for (k, v) in map {
                if let Rendered::Value(v) = render_value(v, ctx)? {
                    out.insert(k.clone(), v);
                }
            }
            Rendered::Value(Value::Object(out))
        }
        other => Rendered::Value(other.clone()),
    })
}

/// Fill a tool's input template from `ctx`.
pub fn render(template: &Value, ctx: &Value) -> Result<Value, String> {
    match render_value(template, ctx)? {
        Rendered::Value(v) => Ok(v),
        Rendered::Missing => Ok(Value::Null),
    }
}

/// Executor and commands, as node.yaml configures them.
pub struct ConfiguredTools {
    executor: Option<Executor>,
}

impl ConfiguredTools {
    pub fn new(executor: Option<(ExecutorConfig, String)>) -> ConfiguredTools {
        ConfiguredTools {
            executor: executor.map(|(config, token)| Executor { config, token, client: Mutex::new(None) }),
        }
    }
}

#[async_trait]
impl Tools for ConfiguredTools {
    async fn call(&self, spec: &ToolSpec, input: Value) -> Result<Value, String> {
        match (&spec.path, &spec.command) {
            (Some(path), _) => {
                let ex = self.executor.as_ref().ok_or("runner.executor isn't set")?;
                ex.call(path, input).await
            }
            (None, Some(command)) => run_command(command, &input).await,
            (None, None) => Err("the tool names neither a path nor a command".into()),
        }
    }
}

/// An MCP client to the user's Executor, connected on first use and again after an error.
struct Executor {
    config: ExecutorConfig,
    token: String,
    client: Mutex<Option<RunningService<RoleClient, ()>>>,
}

impl Executor {
    async fn connect(&self) -> Result<RunningService<RoleClient, ()>, String> {
        let config =
            StreamableHttpClientTransportConfig::with_uri(self.config.url.clone()).auth_header(self.token.clone());
        let transport = StreamableHttpClientTransport::with_client(reqwest::Client::new(), config);
        tokio::time::timeout(CALL_TIMEOUT, ().serve(transport))
            .await
            .map_err(|_| "Executor didn't answer in time".to_string())?
            .map_err(|e| format!("connecting to Executor: {e}"))
    }

    async fn call(&self, path: &str, input: Value) -> Result<Value, String> {
        let mut guard = self.client.lock().await;
        if guard.is_none() {
            *guard = Some(self.connect().await?);
        }
        let client = guard.as_ref().expect("connected above");
        let mut args = Map::new();
        args.insert("calls".into(), serde_json::json!({ "path": path, "input": input }));
        let params = CallToolRequestParams::new(self.config.call_tool.clone()).with_arguments(args);
        let result = match tokio::time::timeout(CALL_TIMEOUT, client.call_tool(params)).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                *guard = None;
                return Err(format!("Executor: {e}"));
            }
            Err(_) => {
                *guard = None;
                return Err("Executor didn't answer in time".into());
            }
        };
        let text: String =
            result.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.as_str()).collect::<Vec<_>>().join("");
        let body = match result.structured_content {
            Some(v) if !v.is_null() => v,
            _ => serde_json::from_str(&text).unwrap_or(Value::String(text.clone())),
        };
        if result.is_error == Some(true) {
            return Err(format!("Executor: {}", short(&body)));
        }
        executor_value(body)
    }
}

/// Executor's `call` answers `{status, execution: {ok, value}}`, where `value` is `{ok, value | error}` for the
/// one call the runner sends (an array of them when `calls` is an array).
pub fn executor_value(body: Value) -> Result<Value, String> {
    let status = body.get("status").and_then(Value::as_str).unwrap_or("completed");
    if status != "completed" {
        return Err(format!("Executor wants {status} before this call can run: carry it out by hand"));
    }
    let item = match body.pointer("/execution/value") {
        Some(Value::Array(items)) => items.first().cloned(),
        Some(v @ Value::Object(_)) => Some(v.clone()),
        _ => None,
    }
    .ok_or_else(|| format!("Executor: {}", short(&body)))?;
    if item.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(item.get("value").cloned().unwrap_or(Value::Null));
    }
    let error = item.get("error").cloned().unwrap_or(Value::Null);
    let message = error.get("message").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| short(&error));
    Err(message)
}

/// A value cut to a length that fits in an error or an outcome.
pub fn short(v: &Value) -> String {
    let s = match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if s.chars().count() > 500 { s.chars().take(500).collect::<String>() + "…" } else { s }
}

async fn run_command(command: &[String], input: &Value) -> Result<Value, String> {
    let (prog, args) = command.split_first().ok_or("the command is empty")?;
    let mut child = tokio::process::Command::new(prog)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("{prog}: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.to_string().as_bytes()).await.map_err(|e| format!("{prog}: {e}"))?;
    }
    let out = tokio::time::timeout(CALL_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| format!("{prog} didn't finish in time"))?
        .map_err(|e| format!("{prog}: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("{prog}: {}", short(&Value::String(err.trim().to_string()))));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(text.trim()).map_err(|e| format!("{prog} didn't print JSON: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn placeholders_keep_their_types_and_missing_ones_drop_out() {
        let ctx = json!({"account": {"address": "me@example.com", "executor_account": "acc_1"},
            "action": {"thread_id": "t1", "add": ["A", "B"]}, "history_id": "99"});
        let t = json!({"accountId": "${account.executor_account}", "input": {"path": {"userId": "me"},
            "query": {"startHistoryId": "${history_id}", "pageToken": "${page_token}"}},
            "threadIds": ["${action.thread_id}"], "addLabels": "${action.add}", "note": "for ${account.address}"});
        let v = render(&t, &ctx).unwrap();
        assert_eq!(v["accountId"], "acc_1");
        assert_eq!(v["input"]["query"], json!({"startHistoryId": "99"}), "no page token on the first page");
        assert_eq!(v["threadIds"], json!(["t1"]));
        assert_eq!(v["addLabels"], json!(["A", "B"]));
        assert_eq!(v["note"], "for me@example.com");
    }

    #[test]
    fn a_missing_value_inside_text_is_an_error() {
        assert!(render(&json!("Re: ${card.subject}"), &json!({})).unwrap_err().contains("card.subject"));
    }

    #[test]
    fn executor_results_and_errors() {
        // One call (what the runner sends) answers with an object, an array of calls with an array.
        let ok = json!({"status": "completed", "execution": {"ok": true,
            "value": {"path": "p", "ok": true, "value": {"historyId": "5"}}}});
        assert_eq!(executor_value(ok).unwrap(), json!({"historyId": "5"}));
        let ok = json!({"status": "completed", "execution": {"ok": true,
            "value": [{"path": "p", "ok": true, "value": {"historyId": "6"}}]}});
        assert_eq!(executor_value(ok).unwrap(), json!({"historyId": "6"}));
        let err = json!({"status": "completed", "execution": {"ok": true,
            "value": [{"ok": false, "error": {"message": "HTTP 404"}}]}});
        assert_eq!(executor_value(err).unwrap_err(), "HTTP 404");
        let approval = json!({"status": "approval-required"});
        assert!(executor_value(approval).unwrap_err().contains("approval-required"));
    }

    #[tokio::test]
    async fn commands_take_json_on_stdin() {
        let spec = ToolSpec { path: None, command: Some(vec!["cat".into()]), input: Value::Null };
        let v = ConfiguredTools::new(None).call(&spec, json!({"a": 1})).await.unwrap();
        assert_eq!(v, json!({"a": 1}));
        let spec = ToolSpec { path: None, command: Some(vec!["false".into()]), input: Value::Null };
        assert!(ConfiguredTools::new(None).call(&spec, json!({})).await.is_err());
    }
}
