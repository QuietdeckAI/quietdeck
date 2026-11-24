-- Moving a session to another machine: the old session writes a handoff (what the new one needs to know) and an
-- access manifest (what it needs to reach), and the session started from it continues the old one.
CREATE TABLE handoffs (
    id INTEGER PRIMARY KEY,
    from_node TEXT NOT NULL,
    from_tab TEXT NOT NULL,
    from_session_id TEXT,
    handoff TEXT NOT NULL,
    manifest TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    -- Set when a session starts from it: the session that continues the old one.
    to_node TEXT,
    to_tab TEXT,
    to_session_id TEXT,
    continued_at INTEGER
);
CREATE INDEX handoffs_from ON handoffs (from_node, from_session_id);
CREATE INDEX handoffs_to ON handoffs (to_node, to_session_id);
