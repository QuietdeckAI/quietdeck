//! `quietdeck`: the CLI agents (and people) use to reach the deck. It asks the local node for a short-lived session
//! token, then calls the deck's REST API with it. `--json` prints the deck's answer as it came.

mod client;
mod print;

use std::io::Read;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use quietdeck_types::api::{EnrolRequest, Enrolled};
use quietdeck_types::card::{CardInput, CardResolution};
use quietdeck_types::relay::MsgSend;
use quietdeck_types::timeline::{TimelineAdd, TimelineKind};

use crate::client::Client;

#[derive(Parser)]
#[command(name = "quietdeck", version, about = "Quietdeck: cards, sessions, the message relay and the timeline")]
struct Cli {
    /// Print the deck's JSON answer instead of a summary.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// This machine's node: enrolment.
    Node {
        #[command(subcommand)]
        command: NodeCommand,
    },
    /// Credentials for other tools.
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Who you are to the deck, and its accounts, boards and settings.
    Whoami,
    /// Cards: post one, report a recheck, list them.
    Card {
        #[command(subcommand)]
        command: CardCommand,
    },
    /// List cards, newest first.
    Cards {
        /// Only cards still on the Inbox.
        #[arg(long)]
        open: bool,
        #[arg(long)]
        account: Option<String>,
    },
    /// Every session on every machine. `sessions search <words>` searches their summaries.
    Sessions(SessionsArgs),
    /// Who has this ID? Summaries on the deck, raw transcripts on every connected machine.
    WhoHas {
        /// The ID or words to look for.
        query: Vec<String>,
    },
    /// Messages to sessions on other machines.
    Msg {
        #[command(subcommand)]
        command: MsgCommand,
    },
    /// The automatic-actions timeline.
    Timeline {
        #[command(subcommand)]
        command: TimelineCommand,
    },
    /// What happened since you last asked: timeline entries and cards that changed.
    History {
        /// `last`, or an RFC 3339 time.
        #[arg(long, default_value = "last")]
        since: String,
    },
}

#[derive(Subcommand)]
enum NodeCommand {
    /// Enrol this machine with a one-time code from the deck's Settings (or `quietdeck-deck enrolment-code`).
    Enroll {
        code: String,
        /// The deck's URL.
        #[arg(long)]
        deck: String,
        /// This machine's name on the deck. Default: the host name.
        #[arg(long)]
        name: Option<String>,
        /// Where to keep the node token (mode 600).
        #[arg(long)]
        token_file: Option<PathBuf>,
    },
    /// The node's own state.
    Status,
}

#[derive(Subcommand)]
enum AuthCommand {
    /// Print `{"Authorization": "Bearer <session token>"}` for Claude Code's MCP `headersHelper`.
    Headers,
}

#[derive(Subcommand)]
enum CardCommand {
    /// Post a card from JSON (a file, or - for stdin). One card per thread: a thread's open card gets a new version.
    Add {
        /// The card as JSON (the CardInput shape).
        file: String,
        /// You want to carry it out yourself, whatever the runner could do.
        #[arg(long)]
        needs_agent: bool,
        /// Ask for an automatic action under this grant (`g_3` or `3`).
        #[arg(long, requires = "auto")]
        grant: Option<String>,
        #[arg(long)]
        auto: bool,
    },
    /// Report your recheck of a card.
    Resolve {
        /// The card (`c_812` or `812`).
        id: String,
        /// The thread is no longer in the inbox.
        #[arg(long, conflicts_with_all = ["handled_elsewhere", "rechecked"])]
        archived_in_gmail: bool,
        /// Someone handled it: the note says who and when ("you replied at 14:20").
        #[arg(long, value_name = "NOTE")]
        handled_elsewhere: Option<String>,
        /// It still stands, with an optional note.
        #[arg(long, value_name = "NOTE", num_args = 0..=1, default_missing_value = "")]
        rechecked: Option<String>,
    },
    /// Show one card.
    Show { id: String },
}

#[derive(Args)]
struct SessionsArgs {
    #[command(subcommand)]
    command: Option<SessionsCommand>,
    /// Only this machine's sessions.
    #[arg(long)]
    node: Option<String>,
    /// Include sessions that aren't running.
    #[arg(long)]
    all: bool,
}

#[derive(Subcommand)]
enum SessionsCommand {
    /// Search every session's summaries.
    Search { words: Vec<String> },
}

