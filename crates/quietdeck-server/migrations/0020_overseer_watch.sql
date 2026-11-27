-- The overseer watch (overseer.rs): which session changes the writer of a Work document was told about, and the
-- watch's own health for /healthz/overseer.
CREATE TABLE overseer_told (
    node TEXT NOT NULL,
    session_id TEXT NOT NULL,
    -- The status change it was told about.
    status TEXT NOT NULL,
    status_changed_at INTEGER NOT NULL,
    told_at INTEGER NOT NULL,
    PRIMARY KEY (node, session_id)
);
CREATE TABLE overseer_watch (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    ticked_at INTEGER NOT NULL,
    told_at INTEGER,
    refresh_at INTEGER,
    error TEXT,
    error_at INTEGER
);
