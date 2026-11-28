//! The deck's environment (deploy/README.md) and deck.yaml.

use std::collections::BTreeMap;
use std::path::PathBuf;

use quietdeck_types::ActionType;
use quietdeck_types::api::AccountLabel;
use serde::Deserialize;

/// Settings from the environment. `serve` reads these; the other subcommands read only what they need.
#[derive(Debug, Clone, clap::Args)]
pub struct Env {
    /// The deck's one stable HTTPS origin, on a private network. Passkeys are tied to its host name.
    #[arg(long, env = "QUIETDECK_PUBLIC_URL")]
    pub public_url: String,
    /// The public origin of the hooks port.
    #[arg(long, env = "QUIETDECK_HOOKS_PUBLIC_URL")]
    pub hooks_public_url: Option<String>,
    /// Fleetlayer's control plane, for the summaries its plugin writes on machines that use it.
    #[arg(long, env = "QUIETDECK_FLEETLAYER_URL")]
    pub fleetlayer_url: Option<String>,
    /// A read-scoped Fleetlayer token (`fleetlayer-control token add quietdeck-deck --scope read`).
    #[arg(long, env = "QUIETDECK_FLEETLAYER_TOKEN", hide_env_values = true)]
    pub fleetlayer_token: Option<String>,
    /// A send-scoped Fleetlayer token (`fleetlayer-control token add quietdeck --scope send`): the deck's own words to
    /// sessions (the Inbox chat, report answers, handoffs) then go through Fleetlayer as service:quietdeck, and through
    /// the session's node only where Fleetlayer doesn't know the session.
    #[arg(long, env = "QUIETDECK_FLEETLAYER_SEND_TOKEN", hide_env_values = true)]
    pub fleetlayer_send_token: Option<String>,
    /// A delegate-scoped Fleetlayer token (`fleetlayer-control token add quietdeck-delegate --scope delegate`): the
    /// person's answers on the Work tab are typed into the session they're about as the person's own words (`POST
    /// /v1/sessions/say` with the signed-in person's ID), so a session acts on them as his. Keep it with the deck only.
    #[arg(long, env = "QUIETDECK_FLEETLAYER_DELEGATE_TOKEN", hide_env_values = true)]
    pub fleetlayer_delegate_token: Option<String>,
    /// An approve-scoped Fleetlayer token (`fleetlayer-control token add quietdeck-approve --scope approve --owner
    /// <person>`): Fleetlayer's access requests and admin windows show on the Work tab, decided there as that person.
    /// Keep it with the deck only, where no agent can read it: whoever reads it can approve any request but a window.
    #[arg(long, env = "QUIETDECK_FLEETLAYER_APPROVE_TOKEN", hide_env_values = true)]
    pub fleetlayer_approve_token: Option<String>,
    /// Fleetlayer's sign-in service (`https://signin.example.com`), whose `/stepup` page confirms opening an admin
    /// window with the person's passkey. The deck's address must be in its `stepup_return_origins`.
    #[arg(long, env = "QUIETDECK_FLEETLAYER_SIGNIN_URL")]
    pub fleetlayer_signin_url: Option<String>,
    /// The deck's client ID at Fleetlayer's sign-in service, for signing the owner in there (OpenID Connect). `quietdeck`
    /// when unset. Sign-in through Fleetlayer is on when QUIETDECK_FLEETLAYER_SIGNIN_URL, the secret file and the owner
    /// are all set, and the deck's own passkeys are off then.
    #[arg(long, env = "QUIETDECK_FLEETLAYER_SIGNIN_CLIENT")]
    pub fleetlayer_signin_client: Option<String>,
    /// The file holding the deck's client secret at the sign-in service (mode 600).
    #[arg(long, env = "QUIETDECK_FLEETLAYER_SIGNIN_SECRET_FILE")]
    pub fleetlayer_signin_secret_file: Option<std::path::PathBuf>,
    /// Who may sign in to the deck through Fleetlayer: the person's handle (`nathan`) or ID (`p_…`) at the sign-in
    /// service. Anyone else the sign-in service lets through is refused here: a deck has one owner.
    #[arg(long, env = "QUIETDECK_FLEETLAYER_SIGNIN_OWNER")]
    pub fleetlayer_signin_owner: Option<String>,
    /// Matrix rooms read through Fleetlayer's Matrix plugin: their channels, comma-separated
    /// (`infrastructure-alerts,family`). The deck reads them from the control plane's event log with the read token and
    /// stores their messages as the node's own Matrix reader does, which can then be turned off.
    #[arg(long, env = "QUIETDECK_FLEETLAYER_ROOM_CHANNELS")]
    pub fleetlayer_room_channels: Option<String>,
    /// Fleetlayer's place (tenant) slug, for the passkey page. `default` when unset.
    #[arg(long, env = "QUIETDECK_FLEETLAYER_TENANT")]
    pub fleetlayer_tenant: Option<String>,
    #[arg(long, env = "QUIETDECK_VAPID_PUBLIC_KEY", hide_env_values = true)]
    pub vapid_public_key: Option<String>,
    #[arg(long, env = "QUIETDECK_VAPID_PRIVATE_KEY", hide_env_values = true)]
    pub vapid_private_key: Option<String>,
    #[arg(long, env = "QUIETDECK_VAPID_SUBJECT")]
    pub vapid_subject: Option<String>,
    #[command(flatten)]
    pub data: DataEnv,
    #[arg(long, env = "QUIETDECK_LISTEN", default_value = "0.0.0.0:8080")]
    pub listen: String,
    #[arg(long, env = "QUIETDECK_HOOKS_LISTEN", default_value = "0.0.0.0:8081")]
    pub hooks_listen: String,
    /// Development only: treat every browser request as signed in, with no passkey. Never set this on a deck
    /// that's reachable from anywhere but your own machine.
    #[arg(long, env = "QUIETDECK_DEV_TRUST_LOCAL", action = clap::ArgAction::SetTrue, value_parser = clap::builder::BoolishValueParser::new())]
    pub dev_trust_local: bool,
    /// Quietdeck solo: the deck on the person's own machine, with no passkeys. It must listen on loopback, every
    /// request goes through the solo gate (Host, Origin, and a session from a one-time login link), and there's no
    /// webhook receiver. See `solo.rs`.
    #[arg(long, env = "QUIETDECK_LOCAL_ONLY", action = clap::ArgAction::SetTrue, value_parser = clap::builder::BoolishValueParser::new())]
    pub local_only: bool,
    /// A frontend of your own: serve this directory's built files in place of the web app built into the deck, with
    /// its index.html for any other path. Served from the deck's origin, its pages sign in as the owner like the
    /// default web app's. A directory with no index.html serves no UI at all.
    #[arg(long, env = "QUIETDECK_UI_DIR")]
    pub ui_dir: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, clap::Args)]
