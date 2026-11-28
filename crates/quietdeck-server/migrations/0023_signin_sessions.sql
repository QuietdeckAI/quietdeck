-- Browser sessions from Fleetlayer's sign-in service (OpenID Connect), beside passkey sessions: the person's ID (`sub`),
-- the sign-in session a back-channel logout names (`sid`), and their handle, for the Owner's description.
ALTER TABLE auth_sessions ADD COLUMN signin_sub TEXT;
ALTER TABLE auth_sessions ADD COLUMN signin_sid TEXT;
ALTER TABLE auth_sessions ADD COLUMN signin_handle TEXT;
CREATE INDEX auth_sessions_signin ON auth_sessions (signin_sub, signin_sid);
