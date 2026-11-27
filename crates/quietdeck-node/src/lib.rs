//! quietdeck-node as a library: the node's modules, [`Node`] and [`run`], for the `quietdeck-node` binary and for
//! Quietdeck solo, which runs a node and the deck in one process.

pub mod config;
pub mod control;
pub mod deliver;
pub mod fleetlayer;
pub mod link;
pub mod local;
pub mod matrix;
pub mod orchestrator;
pub mod runner;
pub mod service;
pub mod summaries;
pub mod transcripts;
pub mod usage;
pub mod viewer;
pub mod watch;

use std::path::PathBuf;
use std::sync::Arc;

use quietdeck_types::directory::SessionInfo;
use quietdeck_types::node::NodeFrame;
use quietdeck_types::summary::SessionSignal;
use tokio::sync::{Mutex, mpsc};

use crate::config::NodeConfig;

/// Everything the node's tasks share.
pub struct Node {
    pub config: NodeConfig,
    pub home: PathBuf,
    pub link: link::Link,
    pub deliverer: deliver::Deliverer,
    /// The program that runs this machine's sessions (herdr's TUI).
    pub orchestrator: Arc<dyn orchestrator::Orchestrator>,
    pub sessions: Mutex<Vec<SessionInfo>>,
    pub summaries: Option<summaries::SummariesHandle>,
    pub usage: Option<usage::UsageHandle>,
    pub tokens: local::TokenCache,
    /// The runner's headless agent turns running now, for the identity of the CLI calls they make.
    pub headless_turns: Arc<local::HeadlessTurns>,
    /// Set once at start when node.yaml switches the runner on.
    pub runner: std::sync::OnceLock<runner::RunnerHandle>,
}

impl Node {
    /// A message was just put in front of this session: it's working on it. Shown at once (here and on the deck)
    /// rather than when the orchestrator next reports the session's status, which can be seconds later; that report
    /// then takes over.
    pub async fn mark_working(&self, session_id: &str) {
        let changed = {
            let mut sessions = self.sessions.lock().await;
            sessions.iter_mut().find(|s| s.session_id == session_id && s.running).and_then(|s| {
                (s.status != quietdeck_types::AgentStatus::Working).then(|| {
                    s.status = quietdeck_types::AgentStatus::Working;
                    s.status_changed_at = jiff::Timestamp::now();
                    s.clone()
                })
            })
        };
        if let Some(session) = changed {
            self.link.push(NodeFrame::Session { session }).await;
        }
    }
}

/// How [`run`] runs, beyond node.yaml.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Watch the orchestrator (herdr) for sessions. Off in solo, whose one agent is headless.
    pub watch_orchestrator: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        RunOptions { watch_orchestrator: true }
    }
}