#[derive(Subcommand)]
enum MsgCommand {
    /// Send a message to `tab` or `tab@node`. Fails at once when that session is offline.
    Send {
        #[arg(long)]
        to: String,
        /// The text (or - for stdin).
        text: String,
    },
}

#[derive(Subcommand)]
enum TimelineCommand {
    /// Post an entry: an automatic action you carried out, or mail a lease absorbed.
    Add {
        #[arg(long, value_parser = parse_kind)]
        kind: TimelineKind,
        text: String,
        #[arg(long)]
        account: Option<String>,
        #[arg(long = "ref")]
        refs: Vec<String>,
        #[arg(long)]
        lease: Option<String>,
    },
}

fn parse_kind(s: &str) -> Result<TimelineKind, String> {
    serde_json::from_value(serde_json::Value::String(s.replace('-', "_"))).map_err(|_| {
        "one of: auto_action, sent_forwarded, lease_started, lease_absorbed, lease_expired, lease_cleanup_overdue, \
         filter_without_proposal, session_started, webhook_delivered, grant_archived"
            .to_string()
    })
}

/// `c_812`, `g_3`, `812` → 812.
pub fn parse_id(s: &str) -> anyhow::Result<i64> {
    let digits = s.rsplit_once('_').map(|(_, d)| d).unwrap_or(s);
    digits.parse().map_err(|_| anyhow::anyhow!("{s:?} isn't an ID"))
}

fn read_arg(s: &str) -> anyhow::Result<String> {
    if s == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        Ok(buf)
    } else {
        Ok(s.to_string())
    }
}

