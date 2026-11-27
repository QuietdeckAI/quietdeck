//! quietdeck-solo: Quietdeck on the person's own machine, as one process: the deck (bound to 127.0.0.1, SQLite, no
//! passkeys, no public URL) and one node whose runner reads mail with the person's gog CLI and runs the triage agent
//! headless (`claude -p`). No Executor, no herdr, no enrolment code.
//!
//! The node reaches the deck over a WebSocket on loopback, through the same code as on any other machine. Everything
//! lives in two places: the settings in `~/.config/quietdeck/solo.yaml`, and the deck's data, the node token and the
//! sign-in sessions in `~/.local/share/quietdeck/solo` (mode 700).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Parser, Subcommand};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Parser)]
#[command(
    name = "quietdeck-solo",
    version,
    about = "Quietdeck on your own machine: the deck, the runner and one agent"
)]
struct Cli {
    /// The settings file. Default: ~/.config/quietdeck/solo.yaml.
    #[arg(long, global = true, env = "QUIETDECK_SOLO_CONFIG")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the deck, the runner and the agent (the default). This is what the service runs.
    Run,
    /// Open the Inbox in your browser, with a one-time sign-in link (valid for 5 minutes).
    Open {
        /// Print the link instead of opening a browser.
        #[arg(long)]
        print: bool,
    },
    /// How the runner and the agent are doing. Exits 1 when something is stale, for a health checker.
    Status,
}

