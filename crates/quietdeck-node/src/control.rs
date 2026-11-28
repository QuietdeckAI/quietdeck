//! Sessions the deck starts, resumes and closes on this machine (flow 3.15). The person's click or submitted form
//! is the approval, and the deck logs it. The node checks the request, then has it done without taking focus, so
//! nothing moves on the person's screen.
//!
//! Where this machine runs a Fleetlayer node, Fleetlayer does it (`node/session_open`, `node/session_close` on its local
//! socket, as `fleetlayer session start|resume|close` would), and the agents' flags come from Fleetlayer's node.yaml
//! (`harnesses:`), with the model and effort asked for. Without Fleetlayer, or for a start naming a session other than
//! its tab, the orchestrator (herdr) does it with node.yaml's `sessions:` flags.

use std::path::Path;
use std::time::Duration;

use quietdeck_types::AgentKind;
use quietdeck_types::directory::{SessionResume, SessionStart, SessionStarted};

use crate::Node;
use crate::config::expand;
use crate::orchestrator::OpenTab;

/// Fleetlayer waits for the agent to take input before answering a start; the deck gives up after 150 seconds.
const FLEETLAYER_OPENS_WITHIN: Duration = Duration::from_secs(140);
/// The deck gives a close 20 seconds.
const FLEETLAYER_CLOSES_WITHIN: Duration = Duration::from_secs(18);

/// Fleetlayer's answer to `node/session_open`.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FlStarted {
    node: String,
    tab: String,
    address: String,
    session_id: Option<String>,
    prompt_error: Option<String>,
}

fn harness(agent: AgentKind) -> &'static str {
    match agent {
        AgentKind::Claude => "claude",
        AgentKind::Codex => "codex",
    }
}

/// Whether Fleetlayer can start this one as asked: it names a session after its tab, so a different name needs herdr.
/// Model and effort go through (Fleetlayer 0.4.0).
fn fleetlayer_can_start(spec: &SessionStart) -> bool {
    spec.name.as_deref().is_none_or(|n| n == spec.tab)
}

fn start_params(spec: &SessionStart, cwd: &str) -> serde_json::Value {
    serde_json::json!({
        "tab": spec.tab, "cwd": cwd, "harness": harness(spec.agent), "prompt": spec.prompt,
        "model": spec.model, "effort": spec.effort, "why": "started from Quietdeck's deck",
    })
}

fn resume_params(spec: &SessionResume, cwd: &str) -> serde_json::Value {
    serde_json::json!({"resume": spec.session_id, "tab": spec.tab, "cwd": cwd, "why": "resumed from Quietdeck's deck"})
}

/// A start or resume through Fleetlayer's node at `socket`. None when no Fleetlayer node answers there.
async fn fleetlayer_open(socket: &Path, params: serde_json::Value) -> Option<Result<SessionStarted, String>> {
    let answer = crate::fleetlayer::node_call(socket, "node/session_open", params, FLEETLAYER_OPENS_WITHIN).await?;
    Some(match answer {
        Ok(v) => serde_json::from_value::<FlStarted>(v)
            .map(|f| SessionStarted {
                node: f.node,
                tab: f.tab,
                address: f.address,
                session_id: f.session_id,
                prompt_error: f.prompt_error,
            })
            .map_err(|e| format!("Fleetlayer's answer to the start: {e}")),
        Err(e) => Err(e.message),
    })
}

fn program(node: &Node, agent: AgentKind) -> (&str, &[String]) {
    match agent {
        AgentKind::Claude => (&node.config.programs.claude, &node.config.sessions.claude_args),
        AgentKind::Codex => (&node.config.programs.codex, &node.config.sessions.codex_args),
    }
}

/// What a person pastes into a terminal to resume a session as this machine would: its directory, then the agent
/// with the `sessions:` arguments from node.yaml, its tab name, and the session.
pub fn resume_command(node: &Node, s: &quietdeck_types::directory::SessionInfo) -> Option<String> {
    let cwd = s.cwd.as_deref()?;
    let (program, args) = program(node, s.agent);
    let mut words = vec![quote(cwd)];
    words.insert(0, "cd".into());
    words.push("&&".into());
    words.push(quote(program));
    words.extend(args.iter().map(|a| quote(a)));
    match s.agent {
        AgentKind::Claude => {
            if let Some(tab) = &s.tab {
                words.extend(["--name".into(), quote(tab)]);
            }
            words.extend(["--resume".into(), quote(&s.session_id)]);
        }
        AgentKind::Codex => words.extend(["resume".into(), quote(&s.session_id)]),
    }
    Some(words.join(" "))
}

