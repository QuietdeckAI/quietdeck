-- How a session ended, from herdr's close events (JSON: {"how": "tab_closed"} or {"how": "agent_exited", ...}).
-- NULL while it runs, and for a session that went away without a close event (herdr stopped).
ALTER TABLE sessions ADD COLUMN ended TEXT;
