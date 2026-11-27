//! Sessions the deck starts, resumes and closes on this machine (flow 3.15). The person's click or submitted form
//! is the approval, and the deck logs it. The node checks the request, then has the orchestrator do it, without taking focus, so
//! nothing moves on the person's screen. The agents' permission flags come from node.yaml only.

use quietdeck_types::AgentKind;
use quietdeck_types::directory::{SessionResume, SessionStart, SessionStarted};

use crate::Node;
use crate::config::expand;
use crate::orchestrator::OpenTab;

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
    let (prog, args) = program(node, spec.agent);
    if !installed(prog) {
        return Err(format!("{} isn't installed on this machine ({prog} isn't on PATH)", agent_name(spec.agent)));
    }
    let cwd = session_dir(directory(&spec.cwd)?, &spec.tab)?;
    if spec.agent == AgentKind::Claude {
        trust_for_claude(&cwd);
    }
    if node.sessions.lock().await.iter().any(|s| s.running && s.tab.as_deref() == Some(spec.tab.as_str())) {
        return Err(format!("a session is already running in a tab named {}", spec.tab));
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
    if let Some(s) = node.sessions.lock().await.iter().find(|s| s.session_id == spec.session_id && s.running) {
        return Err(format!("that session is already running, in {}", s.tab.as_deref().unwrap_or("a tab")));
    }
    let (prog, args) = program(node, spec.agent);
    if !installed(prog) {
        return Err(format!("{} isn't installed on this machine ({prog} isn't on PATH)", agent_name(spec.agent)));
    }
    let cwd = directory(&spec.cwd)?;
    if spec.agent == AgentKind::Claude {
        trust_for_claude(&cwd);
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
}
