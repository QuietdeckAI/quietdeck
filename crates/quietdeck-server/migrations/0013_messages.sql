-- Messages from chat sources other than mail (Matrix first, sc-7779): a node reads its rooms and hands each new
-- message to the deck, which keeps it once by room and ID.
CREATE TABLE messages (
    id INTEGER PRIMARY KEY,
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    source TEXT NOT NULL,
    channel TEXT NOT NULL,
    channel_name TEXT NOT NULL,
    external_id TEXT NOT NULL,
    sender TEXT NOT NULL,
    body TEXT NOT NULL,
    sent_at INTEGER NOT NULL,
    node TEXT NOT NULL,
    received_at INTEGER NOT NULL,
    UNIQUE (source, channel, external_id)
);

-- The last message each session read with `quietdeck messages --new`, per source.
CREATE TABLE message_cursors (
    node TEXT NOT NULL,
    session_key TEXT NOT NULL,
    source TEXT NOT NULL,
    read_until INTEGER NOT NULL,
    PRIMARY KEY (node, session_key, source)
);
