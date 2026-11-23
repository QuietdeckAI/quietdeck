-- Quietdeck's deck. Timestamps are INTEGER Unix milliseconds. JSON columns hold serde JSON of quietdeck-types.
-- Every record has an owner (owner_type 'user' or 'organisation', owner_id). Only one owner exists for now: user 1.

CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    tailscale_login TEXT NOT NULL UNIQUE
);

CREATE TABLE organisations (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL
);

CREATE TABLE memberships (
    user_id INTEGER NOT NULL REFERENCES users(id),
    organisation_id INTEGER NOT NULL REFERENCES organisations(id),
    role TEXT NOT NULL,
    PRIMARY KEY (user_id, organisation_id)
);

-- Secrets the deck makes for itself (the session token signing key). Never service credentials.
CREATE TABLE deck_secrets (
    name TEXT PRIMARY KEY,
    value BLOB NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE settings (
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    name TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (owner_type, owner_id, name)
);

CREATE TABLE nodes (
    id INTEGER PRIMARY KEY,
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    name TEXT NOT NULL UNIQUE,
    token_hash TEXT NOT NULL UNIQUE,
    version TEXT,
    last_seen_at INTEGER,
    runner INTEGER NOT NULL DEFAULT 0,
    runner_info TEXT,
    created_at INTEGER NOT NULL,
    revoked_at INTEGER
);

CREATE TABLE enrolment_codes (
    code_hash TEXT PRIMARY KEY,
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    expires_at INTEGER NOT NULL,
    used_at INTEGER,
    used_by_node TEXT
);

-- Cards, actions and decisions.

CREATE TABLE cards (
    id INTEGER PRIMARY KEY,
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    version INTEGER NOT NULL DEFAULT 1,
    posted_node TEXT NOT NULL,
    posted_tab TEXT NOT NULL,
    posted_session_id TEXT,
    account TEXT NOT NULL,
    thread_key TEXT NOT NULL,
    section TEXT NOT NULL,
    state TEXT NOT NULL,
    handoff_reason TEXT,
    grant_id INTEGER,
    -- The CardInput as posted (summary, facts, sender, actions...).
    input TEXT NOT NULL,
    prepared_at INTEGER NOT NULL,
    rechecked_at INTEGER,
    recheck_note TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX cards_thread ON cards (owner_type, owner_id, account, thread_key);
CREATE INDEX cards_state ON cards (state, updated_at);
CREATE INDEX cards_updated ON cards (updated_at);

CREATE TABLE actions (
    id INTEGER PRIMARY KEY,
    card_id INTEGER NOT NULL REFERENCES cards(id),
    card_version INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    type TEXT NOT NULL,
    payload TEXT NOT NULL,
    status TEXT NOT NULL,
    run_by TEXT,
    outcome TEXT,
    error TEXT,
    started_at INTEGER,
    finished_at INTEGER,
    UNIQUE (card_id, card_version, seq)
);
CREATE INDEX actions_card ON actions (card_id, card_version);

CREATE TABLE decisions (
    id INTEGER PRIMARY KEY,
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    card_id INTEGER NOT NULL REFERENCES cards(id),
    card_version INTEGER NOT NULL,
    kind TEXT NOT NULL,
    fields TEXT,
    text TEXT,
    release_after INTEGER,
    released_at INTEGER,
    claimed_by TEXT,
    picked_up_at INTEGER,
    handoff_reason TEXT,
    features TEXT,
    at INTEGER NOT NULL
);
CREATE INDEX decisions_card ON decisions (card_id, card_version);
CREATE INDEX decisions_open ON decisions (released_at, claimed_by);

-- The timeline, History and comments.

CREATE TABLE timeline_entries (
    id INTEGER PRIMARY KEY,
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    account TEXT,
    kind TEXT NOT NULL,
    source TEXT NOT NULL,
    refs TEXT NOT NULL DEFAULT '[]',
    text TEXT NOT NULL,
    count INTEGER NOT NULL DEFAULT 1,
    lease_id INTEGER,
    created_at INTEGER NOT NULL,
    read_at INTEGER
);
CREATE INDEX timeline_created ON timeline_entries (created_at);
CREATE INDEX timeline_unread ON timeline_entries (read_at);

CREATE TABLE comments (
    id INTEGER PRIMARY KEY,
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    target_type TEXT NOT NULL,
    target_id INTEGER NOT NULL,
    text TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    picked_up_at INTEGER
);

CREATE TABLE thread_links (
    id INTEGER PRIMARY KEY,
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    account TEXT NOT NULL,
    thread_key TEXT NOT NULL,
    node TEXT NOT NULL,
    tab TEXT NOT NULL,
    session_id TEXT,
    via TEXT NOT NULL,
    card_id INTEGER,
    at INTEGER NOT NULL
);
CREATE INDEX thread_links_thread ON thread_links (account, thread_key);

-- Where each session last read `history --since last`.
CREATE TABLE history_cursors (
    node TEXT NOT NULL,
    session_key TEXT NOT NULL,
    read_until INTEGER NOT NULL,
    PRIMARY KEY (node, session_key)
);

CREATE TABLE rounds (
    id INTEGER PRIMARY KEY,
    node TEXT NOT NULL,
    tab TEXT NOT NULL,
    session_id TEXT,
    done_at INTEGER NOT NULL
);

-- Rules and policies.

CREATE TABLE rules (
    id INTEGER PRIMARY KEY,
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    mode TEXT NOT NULL,
    instruction TEXT,
    criteria TEXT NOT NULL DEFAULT '{}',
    action_template TEXT NOT NULL,
    session_ref TEXT,
    project TEXT,
    topic TEXT,
    known_sender_domains TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL,
    archived_reason TEXT,
    match_count INTEGER NOT NULL DEFAULT 0,
    last_matched_at INTEGER,
    created_from_card_id INTEGER,
    builtin TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE auto_session_starts (
    id INTEGER PRIMARY KEY,
    card_id INTEGER NOT NULL,
    at INTEGER NOT NULL
);

-- Leases and locks.

CREATE TABLE lease_patterns (
    id INTEGER PRIMARY KEY,
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    kind TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    confirmed_by_card_id INTEGER,
    confirmed_at INTEGER,
    revoked_at INTEGER,
    UNIQUE (owner_type, owner_id, kind, fingerprint)
);

CREATE TABLE leases (
    id INTEGER PRIMARY KEY,
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    kind TEXT NOT NULL,
    resource TEXT NOT NULL,
    params TEXT NOT NULL DEFAULT '{}',
    holder_node TEXT NOT NULL,
    holder_tab TEXT NOT NULL,
    holder_session_id TEXT,
    reason TEXT NOT NULL,
    state TEXT NOT NULL,
    ttl_seconds INTEGER NOT NULL,
    idle_minutes INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    activated_at INTEGER,
    last_renewed_at INTEGER,
    expires_at INTEGER,
    cleanup_requested_at INTEGER,
    cleanup_by TEXT,
    ended_at INTEGER,
    end_reason TEXT,
    pattern_id INTEGER REFERENCES lease_patterns(id),
    kind_state TEXT,
    card_id INTEGER
);
CREATE INDEX leases_resource ON leases (kind, resource, state);
CREATE INDEX leases_holder ON leases (holder_node, holder_session_id);

CREATE TABLE lease_queue (
    resource TEXT NOT NULL,
    lease_id INTEGER NOT NULL REFERENCES leases(id),
    position INTEGER NOT NULL,
    PRIMARY KEY (resource, lease_id)
);

-- The relays: counts only. Webhook payloads are never stored.

CREATE TABLE relay_stats (
    node TEXT PRIMARY KEY,
    delivered_count INTEGER NOT NULL DEFAULT 0,
    failed_count INTEGER NOT NULL DEFAULT 0,
    last_failed_at INTEGER
);

CREATE TABLE hook_endpoints (
    id TEXT PRIMARY KEY,
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    source TEXT NOT NULL,
    secret_hash TEXT NOT NULL,
    signing_secret TEXT,
    target_node TEXT NOT NULL,
    target_tab TEXT NOT NULL,
    account TEXT,
    label TEXT NOT NULL,
    fail_to_sender INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    deleted_at INTEGER
);

CREATE TABLE webhook_failures (
    id INTEGER PRIMARY KEY,
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    endpoint_id TEXT NOT NULL,
    source TEXT NOT NULL,
    monitor TEXT,
    state TEXT,
    title TEXT,
    target TEXT NOT NULL,
    reason TEXT NOT NULL,
    at INTEGER NOT NULL,
    seen_at INTEGER
);

-- Pings, watches and push.

CREATE TABLE pings (
    id INTEGER PRIMARY KEY,
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    sent_node TEXT NOT NULL,
    sent_tab TEXT NOT NULL,
    sent_session_id TEXT,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    link TEXT,
    reason TEXT NOT NULL,
    account TEXT,
    thread_key TEXT,
    sent_at INTEGER NOT NULL,
    result TEXT,
    feedback TEXT,
    feedback_at INTEGER,
    feedback_picked_up_at INTEGER
);

CREATE TABLE thread_watches (
    id INTEGER PRIMARY KEY,
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    account TEXT NOT NULL,
    thread_key TEXT NOT NULL,
    subject TEXT,
    created_at INTEGER NOT NULL,
    active INTEGER NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX thread_watches_thread ON thread_watches (owner_type, owner_id, account, thread_key);

CREATE TABLE push_subscriptions (
    endpoint TEXT PRIMARY KEY,
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    p256dh TEXT NOT NULL,
    auth TEXT NOT NULL,
    device TEXT,
    created_at INTEGER NOT NULL,
    last_ok_at INTEGER,
    needs_you INTEGER NOT NULL DEFAULT 0
);

-- Sessions directory.

CREATE TABLE sessions (
    node TEXT NOT NULL,
    session_id TEXT NOT NULL,
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    agent TEXT NOT NULL,
    workspace TEXT,
    tab TEXT,
    pane TEXT,
    cwd TEXT,
    project TEXT,
    status TEXT NOT NULL,
    status_changed_at INTEGER NOT NULL,
    resume_command TEXT,
    running INTEGER NOT NULL DEFAULT 0,
    sentence TEXT,
    paragraph TEXT,
    page TEXT,
    last_activity INTEGER,
    pushed_at INTEGER NOT NULL,
    closed_at INTEGER,
    blocked_since INTEGER,
    PRIMARY KEY (node, session_id)
);
CREATE INDEX sessions_tab ON sessions (tab);

CREATE VIRTUAL TABLE sessions_fts USING fts5 (
    node UNINDEXED,
    session_id UNINDEXED,
    tab,
    cwd,
    sentence,
    paragraph,
    page,
    tokenize = 'unicode61 remove_diacritics 2 tokenchars ''-_#/'''
);

CREATE TABLE session_updates (
    id INTEGER PRIMARY KEY,
    node TEXT NOT NULL,
    session_id TEXT NOT NULL,
    period_kind TEXT NOT NULL,
    period_key TEXT NOT NULL,
    record TEXT NOT NULL,
    activity_to INTEGER NOT NULL,
    written_at INTEGER NOT NULL,
    UNIQUE (node, session_id, period_kind, period_key)
);
CREATE INDEX session_updates_session ON session_updates (node, session_id, activity_to);

CREATE TABLE session_links (
    node TEXT NOT NULL,
    session_id TEXT NOT NULL,
    url TEXT NOT NULL,
    kind TEXT NOT NULL,
    ref TEXT,
    title TEXT,
    state TEXT,
    source TEXT NOT NULL,
    added_at INTEGER NOT NULL,
    checked_at INTEGER,
    hide_after INTEGER,
    PRIMARY KEY (node, session_id, url)
);

CREATE TABLE transcript_cache (
    node TEXT NOT NULL,
    session_id TEXT NOT NULL,
    bytes BLOB NOT NULL,
    cached_at INTEGER NOT NULL,
    PRIMARY KEY (node, session_id)
);

CREATE TABLE later (
    id INTEGER PRIMARY KEY,
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    account TEXT NOT NULL,
    thread_key TEXT NOT NULL,
    left_at INTEGER NOT NULL,
    last_message_ref TEXT,
    UNIQUE (owner_type, owner_id, account, thread_key)
);

-- Usage across machines. No tokens.

CREATE TABLE usage_accounts (
    account_key TEXT PRIMARY KEY,
    owner_type TEXT NOT NULL DEFAULT 'user',
    owner_id INTEGER NOT NULL DEFAULT 1,
    provider TEXT NOT NULL,
    email TEXT,
    org_name TEXT,
    plan TEXT,
    weight INTEGER NOT NULL DEFAULT 1,
    updated_at INTEGER NOT NULL
);

CREATE TABLE usage_readings (
    id INTEGER PRIMARY KEY,
    account_key TEXT NOT NULL,
    node TEXT NOT NULL,
    reading TEXT NOT NULL,
    read_at INTEGER NOT NULL
);
CREATE INDEX usage_readings_account ON usage_readings (account_key, node, read_at);

-- Passkeys and browser sign-ins.

CREATE TABLE passkeys (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id),
    credential_id TEXT NOT NULL UNIQUE,
    passkey TEXT NOT NULL,
    device_name TEXT,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER
);

CREATE TABLE auth_sessions (
    token_hash TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id),
    device TEXT,
    created_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    verified_at INTEGER NOT NULL
);

-- Counts only.

CREATE TABLE runner_handoff_stats (
    node TEXT NOT NULL,
    action_type TEXT NOT NULL,
    count INTEGER NOT NULL DEFAULT 0,
    last_at INTEGER NOT NULL,
    PRIMARY KEY (node, action_type)
);
