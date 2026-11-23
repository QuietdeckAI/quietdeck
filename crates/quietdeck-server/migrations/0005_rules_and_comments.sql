-- The session a comment goes to, and the session that posted a timeline entry (NULL for the deck's own entries).
ALTER TABLE comments ADD COLUMN to_node TEXT;
ALTER TABLE comments ADD COLUMN to_tab TEXT;
CREATE INDEX comments_to ON comments (to_node, to_tab, picked_up_at);
ALTER TABLE timeline_entries ADD COLUMN posted_node TEXT;
ALTER TABLE timeline_entries ADD COLUMN posted_tab TEXT;
CREATE INDEX rules_status ON rules (status);
CREATE INDEX auto_session_starts_at ON auto_session_starts (at);
