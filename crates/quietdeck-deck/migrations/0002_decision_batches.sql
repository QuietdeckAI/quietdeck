-- Approve All batches: one per press, released together when the undo window ends.
CREATE TABLE decision_batches (
    id TEXT PRIMARY KEY,
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    -- The timeline entries on screen, marked read when the batch is released.
    timeline TEXT NOT NULL DEFAULT '[]',
    release_after INTEGER NOT NULL,
    released_at INTEGER,
    created_at INTEGER NOT NULL
);
CREATE INDEX decision_batches_due ON decision_batches (released_at, release_after);

ALTER TABLE decisions ADD COLUMN batch TEXT REFERENCES decision_batches(id) ON DELETE CASCADE;
CREATE INDEX decisions_batch ON decisions (batch);
CREATE INDEX decisions_due ON decisions (released_at, release_after);
CREATE INDEX actions_status ON actions (status);
-- Set when a released decision did nothing: its card had a newer version, or was finished or left for later.
ALTER TABLE decisions ADD COLUMN skipped TEXT;
