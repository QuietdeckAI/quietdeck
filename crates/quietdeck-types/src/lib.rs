//! Every type Quietdeck's parts share: the deck's REST API and MCP endpoint, the node, the runner, the CLI and,
//! through ts-rs, the web app (`web/src/types/`, written when `cargo test` runs).

pub mod action;
pub mod api;
pub mod card;
pub mod decision;
pub mod directory;
pub mod lease;
pub mod node;
pub mod relay;
pub mod report;
pub mod rules;
pub mod session;
pub mod summary;
pub mod timeline;
pub mod transcript;
pub mod usage;

pub use action::{Action, ActionType, Compose, MessageRef, SessionRef};
pub use session::{AgentKind, AgentStatus, SessionKey};

/// The environment variables orchestrators set in a session's shell to say where it runs, checked in this order. The
/// CLI sends the first one it finds as the caller's session address. An adapter for another orchestrator adds its
/// variable here (`docs/orchestrators.md`).
pub const SESSION_ADDRESS_ENV: &[&str] = &["QUIETDECK_SESSION_ADDRESS", "HERDR_PANE_ID"];