/// `solo.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoloConfig {
    /// The deck's port on 127.0.0.1.
    #[serde(default = "default_port")]
    pub port: u16,
    /// The mailboxes, by an ID of your choosing (`personal`), each with its address and how the Inbox names it.
    pub accounts: BTreeMap<String, Account>,
    #[serde(default)]
    pub agent: AgentSettings,
    /// Where gog is, when it isn't on PATH: `{ bin: /opt/homebrew/bin/gog }`.
    #[serde(default)]
    pub gog: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Account {
    pub address: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSettings {
    /// The agent's name in messages and on its cards.
    #[serde(default = "default_agent")]
    pub name: String,
    /// `runner.agent.headless` as node.yaml takes it: bin, model, instructions_file, allowed_tools, turn_timeout.
    #[serde(default = "empty_object")]
    pub headless: Value,
}

impl Default for AgentSettings {
    fn default() -> Self {
        AgentSettings { name: default_agent(), headless: empty_object() }
    }
}

fn default_port() -> u16 {
    7350
}

fn default_agent() -> String {
    "email triage".into()
}

fn empty_object() -> Value {
    json!({})
}

/// The node's name on its own deck.
const NODE: &str = "solo";

fn config_path(cli: &Cli) -> PathBuf {
    cli.config.clone().unwrap_or_else(|| quietdeck_node::config::home().join(".config/quietdeck/solo.yaml"))
}

fn data_dir() -> PathBuf {
    std::env::var_os("QUIETDECK_SOLO_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| quietdeck_node::config::home().join(".local/share/quietdeck/solo"))
}

fn load(path: &Path) -> anyhow::Result<SoloConfig> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let c: SoloConfig = yaml_serde::from_str(&text).with_context(|| format!("reading {}", path.display()))?;
    anyhow::ensure!(!c.accounts.is_empty(), "{}: name at least one mail account under accounts", path.display());
    Ok(c)
}

fn private_dir(dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn write_private(path: &Path, body: &str) -> anyhow::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn deck_url(c: &SoloConfig) -> String {
    format!("http://127.0.0.1:{}", c.port)
}

/// The deck's own idea of its address. Passkeys need a host name rather than an IP address, and the gate takes both.
fn public_url(c: &SoloConfig) -> String {
    format!("http://localhost:{}", c.port)
}

/// deck.yaml: the accounts, so the Inbox names them. (JSON is YAML.)
pub fn deck_config(c: &SoloConfig) -> Value {
    let accounts: Vec<Value> =
        c.accounts.iter().map(|(id, a)| json!({"id": id, "label": label(id, a), "address": a.address})).collect();
    json!({"accounts": accounts})
}

fn label(id: &str, a: &Account) -> String {
    a.label.clone().unwrap_or_else(|| {
        let mut ch = id.chars();
        ch.next().map(|f| f.to_uppercase().collect::<String>() + ch.as_str()).unwrap_or_default()
    })
}

/// The node's settings: the runner on, gog for mail, the agent headless, and nothing else (no summaries, no usage,
/// no Matrix).
pub fn node_config(c: &SoloConfig, token_file: &Path) -> anyhow::Result<quietdeck_node::config::NodeConfig> {
    let accounts: serde_json::Map<String, Value> =
        c.accounts.iter().map(|(id, a)| (id.clone(), json!({"address": a.address, "label": label(id, a)}))).collect();
    let mut runner = json!({
        "enabled": true,
        "agent": {"tab": c.agent.name, "headless": c.agent.headless},
        "mail": "gog",
        "accounts": accounts,
        // What solo carries out: the Inbox's mail actions. Forwards, sessions, stories and leases need other
        // machines or services.
        "eligible": ["archive", "mark_read", "label", "draft"],
    });
    if let Some(g) = &c.gog {
        runner["gog"] = g.clone();
    }
    let v = json!({
        "node": {"name": NODE, "deck": deck_url(c), "token_file": token_file.display().to_string()},
        "summaries": {"enabled": false},
        "usage": {"enabled": false},
        "runner": runner,
    });
    let mut node: quietdeck_node::config::NodeConfig =
        serde_json::from_value(v).context("making the node's settings from solo.yaml")?;
    if let Some(r) = &mut node.runner {
        r.apply_presets();
        r.check().map_err(|e| anyhow::anyhow!("solo.yaml: {e}"))?;
    }
    Ok(node)
}

/// The node's token on its own deck: the saved one while the deck still knows it, otherwise a new enrolment, made
/// straight in the database (there's no one to type a code).
async fn node_token(db: &quietdeck_server::db::Db, path: &Path) -> anyhow::Result<()> {
    if let Ok(t) = std::fs::read_to_string(path)
        && quietdeck_server::auth::node_for_token(db, t.trim()).await?.as_deref() == Some(NODE)
    {
        return Ok(());
    }
    let code = quietdeck_server::store::nodes::create_enrolment_code(db).await?;
    let enrolled = quietdeck_server::store::nodes::enrol(db, &code.code, NODE).await?;
    write_private(path, &enrolled.token)?;
    tracing::info!("this machine's node is enrolled on its own deck");
    Ok(())
}

async fn run(c: SoloConfig) -> anyhow::Result<()> {
    let data = data_dir();
    private_dir(&data)?;
    let deck_yaml = data.join("deck.yaml");
    std::fs::write(&deck_yaml, serde_json::to_string_pretty(&deck_config(&c))?)?;
    let token_file = data.join("node-token");
    {
        let db = quietdeck_server::db::open(&data.join("quietdeck.db")).await?;
        node_token(&db, &token_file).await?;
        db.close().await;
    }
    let node = node_config(&c, &token_file)?;
    let env = quietdeck_server::config::Env {
        public_url: public_url(&c),
        hooks_public_url: None,
        fleetlayer_url: None,
        fleetlayer_token: None,
        vapid_public_key: None,
        vapid_private_key: None,
        vapid_subject: None,
        data: quietdeck_server::config::DataEnv { data_dir: data.clone(), config: Some(deck_yaml) },
        listen: format!("127.0.0.1:{}", c.port),
        hooks_listen: String::new(),
        dev_trust_local: false,
        local_only: true,
    };
    tracing::info!(url = %deck_url(&c), "Quietdeck solo: open the Inbox with `quietdeck-solo open`");
    let opts = quietdeck_node::RunOptions { watch_orchestrator: false };
    tokio::try_join!(quietdeck_server::serve(env), quietdeck_node::run(node, opts))?;
    Ok(())
}

fn open(c: &SoloConfig, print: bool) -> anyhow::Result<()> {
    let code = quietdeck_server::solo::mint_login_code(&data_dir().join("solo"))?;
    let url = format!("{}/solo/login?code={code}", deck_url(c));
    if !print {
        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        if std::process::Command::new(opener).arg(&url).status().is_ok_and(|s| s.success()) {
            println!("Opened the Inbox in your browser.");
            return Ok(());
        }
    }
    println!("{url}");
    eprintln!("A one-time link, valid for 5 minutes: open it in the browser on this machine.");
    Ok(())
}

async fn status(c: &SoloConfig) -> anyhow::Result<bool> {
    let url = format!("{}/api/solo/health", deck_url(c));
    let res = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?
        .get(&url)
        .send()
        .await
        .with_context(|| format!("Quietdeck solo isn't answering at {}", deck_url(c)))?;
    let body: Value = res.json().await?;
    let ok = body.get("ok").and_then(Value::as_bool).unwrap_or(false);
    if ok {
        println!("ok: the runner and the agent are keeping up");
    }
    for p in body.get("problems").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        println!("problem: {p}");
    }
    Ok(ok)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,sqlx=warn,rmcp=warn".into()),
        )
        .init();
    let cli = Cli::parse();
    let c = load(&config_path(&cli))?;
    match cli.command.unwrap_or(Command::Run) {
        Command::Run => run(c).await,
        Command::Open { print } => open(&c, print),
        Command::Status => {
            if !status(&c).await? {
                std::process::exit(1);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solo(yaml: &str) -> SoloConfig {
        yaml_serde::from_str(yaml).unwrap()
    }

    #[test]
    fn solo_yaml_makes_a_runner_with_gog_and_a_headless_agent_and_nothing_else() {
        let c = solo("accounts:\n  personal: { address: me@example.com }\n");
        let n = node_config(&c, Path::new("/tmp/t")).unwrap();
        assert_eq!(n.node.deck, "http://127.0.0.1:7350");
        assert!(!n.summaries.enabled && !n.usage.enabled);
        let r = n.runner.unwrap();
        assert!(r.enabled);
        assert_eq!(r.agent.tab, "email triage");
        assert!(r.agent.headless.is_some(), "no tab: the agent runs headless");
        assert!(r.executor.is_none(), "no Executor");
        assert!(r.mail_watch.is_some() && r.tools.contains_key("archive"), "gog fills the mail tools");
        assert_eq!(r.account_label("personal"), "Personal");
        assert_eq!(deck_config(&c)["accounts"][0]["label"], "Personal");
    }

    #[test]
    fn the_agents_settings_pass_through_and_are_checked() {
        let c = solo(
            "port: 7400\naccounts:\n  work: { address: w@example.com, label: Acme }\n\
             agent:\n  name: triage\n  headless: { model: sonnet, allowed_tools: [Bash] }\n",
        );
        let e = node_config(&c, Path::new("/tmp/t")).unwrap_err().to_string();
        assert!(e.contains("run anything"), "a bare Bash is refused: {e}");
        let c = solo("accounts:\n  work: { address: w@example.com }\nagent:\n  headless: { model: sonnet }\n");
        let r = node_config(&c, Path::new("/tmp/t")).unwrap().runner.unwrap();
        assert_eq!(r.agent.headless.unwrap().model.as_deref(), Some("sonnet"));
    }

    #[test]
    fn a_solo_yaml_without_accounts_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("solo.yaml");
        std::fs::write(&p, "accounts: {}\n").unwrap();
        assert!(load(&p).unwrap_err().to_string().contains("at least one mail account"));
    }

    /// The whole thing on loopback: the deck comes up, the node enrols on it and connects, and the gate refuses a
    /// browser that hasn't signed in while a login link signs it in.
    #[tokio::test]
    async fn the_deck_and_the_node_run_together_on_loopback() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().to_path_buf();
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let c = solo(&format!("port: {port}\naccounts:\n  personal: {{ address: me@example.com }}\n"));
        let token_file = data.join("node-token");
        let db_path = data.join("quietdeck.db");
        {
            let db = quietdeck_server::db::open(&db_path).await.unwrap();
            node_token(&db, &token_file).await.unwrap();
            let first = std::fs::read_to_string(&token_file).unwrap();
            node_token(&db, &token_file).await.unwrap();
            assert_eq!(std::fs::read_to_string(&token_file).unwrap(), first, "the saved token is kept");
            db.close().await;
        }
        let env = quietdeck_server::config::Env {
            public_url: public_url(&c),
            hooks_public_url: None,
            fleetlayer_url: None,
            fleetlayer_token: None,
            vapid_public_key: None,
            vapid_private_key: None,
            vapid_subject: None,
            data: quietdeck_server::config::DataEnv { data_dir: data.clone(), config: None },
            listen: format!("127.0.0.1:{port}"),
            hooks_listen: String::new(),
            dev_trust_local: false,
            local_only: true,
        };
        let deck = tokio::spawn(quietdeck_server::serve(env));
        let base = deck_url(&c);
        let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
        let mut up = false;
        for _ in 0..100 {
            if http.get(format!("{base}/healthz")).send().await.is_ok_and(|r| r.status().is_success()) {
                up = true;
                break;
            }
            if deck.is_finished() {
                panic!("the deck stopped: {:?}", deck.await);
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(up, "the deck came up");
        assert_eq!(http.get(format!("{base}/")).send().await.unwrap().status(), 401, "not signed in");
        let code = quietdeck_server::solo::mint_login_code(&data.join("solo")).unwrap();
        let res = http.get(format!("{base}/solo/login?code={code}")).send().await.unwrap();
        assert_eq!(res.status(), 303);
        let cookie = res.headers()["set-cookie"].to_str().unwrap().split(';').next().unwrap().to_string();
        let me = http.get(format!("{base}/api/auth/state")).header("cookie", &cookie).send().await.unwrap();
        assert!(me.status().is_success(), "{}", me.status());
        let health = http.get(format!("{base}/api/solo/health")).send().await.unwrap();
        assert!(health.status() == 200 || health.status() == 503);

        // The node's token opens the node WebSocket, as on any machine.
        let token = std::fs::read_to_string(&token_file).unwrap();
        let db = quietdeck_server::db::open(&db_path).await.unwrap();
        assert_eq!(quietdeck_server::auth::node_for_token(&db, token.trim()).await.unwrap().as_deref(), Some(NODE));
    }
}