fn read_file_arg(s: &str) -> anyhow::Result<String> {
    if s == "-" { read_arg(s) } else { std::fs::read_to_string(s).map_err(|e| anyhow::anyhow!("{s}: {e}")) }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli).await {
        eprintln!("quietdeck: {e:#}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let json = cli.json;
    match cli.command {
        Command::Node { command: NodeCommand::Enroll { code, deck, name, token_file } } => {
            enroll(code, deck, name, token_file).await
        }
        Command::Node { command: NodeCommand::Status } => {
            let status = client::node_status().await?;
            print::out(json, &status, |s| {
                format!("{} → {} ({})", s.name, s.deck_url, if s.connected { "connected" } else { "not connected" })
            })
        }
        Command::Auth { command: AuthCommand::Headers } => {
            let t = client::session_token().await?;
            println!("{}", serde_json::json!({ "Authorization": format!("Bearer {}", t.token) }));
            Ok(())
        }
        Command::Whoami => {
            let c = Client::connect().await?;
            let info: quietdeck_types::api::DeckInfo = c.get("/api/info", &[]).await?;
            print::out(json, &info, print::info)
        }
        Command::Card { command } => card(json, command).await,
        Command::Cards { open, account } => {
            let c = Client::connect().await?;
            let mut q = vec![("open", open.to_string())];
            if let Some(a) = account {
                q.push(("account", a));
            }
            let cards: Vec<quietdeck_types::card::Card> = c.get("/api/cards", &q).await?;
            print::out(json, &cards, |cs| cs.iter().map(print::card_line).collect::<Vec<_>>().join("\n"))
        }
        Command::Sessions(args) => {
            let c = Client::connect().await?;
            let rows: Vec<quietdeck_types::directory::SessionRow> = match args.command {
                Some(SessionsCommand::Search { words }) => {
                    c.get("/api/sessions/search", &[("q", words.join(" "))]).await?
                }
                None => {
                    let mut q = vec![("all", args.all.to_string())];
                    if let Some(n) = args.node {
                        q.push(("node", n));
                    }
                    c.get("/api/sessions", &q).await?
                }
            };
            print::out(json, &rows, |rs| rs.iter().map(print::session_line).collect::<Vec<_>>().join("\n"))
        }
        Command::WhoHas { query } => {
            let c = Client::connect().await?;
            let r: quietdeck_types::directory::WhoHas = c.get("/api/who-has", &[("q", query.join(" "))]).await?;
            print::out(json, &r, print::who_has)
        }
        Command::Msg { command: MsgCommand::Send { to, text } } => {
            let c = Client::connect().await?;
            let r: quietdeck_types::node::Delivered =
                c.post("/api/msg", &MsgSend { to: to.clone(), text: read_arg(&text)? }).await?;
            print::out(json, &r, |_| format!("Delivered to {to}."))
        }
        Command::Timeline { command: TimelineCommand::Add { kind, text, account, refs, lease } } => {
            let c = Client::connect().await?;
            let lease_id = lease.as_deref().map(parse_id).transpose()?;
            let e: quietdeck_types::timeline::TimelineEntry =
                c.post("/api/timeline", &TimelineAdd { kind, account, refs, text, lease_id }).await?;
            print::out(json, &e, |e| format!("Timeline entry {} added.", e.id))
        }
        Command::History { since } => {
            let c = Client::connect().await?;
            let h: quietdeck_types::timeline::History = c.get("/api/history", &[("since", since)]).await?;
            print::out(json, &h, print::history)
        }
    }
}

async fn card(json: bool, command: CardCommand) -> anyhow::Result<()> {
    let c = Client::connect().await?;
    match command {
        CardCommand::Add { file, needs_agent, grant, auto } => {
            let text = read_file_arg(&file)?;
            let mut input: CardInput =
                serde_json::from_str(&text).map_err(|e| anyhow::anyhow!("the card's JSON: {e}"))?;
            if needs_agent {
                input.needs_agent = true;
            }
            if auto && let Some(g) = grant {
                input.auto_grant = Some(parse_id(&g)?);
            }
            let added: quietdeck_types::card::CardAdded = c.post("/api/cards", &input).await?;
            print::out(json, &added, print::card_added)
        }
        CardCommand::Resolve { id, archived_in_gmail, handled_elsewhere, rechecked } => {
            let how = if archived_in_gmail {
                CardResolution::ArchivedInGmail
            } else if let Some(note) = handled_elsewhere {
                CardResolution::HandledElsewhere { note }
            } else if let Some(note) = rechecked {
                CardResolution::Rechecked { note: (!note.is_empty()).then_some(note) }
            } else {
                anyhow::bail!("say how: --archived-in-gmail, --handled-elsewhere <note> or --rechecked [note]");
            };
            let card: quietdeck_types::card::Card =
                c.post(&format!("/api/cards/{}/resolve", parse_id(&id)?), &how).await?;
            print::out(json, &card, print::card_line)
        }
        CardCommand::Show { id } => {
            let card: quietdeck_types::card::Card = c.get(&format!("/api/cards/{}", parse_id(&id)?), &[]).await?;
            print::out(json, &card, print::card_full)
        }
    }
}

async fn enroll(code: String, deck: String, name: Option<String>, token_file: Option<PathBuf>) -> anyhow::Result<()> {
    let deck = deck.trim_end_matches('/').to_string();
    let name = match name {
        Some(n) => n,
        None => hostname()?,
    };
    let http = reqwest::Client::new();
    let resp = http.post(format!("{deck}/api/nodes/enroll")).json(&EnrolRequest { code, name }).send().await?;
    let enrolled: Enrolled = client::read(resp).await?;
    let config_dir = client::config_dir();
    let token_path = token_file.unwrap_or_else(|| config_dir.join("node-token"));
    client::write_secret_file(&token_path, &enrolled.token)?;
    let yaml = config_dir.join("node.yaml");
    if yaml.exists() {
        println!(
            "Enrolled as {}. Token saved to {}. node.yaml already exists; check node.name and node.deck in it.",
            enrolled.node,
            token_path.display()
        );
    } else {
        let token_ref = client::tilde(&token_path);
        std::fs::write(
            &yaml,
            format!("node:\n  name: {}\n  deck: {deck}\n  token_file: {token_ref}\n", enrolled.node),
        )?;
        println!(
            "Enrolled as {}. Token saved to {}, config to {}.",
            enrolled.node,
            token_path.display(),
            yaml.display()
        );
    }
    println!("Start the node: quietdeck-node (or install its service; see the getting-started guide).");
    Ok(())
}

fn hostname() -> anyhow::Result<String> {
    let out = std::process::Command::new("hostname").arg("-s").output()?;
    let h = String::from_utf8_lossy(&out.stdout).trim().to_lowercase();
    if h.is_empty() {
        anyhow::bail!("couldn't read the host name: pass --name");
    }
    Ok(h.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '-' }).collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn ids_take_prefixes() {
        assert_eq!(super::parse_id("c_812").unwrap(), 812);
        assert_eq!(super::parse_id("812").unwrap(), 812);
        assert!(super::parse_id("card").is_err());
    }

    #[test]
    fn the_cli_parses() {
        use clap::CommandFactory;
        super::Cli::command().debug_assert();
    }
}
