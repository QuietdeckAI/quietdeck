//! The gog mail backend: the user's own `gog` CLI (github.com/steipete/gogcli, MIT), already signed in to their Gmail
//! accounts, in place of Executor. Each op runs one or more gog commands and answers in the shape of Gmail's API that
//! the runner already reads (`historyId`, `history[].messagesAdded`, `labelIds`, `payload.headers`, `sendAs`,
//! `drafts[].message.id`), so the mail watch, the freshness checks and the actions don't change.
//!
//! - The account always comes from the op's input, which `runner.mail: gog` fills from node.yaml's account address,
//!   never from mail content.
//! - IDs go in as positional arguments only when they look like Gmail IDs, and every other value as `--flag=value`,
//!   so nothing can turn into a flag. A draft's body goes on stdin, not the command line.
//! - Reads carry gog's own `--readonly`, so a read op can't change the mailbox even by mistake.
//! - gog's exit codes are stable: 5 is "not found" (an expired history ID, a message that's gone), which the runner
//!   treats as Gmail's 404, and 4 means the account needs signing in again.
//!
//! `gog gmail history` asks Gmail for `messageAdded` only and prints bare message IDs, so each new message's labels and
//! thread come from `gog gmail raw <id> --format minimal`, and threads that leave the inbox come from the `inbox` op
//! (thread IDs only), which the mail watch compares between listings.

use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use super::tools::short;

const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// Listing a whole inbox, 500 thread IDs a page.
const LIST_TIMEOUT: Duration = Duration::from_secs(180);
/// History entries per page, each a `raw` call.
const HISTORY_PAGE: u32 = 100;

const EXIT_AUTH: i32 = 4;
const EXIT_NOT_FOUND: i32 = 5;
const EXIT_RATE_LIMITED: i32 = 7;

/// `runner.gog`: where the CLI is. Optional; the default is `gog` on PATH.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GogConfig {
    #[serde(default = "default_bin")]
    pub bin: String,
}

impl Default for GogConfig {
    fn default() -> Self {
        GogConfig { bin: default_bin() }
    }
}

fn default_bin() -> String {
    "gog".into()
}

/// Every op `gog:` takes in a tool spec.
pub const OPS: [&str; 14] = [
    "profile",
    "history",
    "message",
    "message_text",
    "recent",
    "drafts",
    "inbox",
    "thread",
    "send_as",
    "labels",
    "archive",
    "mark_read",
    "label",
    "draft",
];

pub struct Gog {
    bin: String,
}

struct Output {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Gog {
    pub fn new(cfg: &GogConfig) -> Gog {
        Gog { bin: crate::config::expand(&cfg.bin).to_string_lossy().into_owned() }
    }

    /// Run one op. `input` is the op's filled-in template: always `account`, plus the op's own fields.
    pub async fn call(&self, op: &str, input: &Value) -> Result<Value, String> {
        let account = text(input, "account").ok_or_else(|| format!("gog {op}: the input has no account"))?;
        match op {
            "profile" => self.profile(&account).await,
            "history" => self.history(&account, input).await,
            "message" => {
                let id = gmail_id(input, "id")?;
                let v = self.json(&account, true, &["gmail", "raw", &id, "--format=metadata"], None, None).await?;
                Ok(unwrap_message(v))
            }
            "message_text" => {
                let id = gmail_id(input, "id")?;
                let args = ["gmail", "get", &id, "--format=full", "--sanitize-content"];
                let v = self.json(&account, true, &args, None, None).await?;
                Ok(normalize_message_text(&v))
            }
            "recent" => {
                let args = ["gmail", "messages", "search", "--max=25", "--", "in:inbox"];
                let v = self.json(&account, true, &args, None, None).await?;
                Ok(normalize_recent(&v))
            }
            "thread" => {
                let id = gmail_id(input, "thread_id")?;
                let v = self.json(&account, true, &["gmail", "thread", "get", &id], None, None).await?;
                Ok(normalize_thread(&v))
            }
            "drafts" => {
                let v =
                    self.json(&account, true, &["gmail", "drafts", "list", "--all", "--max=500"], None, None).await?;
                Ok(normalize_drafts(&v))
            }
            "inbox" => {
                let args = ["gmail", "search", "--ids-only", "--all", "--max=500", "--", "in:inbox"];
                let v = self.json(&account, true, &args, None, Some(LIST_TIMEOUT)).await?;
                Ok(normalize_inbox(&v))
            }
            "send_as" => {
                let v = self.json(&account, true, &["gmail", "settings", "sendas", "list"], None, None).await?;
                Ok(normalize_send_as(&v))
            }
            "labels" => {
                let v = self.json(&account, true, &["gmail", "labels", "list"], None, None).await?;
                Ok(normalize_labels(&v))
            }
            "archive" => self.modify(&account, input, &[], &["INBOX".into()]).await,
            "mark_read" => self.modify(&account, input, &[], &["UNREAD".into()]).await,
            "label" => {
                let add = strings(input, "add");
                let remove = strings(input, "remove");
                if add.is_empty() && remove.is_empty() {
                    return Err("gog label: nothing to add or remove".into());
                }
                self.modify(&account, input, &add, &remove).await
            }
            "draft" => self.draft(&account, input).await,
            other => Err(format!("gog has no op {other:?}; the runner takes {}", OPS.join(", "))),
        }
    }

