-- While the person reads the Inbox (25 November): each change to the report (for the toast and Approve All's settle
-- window), each browser tab's activity, and the "while I'm reading" rules that let items through anyway.
CREATE TABLE report_changes (
    id INTEGER PRIMARY KEY,
    report_id INTEGER NOT NULL REFERENCES reports (id),
    version INTEGER NOT NULL,
    summary TEXT NOT NULL,
    cards TEXT NOT NULL DEFAULT '[]',
    at INTEGER NOT NULL
);
CREATE INDEX report_changes_report ON report_changes (report_id, id);

CREATE TABLE presence (
    device TEXT PRIMARY KEY,
    active_until INTEGER NOT NULL
);

CREATE TABLE reading_rules (
    id INTEGER PRIMARY KEY,
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    input TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    match_count INTEGER NOT NULL DEFAULT 0,
    last_matched_at INTEGER,
    created_at INTEGER NOT NULL
);

-- What each runner holds while the person reads, for the Inbox's "waiting" line.
CREATE TABLE runner_held (
    node TEXT PRIMARY KEY,
    summary TEXT,
    count INTEGER NOT NULL,
    at INTEGER NOT NULL
);
