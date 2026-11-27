//! The headless agent: no tab, no herdr. Each message the runner sends starts one `claude -p` turn that resumes the
//! same conversation (`--resume <session id>`, kept in the state directory), one turn at a time. Messages that arrive
//! during a turn wait and go together in the next one.
//!
//! The agent gets only what it needs to triage: the Bash tool, allowed to run the `quietdeck` CLI and nothing else
//! (`--permission-mode dontAsk` refuses everything not allowed, without a prompt nobody would answer), and no MCP
//! servers (`--strict-mcp-config`), so none of the user's mail or other connectors. The runner is the only thing that
//! touches the mailbox.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use quietdeck_types::AgentKind;
use quietdeck_types::relay::{Delivery, DeliveryKind, DeliveryTarget};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

use super::deck::AgentInbox;

/// Messages waiting for the next turn. More than this and the runner hears the agent is behind.
const QUEUE: usize = 32;

/// `runner.agent.headless`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeadlessConfig {
    /// The Claude Code CLI. Default `claude` on PATH.
    #[serde(default = "default_bin")]
    pub bin: String,
    /// Where each turn runs. Default `~/.local/share/quietdeck/agent`, made on first use.
    #[serde(default)]
    pub cwd: Option<String>,
    /// The triage instructions, added to Claude Code's system prompt (`--append-system-prompt-file`).
    #[serde(default)]
    pub instructions_file: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// What the agent may run, as Claude Code permission rules. Default: the quietdeck CLI only.
    #[serde(default = "default_allowed")]
    pub allowed_tools: Vec<String>,
    /// The longest one turn may take before it's stopped.
    #[serde(default = "default_turn_timeout")]
    pub turn_timeout: String,
}

fn default_bin() -> String {
    "claude".into()
}

/// The quietdeck subcommands triage needs, and no more: the agent reads untrusted mail, so `msg send`, `session
/// start` and the like stay one config change away rather than one prompt injection away.
pub const TRIAGE_COMMANDS: [&str; 10] =
    ["whoami", "card", "cards", "report", "round", "chat", "decisions", "action", "actions", "history"];

fn default_allowed() -> Vec<String> {
    TRIAGE_COMMANDS.iter().map(|c| format!("Bash(quietdeck {c}:*)")).collect()
}

fn default_turn_timeout() -> String {
    "15m".into()
}