/// A word for a POSIX shell: bare when it's plainly safe, otherwise in single quotes.
fn quote(w: &str) -> String {
    if !w.is_empty() && w.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+".contains(c)) {
        w.to_string()
    } else {
        format!("'{}'", w.replace('\'', "'\\''"))
    }
}

fn agent_name(agent: AgentKind) -> &'static str {
    match agent {
        AgentKind::Claude => "Claude Code",
        AgentKind::Codex => "Codex",
    }
}

/// The working directory as an absolute path on this machine, which must exist.
fn directory(cwd: &str) -> Result<String, String> {
    let path = expand(cwd);
    if !path.is_dir() {
        return Err(format!("{} isn't a directory on this machine", path.display()));
    }
    Ok(path.display().to_string())
}

/// Where a new session works. Claude Code never remembers trusting the home folder (it asks every time), so a session
/// asked to start there works in its tab's kept scratchpad instead, `~/Notes/Agents/<tab>` (or `~/agents/<tab>` without
/// a notes vault), made if need be.
fn session_dir(cwd: String, tab: &str) -> Result<String, String> {
    let home = crate::config::home();
    if std::path::Path::new(&cwd) != home {
        return Ok(cwd);
    }
    let name: String =
        tab.chars().map(|c| if c.is_alphanumeric() || "-_ ".contains(c) { c } else { '-' }).collect::<String>();
    let name = name.trim();
    let name = if name.is_empty() { "session" } else { name };
    let base = if home.join("Notes").is_dir() { home.join("Notes/Agents") } else { home.join("agents") };
    let dir = base.join(name);
    std::fs::create_dir_all(&dir).map_err(|e| format!("making {}: {e}", dir.display()))?;
    Ok(dir.display().to_string())
}

/// Marks a folder trusted for Claude Code before a session starts there, so the person never meets its "trust this
/// folder?" prompt: the deck's start is already their approval, and node.yaml's flags decide what the agent may do.
/// Claude Code keeps trust per folder in `~/.claude.json` (`projects.<path>.hasTrustDialogAccepted`), and a trusted
/// folder's entry covers only that folder. Written atomically; a failure is logged, and the prompt then shows as before.
fn trust_for_claude(dir: &str) {
    let path = crate::config::home().join(".claude.json");
    if let Err(e) = trust_in(&path, dir) {
        tracing::warn!(%dir, error = %e, "marking the folder trusted for Claude Code");
    }
}

fn trust_in(path: &std::path::Path, dir: &str) -> Result<(), String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "{}".to_string(),
        Err(e) => return Err(e.to_string()),
    };
    let mut v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let projects = v
        .as_object_mut()
        .ok_or("not a JSON object")?
        .entry("projects")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or("projects isn't an object")?;
    // Only the folder's own entry counts: Claude Code 2.1 asks about a folder inside a trusted one (seen on a Mac whose
    // `/` and `~/code` were both trusted, 27 Nov 2026).
    if projects.get(dir).and_then(|p| p.get("hasTrustDialogAccepted")) == Some(&serde_json::Value::Bool(true)) {
        return Ok(());
    }
    projects
        .entry(dir.to_string())
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or("the folder's entry isn't an object")?
        .insert("hasTrustDialogAccepted".into(), serde_json::Value::Bool(true));
    let tmp = path.with_file_name(format!(".claude.json.quietdeck-{}", std::process::id()));
    let body = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        let mut f = opts.open(&tmp).map_err(|e| e.to_string())?;
        f.write_all(body.as_bytes()).and_then(|()| f.sync_all()).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Whether a program can be run: a path that exists, or a name found on PATH.
pub fn installed(program: &str) -> bool {
    if program.contains('/') {
        return std::path::Path::new(program).is_file();
    }
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(program).is_file()))
}

