-- The Inbox document as it was at each Approve All, with the person's changes and answers and the cards it named,
-- kept for History. An undone Approve All drops its snapshot.
CREATE TABLE report_snapshots (
    id INTEGER PRIMARY KEY,
    report_id INTEGER NOT NULL REFERENCES reports (id),
    batch TEXT NOT NULL,
    taken_at INTEGER NOT NULL,
    -- The highest answer ID it includes, so the next snapshot of the same report lists only newer ones.
    last_answer_id INTEGER,
    approved INTEGER NOT NULL,
    snapshot TEXT NOT NULL
);
CREATE INDEX report_snapshots_taken ON report_snapshots (taken_at);
CREATE INDEX report_snapshots_batch ON report_snapshots (batch);
