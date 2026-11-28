-- When each top-level list item of a document's report last changed, keyed by a hash of its text: an item first seen
-- (new, or reworded) gets the time it was written. The Work tab shows it as "(1 hour ago)".
CREATE TABLE report_items (
    document_id INTEGER NOT NULL,
    hash TEXT NOT NULL,
    changed_at INTEGER NOT NULL,
    PRIMARY KEY (document_id, hash)
);
