-- Webhook endpoints (build plan 3.7, as Nathan amended it on 25 November: alerts go straight to the session that
-- owns them, never into his Inbox). An agent proposes an endpoint; it receives nothing until he approves it in
-- Settings. Events are kept like chat messages (source 'webhook'), so an owner that was offline reads them later.
ALTER TABLE hook_endpoints ADD COLUMN approved_at INTEGER;
ALTER TABLE hook_endpoints ADD COLUMN proposed_by TEXT;
ALTER TABLE hook_endpoints ADD COLUMN last_event_at INTEGER;
ALTER TABLE hook_endpoints ADD COLUMN event_count INTEGER NOT NULL DEFAULT 0;