    /// The mailbox's history ID to start from: gog has no getProfile, so the newest message's.
    async fn profile(&self, account: &str) -> Result<Value, String> {
        let args = ["gmail", "messages", "search", "--max=1", "--", "in:anywhere"];
        let v = self.json(account, true, &args, None, None).await?;
        let newest = first_id(&v, &["messages", "threads"])
            .ok_or_else(|| format!("{account} has no mail yet, so there's no history to start from"))?;
        let id = checked_id(&newest)?;
        let m = unwrap_message(self.json(account, true, &["gmail", "raw", &id, "--format=minimal"], None, None).await?);
        let h = history_id(&m).ok_or("gog's newest message has no historyId")?;
        Ok(json!({"historyId": h}))
    }

    /// One page of new messages since `since`, in the shape of Gmail's `users.history.list`.
    async fn history(&self, account: &str, input: &Value) -> Result<Value, String> {
        let since = text(input, "since").ok_or("gog history: the input has no since")?;
        if since.is_empty() || !since.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!("gog history: {since:?} isn't a history ID"));
        }
        let mut args =
            vec!["gmail".to_string(), "history".into(), format!("--since={since}"), format!("--max={HISTORY_PAGE}")];
        if let Some(page) = text(input, "page").filter(|p| !p.is_empty()) {
            args.push(format!("--page={page}"));
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let v = self.json(account, true, &args, None, None).await?;
        let mut added = vec![];
        for id in v.get("messages").and_then(Value::as_array).into_iter().flatten() {
            let id = match id {
                Value::String(s) => s.clone(),
                other => match other.get("id").and_then(Value::as_str) {
                    Some(s) => s.to_string(),
                    None => continue,
                },
            };
            let id = checked_id(&id)?;
            match self.json(account, true, &["gmail", "raw", &id, "--format=minimal"], None, None).await {
                Ok(m) => {
                    let m = unwrap_message(m);
                    added.push(json!({"message": {"id": id, "threadId": m.get("threadId"),
                        "labelIds": m.get("labelIds").cloned().unwrap_or(json!([]))}}));
                }
                // Gone again already (a follow-up nudge Gmail took back): not new mail.
                Err(e) if e.contains("404") => {}
                Err(e) => return Err(e),
            }
        }
        let mut out = json!({"history": [{"messagesAdded": added}]});
        if let Some(h) = history_id(&v) {
            out["historyId"] = json!(h);
        }
        if let Some(p) = v.get("nextPageToken").and_then(Value::as_str).filter(|p| !p.is_empty()) {
            out["nextPageToken"] = json!(p);
        }
        Ok(out)
    }

