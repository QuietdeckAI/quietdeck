//! node.yaml and the machine's paths.
//!
//! Every token is read from a file or an environment variable that node.yaml names, filled by the user's secrets
//! manager. Nothing is hard-coded, and no token is sent to the deck or logged.

use std::path::{Path, PathBuf};

use quietdeck_types::summary::SummariesConfig;
use quietdeck_types::usage::UsageConfig;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeConfig {
    pub node: NodeSection,
    #[serde(default)]
    pub summaries: SummariesConfig,
    #[serde(default)]
    pub usage: UsageConfig,
    #[serde(default)]
    pub programs: Programs,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeSection {
    /// This machine's name on the deck.
    pub name: String,
    /// The deck's URL (QUIETDECK_PUBLIC_URL), e.g. `https://quietdeck.example.com`.
    pub deck: String,
    /// Where the node token is: a file only this user can read...
    #[serde(default)]
    pub token_file: Option<String>,
    /// ...or an environment variable (for a node started under `op run` and the like).
    #[serde(default)]
    pub token_env: Option<String>,
}

/// The programs the node runs. Defaults look them up on PATH.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Programs {
    pub herdr: String,
    pub claude: String,
}

impl Default for Programs {
    fn default() -> Self {
        Programs { herdr: "herdr".into(), claude: "claude".into() }
    }
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

pub fn expand(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None => PathBuf::from(p),
    }
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("QUIETDECK_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| home().join(".config/quietdeck"))
}

pub fn config_path() -> PathBuf {
    config_dir().join("node.yaml")
}

pub fn default_token_file() -> PathBuf {
    config_dir().join("node-token")
}

pub fn state_dir() -> PathBuf {
    std::env::var_os("QUIETDECK_STATE_DIR").map(PathBuf::from).unwrap_or_else(|| home().join(".local/state/quietdeck"))
}

/// The local socket the CLI, the MCP headers helper and the menu bar app use.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("QUIETDECK_NODE_SOCKET") {
        return PathBuf::from(p);
    }
    if cfg!(target_os = "macos") {
        home().join("Library/Application Support/quietdeck/node.sock")
    } else if let Some(run) = std::env::var_os("XDG_RUNTIME_DIR") {
        PathBuf::from(run).join("quietdeck/node.sock")
    } else {
        state_dir().join("node.sock")
    }
}

impl NodeConfig {
    pub fn load(path: &Path) -> anyhow::Result<NodeConfig> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            anyhow::anyhow!("reading {}: {e} (enrol this machine with `quietdeck node enroll`)", path.display())
        })?;
        let c: NodeConfig =
            yaml_serde::from_str(&text).map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
        if c.node.token_file.is_some() && c.node.token_env.is_some() {
            anyhow::bail!("{}: set node.token_file or node.token_env, not both", path.display());
        }
        Ok(c)
    }

    /// The deck URL without a trailing slash.
    pub fn deck_url(&self) -> String {
        self.node.deck.trim_end_matches('/').to_string()
    }

    /// The node token, from wherever node.yaml says.
    pub fn node_token(&self) -> anyhow::Result<String> {
        self.node_token_from(|var| std::env::var(var).ok())
    }

    fn node_token_from(&self, env: impl Fn(&str) -> Option<String>) -> anyhow::Result<String> {
        if let Some(var) = &self.node.token_env {
            let token = env(var).map(|t| t.trim().to_string()).unwrap_or_default();
            if token.is_empty() {
                anyhow::bail!("node.token_env names {var}, which isn't set");
            }
            return Ok(token);
        }
        let path = self.node.token_file.as_deref().map(expand).unwrap_or_else(default_token_file);
        read_secret_file(&path)
    }
}

/// Read a token file, refusing one that other users can read.
pub fn read_secret_file(path: &Path) -> anyhow::Result<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(path).map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
        let mode = meta.permissions().mode();
        if mode & 0o077 != 0 {
            anyhow::bail!(
                "{} can be read by other users (mode {:o}): run `chmod 600 {}`",
                path.display(),
                mode & 0o777,
                path.display()
            );
        }
    }
    let t = std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
    let t = t.trim().to_string();
    if t.is_empty() {
        anyhow::bail!("{} is empty", path.display());
    }
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_example_parses() {
        let c: NodeConfig = yaml_serde::from_str(include_str!("../../../deploy/node.example.yaml")).unwrap();
        assert_eq!(c.node.name, "laptop");
        assert!(c.summaries.enabled);
    }

    #[test]
    fn a_token_can_come_from_the_environment() {
        let yaml = "node: { name: m, deck: \"https://d.example\", token_env: QD_TEST_TOKEN_A1 }\n";
        let c: NodeConfig = yaml_serde::from_str(yaml).unwrap();
        assert!(c.node_token_from(|_| None).unwrap_err().to_string().contains("QD_TEST_TOKEN_A1"));
        let env = |v: &str| (v == "QD_TEST_TOKEN_A1").then(|| " qdn_x\n".to_string());
        assert_eq!(c.node_token_from(env).unwrap(), "qdn_x");
        assert!(c.node_token_from(|_| Some(String::new())).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn token_files_must_be_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t");
        std::fs::write(&p, "qdn_x\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read_secret_file(&p).unwrap(), "qdn_x");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_secret_file(&p).unwrap_err().to_string().contains("chmod 600"));
    }
}
