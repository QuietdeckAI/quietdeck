-- When the deck asked for a lease's create step (the runner, or the holder told to do it).
ALTER TABLE leases ADD COLUMN create_requested_at INTEGER;
CREATE INDEX leases_card ON leases (card_id);