impl HeadlessConfig {
    pub fn check(&self) -> Result<(), String> {
        super::config::parse_duration(&self.turn_timeout)
            .map_err(|e| format!("runner.agent.headless.turn_timeout: {e}"))?;
        if self.allowed_tools.iter().any(|t| t.trim() == "Bash" || t.trim() == "Bash(*)") {
            return Err(
                "runner.agent.headless.allowed_tools: Bash with no command pattern lets the agent run anything; \
                name the commands, as Bash(quietdeck:*)"
                    .into(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SavedSession {
    session_id: Option<String>,
}

/// Messages to the agent, run as headless turns by a task of its own.
pub struct HeadlessAgent {
    tab: String,
    tx: mpsc::Sender<String>,
}

impl HeadlessAgent {
    /// Start the turn loop. `tab` is the agent's name in messages (`runner.agent.tab`).
    /// Each turn's `claude` process is in `registry` while it runs, so the CLI calls it makes are the agent's.
    pub fn spawn(
        cfg: HeadlessConfig,
        tab: String,
        state_dir: PathBuf,
        registry: Arc<crate::local::HeadlessTurns>,
    ) -> HeadlessAgent {
        let (tx, rx) = mpsc::channel(QUEUE);
        let turns = Turns {
            timeout: super::config::parse_duration(&cfg.turn_timeout).unwrap_or(Duration::from_secs(900)),
            cfg,
            session_path: state_dir.join("headless-agent.json"),
            tab: tab.clone(),
            registry,
        };
        tokio::spawn(turns.run(rx));
        HeadlessAgent { tab, tx }
    }
}

#[async_trait]
impl AgentInbox for HeadlessAgent {
    async fn tell(&self, kind: DeliveryKind, body: String) -> Result<(), String> {
        let d = Delivery {
            id: String::new(),
            target: DeliveryTarget { tab: self.tab.clone(), session_id: None, address: None, agent: AgentKind::Claude },
            kind,
            body,
        };
        let text = crate::deliver::envelope(&d.from_name(), &d.render());
        self.tx.try_send(text).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => format!("{} is {QUEUE} messages behind", self.tab),
            mpsc::error::TrySendError::Closed(_) => format!("{}'s turn loop has stopped", self.tab),
        })
    }
}

struct Turns {
    cfg: HeadlessConfig,
    timeout: Duration,
    session_path: PathBuf,
    tab: String,
    registry: Arc<crate::local::HeadlessTurns>,
}

/// Which conversation a turn is in: an old one to resume, or a new one under an ID chosen here, so the turn's
/// identity carries its session ID from its first call.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Conversation<'a> {
    Resume(&'a str),
    New(&'a str),
}

impl Conversation<'_> {
    fn id(&self) -> &str {
        match self {
            Conversation::Resume(id) | Conversation::New(id) => id,
        }
    }
}

/// A random version 4 UUID, which `--session-id` needs.
fn new_session_id() -> String {
    let mut b: [u8; 16] = rand::random();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex::encode(b);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// What one `claude -p --output-format json` printed.
#[derive(Debug, PartialEq)]
struct TurnResult {
    session_id: Option<String>,
    is_error: bool,
    result: String,
}

impl Turns {
    async fn run(self, mut rx: mpsc::Receiver<String>) {
        while let Some(first) = rx.recv().await {
            let mut batch = vec![first];
            while let Ok(more) = rx.try_recv() {
                batch.push(more);
            }
            let prompt = batch.join("\n\n");
            match self.turn(&prompt).await {
                Ok(()) => {}
                Err(e) => tracing::warn!(error = %e, messages = batch.len(), "the headless agent's turn failed"),
            }
        }
    }

    fn saved(&self) -> Option<String> {
        let s = std::fs::read_to_string(&self.session_path).ok()?;
        serde_json::from_str::<SavedSession>(&s).ok()?.session_id
    }

    fn save(&self, id: &str) {
        let s = SavedSession { session_id: Some(id.to_string()) };
        let tmp = self.session_path.with_extension("tmp");
        let write = || -> std::io::Result<()> {
            if let Some(dir) = self.session_path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&tmp, serde_json::to_vec(&s).expect("serialises"))?;
            std::fs::rename(&tmp, &self.session_path)
        };
        if let Err(e) = write() {
            tracing::warn!(error = %e, path = %self.session_path.display(), "saving the headless agent's session ID");
        }
    }

    /// One turn, resuming the saved conversation. A conversation Claude Code no longer has starts a new one.
    async fn turn(&self, prompt: &str) -> Result<(), String> {
        let resume = self.saved();
        let fresh = new_session_id();
        let first = match &resume {
            Some(id) => Conversation::Resume(id),
            None => Conversation::New(&fresh),
        };
        let r = match self.claude(first, prompt).await {
            Err(e) if resume.is_some() && e.contains("No conversation found") => {
                tracing::warn!("the headless agent's conversation is gone: starting a new one");
                self.claude(Conversation::New(&fresh), prompt).await
            }
            other => other,
        }?;
        if let Some(id) = &r.session_id {
            if resume.as_deref() != Some(id.as_str()) {
                tracing::info!(session = %id, "the headless agent's conversation");
            }
            self.save(id);
        }
        if r.is_error {
            return Err(format!("the turn ended in an error: {}", super::tools::short(&Value::String(r.result))));
        }
        Ok(())
    }

    fn cwd(&self) -> PathBuf {
        match &self.cfg.cwd {
            Some(c) => crate::config::expand(c),
            None => crate::config::expand("~/.local/share/quietdeck/agent"),
        }
    }

    async fn claude(&self, conversation: Conversation<'_>, prompt: &str) -> Result<TurnResult, String> {
        let cwd = self.cwd();
        std::fs::create_dir_all(&cwd).map_err(|e| format!("{}: {e}", cwd.display()))?;
        let args = claude_args(&self.cfg, conversation);
        let bin = &self.cfg.bin;
        let mut child = tokio::process::Command::new(bin)
            .args(&args)
            .current_dir(&cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => {
                    format!("{bin} isn't installed or isn't on PATH (set runner.agent.headless.bin)")
                }
                _ => format!("{bin}: {e}"),
            })?;
        // The turn is the agent until this goes out of scope: when the child exits, times out or this future is
        // dropped (which kills the child too).
        let _registration =
            child.id().map(|pid| self.registry.begin(pid, self.tab.clone(), conversation.id().to_string()));
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(prompt.as_bytes()).await.map_err(|e| format!("{bin}: {e}"))?;
        }
        let out = tokio::time::timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| format!("the turn took longer than {}, so it was stopped", self.cfg.turn_timeout))?
            .map_err(|e| format!("{bin}: {e}"))?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        if let Some(r) = parse_result(&stdout) {
            return Ok(r);
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        let said = if stderr.trim().is_empty() { stdout.trim() } else { stderr.trim() };
        Err(format!("{bin} exited ({}): {}", out.status, super::tools::short(&Value::String(said.to_string()))))
    }
}

/// `claude -p` with the prompt on stdin, limited to the allowed commands and no MCP servers.
fn claude_args(cfg: &HeadlessConfig, conversation: Conversation<'_>) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "-p".into(),
        "--output-format=json".into(),
        "--permission-mode=dontAsk".into(),
        "--tools=Bash".into(),
        "--strict-mcp-config".into(),
    ];
    // One `=` form each: --allowedTools takes a list, and a bare value after it would swallow what follows.
    for t in &cfg.allowed_tools {
        a.push(format!("--allowedTools={t}"));
    }
    if let Some(m) = &cfg.model {
        a.push(format!("--model={m}"));
    }
    if let Some(f) = &cfg.instructions_file {
        a.push(format!("--append-system-prompt-file={}", crate::config::expand(f).display()));
    }
    a.push(match conversation {
        Conversation::Resume(id) => format!("--resume={id}"),
        Conversation::New(id) => format!("--session-id={id}"),
    });
    a
}