/// A new session's arguments for its agent, after the agent's program: node.yaml's own, then the name, model and
/// effort in the agent's flags.
fn start_args(
    agent: AgentKind,
    name: &str,
    model: Option<&str>,
    effort: Option<&str>,
    agent_args: &[String],
) -> Vec<String> {
    let mut a = agent_args.to_vec();
    match agent {
        AgentKind::Claude => {
            a.extend(["--name".into(), name.into()]);
            if let Some(m) = model {
                a.extend(["--model".into(), m.into()]);
            }
            if let Some(e) = effort {
                a.extend(["--effort".into(), e.into()]);
            }
        }
        AgentKind::Codex => {
            if let Some(m) = model {
                a.extend(["-m".into(), m.into()]);
            }
            if let Some(e) = effort {
                a.extend(["-c".into(), format!("model_reasoning_effort={e}")]);
            }
        }
    }
    a
}

/// The arguments that resume a session.
fn resume_args(agent: AgentKind, tab: &str, session_id: &str, agent_args: &[String]) -> Vec<String> {
    let mut a = agent_args.to_vec();
    match agent {
        AgentKind::Claude => a.extend(["--name".into(), tab.into(), "--resume".into(), session_id.into()]),
        AgentKind::Codex => a.extend(["resume".into(), session_id.into()]),
    }
    a
}

pub async fn start(node: &Node, spec: SessionStart) -> Result<SessionStarted, String> {
    start_via(node, &crate::fleetlayer::node_socket(), spec).await
}

async fn start_via(node: &Node, socket: &Path, spec: SessionStart) -> Result<SessionStarted, String> {
    let cwd = session_dir(directory(&spec.cwd)?, &spec.tab)?;
    if spec.agent == AgentKind::Claude {
        trust_for_claude(&cwd);
    }
    if node.sessions.lock().await.iter().any(|s| s.running && s.tab.as_deref() == Some(spec.tab.as_str())) {
        return Err(format!("a session is already running in a tab named {}", spec.tab));
    }
    if fleetlayer_can_start(&spec)
        && let Some(started) = fleetlayer_open(socket, start_params(&spec, &cwd)).await
    {
        if let Ok(s) = &started {
            tracing::info!(tab = %s.tab, address = %s.address, "started a session for the deck, through Fleetlayer");
        }
        return started;
    }
    let (prog, args) = program(node, spec.agent);
    if !installed(prog) {
        return Err(format!("{} isn't installed on this machine ({prog} isn't on PATH)", agent_name(spec.agent)));
    }
    let name = spec.name.as_deref().unwrap_or(&spec.tab);
    let tab = OpenTab {
        label: spec.tab.clone(),
        cwd,
        agent: spec.agent,
        args: start_args(spec.agent, name, spec.model.as_deref(), spec.effort.as_deref(), args),
    };
    let created = node.orchestrator.open(&tab).await?;
    tracing::info!(tab = %spec.tab, address = %created.address, "started a session for the deck");
    // The prompt goes in once the agent takes input, never on the command line: a session started with a prompt
    // goes straight to working, and the orchestrator waits for its idle screen.
    let prompt_error = match &spec.prompt {
        Some(text) => node.orchestrator.deliver(&created.address, text).await.err(),
        None => None,
    };
    Ok(SessionStarted {
        node: node.config.node.name.clone(),
        tab: spec.tab,
        address: created.address,
        session_id: created.session_id,
        prompt_error,
    })
}

pub async fn resume(node: &Node, spec: SessionResume) -> Result<SessionStarted, String> {
    resume_via(node, &crate::fleetlayer::node_socket(), spec).await
}

async fn resume_via(node: &Node, socket: &Path, spec: SessionResume) -> Result<SessionStarted, String> {
    if let Some(s) = node.sessions.lock().await.iter().find(|s| s.session_id == spec.session_id && s.running) {
        return Err(format!("that session is already running, in {}", s.tab.as_deref().unwrap_or("a tab")));
    }
    let cwd = directory(&spec.cwd)?;
    if spec.agent == AgentKind::Claude {
        trust_for_claude(&cwd);
    }
    if let Some(resumed) = fleetlayer_open(socket, resume_params(&spec, &cwd)).await {
        return resumed.map(|mut s| {
            tracing::info!(tab = %s.tab, address = %s.address, "resumed a session for the deck, through Fleetlayer");
            s.session_id = s.session_id.or(Some(spec.session_id));
            s
        });
    }
    let (prog, args) = program(node, spec.agent);
    if !installed(prog) {
        return Err(format!("{} isn't installed on this machine ({prog} isn't on PATH)", agent_name(spec.agent)));
    }
    let short: String = spec.session_id.chars().take(8).collect();
    let tab = spec.tab.clone().unwrap_or_else(|| format!("resumed {short}"));
    let open = OpenTab {
        label: tab.clone(),
        cwd,
        agent: spec.agent,
        args: resume_args(spec.agent, &tab, &spec.session_id, args),
    };
    let created = node.orchestrator.open(&open).await?;
    tracing::info!(%tab, address = %created.address, "resumed a session for the deck");
    Ok(SessionStarted {
        node: node.config.node.name.clone(),
        tab,
        address: created.address,
        session_id: created.session_id.or(Some(spec.session_id)),
        prompt_error: None,
    })
}

