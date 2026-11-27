//! quietdeck-node: the daemon on each machine that runs agent sessions. It connects out to the deck, watches the orchestrator (herdr),
//! delivers messages to sessions, writes their summaries, monitors Codex usage, and hands session tokens to the
//! CLI over its local socket.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use quietdeck_node::config::{self, NodeConfig};
use quietdeck_node::{RunOptions, matrix, service};
use quietdeck_types::node::{LocalRequest, LocalResponse};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            // matrix-sdk logs every sync at info and every missing key backup at warn.
            "info,rmcp=warn,matrix_sdk=warn,matrix_sdk_base=warn,matrix_sdk_crypto=warn,\
                 matrix_sdk_crypto::backups=error"
                .into()
        }))
        .init();
    match Cli::parse().command.unwrap_or(Command::Run) {
        Command::Run => quietdeck_node::run(NodeConfig::load(&config::config_path())?, RunOptions::default()).await,
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