/// The `result` object `--output-format json` prints last.
fn parse_result(stdout: &str) -> Option<TurnResult> {
    let v: Value = stdout
        .lines()
        .rev()
        .find_map(|l| serde_json::from_str::<Value>(l.trim()).ok().filter(|v| v.get("type").is_some()))
        .or_else(|| serde_json::from_str(stdout.trim()).ok())?;
    if v.get("type").and_then(Value::as_str) != Some("result") {
        return None;
    }
    Some(TurnResult {
        session_id: v.get("session_id").and_then(Value::as_str).map(str::to_string),
        is_error: v.get("is_error").and_then(Value::as_bool).unwrap_or(false),
        result: v.get("result").and_then(Value::as_str).unwrap_or_default().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(bin: &str) -> HeadlessConfig {
        yaml_serde::from_str(&format!("bin: {bin}\n")).unwrap()
    }

    #[test]
    fn turns_are_limited_to_the_quietdeck_cli() {
        let a = claude_args(&cfg("claude"), Conversation::Resume("abc"));
        assert!(a.contains(&"--permission-mode=dontAsk".to_string()));
        assert!(a.contains(&"--tools=Bash".to_string()));
        assert!(a.contains(&"--allowedTools=Bash(quietdeck card:*)".to_string()));
        assert!(
            !a.iter().any(|x| x.contains("quietdeck msg")
                || x.contains("quietdeck session")
                || x == "--allowedTools=Bash(quietdeck:*)"),
            "no messages to other sessions or new sessions by default: {a:?}"
        );
        assert!(a.contains(&"--strict-mcp-config".to_string()), "none of the user's MCP servers");
        assert_eq!(a.last().unwrap(), "--resume=abc");
        let n = claude_args(&cfg("claude"), Conversation::New("0f0e"));
        assert_eq!(n.last().unwrap(), "--session-id=0f0e");
        assert!(!n.iter().any(|x| x.starts_with("--resume")));
        let id = new_session_id();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");

        let mut c = cfg("claude");
        c.allowed_tools = vec!["Bash".into()];
        assert!(c.check().unwrap_err().contains("run anything"));
        cfg("claude").check().unwrap();
    }

    #[test]
    fn the_result_line_gives_the_session() {
        let out = r#"{"type":"result","subtype":"success","is_error":false,"result":"Done.","session_id":"s-1"}"#;
        assert_eq!(
            parse_result(out),
            Some(TurnResult { session_id: Some("s-1".into()), is_error: false, result: "Done.".into() })
        );
        assert_eq!(parse_result("not json"), None);
    }

    /// A stand-in claude: logs its arguments and stdin, and prints a result with a session ID, or fails to resume.
    fn fake_claude(dir: &std::path::Path) -> String {
        let script = dir.join("claude");
        std::fs::write(
            &script,
            format!(
                r#"#!/usr/bin/env bash
dir={dir}
printf '%s\n' "$*" >> "$dir/calls"
cat >> "$dir/prompts"; printf '\n---\n' >> "$dir/prompts"
if grep -q slow "$dir/prompts"; then sleep 1; fi
for a in "$@"; do
  if [ "$a" = "--resume=gone" ]; then echo "No conversation found with session ID: gone" >&2; exit 1; fi
done
echo '{{"type":"result","is_error":false,"result":"ok","session_id":"s-new"}}'
"#,
                dir = dir.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script.to_string_lossy().into_owned()
    }

    #[tokio::test]
    async fn a_turn_resumes_and_keeps_the_session_and_a_lost_one_starts_again() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg(&fake_claude(dir.path()));
        c.cwd = Some(dir.path().join("agent").to_string_lossy().into_owned());
        let turns = Turns {
            cfg: c,
            timeout: Duration::from_secs(10),
            session_path: dir.path().join("s.json"),
            tab: "email triage".into(),
            registry: Arc::default(),
        };

        turns.turn("first").await.unwrap();
        assert_eq!(turns.saved().as_deref(), Some("s-new"));
        turns.turn("second").await.unwrap();
        turns.save("gone");
        turns.turn("third").await.unwrap();
        assert_eq!(turns.saved().as_deref(), Some("s-new"), "a lost conversation starts a new one");

        let calls = std::fs::read_to_string(dir.path().join("calls")).unwrap();
        let lines: Vec<&str> = calls.lines().collect();
        assert!(!lines[0].contains("--resume") && lines[0].contains("--session-id="), "the first turn starts one");
        assert!(lines[1].ends_with("--resume=s-new"));
        assert!(lines[2].ends_with("--resume=gone") && lines[3].contains("--session-id="));
        assert!(turns.registry.caller(std::process::id()).await.is_none(), "no turn is registered after it ends");
        let prompts = std::fs::read_to_string(dir.path().join("prompts")).unwrap();
        assert!(prompts.starts_with("first\n---\nsecond"), "the prompt goes on stdin: {prompts}");
    }

    #[tokio::test]
    async fn a_turn_is_the_agent_only_while_it_runs() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg(&fake_claude(dir.path()));
        c.cwd = Some(dir.path().join("agent").to_string_lossy().into_owned());
        let registry: Arc<crate::local::HeadlessTurns> = Arc::default();
        let turns = Arc::new(Turns {
            cfg: c,
            timeout: Duration::from_secs(10),
            session_path: dir.path().join("s.json"),
            tab: "email triage".into(),
            registry: registry.clone(),
        });
        let t = turns.clone();
        let running = tokio::spawn(async move { t.turn("slow").await });
        let mut seen = None;
        for _ in 0..80 {
            if let Some(s) = registry.running().into_iter().next() {
                seen = Some(s);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let s = seen.expect("the turn is registered while it runs");
        assert_eq!(s.tab.as_deref(), Some("email triage"));
        assert_eq!(s.session_id.as_ref().map(String::len), Some(36), "a new conversation's ID, known up front");
        running.await.unwrap().unwrap();
        assert!(registry.running().is_empty(), "and not after");
    }

    #[tokio::test]
    async fn messages_during_a_turn_go_together_in_the_next() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg(&fake_claude(dir.path()));
        c.cwd = Some(dir.path().join("agent").to_string_lossy().into_owned());
        let agent = HeadlessAgent::spawn(c, "email triage".into(), dir.path().to_path_buf(), Arc::default());
        agent.tell(DeliveryKind::Wakeup, "New mail: 1 Personal.".into()).await.unwrap();
        agent.tell(DeliveryKind::Handoff, "Card 7 needs you.".into()).await.unwrap();
        for _ in 0..100 {
            if std::fs::read_to_string(dir.path().join("headless-agent.json")).is_ok_and(|s| s.contains("s-new"))
                && std::fs::read_to_string(dir.path().join("prompts")).is_ok_and(|p| p.contains("Card 7"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let prompts = std::fs::read_to_string(dir.path().join("prompts")).unwrap();
        assert!(prompts.contains("<cross-session-message from-name=\"Quietdeck runner\">\nNew mail: 1 Personal."));
        assert!(prompts.contains("Card 7 needs you."));
    }
}
