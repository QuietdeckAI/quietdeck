//! quietdeck-node: the daemon on each machine that runs agent sessions. It connects out to the deck, watches the orchestrator (herdr),
//! delivers messages to sessions, writes their summaries, monitors account usage, and hands session tokens to the
//! CLI over its local socket.

mod config;
mod control;
mod deliver;
mod link;
mod local;
mod matrix;
mod orchestrator;
mod runner;
mod service;
mod summaries;
mod transcripts;
mod usage;
mod viewer;
mod watch;

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use quietdeck_types::directory::SessionInfo;
use quietdeck_types::node::{LocalRequest, LocalResponse, NodeFrame};
use quietdeck_types::summary::SessionSignal;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc};

use crate::config::NodeConfig;

#[derive(Parser)]
#[command(name = "quietdeck-node", version, about = "Quietdeck's node daemon")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the daemon (the default).
    Run,
    /// Import claude-summarizer's history into the running node, once per machine, before its first sweep.
    ImportSummaries {
        /// claude-summarizer's output directory (the one holding index.json).
        dir: PathBuf,
    },
    /// Run the node as a service: a launchd agent on macOS, a systemd user service on Linux. Run it again after
    /// moving the binary or changing PATH.
    Install {
        /// Write the service file without starting it.
        #[arg(long)]
        no_start: bool,
    },
    /// Stop the service and remove its file.
    Uninstall,
    /// Check the Matrix reader: sign in (or reuse its device), sync for a while and print the messages it would hand
    /// to the deck (they also wait in its outbox for the node). Stop the node first.
    MatrixCheck {
        /// A node.yaml with a `matrix:` section. Default: this machine's.
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long, default_value_t = 60)]
        seconds: u64,
    },
}

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
    /// Set once at start when node.yaml switches the runner on.
    pub runner: std::sync::OnceLock<runner::RunnerHandle>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,rmcp=warn".into()),
        )
        .init();
    match Cli::parse().command.unwrap_or(Command::Run) {
        Command::Run => run().await,
        Command::Install { no_start } => service::install(&config::home(), !no_start),
        Command::Uninstall => service::uninstall(&config::home()),
        Command::MatrixCheck { config: path, seconds } => {
            let c = NodeConfig::load(&path.unwrap_or_else(config::config_path))?;
            let m = c.matrix.ok_or_else(|| anyhow::anyhow!("node.yaml has no matrix: section"))?;
            matrix::check(&c.node.name, &m, seconds).await
        }
        Command::ImportSummaries { dir } => {
            let dir = std::fs::canonicalize(&dir).map_err(|e| anyhow::anyhow!("{}: {e}", dir.display()))?;
            match ask_local(&LocalRequest::ImportSummaries { dir: dir.display().to_string() }).await? {
                LocalResponse::Imported { records } => {
                    println!("Imported {records} summary records. The node's sweep runs now.");
                    Ok(())
                }
                LocalResponse::Err { error } => anyhow::bail!(error),
                other => anyhow::bail!("unexpected answer: {other:?}"),
            }
        }
    }
}

/// One request to the running node on this machine.
async fn ask_local(req: &LocalRequest) -> anyhow::Result<LocalResponse> {
    let path = config::socket_path();
    let stream = tokio::net::UnixStream::connect(&path)
        .await
        .map_err(|e| anyhow::anyhow!("the node isn't running ({}: {e})", path.display()))?;
    let (r, mut w) = stream.into_split();
    w.write_all(format!("{}\n", serde_json::to_string(req)?).as_bytes()).await?;
    let line = BufReader::new(r).lines().next_line().await?.ok_or_else(|| anyhow::anyhow!("the node hung up"))?;
    Ok(serde_json::from_str(&line)?)
}

async fn run() -> anyhow::Result<()> {
    let config = NodeConfig::load(&config::config_path())?;
    let orchestrator: Arc<dyn orchestrator::Orchestrator> = Arc::new(orchestrator::herdr::Herdr {
        program: config.programs.herdr.clone(),
        machine: config.node.name.clone(),
    });
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
    let usage = config.usage.enabled.then(|| {
        usage::spawn(
            config.usage.clone(),
            usage::UsageEnv {
                rotator_dir: rotator_dir(&home),
                home: home.clone(),
                claude_program: PathBuf::from(&config.programs.claude),
                state_dir: state.join("usage"),
            },
            usage_tx,
        )
    });

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
            Arc::new(runner::ConfiguredTools::new(executor)),
            Arc::new(runner::LocalAgent { node: node.clone(), tab: cfg.agent.tab.clone() }),
            state.clone(),
        ));
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

    tokio::spawn(watch::run(node.clone(), signal_tx));
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
