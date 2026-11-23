-- A link that brings the session forward in its orchestrator, and the orchestrator's name, as the node reports them.
ALTER TABLE sessions ADD COLUMN open_url TEXT;
ALTER TABLE sessions ADD COLUMN opens_in TEXT;