    /// Labels on every message in a thread. Archive takes INBOX off, mark read takes UNREAD off.
    async fn modify(&self, account: &str, input: &Value, add: &[String], remove: &[String]) -> Result<Value, String> {
        let id = gmail_id(input, "thread_id")?;
        let mut args = vec!["gmail".to_string(), "thread".into(), "modify".into(), id.clone()];
        for l in add {
            args.push(format!("--add-label={}", label_arg(l)?));
        }
        for l in remove {
            args.push(format!("--remove-label={}", label_arg(l)?));
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let v = self.json(account, false, &args, None, None).await?;
        Ok(if v.is_null() { json!({"thread_id": id}) } else { v })
    }

    /// A draft, as a reply when the input names the message it answers. The body goes on stdin.
    async fn draft(&self, account: &str, input: &Value) -> Result<Value, String> {
        let subject = text(input, "subject").ok_or("gog draft: no subject")?;
        let body = text(input, "body").ok_or("gog draft: no body")?;
        let to = strings(input, "to");
        if to.is_empty() {
            return Err("gog draft: no recipients".into());
        }
        let mut args = vec![
            "gmail".to_string(),
            "drafts".into(),
            "create".into(),
            format!("--to={}", to.join(",")),
            format!("--subject={subject}"),
            "--body-file=-".into(),
        ];
        let cc = strings(input, "cc");
        if !cc.is_empty() {
            args.push(format!("--cc={}", cc.join(",")));
        }
        if let Some(from) = text(input, "from").filter(|f| !f.is_empty()) {
            args.push(format!("--from={from}"));
        }
        if input.get("reply_to_message_id").is_some_and(|v| !v.is_null()) {
            args.push(format!("--reply-to-message-id={}", gmail_id(input, "reply_to_message_id")?));
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.json(account, false, &args, Some(body.as_bytes()), None).await
    }

    /// Run gog with `--account`, `--json` and `--no-input` (and `--readonly` for a read), and parse what it prints.
    async fn json(
        &self,
        account: &str,
        read: bool,
        args: &[&str],
        stdin: Option<&[u8]>,
        timeout: Option<Duration>,
    ) -> Result<Value, String> {
        if account.is_empty() || account.starts_with('-') || account.chars().any(char::is_whitespace) {
            return Err(format!("gog: {account:?} isn't an account address"));
        }
        let mut full = vec![format!("--account={account}"), "--json".into(), "--no-input".into()];
        if read {
            full.push("--readonly".into());
        }
        full.extend(args.iter().map(|a| a.to_string()));
        let out = self.run(&full, stdin, timeout.unwrap_or(CALL_TIMEOUT)).await?;
        let what = args.iter().take_while(|a| !a.starts_with('-')).take(3).copied().collect::<Vec<_>>().join(" ");
        match out.code {
            Some(0) => {}
            Some(EXIT_NOT_FOUND) => return Err(format!("404 not found (gog {what}): {}", error_message(&out))),
            Some(EXIT_AUTH) => {
                return Err(format!(
                    "gog isn't signed in to {account} any more (or lacks Gmail access): run `gog auth add {account} \
                     --services gmail` on this machine"
                ));
            }
            Some(EXIT_RATE_LIMITED) => return Err(format!("Gmail is rate-limiting gog {what}: try again shortly")),
            code => {
                let code = code.map_or("a signal".to_string(), |c| format!("exit {c}"));
                return Err(format!("gog {what} failed ({code}): {}", error_message(&out)));
            }
        }
        let text = out.stdout.trim();
        if text.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(text).map_err(|e| format!("gog {what} didn't print JSON: {e}"))
    }

    async fn run(&self, args: &[String], stdin: Option<&[u8]>, timeout: Duration) -> Result<Output, String> {
        let bin = &self.bin;
        let mut cmd = tokio::process::Command::new(bin);
        cmd.args(args)
            .stdin(if stdin.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() })
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = spawn(&mut cmd).await.map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                format!("{bin} isn't installed or isn't on PATH (set runner.gog.bin to its full path)")
            }
            _ => format!("{bin}: {e}"),
        })?;
        if let (Some(data), Some(mut pipe)) = (stdin, child.stdin.take()) {
            pipe.write_all(data).await.map_err(|e| format!("{bin}: {e}"))?;
            drop(pipe);
        }
        let out = tokio::time::timeout(timeout, child.wait_with_output())
            .await
            .map_err(|_| format!("{bin} didn't finish in {}s", timeout.as_secs()))?
            .map_err(|e| format!("{bin}: {e}"))?;
        Ok(Output {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

/// Spawn, trying again briefly when the program is busy being written ("Text file busy"): an upgrade of the CLI
/// in progress, or a script another thread has just written while a third forks.
pub(super) async fn spawn(cmd: &mut tokio::process::Command) -> std::io::Result<tokio::process::Child> {
    let mut tries = 0;
    loop {
        match cmd.spawn() {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 20 => {
                tries += 1;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            other => return other,
        }
    }
}

/// What gog said went wrong: with `--json` it prints `{"error": {"message": …}}` on stdout, and usually the same on
/// stderr. The JSON's message first, then stderr.
fn error_message(out: &Output) -> String {
    let from_json = serde_json::from_str::<Value>(out.stdout.trim())
        .ok()
        .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_string))
        .filter(|m| !m.trim().is_empty());
    match from_json {
        Some(m) => short(&Value::String(m)),
        None => stderr_line(&out.stderr),
    }
}

fn stderr_line(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() { "no message".into() } else { short(&Value::String(s.to_string())) }
}

fn text(v: &Value, key: &str) -> Option<String> {
    match v.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn strings(v: &Value, key: &str) -> Vec<String> {
    match v.get(key) {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
        _ => vec![],
    }
}

/// Gmail's message, thread and draft IDs: letters, digits, `-` and `_`, never starting with `-`.
fn checked_id(id: &str) -> Result<String, String> {
    let ok = !id.is_empty()
        && id.len() <= 200
        && !id.starts_with('-')
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok { Ok(id.to_string()) } else { Err(format!("{id:?} isn't a Gmail ID")) }
}

fn gmail_id(input: &Value, key: &str) -> Result<String, String> {
    checked_id(&text(input, key).ok_or_else(|| format!("the input has no {key}"))?)
}

/// A label name or ID for `--add-label=` / `--remove-label=`, which take it literally.
fn label_arg(l: &str) -> Result<&str, String> {
    if l.trim().is_empty() || l.len() > 225 || l.chars().any(char::is_control) {
        return Err(format!("{l:?} isn't a label name"));
    }
    Ok(l)
}

fn history_id(v: &Value) -> Option<String> {
    match v.get("historyId")? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// `gog gmail raw` prints Gmail's message; accept it under a `message` key too.
fn unwrap_message(v: Value) -> Value {
    match v.get("message") {
        Some(m @ Value::Object(_)) if v.get("id").is_none() => m.clone(),
        _ => v,
    }
}

/// The first item's `id` under any of `keys`, or of a bare array.
fn first_id(v: &Value, keys: &[&str]) -> Option<String> {
    let list = keys.iter().find_map(|k| v.get(*k).and_then(Value::as_array)).or_else(|| v.as_array())?;
    list.first().and_then(|m| m.get("id")).and_then(Value::as_str).map(str::to_string)
}

fn label_ids(m: &Value) -> Value {
    m.get("labelIds").or_else(|| m.get("labels")).cloned().unwrap_or(json!([]))
}

/// `gog gmail thread get` → Gmail's `users.threads.get` with `format: minimal`: each message's ID, labels and
/// `internalDate`.
pub fn normalize_thread(v: &Value) -> Value {
    let thread = v.get("thread").filter(|t| t.is_object()).unwrap_or(v);
    let messages: Vec<Value> = thread
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let id = m.get("id")?.as_str()?;
            let mut out = json!({"id": id, "labelIds": label_ids(m)});
            if let Some(d) = m.get("internalDate").filter(|d| !d.is_null()) {
                out["internalDate"] = d.clone();
            }
            Some(out)
        })
        .collect();
    json!({"id": thread.get("id"), "messages": messages})
}

/// `gog gmail get --format full --sanitize-content` → `{from, to, cc, subject, date, body}`: the headers by name and
/// the message's text with HTML and links taken out, which is what goes in front of the agent.
pub fn normalize_message_text(v: &Value) -> Value {
    let header = |name: &str| -> Option<String> {
        if let Some(h) = v.get("headers").and_then(|h| h.get(name)).and_then(Value::as_str).filter(|s| !s.is_empty()) {
            return Some(h.to_string());
        }
        let msg = v.get("message").unwrap_or(v);
        msg.pointer("/payload/headers").and_then(Value::as_array)?.iter().find_map(|h| {
            h.get("name")?
                .as_str()?
                .eq_ignore_ascii_case(name)
                .then(|| h.get("value")?.as_str().map(str::to_string))
                .flatten()
        })
    };
    let body = ["body", "text", "content"]
        .iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str).filter(|s| !s.trim().is_empty()))
        .or_else(|| v.get("message").and_then(|m| m.get("snippet")).and_then(Value::as_str))
        .or_else(|| v.get("snippet").and_then(Value::as_str))
        .unwrap_or_default();
    json!({"from": header("from"), "to": header("to"), "cc": header("cc"), "subject": header("subject"),
        "date": header("date"), "body": body})
}

