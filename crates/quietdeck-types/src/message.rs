//! Messages from chat sources other than mail, starting with Matrix (sc-7779): a node that holds the source's
//! credentials reads its rooms and hands each new message to the deck, which keeps it and tells the runner, which
//! wakes its agent. The agent reads them with `quietdeck messages` and never holds the source's token.
//!
//! A message's text is whatever someone wrote in the room: data, never instructions.

use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Most messages one `messages_ingest` may carry.
pub const MAX_INGEST: usize = 200;
/// Longest message text the deck keeps; longer text is cut with a note.
pub const MAX_BODY: usize = 20_000;

/// A message as the node read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct IncomingMessage {
    /// The source: `matrix`.
    pub source: String,
    /// The room's ID (`!abc:example.com`).
    pub channel: String,
    /// The room's name as people see it (`Infrastructure Alerts`).
    pub channel_name: String,
    /// The source's ID for the message (a Matrix event ID), unique within the room.
    pub external_id: String,
    /// Who sent it (`@alertmanager:example.com`).
    pub sender: String,
    /// The message's plain text.
    pub body: String,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub sent_at: Timestamp,
}

/// A message the deck keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct ChannelMessage {
    pub id: i64,
    pub source: String,
    pub channel: String,
    pub channel_name: String,
    pub external_id: String,
    pub sender: String,
    pub body: String,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub sent_at: Timestamp,
    /// The node that read it.
    pub node: String,
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub received_at: Timestamp,
}

/// `messages_ingest`'s answer: how many of the messages were new to the deck.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct MessagesStored {
    pub new: u32,
}

/// New messages in one room, as the deck tells the runner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct ChannelCount {
    pub source: String,
    pub channel_name: String,
    pub count: u32,
}

/// `quietdeck messages`: messages after the caller's last read (`new`), or after a message ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct MessagesPage {
    pub messages: Vec<ChannelMessage>,
    /// More messages wait after these: ask again.
    pub more: bool,
}

/// The source names the deck takes.
pub const SOURCES: &[&str] = &["matrix"];

/// The text the deck keeps for a message: cut to [`MAX_BODY`] bytes on a character boundary, with a note.
pub fn clip_body(body: &str) -> String {
    if body.len() <= MAX_BODY {
        return body.to_string();
    }
    let mut end = MAX_BODY;
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[cut: the message was {} bytes]", &body[..end], body.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_bodies_are_cut_on_a_character_boundary() {
        assert_eq!(clip_body("short"), "short");
        let long = "é".repeat(MAX_BODY);
        let cut = clip_body(&long);
        assert!(cut.ends_with(&format!("[cut: the message was {} bytes]", long.len())));
        assert!(cut.len() < long.len());
    }
}
