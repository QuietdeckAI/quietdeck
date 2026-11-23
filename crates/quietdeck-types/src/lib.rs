//! Every type Quietdeck's parts share: the deck's REST API and MCP endpoint, the node, the runner, the CLI and,
//! through ts-rs, the web app (`web/src/types/`, written when `cargo test` runs).

pub mod session;
pub mod summary;
pub mod usage;

pub use session::{AgentKind, AgentStatus, SessionKey};