/// `gog gmail messages search in:inbox` → `{messages: [{id, threadId, labelIds}]}`, newest first.
pub fn normalize_recent(v: &Value) -> Value {
    let list = v.get("messages").and_then(Value::as_array).or_else(|| v.as_array()).cloned().unwrap_or_default();
    let out: Vec<Value> = list
        .iter()
        .filter_map(|m| {
            let id = m.get("id")?.as_str()?;
            Some(json!({"id": id, "threadId": m.get("threadId"), "labelIds": label_ids(m)}))
        })
        .collect();
    json!({"messages": out})
}

/// `gog gmail drafts list` → `{drafts: [{id, message: {id, threadId}}]}`.
pub fn normalize_drafts(v: &Value) -> Value {
    let list = v.get("drafts").and_then(Value::as_array).or_else(|| v.as_array()).cloned().unwrap_or_default();
    let drafts: Vec<Value> = list
        .iter()
        .filter_map(|d| {
            let id = d.get("id").or_else(|| d.get("draftId"))?.as_str()?;
            let message = d.get("message").filter(|m| m.is_object());
            let mid = message
                .and_then(|m| m.get("id"))
                .or_else(|| d.get("messageId"))
                .or_else(|| d.get("message_id"))?
                .as_str()?;
            let tid = message.and_then(|m| m.get("threadId")).or_else(|| d.get("threadId")).cloned();
            Some(json!({"id": id, "message": {"id": mid, "threadId": tid}}))
        })
        .collect();
    json!({"drafts": drafts})
}

