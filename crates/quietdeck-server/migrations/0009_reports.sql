-- The triage report: one living Markdown document per round, and the person's answers on it.
CREATE TABLE reports (
    id INTEGER PRIMARY KEY,
    version INTEGER NOT NULL DEFAULT 1,
    markdown TEXT NOT NULL,
    posted_node TEXT NOT NULL,
    posted_tab TEXT NOT NULL,
    posted_session_id TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    closed_at INTEGER
);
CREATE INDEX reports_open ON reports (closed_at, id);

CREATE TABLE report_answers (
    id INTEGER PRIMARY KEY,
    report_id INTEGER NOT NULL REFERENCES reports (id),
    report_version INTEGER NOT NULL,
    control TEXT NOT NULL,
    kind TEXT NOT NULL,
    about TEXT NOT NULL,
    value TEXT NOT NULL,
    answered_at INTEGER NOT NULL,
    picked_up_at INTEGER
);
CREATE INDEX report_answers_report ON report_answers (report_id, id);
