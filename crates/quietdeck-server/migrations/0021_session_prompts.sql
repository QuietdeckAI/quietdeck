-- A blocked session's prompt (the harness's own menu), as its node reads it through Fleetlayer.
ALTER TABLE sessions ADD COLUMN prompt TEXT;
