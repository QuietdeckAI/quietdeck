-- Handoffs made through Fleetlayer (`fleetlayer session handoff`), copied from the control plane's /v1/handoffs feed:
-- its own ID (h and 16 hex digits), so each is copied once.
ALTER TABLE handoffs ADD COLUMN fleetlayer_id TEXT;
CREATE UNIQUE INDEX handoffs_fleetlayer_id ON handoffs (fleetlayer_id) WHERE fleetlayer_id IS NOT NULL;
