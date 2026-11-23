-- One-time codes are for enrolling a node or for registering a passkey without being signed in (the first one, or
-- recovery). The deck has no other way to tell it's the owner.
ALTER TABLE enrolment_codes ADD COLUMN purpose TEXT NOT NULL DEFAULT 'node';
-- Which passkey a browser session signed in with.
ALTER TABLE auth_sessions ADD COLUMN passkey_id INTEGER REFERENCES passkeys(id) ON DELETE CASCADE;