pub struct DataEnv {
    #[arg(long, env = "QUIETDECK_DATA_DIR", default_value = "/data")]
    pub data_dir: PathBuf,
    #[arg(long, env = "QUIETDECK_CONFIG")]
    pub config: Option<PathBuf>,
}

impl DataEnv {
    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("quietdeck.db")
    }

    pub fn config_path(&self) -> PathBuf {
        self.config.clone().unwrap_or_else(|| self.data_dir.join("deck.yaml"))
    }
}

/// deck.yaml. Missing means defaults.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DeckConfig {
    pub accounts: Vec<AccountLabel>,
    pub boards: Vec<String>,
    pub tasks: Option<TasksConfig>,
    pub pings: PingsConfig,
    /// Your triage tab: it gets your comments on the deck's own timeline entries.
    pub triage: Option<FallbackHolder>,
    /// Retired: leases moved to Fleetlayer. Still read so a deck.yaml from before starts; only `fallback` is used,
    /// as `triage` when that isn't set.
    pub leases: RetiredLeasesConfig,
    pub projects: BTreeMap<String, ProjectConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TasksConfig {
    pub label: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PingsConfig {
    pub weekly_budget: u32,
}

impl Default for PingsConfig {
    fn default() -> Self {
        PingsConfig { weekly_budget: 10 }
    }
}

/// The `leases:` section from before leases moved to Fleetlayer (`fleetlayer lease`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RetiredLeasesConfig {
    pub fallback: Option<FallbackHolder>,
    /// Ignored: build slots are Fleetlayer's heavy-build leases.
    pub heavy_build_slots: BTreeMap<String, i64>,
}

/// A session by node and tab.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FallbackHolder {
    pub node: String,
    pub tab: String,
}

/// A project: a set of working directories, or of accounts for actions with no session, and the action types its
/// policy allows automatically.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectConfig {
    pub dirs: Vec<String>,
    pub accounts: Vec<String>,
    pub automatic: Vec<ActionType>,
}

