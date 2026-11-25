-- The Drafts and Sent tabs (sc-7782): every draft and sent email on the person's accounts, as the runner sees them
-- in Gmail (headers and snippets only), and when the person last opened each tab.
CREATE TABLE mail_items (
    id INTEGER PRIMARY KEY,
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    account TEXT NOT NULL,
    kind TEXT NOT NULL,
    message_id TEXT NOT NULL,
    thread_id TEXT,
    draft_id TEXT,
    from_addr TEXT,
    to_addr TEXT,
    subject TEXT,
    snippet TEXT,
    at INTEGER NOT NULL,
    card_id INTEGER,
    posted_node TEXT,
    posted_tab TEXT,
    posted_session_id TEXT,
    received_at INTEGER NOT NULL,
    UNIQUE (account, kind, message_id)
);
CREATE INDEX mail_items_kind ON mail_items (kind, at);

CREATE TABLE mail_seen (
    kind TEXT PRIMARY KEY,
    seen_until INTEGER NOT NULL
);
