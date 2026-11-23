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
    pub leases: LeasesConfig,
    pub projects: BTreeMap<String, ProjectConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TasksConfig {
    #[expect(dead_code, reason = "the Inbox shows it (build step 2)")]
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

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LeasesConfig {
    /// Who cleans up a lease when there's no runner and the holder doesn't answer.
    pub fallback: Option<FallbackHolder>,
}

/// Also where comments on the deck's own timeline entries go.
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
        Ok(())
    }

    /// Whether `account` is one of the configured labels. With no accounts configured, any label is accepted.
    pub fn knows_account(&self, account: &str) -> bool {
        self.accounts.is_empty() || self.accounts.iter().any(|a| a.id == account)
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
    fn unknown_keys_are_refused() {
        assert!(yaml_serde::from_str::<DeckConfig>("acounts: []").is_err());
    }
}
