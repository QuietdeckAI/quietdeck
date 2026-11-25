-- Threads the person archived (or deleted) in Gmail themselves, as the runner saw them leave the inbox: a card whose
-- threads have all left is resolved, and the agent tidies the report.
CREATE TABLE left_inbox (
    account TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    at INTEGER NOT NULL,
    PRIMARY KEY (account, thread_id)
);

-- `quietdeck report done`: nothing on the report waits for the person any more, so the Inbox shows "All done." (with
-- its bar and chat) until the agent edits it or starts the next report.
ALTER TABLE reports ADD COLUMN finished_at INTEGER;
