-- Documents: the living pages agents write and the person reads, each under a top-level tab. The Inbox document is
-- id 1, and every report before documents belongs to it.
CREATE TABLE documents (
    id INTEGER PRIMARY KEY,
    slug TEXT NOT NULL,
    name TEXT NOT NULL,
    tab TEXT NOT NULL,
    kind TEXT NOT NULL,
    writer_node TEXT,
    writer_tab TEXT,
    position INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    removed_at INTEGER
);
CREATE UNIQUE INDEX documents_slug ON documents (slug) WHERE removed_at IS NULL;
CREATE UNIQUE INDEX documents_writer ON documents (writer_node, writer_tab)
    WHERE removed_at IS NULL AND writer_node IS NOT NULL;
INSERT INTO documents (id, slug, name, tab, kind, position, created_at)
    VALUES (1, 'inbox', 'Today', 'Inbox', 'inbox', 0, CAST(unixepoch('subsec') * 1000 AS INTEGER));

ALTER TABLE reports ADD COLUMN document_id INTEGER NOT NULL DEFAULT 1;
CREATE INDEX reports_document_open ON reports (document_id, closed_at, id);

-- The person's marks on sessions: ignored (the overseer leaves it out) and parked (put aside, until a time or for
-- good).
CREATE TABLE session_marks (
    node TEXT NOT NULL,
    tab TEXT NOT NULL,
    mark TEXT NOT NULL,
    until INTEGER,
    note TEXT,
    at INTEGER NOT NULL,
    PRIMARY KEY (node, tab, mark)
);
