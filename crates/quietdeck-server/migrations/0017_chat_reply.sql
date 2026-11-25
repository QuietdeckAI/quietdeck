-- The agent's answers to the person's chat in the Inbox (`quietdeck chat reply`): kept with the report's changes and
-- toasted the same way, headed by the agent's name instead of "Inbox changed". A reply names no cards, so it never
-- holds Approve All back.
ALTER TABLE report_changes ADD COLUMN kind TEXT NOT NULL DEFAULT 'change';