/// Run the node until SIGTERM or Ctrl-C.
pub async fn run(config: NodeConfig, opts: RunOptions) -> anyhow::Result<()> {
    // herdr acts on sessions; where this machine runs a Fleetlayer node, the list of sessions comes from its registry.
    let orchestrator: Arc<dyn orchestrator::Orchestrator> =
        Arc::new(orchestrator::fleetlayer::Fleetlayer::new(Box::new(orchestrator::herdr::Herdr {
            program: config.programs.herdr.clone(),
            machine: config.node.name.clone(),
        })));
    // A message the orchestrator types into a session by itself is typed like a person's: it isn't one.
    for message in orchestrator.own_messages() {
        summaries::set_auto_continue_message(&message);
    }
    // Check the token now, so a bad one stops the daemon at start instead of looping on reconnects.
    config.node_token()?;
    let home = config::home();
    let state = config::state_dir();
    std::fs::create_dir_all(&state)?;

    let (signal_tx, signal_rx) = mpsc::channel::<SessionSignal>(256);
    let (summary_tx, mut summary_rx) = mpsc::channel(256);
    let summaries = config.summaries.enabled.then(|| {
        summaries::spawn(
            config.summaries.clone(),
            summaries::SummariesEnv {
                state_dir: state.join("summaries"),
                home: home.clone(),
                claude_program: PathBuf::from(&config.programs.claude),
                coordinator_socket: Some(rotator_dir(&home).join("coordinator.sock")),
                time_zone: jiff::tz::TimeZone::system().iana_name().unwrap_or("UTC").to_string(),
            },
            signal_rx,
            summary_tx,
        )
    });
    let (usage_tx, mut usage_rx) = mpsc::channel(64);
    let usage = config
        .usage
        .enabled
        .then(|| usage::spawn(config.usage.clone(), usage::UsageEnv { home: home.clone() }, usage_tx));

    let node = Arc::new(Node {
        deliverer: deliver::Deliverer::new(&home, orchestrator.clone()),
        orchestrator,
        config,
        home,
        link: link::Link::default(),
        sessions: Mutex::default(),
        summaries,
        usage,
        tokens: local::TokenCache::default(),
        headless_turns: Arc::default(),
        runner: std::sync::OnceLock::new(),
    });

    if let Some(cfg) = node.config.runner.clone().filter(|r| r.enabled) {
        let executor = match &cfg.executor {
            Some(e) => Some((e.clone(), runner::executor_token(e)?)),
            None => None,
        };
        let r = Arc::new(runner::Runner::new(
            cfg.clone(),
            Arc::new(runner::LinkDeck(node.clone())),
            Arc::new(match &cfg.gog {
                Some(g) => runner::ConfiguredTools::new(executor).with_gog(g),
                None => runner::ConfiguredTools::new(executor),
            }),
            match &cfg.agent.headless {
                Some(h) => Arc::new(runner::HeadlessAgent::spawn(
                    h.clone(),
                    cfg.agent.tab.clone(),
                    state.clone(),
                    node.headless_turns.clone(),
                )) as Arc<dyn runner::AgentInbox>,
                None => Arc::new(runner::LocalAgent { node: node.clone(), tab: cfg.agent.tab.clone() }),
            },
            state.clone(),
        ));
        // A headless agent has no tab for the orchestrator to report: the node lists it itself, so the deck can reach it.
        if let Some(session) = r.headless_session() {
            node.sessions.lock().await.push(session);
        }
        let _ = node.runner.set(runner::spawn(r));
        tracing::info!(agent = %cfg.agent.tab, "runner on");
    }

    if let Some(cfg) = node.config.matrix.clone() {
        tracing::info!(user = %cfg.user, rooms = cfg.rooms.len(), "Matrix reader on");
        matrix::spawn(node.clone(), cfg);
    }

    // Summaries and usage go to the deck as they come.
    {
        let node = node.clone();
        tokio::spawn(async move {
            while let Some(event) = summary_rx.recv().await {
                node.link.push_kept(NodeFrame::Summary { event }).await;
            }
        });
    }
    {
        let node = node.clone();
        tokio::spawn(async move {
            let mut last = None;
            while let Some(usage::UsageEvent::Snapshot(accounts)) = usage_rx.recv().await {
                if last.as_ref() == Some(&accounts) {
                    continue;
                }
                node.link.push(NodeFrame::Usage { accounts: accounts.clone(), at: jiff::Timestamp::now() }).await;
                last = Some(accounts);
            }
        });
    }

    if opts.watch_orchestrator {
        tokio::spawn(watch::run(node.clone(), signal_tx));
    } else {
        drop(signal_tx);
    }
    tokio::spawn(link::run(node.clone()));
    let socket = config::socket_path();
    tokio::select! {
        r = local::serve(node.clone(), &socket) => r?,
        _ = shutdown() => tracing::info!("stopping"),
    }
    let _ = std::fs::remove_file(&socket);
    Ok(())
}

fn rotator_dir(home: &std::path::Path) -> PathBuf {
    std::env::var_os("CLAUDE_ACCOUNT_ROTATOR_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude/account-rotator"))
}

async fn shutdown() {
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term => {} }
}