/// `gog gmail search --ids-only` → `{threads: [{id}], nextPageToken}`. A token left over means the listing stopped
/// part way, which the mail watch must not take as every other thread leaving.
pub fn normalize_inbox(v: &Value) -> Value {
    let ids: Vec<Value> = v
        .get("threads")
        .and_then(Value::as_array)
        .or_else(|| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|t| match t {
            Value::String(s) => Some(json!({"id": s})),
            other => other.get("id").and_then(Value::as_str).map(|s| json!({"id": s})),
        })
        .collect();
    let mut out = json!({"threads": ids});
    if let Some(p) = v.get("nextPageToken").and_then(Value::as_str).filter(|p| !p.is_empty()) {
        out["nextPageToken"] = json!(p);
    }
    out
}

/// `gog gmail settings sendas list` → `{sendAs: [{sendAsEmail}]}`.
pub fn normalize_send_as(v: &Value) -> Value {
    let list = ["sendAs", "send_as", "aliases"]
        .iter()
        .find_map(|k| v.get(*k).and_then(Value::as_array))
        .or_else(|| v.as_array())
        .cloned()
        .unwrap_or_default();
    let out: Vec<Value> = list
        .iter()
        .filter_map(|s| {
            let email = s.get("sendAsEmail").or_else(|| s.get("email"))?.as_str()?;
            Some(json!({"sendAsEmail": email}))
        })
        .collect();
    json!({"sendAs": out})
}

/// `gog gmail labels list` → `{labels: [{id, name}]}`.
pub fn normalize_labels(v: &Value) -> Value {
    let list = v.get("labels").and_then(Value::as_array).or_else(|| v.as_array()).cloned().unwrap_or_default();
    let out: Vec<Value> = list
        .iter()
        .filter_map(|l| {
            let name = l.get("name")?.as_str()?;
            Some(json!({"id": l.get("id").and_then(Value::as_str).unwrap_or(name), "name": name}))
        })
        .collect();
    json!({"labels": out})
}