impl DeckConfig {
    pub fn load(path: &std::path::Path) -> anyhow::Result<DeckConfig> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let config: DeckConfig =
                    yaml_serde::from_str(&text).map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
                config.check()?;
                Ok(config)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DeckConfig::default()),
            Err(e) => Err(anyhow::anyhow!("reading {}: {e}", path.display())),
        }
    }

    fn check(&self) -> anyhow::Result<()> {
        let mut seen = std::collections::HashSet::new();
        for a in &self.accounts {
            if !seen.insert(&a.id) {
                anyhow::bail!("deck.yaml: account {} is listed twice", a.id);
            }
        }
        for (name, p) in &self.projects {
            if p.automatic.contains(&ActionType::Reply) {
                // Allowed by the plan only once the person adds it; say so in the log rather than refusing.
                tracing::warn!(project = %name, "deck.yaml: replies are automatic in this project");
            }
        }
        if self.leases.fallback.is_some() || !self.leases.heavy_build_slots.is_empty() {
            tracing::warn!(
                "deck.yaml: `leases` is retired (leases moved to Fleetlayer); move `leases.fallback` to `triage` \
                 and drop the section"
            );
        }
        Ok(())
    }

    /// Who gets the person's comments on the deck's own entries: `triage`, or the retired `leases.fallback`.
    pub fn triage_tab(&self) -> Option<&FallbackHolder> {
        self.triage.as_ref().or(self.leases.fallback.as_ref())
    }

    /// Whether `account` is one of the configured labels, or a chat source the deck reads (`matrix`: a card about a
    /// Matrix message names it, with the message's `m_<id>` as its thread). With no accounts configured, any label is
    /// accepted.
    pub fn knows_account(&self, account: &str) -> bool {
        self.accounts.is_empty()
            || self.accounts.iter().any(|a| a.id == account)
            || quietdeck_types::message::SOURCES.contains(&account)
            || account == quietdeck_types::document::SESSIONS_ACCOUNT
    }

    /// The project a working directory belongs to, by the longest matching directory.
    pub fn project_for_dir(&self, cwd: &str, home: Option<&str>) -> Option<String> {
        let mut best: Option<(usize, &str)> = None;
        for (name, p) in &self.projects {
            for d in &p.dirs {
                let d = expand_home(d, home);
                let d = d.trim_end_matches('/');
                if (cwd == d || cwd.starts_with(&format!("{d}/"))) && best.is_none_or(|(len, _)| d.len() > len) {
                    best = Some((d.len(), name));
                }
            }
        }
        best.map(|(_, n)| n.to_string())
    }
}

fn expand_home(dir: &str, home: Option<&str>) -> String {
    match (dir.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => format!("{}/{rest}", home.trim_end_matches('/')),
        _ => dir.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_example_config_parses() {
        let text = include_str!("../../../deploy/deck.example.yaml");
        let c: DeckConfig = yaml_serde::from_str(text).unwrap();
        c.check().unwrap();
        assert!(c.knows_account("personal"));
        assert!(!c.knows_account("nope"));
        assert!(c.knows_account("matrix"), "a card about a Matrix message names the source");
    }

    #[test]
    fn projects_match_the_longest_directory() {
        let c: DeckConfig =
            yaml_serde::from_str("projects:\n  app: { dirs: [~/code/app] }\n  infra: { dirs: [~/code/app/infra] }\n")
                .unwrap();
        assert_eq!(c.project_for_dir("/home/sam/code/app/src", Some("/home/sam")).as_deref(), Some("app"));
        assert_eq!(c.project_for_dir("/home/sam/code/app/infra/x", Some("/home/sam")).as_deref(), Some("infra"));
        assert_eq!(c.project_for_dir("/home/sam/code/apples", Some("/home/sam")), None);
    }

    #[test]
    fn the_retired_leases_section_still_parses_and_names_the_triage_tab() {
        let c: DeckConfig = yaml_serde::from_str(
            "leases:\n  fallback: { node: server, tab: email triage }\n  heavy_build_slots: { server: 2 }\n",
        )
        .unwrap();
        assert_eq!(c.triage_tab().map(|t| t.tab.as_str()), Some("email triage"));
        let c: DeckConfig = yaml_serde::from_str(
            "triage: { node: server, tab: triage }\nleases:\n  fallback: { node: server, tab: email triage }\n",
        )
        .unwrap();
        assert_eq!(c.triage_tab().map(|t| t.tab.as_str()), Some("triage"), "triage wins");
    }

    #[test]
    fn unknown_keys_are_refused() {
        assert!(yaml_serde::from_str::<DeckConfig>("acounts: []").is_err());
    }
}
