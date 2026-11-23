//! Every type Quietdeck's parts share: the deck's REST API and MCP endpoint, the node, the runner, the CLI and,
//! through ts-rs, the web app (`web/src/types/`, written when `cargo test` runs).

pub mod action;
pub mod api;
pub mod card;
pub mod directory;
pub mod lease;
pub mod node;
pub mod relay;
pub mod rules;
pub mod session;
pub mod summary;
pub mod timeline;
pub mod usage;

pub use action::{Action, ActionType, Compose, MessageRef, SessionRef};
pub use session::{AgentKind, AgentStatus, SessionKey};
