-- Whether a session the overseer was told about has been seen working since: an idle session is named again only
-- then (done to idle, herdr's "finished" to "seen", isn't new work).
ALTER TABLE overseer_told ADD COLUMN worked INTEGER NOT NULL DEFAULT 0;
