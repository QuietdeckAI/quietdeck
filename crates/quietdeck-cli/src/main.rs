//! `quietdeck`: the CLI agents (and people) use to reach the deck. It asks the local node for a short-lived session
//! token, then calls the deck's REST API with it. `--json` prints the deck's answer as it came.

mod client;
mod print;

use std::io::Read;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use quietdeck_types::api::{EnrolRequest, Enrolled};
use quietdeck_types::card::{ActionRecord, ActionReport, ActionResult, CardInput, CardResolution, Released};
use quietdeck_types::decision::ReportReceipt;
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
    /// One session: resume it.
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
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
    /// The person's released decisions on your cards: take them to carry them out.
    Decisions {
        #[command(subcommand)]
        command: DecisionsCommand,
    },
    /// Auto-approve rules: list the ones the person granted, or check a card against one.
    Rules {
        #[command(subcommand)]
        command: RulesCommand,
    },
    /// The triage round: say it's finished, so the runner may wake you again.
    Round {
        #[command(subcommand)]
        command: RoundCommand,
    },
    /// One approved action: start it before you run it, then mark it done or failed.
    Action {
        #[command(subcommand)]
        command: ActionCommand,
    },
    /// Report the results of several actions in one call.
    Actions {
        #[command(subcommand)]
        command: ActionsCommand,
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
    /// Enrol this machine with a one-time code from the deck's Settings (or `quietdeck-server enrolment-code`).
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
        /// Try it as an automatic action under `--grant`: the deck checks the rules and, if they allow it, carries it
        /// out without asking. Otherwise it's stored as a normal card.
        #[arg(long, requires = "grant")]
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
enum DecisionsCommand {
    /// Claim every released decision on your cards, with the actions exactly as approved.
    Take,
}

#[derive(Subcommand)]
enum RulesCommand {
    /// Every rule (active, archived and off) and each project's autonomy policy.
    List,
    /// Would this card be carried out without the person under this rule? Posts nothing; `card add --auto
    /// --grant` runs the same check for real.
    Match {
        /// The card as JSON (the CardInput shape), or - for stdin.
        file: String,
        /// The rule (`g_3` or `3`).
        #[arg(long)]
        grant: String,
    },
}

#[derive(Subcommand)]
enum RoundCommand {
    /// The round is finished: the runner may send the next wake-up.
    Done,
}

#[derive(Subcommand)]
enum ActionCommand {
    /// Start an approved action. Refused once the person has pressed Stop: then don't run it.
    Start { id: String },
    /// Mark a started action done.
    Done {
        id: String,
        /// What was sent and what came back, as JSON (or - for stdin).
        #[arg(long)]
        outcome: Option<String>,
    },
    /// Mark a started action failed, saying why in words the person can act on.
    Fail { id: String, error: String },
}

#[derive(Subcommand)]
enum ActionsCommand {
    /// Report many results at once from a JSON array of
    /// `{"action_id": 1, "result": "done", "outcome": {...}}` or `{"action_id": 2, "result": "failed", "error": "..."}`.
    Report {
        #[arg(long)]
        file: String,
    },
}

#[derive(Subcommand)]
enum SessionsCommand {
    /// Search every session's summaries.
    Search { words: Vec<String> },
}

#[derive(Subcommand)]
enum SessionCommand {
    /// Resume a session that isn't running, in a new tab on its machine, in its own working directory.
    Resume {
        /// tab@machine, a tab name (the session that ended last in it), or a session ID.
        session: String,
    },
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
        Command::Session { command: SessionCommand::Resume { session } } => {
            let c = Client::connect().await?;
            let r: quietdeck_types::directory::SessionStarted =
                c.post("/api/sessions/resume", &quietdeck_types::directory::SessionTarget { session }).await?;
            print::out(json, &r, |r| format!("Resumed {} on {} (pane {}).", r.tab, r.node, r.pane))
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
        Command::Decisions { command: DecisionsCommand::Take } => {
            let c = Client::connect().await?;
            let r: Vec<Released> = c.post("/api/decisions/take", &serde_json::json!({})).await?;
            print::out(json, &r, |r| print::released(r))
        }
        Command::Rules { command: RulesCommand::List } => {
            let c = Client::connect().await?;
            let r: quietdeck_types::rules::RulesView = c.get("/api/rules", &[]).await?;
            print::out(json, &r, print::rules)
        }
        Command::Rules { command: RulesCommand::Match { file, grant } } => {
            let c = Client::connect().await?;
            let card: CardInput =
                serde_json::from_str(&read_file_arg(&file)?).map_err(|e| anyhow::anyhow!("the card's JSON: {e}"))?;
            let req = quietdeck_types::rules::MatchRequest { card, grant_id: parse_id(&grant)? };
            let r: quietdeck_types::rules::MatchResult = c.post("/api/rules/match", &req).await?;
            print::out(json, &r, |m| {
                format!("{} under rule {}: {}", if m.allow { "Allowed" } else { "Denied" }, m.grant_id, m.reason)
            })
        }
        Command::Round { command: RoundCommand::Done } => {
            let c = Client::connect().await?;
            let r: quietdeck_types::api::RoundDone = c.post("/api/round/done", &serde_json::json!({})).await?;
            print::out(json, &r, |r| {
                if r.runner_told {
                    "Round done: the runner will wake you for what comes next.".to_string()
                } else {
                    "Round done. No runner runs for this session, so nothing was waiting on it.".to_string()
                }
            })
        }
        Command::Action { command } => {
            let c = Client::connect().await?;
            let (id, result) = match command {
                ActionCommand::Start { id } => {
                    let a: ActionRecord =
                        c.post(&format!("/api/actions/{}/start", parse_id(&id)?), &serde_json::json!({})).await?;
                    return print::out(json, &a, |a| format!("a_{} started.", a.id));
                }
                ActionCommand::Done { id, outcome } => {
                    let outcome = outcome
                        .map(|o| {
                            serde_json::from_str(&read_arg(&o)?).map_err(|e| anyhow::anyhow!("the outcome's JSON: {e}"))
                        })
                        .transpose()?;
                    (id, ActionResult::Done { outcome })
                }
                ActionCommand::Fail { id, error } => (id, ActionResult::Failed { error: read_arg(&error)? }),
            };
            let r: Vec<ReportReceipt> =
                c.post("/api/actions/report", &[ActionReport { action_id: parse_id(&id)?, result }]).await?;
            print::out(json, &r, |r| print::receipts(r))
        }
        Command::Actions { command: ActionsCommand::Report { file } } => {
            let reports: Vec<ActionReport> =
                serde_json::from_str(&read_file_arg(&file)?).map_err(|e| anyhow::anyhow!("the report's JSON: {e}"))?;
            let c = Client::connect().await?;
            let r: Vec<ReportReceipt> = c.post("/api/actions/report", &reports).await?;
            print::out(json, &r, |r| print::receipts(r))
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
