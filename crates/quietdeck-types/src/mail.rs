//! The Drafts and Sent tabs (sc-7782): every draft and every sent email on the person's accounts, from the runner's
//! view of Gmail (headers and snippets, never bodies), with a badge on each tab for what's new since they last looked.
//! Where the deck made the message itself (a card's draft or reply), the row names the card and the session.

use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::action::SessionRef;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MailKind {
    Draft,
    Sent,
}

/// One draft or sent message as the runner read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct MailItem {
    pub account: String,
    pub kind: MailKind,
    pub message_id: String,
    #[serde(default)]
    pub thread_id: Option<String>,
    /// Gmail's draft ID, for a draft (its message ID changes with every save).
    #[serde(default)]
    pub draft_id: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub snippet: Option<String>,
    /// When it was written or sent (Gmail's internal date).
    #[ts(type = "string")]
    #[schemars(with = "String")]
    pub at: Timestamp,
}

/// A row on the Drafts or Sent tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct MailRow {
    pub id: i64,
    #[serde(flatten)]
    #[ts(flatten)]
    pub item: MailItem,
    /// The card whose draft or reply this is, when the deck made it.
    pub card_id: Option<i64>,
    /// The session that posted that card.
    pub posted_by: Option<SessionRef>,
    /// New since the person last opened this tab.
    pub new: bool,
}

/// A thread that left the inbox (archived or deleted in Gmail).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct LeftThread {
    pub account: String,
    pub thread_id: String,
}

/// A card the deck resolved because its threads all left the inbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct LeftCard {
    pub id: i64,
    pub subject: String,
}

/// The counts on the Inbox page's tabs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[ts(export)]
pub struct MailBadges {
    /// Emails waiting for the person's approval: the distinct threads across the open cards.
    pub inbox: u32,
    /// How many drafts there are now.
    pub drafts: u32,
    /// Sent messages new since the person last opened the Sent tab.
    pub sent: u32,
}