pub async fn close(node: &Node, session_id: &str) -> Result<(), String> {
    close_via(node, &crate::fleetlayer::node_socket(), session_id).await
}

async fn close_via(node: &Node, socket: &Path, session_id: &str) -> Result<(), String> {
    let params = serde_json::json!({"to": session_id, "why": "closed from Quietdeck's deck"});
    if let Some(closed) =
        crate::fleetlayer::node_call(socket, "node/session_close", params, FLEETLAYER_CLOSES_WITHIN).await
    {
        closed.map_err(|e| e.message)?;
        tracing::info!(%session_id, "closed a session's tab for the deck, through Fleetlayer");
        return Ok(());
    }
    node.orchestrator.close(session_id).await?;
    tracing::info!(%session_id, "closed a session's tab for the deck");
    Ok(())
}

pub async fn interrupt(node: &Node, session_id: &str) -> Result<(), String> {
    node.orchestrator.interrupt(session_id).await?;
    tracing::info!(%session_id, "interrupted a session for the deck");
    Ok(())
}

#[cfg(test)]
mod trust_tests {
    use super::*;

    #[test]
    fn a_folder_is_trusted_once_and_its_neighbours_are_kept() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join(".claude.json");
        std::fs::write(
            &p,
            r#"{"numStartups": 3, "projects": {"/srv/code": {"hasTrustDialogAccepted": true, "x": 1}}}"#,
        )
        .unwrap();
        // Inside a trusted folder: marked too, since Claude Code asks about it anyway.
        trust_in(&p, "/srv/code/app").unwrap();
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["projects"]["/srv/code/app"]["hasTrustDialogAccepted"], true);
        // Already marked: the file is left as it is.
        let before = std::fs::read_to_string(&p).unwrap();
        trust_in(&p, "/srv/code").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), before);
        // Elsewhere: marked, and the rest of the file kept.
        trust_in(&p, "/srv/other").unwrap();
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["projects"]["/srv/other"]["hasTrustDialogAccepted"], true);
        assert_eq!(v["projects"]["/srv/code"]["x"], 1);
        assert_eq!(v["numStartups"], 3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_and_resumes_take_the_agents_own_flags() {
        let claude = start_args(
            AgentKind::Claude,
            "esign cert",
            Some("opus"),
            Some("high"),
            &["--dangerously-skip-permissions".into()],
        );
        assert_eq!(claude.join(" "), "--dangerously-skip-permissions --name esign cert --model opus --effort high");
        let codex = start_args(AgentKind::Codex, "b", Some("gpt-6"), Some("xhigh"), &["--no-daemon".into()]);
        assert_eq!(codex.join(" "), "--no-daemon -m gpt-6 -c model_reasoning_effort=xhigh");
        assert_eq!(resume_args(AgentKind::Claude, "b", "s1", &[]).join(" "), "--name b --resume s1");
        assert_eq!(
            resume_args(AgentKind::Codex, "b", "s1", &["--no-daemon".into()]).join(" "),
            "--no-daemon resume s1"
        );
    }

    /// A pretend Fleetlayer node: answers each request on its socket with `answer(method, params)` and keeps what it
    /// was asked.
    fn fake_fleetlayer(
        dir: &Path,
        answer: fn(&str, &serde_json::Value) -> serde_json::Value,
    ) -> (std::path::PathBuf, std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>) {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let socket = dir.join("node.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let asked = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let kept = asked.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (r, mut w) = stream.into_split();
                let mut lines = BufReader::new(r).lines();
                if let Ok(Some(line)) = lines.next_line().await {
                    let req: serde_json::Value = serde_json::from_str(&line).unwrap();
                    kept.lock().unwrap().push(req.clone());
                    // A notification first, as a real node may send: it must be skipped.
                    let reply = answer(req["method"].as_str().unwrap(), &req["params"]);
                    let out = format!(
                        "{}\n{}\n",
                        serde_json::json!({"jsonrpc": "2.0", "method": "note", "params": {}}),
                        serde_json::json!({"jsonrpc": "2.0", "id": req["id"], "result": reply["result"], "error": reply["error"]}),
                    );
                    let _ = w.write_all(out.as_bytes()).await;
                }
            }
        });
        (socket, asked)
    }

    fn spec(model: Option<&str>, name: Option<&str>) -> SessionStart {
        serde_json::from_value(serde_json::json!({
            "on": "agents", "tab": "esign cert", "name": name, "agent": "claude", "model": model,
            "cwd": "~/code/app", "prompt": "Read the handoff."
        }))
        .unwrap()
    }

    #[test]
    fn fleetlayer_starts_what_it_can_pass_and_herdr_keeps_the_rest() {
        assert!(fleetlayer_can_start(&spec(None, None)));
        assert!(fleetlayer_can_start(&spec(None, Some("esign cert"))), "a name that is the tab");
        assert!(fleetlayer_can_start(&spec(Some("opus"), None)), "a model goes through");
        assert!(!fleetlayer_can_start(&spec(None, Some("other"))), "a name other than the tab");
    }

    #[tokio::test]
    async fn a_start_and_a_resume_go_to_fleetlayers_node_and_come_back_as_quietdecks_answer() {
        let d = tempfile::tempdir().unwrap();
        let (socket, asked) = fake_fleetlayer(d.path(), |_, p| {
            serde_json::json!({"result": {"node": "agents", "tab": p["tab"].as_str().unwrap_or("resumed s-1"),
                "address": "w1:p7", "sessionId": if p.get("resume").is_some() { serde_json::Value::Null } else { "s-new".into() },
                "promptError": null, "extra": 1}})
        });
        let started =
            fleetlayer_open(&socket, start_params(&spec(None, None), "/home/sam/code/app")).await.unwrap().unwrap();
        assert_eq!(
            started,
            SessionStarted {
                node: "agents".into(),
                tab: "esign cert".into(),
                address: "w1:p7".into(),
                session_id: Some("s-new".into()),
                prompt_error: None,
            }
        );
        let resume = SessionResume { session_id: "s-1".into(), agent: AgentKind::Codex, tab: None, cwd: "/srv".into() };
        let resumed = fleetlayer_open(&socket, resume_params(&resume, "/srv")).await.unwrap().unwrap();
        assert_eq!(resumed.tab, "resumed s-1");
        let asked = asked.lock().unwrap().clone();
        assert_eq!(asked[0]["method"], "node/session_open");
        assert_eq!(asked[0]["params"]["tab"], "esign cert");
        assert_eq!(asked[0]["params"]["cwd"], "/home/sam/code/app");
        assert_eq!(asked[0]["params"]["harness"], "claude");
        assert_eq!(asked[0]["params"]["prompt"], "Read the handoff.");
        assert!(asked[0]["params"].get("args").is_none(), "the agent's flags are Fleetlayer's node's to choose");
        assert!(asked[0]["params"]["model"].is_null(), "no model asked for, none sent");
        assert_eq!(asked[1]["params"]["resume"], "s-1");
        assert_eq!(asked[1]["params"]["cwd"], "/srv");
    }

    #[tokio::test]
    async fn fleetlayers_refusal_is_the_answer_and_no_fleetlayer_means_herdr() {
        let d = tempfile::tempdir().unwrap();
        let (socket, _) = fake_fleetlayer(
            d.path(),
            |_, _| serde_json::json!({"error": {"code": -32011, "message": "a session is already running in a tab named esign cert"}}),
        );
        let refused = fleetlayer_open(&socket, start_params(&spec(None, None), "/srv")).await.unwrap();
        assert_eq!(refused.unwrap_err(), "a session is already running in a tab named esign cert");
        let closed = crate::fleetlayer::node_call(
            &socket,
            "node/session_close",
            serde_json::json!({"to": "s-1"}),
            FLEETLAYER_CLOSES_WITHIN,
        )
        .await
        .unwrap();
        assert_eq!(closed.unwrap_err().code, Some(-32011));
        assert!(fleetlayer_open(&d.path().join("none.sock"), serde_json::json!({})).await.is_none());
    }
}
