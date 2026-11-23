//! Sessions the deck starts, resumes and closes on this machine (flow 3.15). The person's click or submitted form
//! is the approval, and the deck logs it. The node checks the request, then has herdr do it, with `--no-focus` so
//! nothing moves on the person's screen. The agents' permission flags come from node.yaml only.

use quietdeck_types::AgentKind;
use quietdeck_types::directory::{SessionResume, SessionStart, SessionStarted};

use crate::Node;
use crate::config::expand;
use crate::herdr;

fn program(node: &Node, agent: AgentKind) -> (&str, &[String]) {
    match agent {
        AgentKind::Claude => (&node.config.programs.claude, &node.config.sessions.claude_args),
        AgentKind::Codex => (&node.config.programs.codex, &node.config.sessions.codex_args),
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

pub async fn start(node: &Node, spec: SessionStart) -> Result<SessionStarted, String> {
    let (prog, args) = program(node, spec.agent);
    if !herdr::installed(prog) {
        return Err(format!("{} isn't installed on this machine ({prog} isn't on PATH)", agent_name(spec.agent)));
    }
    let cwd = directory(&spec.cwd)?;
    if node.sessions.lock().await.iter().any(|s| s.running && s.tab.as_deref() == Some(spec.tab.as_str())) {
        return Err(format!("a session is already running in a tab named {}", spec.tab));
    }
    let name = spec.name.as_deref().unwrap_or(&spec.tab);
    let argv =
        herdr::start_args(spec.agent, &spec.tab, &cwd, name, spec.model.as_deref(), spec.effort.as_deref(), args);
    let created = herdr::create_tab(&node.config.programs.herdr, &argv).await?;
    tracing::info!(tab = %spec.tab, pane = %created.pane_id, "started a session for the deck");
    // The prompt goes in once the agent takes input, never on the command line: a session started with a prompt
    // goes straight to working, and herdr waits for its idle screen.
    let prompt_error = match &spec.prompt {
        Some(text) => crate::deliver::herdr_prompt(&node.config.programs.herdr, &created.pane_id, text).await.err(),
        None => None,
    };
    Ok(SessionStarted {
        node: node.config.node.name.clone(),
        tab: spec.tab,
        pane: created.pane_id,
        session_id: created.session_id,
        prompt_error,
    })
}

pub async fn resume(node: &Node, spec: SessionResume) -> Result<SessionStarted, String> {
    if let Some(s) = node.sessions.lock().await.iter().find(|s| s.session_id == spec.session_id && s.running) {
        return Err(format!("that session is already running, in {}", s.tab.as_deref().unwrap_or("a tab")));
    }
    let (prog, args) = program(node, spec.agent);
    if !herdr::installed(prog) {
        return Err(format!("{} isn't installed on this machine ({prog} isn't on PATH)", agent_name(spec.agent)));
    }
    let cwd = directory(&spec.cwd)?;
    let short: String = spec.session_id.chars().take(8).collect();
    let tab = spec.tab.clone().unwrap_or_else(|| format!("resumed {short}"));
    let argv = herdr::resume_args(spec.agent, &tab, &cwd, &spec.session_id, args);
    let created = herdr::create_tab(&node.config.programs.herdr, &argv).await?;
    tracing::info!(%tab, pane = %created.pane_id, "resumed a session for the deck");
    Ok(SessionStarted {
        node: node.config.node.name.clone(),
        tab,
        pane: created.pane_id,
        session_id: created.session_id.or(Some(spec.session_id)),
        prompt_error: None,
    })
}

pub async fn close(node: &Node, session_id: &str) -> Result<(), String> {
    herdr::close_session(&node.config.programs.herdr, session_id).await?;
    tracing::info!(%session_id, "closed a session's tab for the deck");
    Ok(())
}
