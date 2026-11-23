-- A session's place in its orchestrator is an opaque address (a herdr pane ID, or another orchestrator's own), not a
-- herdr pane.
ALTER TABLE sessions RENAME COLUMN pane TO address;
