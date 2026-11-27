//! Blocked prompts: while a session on this machine is blocked (the orchestrator says so), the node asks Fleetlayer,
//! through Quietdeck's plugin, for the prompt it's waiting on (Claude Code's question form, a permission prompt, a plan
//! to approve) and tells the deck, which shows it on the Work tab as a form. When the session moves on, the deck is told
//! the prompt is gone. Answers come back as [`quietdeck_types::node::DeckRequest::AnswerPrompt`] (link.rs).
//!
//! Only where Fleetlayer runs Quietdeck's plugin (with the `sessions.query` grant) and the deck takes prompts.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use quietdeck_types::AgentStatus;
use quietdeck_types::directory::BlockedPrompt;
use quietdeck_types::node::{FEATURE_PROMPTS, NodeFrame};
use serde_json::json;

use crate::Node;

const EVERY: Duration = Duration::from_secs(4);

pub async fn run(node: Arc<Node>) {
    // What the deck was last told, by session ID.
    let mut told: HashMap<String, BlockedPrompt> = HashMap::new();
    loop {
        tokio::time::sleep(EVERY).await;
        if !node.link.deck_supports(FEATURE_PROMPTS) {
            continue;
        }
        let blocked: Vec<String> = node
            .sessions
            .lock()
            .await
            .iter()
            .filter(|s| s.running && s.status == AgentStatus::Blocked)
            .map(|s| s.session_id.clone())
            .collect();
        if blocked.is_empty() && told.is_empty() {
            continue;
        }
        let now = if blocked.is_empty() {
            HashMap::new()
        } else {
            match crate::fleetlayer::call(crate::fleetlayer::SESSIONS_QUERY, json!({})).await {
                None => continue, // no plugin here
                Some(Err(e)) => {
                    tracing::debug!(error = %e, "reading blocked prompts from Fleetlayer");
                    continue;
                }
                Some(Ok(answer)) => {
                    let mut p = crate::fleetlayer::prompts_of(&answer);
                    p.retain(|id, _| blocked.contains(id));
                    p
                }
            }
        };
        for (id, prompt) in &now {
            if told.get(id) != Some(prompt) {
                node.link.push(NodeFrame::Prompt { session_id: id.clone(), prompt: Some(prompt.clone()) }).await;
            }
        }
        for id in told.keys().filter(|id| !now.contains_key(*id)) {
            node.link.push(NodeFrame::Prompt { session_id: id.clone(), prompt: None }).await;
        }
        told = now;
    }
}