/// The tool specs `runner.mail: gog` fills in, by name: `profile`, `history`, `message`, `drafts`, `inbox` (the mail
/// watch), `thread`, `send_as`, `labels`, and the actions `archive`, `mark_read`, `label`, `draft`.
pub fn preset_input(op: &str) -> Value {
    let account = json!("${account.address}");
    match op {
        "history" => json!({"account": account, "since": "${history_id}", "page": "${page_token}"}),
        "message" | "message_text" => json!({"account": account, "id": "${message_id}"}),
        "thread" | "archive" | "mark_read" => json!({"account": account, "thread_id": "${action.thread_id}"}),
        "label" => json!({"account": account, "thread_id": "${action.thread_id}", "add": "${action.add}",
            "remove": "${action.remove}"}),
        "draft" => json!({"account": account, "from": "${action.from}", "to": "${action.to}", "cc": "${action.cc}",
            "subject": "${action.subject}", "body": "${action.body}",
            "reply_to_message_id": "${action.reply_to_message_id}"}),
        _ => json!({"account": account}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_that_could_be_flags_are_refused() {
        assert!(checked_id("18c2f0a9b1d3e4f5").is_ok());
        assert!(checked_id("r-123_abc").is_ok());
        assert!(checked_id("--account=evil@example.com").is_err());
        assert!(checked_id("-x").is_err());
        assert!(checked_id("a b").is_err());
        assert!(checked_id("").is_err());
        assert!(label_arg("Clients/Acme").is_ok());
        assert!(label_arg("bad\nlabel").is_err());
    }

    #[test]
    fn threads_take_gogs_shape_or_gmails() {
        let gog = json!({"thread": {"id": "t1", "messages": [
            {"id": "m1", "labelIds": ["INBOX", "UNREAD"], "internalDate": "1700000000000", "body": "x"},
            {"id": "m2", "labels": ["SENT"]}]}});
        assert_eq!(
            normalize_thread(&gog),
            json!({"id": "t1", "messages": [
                {"id": "m1", "labelIds": ["INBOX", "UNREAD"], "internalDate": "1700000000000"},
                {"id": "m2", "labelIds": ["SENT"]}]})
        );
        let gmail = json!({"id": "t2", "messages": [{"id": "m3", "labelIds": [], "internalDate": 5}]});
        assert_eq!(normalize_thread(&gmail)["messages"][0]["internalDate"], 5);
    }

    #[test]
    fn drafts_send_as_labels_and_inbox() {
        let d = json!({"drafts": [{"id": "r1", "message": {"id": "m1", "threadId": "t1"}},
            {"draftId": "r2", "messageId": "m2"}, {"id": "r3"}]});
        let n = normalize_drafts(&d);
        assert_eq!(n["drafts"].as_array().unwrap().len(), 2, "a draft without a message ID is skipped");
        assert_eq!(n["drafts"][1]["message"]["id"], "m2");

        let s =
            json!({"sendAs": [{"sendAsEmail": "me@example.com", "isDefault": true}, {"email": "alias@example.com"}]});
        assert_eq!(
            normalize_send_as(&s),
            json!({"sendAs": [{"sendAsEmail": "me@example.com"}, {"sendAsEmail": "alias@example.com"}]})
        );

        let l = json!([{"id": "INBOX", "name": "INBOX"}, {"name": "Clients"}]);
        assert_eq!(normalize_labels(&l)["labels"][1], json!({"id": "Clients", "name": "Clients"}));

        let i = json!({"threads": [{"id": "t1"}, {"id": "t2"}], "nextPageToken": ""});
        assert_eq!(normalize_inbox(&i), json!({"threads": [{"id": "t1"}, {"id": "t2"}]}));
        let partial = json!({"threads": [{"id": "t1"}], "nextPageToken": "p2"});
        assert_eq!(normalize_inbox(&partial)["nextPageToken"], "p2");
    }

    #[test]
    fn message_text_takes_gogs_headers_and_body_or_gmails_headers() {
        let gog = json!({"headers": {"from": "Bob <b@example.com>", "to": "me@example.com", "subject": "Hi",
            "date": "Fri, 27 Nov 2026", "cc": ""}, "body": "Hello\nthere"});
        let n = normalize_message_text(&gog);
        assert_eq!(n["from"], "Bob <b@example.com>");
        assert_eq!(n["cc"], Value::Null, "an empty header is no header");
        assert_eq!(n["body"], "Hello\nthere");
        let gmail = json!({"message": {"snippet": "Just the snippet", "payload": {"headers": [
            {"name": "Subject", "value": "Re: Hi"}]}}});
        let n = normalize_message_text(&gmail);
        assert_eq!(n["subject"], "Re: Hi");
        assert_eq!(n["body"], "Just the snippet", "the snippet when there's no text");
        // gog's messages search (fixture 02's shape) gives labels, not labelIds.
        let r = normalize_recent(&json!({"messages": [{"id": "m1", "threadId": "t1", "labels": ["INBOX"]}]}));
        assert_eq!(r, json!({"messages": [{"id": "m1", "threadId": "t1", "labelIds": ["INBOX"]}]}));
    }

    #[test]
    fn raw_messages_unwrap() {
        assert_eq!(unwrap_message(json!({"message": {"id": "m1"}})), json!({"id": "m1"}));
        let m = json!({"id": "m1", "historyId": "9"});
        assert_eq!(unwrap_message(m.clone()), m);
        assert_eq!(history_id(&json!({"historyId": 12})), Some("12".into()));
    }

    /// A stand-in gog: logs its arguments and stdin, and answers from `GOG_FAKE_DIR/<op>.json` with the exit code in
    /// `<op>.exit`, where the op is the arguments after the global flags joined by `_`.
    fn fake_gog(dir: &std::path::Path) -> Gog {
        let script = dir.join("gog");
        std::fs::write(
            &script,
            format!(
                r#"#!/usr/bin/env bash
dir={dir}
printf '%s\n' "$*" >> "$dir/calls"
words=()
for a in "$@"; do
  case "$a" in --*) ;; *) words+=("$a") ;; esac
done
op=$(IFS=_; echo "${{words[*]:0:3}}")
if [ "${{words[0]}}_${{words[1]}}" = gmail_raw ]; then op="raw_${{words[2]}}"; fi
if [ "${{words[0]}}_${{words[1]}}_${{words[2]}}" = gmail_drafts_create ]; then cat > "$dir/stdin"; fi
[ -f "$dir/$op.json" ] && cat "$dir/$op.json"
exit $(cat "$dir/$op.exit" 2>/dev/null || echo 0)
"#,
                dir = dir.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        Gog { bin: script.to_string_lossy().into_owned() }
    }

    #[tokio::test]
    async fn history_reads_each_new_messages_labels_and_skips_ones_already_gone() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(
            p.join("gmail_history.json"),
            r#"{"historyId":"120","messages":["m1","m2"],"nextPageToken":""}"#,
        )
        .unwrap();
        std::fs::write(p.join("raw_m1.json"), r#"{"id":"m1","threadId":"t1","labelIds":["INBOX","UNREAD"]}"#).unwrap();
        std::fs::write(p.join("raw_m2.exit"), "5").unwrap();
        let gog = fake_gog(p);
        let v = gog.call("history", &json!({"account": "me@example.com", "since": "100"})).await.expect("history");
        assert_eq!(
            v,
            json!({"historyId": "120", "history": [{"messagesAdded": [
                {"message": {"id": "m1", "threadId": "t1", "labelIds": ["INBOX", "UNREAD"]}}]}]})
        );
        let calls = std::fs::read_to_string(p.join("calls")).unwrap();
        assert!(
            calls.lines().all(|l| l.starts_with("--account=me@example.com --json --no-input --readonly ")),
            "{calls}"
        );
        assert!(calls.contains("gmail history --since=100 --max=100"), "{calls}");
    }

    #[tokio::test]
    async fn not_found_is_a_404_and_sign_in_trouble_says_how_to_fix_it() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("gmail_history.exit"), "5").unwrap();
        std::fs::write(p.join("gmail_labels_list.exit"), "4").unwrap();
        let gog = fake_gog(p);
        let e = gog.call("history", &json!({"account": "me@example.com", "since": "1"})).await.unwrap_err();
        assert!(e.contains("404"), "{e}");
        let e = gog.call("labels", &json!({"account": "me@example.com"})).await.unwrap_err();
        assert!(e.contains("gog auth add me@example.com"), "{e}");
        let e = gog.call("history", &json!({"account": "me@example.com", "since": "1; rm"})).await.unwrap_err();
        assert!(e.contains("isn't a history ID"), "{e}");
    }

    #[tokio::test]
    async fn profile_starts_from_the_newest_message() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("gmail_messages_search.json"), r#"{"messages":[{"id":"m9","threadId":"t9"}]}"#).unwrap();
        std::fs::write(p.join("raw_m9.json"), r#"{"id":"m9","historyId":"4242"}"#).unwrap();
        let v = fake_gog(p).call("profile", &json!({"account": "me@example.com"})).await.unwrap();
        assert_eq!(v, json!({"historyId": "4242"}));
    }

    #[tokio::test]
    async fn actions_modify_the_thread_and_drafts_send_the_body_on_stdin() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("gmail_drafts_create.json"), r#"{"draftId":"r1","threadId":"t1"}"#).unwrap();
        let gog = fake_gog(p);
        let a = json!({"account": "me@example.com", "thread_id": "t1"});
        gog.call("archive", &a).await.unwrap();
        gog.call("mark_read", &a).await.unwrap();
        gog.call("label", &json!({"account": "me@example.com", "thread_id": "t1", "add": ["Clients/Acme"]}))
            .await
            .unwrap();
        let d = gog
            .call(
                "draft",
                &json!({"account": "me@example.com", "from": "me@example.com", "to": ["a@example.com", "b@example.com"],
                "subject": "Re: Hi", "body": "--not-a-flag\nThanks!", "reply_to_message_id": "m1"}),
            )
            .await
            .unwrap();
        assert_eq!(d["draftId"], "r1");
        let calls = std::fs::read_to_string(p.join("calls")).unwrap();
        let lines: Vec<&str> = calls.lines().collect();
        assert_eq!(lines[0], "--account=me@example.com --json --no-input gmail thread modify t1 --remove-label=INBOX");
        assert!(lines[1].ends_with("modify t1 --remove-label=UNREAD"));
        assert!(lines[2].ends_with("modify t1 --add-label=Clients/Acme"));
        assert!(!lines[0].contains("--readonly"), "writes can't be read-only");
        assert!(lines[3].contains("--to=a@example.com,b@example.com --subject=Re: Hi --body-file=-"), "{}", lines[3]);
        assert!(lines[3].ends_with("--reply-to-message-id=m1"));
        assert!(!calls.contains("Thanks"), "the body never goes on the command line");
        assert_eq!(std::fs::read_to_string(p.join("stdin")).unwrap(), "--not-a-flag\nThanks!");
    }

    /// Real output from gog 0.46.0 against a personal Gmail account (27 November 2026), scrubbed: what each op reads
    /// from it. The fixtures are in `fixtures/gog/`.
    mod real_gog {
        use super::*;

        fn fixture(name: &str) -> Value {
            let path = format!("{}/src/runner/fixtures/gog/{name}.json", env!("CARGO_MANIFEST_DIR"));
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
        }

        fn copy(dir: &std::path::Path, fixture_name: &str, as_op: &str) {
            std::fs::write(dir.join(format!("{as_op}.json")), fixture(fixture_name).to_string()).unwrap();
        }

        #[tokio::test]
        async fn history_and_profile() {
            let dir = tempfile::tempdir().unwrap();
            let p = dir.path();
            copy(p, "history", "gmail_history");
            copy(p, "messages_search", "gmail_messages_search");
            // Every message the history names reads as the one raw fixture (its labels and thread).
            for id in ["1a1fdeb3931f11b9", "1a1fdeb3f0eb86ba", "1a1fe18c6d392682", "1a21b61acd474ee0"] {
                copy(p, "raw_minimal", &format!("raw_{id}"));
            }
            let gog = fake_gog(p);
            let v = gog.call("history", &json!({"account": "alice@example.com", "since": "28605522"})).await.unwrap();
            assert_eq!(v["historyId"], "28643311");
            assert_eq!(v["nextPageToken"], "10454226333937819819", "a long history pages");
            let added = v["history"][0]["messagesAdded"].as_array().unwrap();
            assert_eq!(added.len(), 3);
            assert_eq!(added[0]["message"]["threadId"], "1a1f37b62f43d035");
            assert_eq!(added[0]["message"]["labelIds"], json!(["SENT"]));

            let profile = gog.call("profile", &json!({"account": "alice@example.com"})).await.unwrap();
            assert_eq!(profile, json!({"historyId": "28605522"}), "the newest message's historyId");
        }

        #[test]
        fn the_error_message_comes_from_gogs_json_then_stderr() {
            let out = |stdout: String, stderr: &str| Output { code: Some(5), stdout, stderr: stderr.into() };
            let m = error_message(&out(fixture("not_found").to_string(), ""));
            assert_eq!(m, "Google API error (404 notFound): Requested entity was not found.");
            assert_eq!(error_message(&out(String::new(), "boom\n")), "boom");
            assert_eq!(error_message(&out("{}".into(), "")), "no message");
        }

        #[tokio::test]
        async fn not_found_is_exit_5_with_the_error_on_stdout() {
            let dir = tempfile::tempdir().unwrap();
            let p = dir.path();
            copy(p, "not_found", "gmail_history");
            std::fs::write(p.join("gmail_history.exit"), "5").unwrap();
            let e = fake_gog(p).call("history", &json!({"account": "alice@example.com", "since": "1"})).await;
            assert!(e.unwrap_err().contains("404"), "an expired history ID starts again from now");
        }

        #[test]
        fn messages_threads_labels_drafts_send_as_and_search() {
            let m = unwrap_message(fixture("raw_metadata"));
            assert_eq!(m["threadId"], "1a1f37b62f43d035");
            let headers = m.pointer("/payload/headers").and_then(Value::as_array).unwrap();
            assert!(headers.iter().any(|h| h["name"] == "Subject" && h["value"] == "Re: Subject 4"));

            let t = normalize_thread(&fixture("thread_get"));
            let msgs = t["messages"].as_array().unwrap();
            assert_eq!(msgs.len(), 2);
            assert!(
                msgs.iter().all(|m| m["id"].is_string() && m["labelIds"].is_array() && m["internalDate"].is_string())
            );
            assert_eq!(t["id"], "1a1f37b62f43d035");

            let l = normalize_labels(&fixture("labels"));
            assert!(l["labels"].as_array().unwrap().iter().any(|x| x == &json!({"id": "Label_10", "name": "Label A"})));

            assert_eq!(normalize_drafts(&fixture("drafts_empty")), json!({"drafts": []}));
            assert_eq!(normalize_send_as(&fixture("sendas"))["sendAs"][0], json!({"sendAsEmail": "alice@example.com"}));

            let i = normalize_inbox(&fixture("search"));
            assert_eq!(i["threads"].as_array().unwrap().len(), 2);
            assert!(i.get("nextPageToken").is_some(), "a page with more after it is partial");
        }
    }

    #[tokio::test]
    async fn a_missing_gog_says_so() {
        let gog = Gog { bin: "/nonexistent/gog".into() };
        let e = gog.call("labels", &json!({"account": "me@example.com"})).await.unwrap_err();
        assert!(e.contains("isn't installed"), "{e}");
    }
}
