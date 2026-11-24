//! The sessions directory: rows pushed by nodes, summaries, links, search.

use jiff::Timestamp;
use quietdeck_types::AgentStatus;
use quietdeck_types::directory::{
    HitSource, LinkKind, LinkState, SessionEnd, SessionInfo, SessionLink, SessionRow, WhoHasHit,
};
use quietdeck_types::summary::{SummaryEvent, SummaryPeriod, SummaryRecord};

use crate::config::DeckConfig;
use crate::db::{Db, enum_str, from_json, json, now_ms, parse_enum, ts, ts_opt};

#[derive(sqlx::FromRow)]
struct Row {
    node: String,
    session_id: String,
    agent: String,
    workspace: Option<String>,
    tab: Option<String>,
    address: Option<String>,
    cwd: Option<String>,
    project: Option<String>,
    status: String,
    status_changed_at: i64,
    resume_command: Option<String>,
    running: i64,
    sentence: Option<String>,
    paragraph: Option<String>,
    page: Option<String>,
    last_activity: Option<i64>,
    pushed_at: i64,
    closed_at: Option<i64>,
    ended: Option<String>,
    open_url: Option<String>,
    opens_in: Option<String>,
}

const COLUMNS: &str = "node, session_id, agent, workspace, tab, address, cwd, project, status, status_changed_at, \
    resume_command, running, sentence, paragraph, page, last_activity, pushed_at, closed_at, ended, open_url, opens_in";

impl Row {
    fn into_row(self, node_online: bool, links: Vec<SessionLink>) -> anyhow::Result<SessionRow> {
        let resume = self.resume_command.clone().or_else(|| {
            let agent = parse_enum(&self.agent).ok()?;
            Some(resume_command(agent, self.cwd.as_deref()?, self.tab.as_deref(), &self.session_id))
        });
        Ok(SessionRow {
            node: self.node,
            info: SessionInfo {
                session_id: self.session_id,
                agent: parse_enum(&self.agent)?,
                workspace: self.workspace,
                tab: self.tab,
                address: self.address,
                cwd: self.cwd,
                status: parse_enum(&self.status)?,
                status_changed_at: ts(self.status_changed_at),
                resume_command: resume,
                running: self.running != 0,
                open_url: self.open_url,
                opens_in: self.opens_in,
            },
            project: self.project,
            sentence: self.sentence,
            paragraph: self.paragraph,
            page: self.page,
            last_activity: ts_opt(self.last_activity),
            pushed_at: ts(self.pushed_at),
            node_online,
            links,
            closed_at: ts_opt(self.closed_at),
            ended: self.ended.as_deref().map(from_json).transpose()?,
        })
    }
}

fn home_of(cwd: Option<&str>) -> Option<String> {
    // Projects in deck.yaml use ~; nodes push absolute paths. /Users/<name> and /home/<name> are the homes we know.
    let cwd = cwd?;
    let mut parts = cwd.splitn(4, '/');
    let (_, root, user) = (parts.next()?, parts.next()?, parts.next()?);
    matches!(root, "Users" | "home").then(|| format!("/{root}/{user}"))
}

/// The project a working directory belongs to, for a path from any node.
pub fn project_for_cwd(config: &DeckConfig, cwd: &str) -> Option<String> {
    config.project_for_dir(cwd, home_of(Some(cwd)).as_deref())
}

/// The project of a session the deck knows.
pub async fn project_of_session(db: &Db, node: &str, session_id: &str) -> anyhow::Result<Option<String>> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT project FROM sessions WHERE node = ? AND session_id = ?")
            .bind(node)
            .bind(session_id)
            .fetch_optional(db)
            .await?;
    Ok(row.and_then(|r| r.0))
}

/// The running session in a tab, if one is.
pub async fn running_in_tab(db: &Db, node: &str, tab: &str) -> anyhow::Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT session_id FROM sessions WHERE node = ? AND tab = ? AND running = 1 ORDER BY pushed_at DESC LIMIT 1",
    )
    .bind(node)
    .bind(tab)
    .fetch_optional(db)
    .await?;
    Ok(row.map(|r| r.0))
}

/// The project of the session in a tab: the running one, else the latest.
pub async fn project_of_tab(db: &Db, node: &str, tab: &str) -> anyhow::Result<Option<String>> {
    let row: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT project FROM sessions WHERE node = ? AND tab = ? ORDER BY running DESC, pushed_at DESC LIMIT 1",
    )
    .bind(node)
    .bind(tab)
    .fetch_optional(db)
    .await?;
    Ok(row.and_then(|r| r.0))
}

/// One session pushed by its node.
pub async fn upsert(db: &Db, config: &DeckConfig, node: &str, s: &SessionInfo) -> anyhow::Result<()> {
    let project = s.cwd.as_deref().and_then(|cwd| config.project_for_dir(cwd, home_of(Some(cwd)).as_deref()));
    let now = now_ms();
    let blocked_since = (s.status == AgentStatus::Blocked).then(|| s.status_changed_at.as_millisecond());
    let mut tx = db.begin().await?;
    sqlx::query(
        "INSERT INTO sessions (node, session_id, agent, workspace, tab, address, cwd, project, status, status_changed_at, \
           resume_command, running, pushed_at, closed_at, blocked_since, open_url, opens_in) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?, ?) \
         ON CONFLICT (node, session_id) DO UPDATE SET agent = excluded.agent, workspace = excluded.workspace, \
           tab = excluded.tab, address = excluded.address, cwd = excluded.cwd, project = excluded.project, \
           status = excluded.status, status_changed_at = excluded.status_changed_at, \
           resume_command = excluded.resume_command, running = excluded.running, pushed_at = excluded.pushed_at, \
           open_url = excluded.open_url, opens_in = excluded.opens_in, \
           closed_at = CASE WHEN excluded.running = 1 THEN NULL ELSE sessions.closed_at END, \
           ended = CASE WHEN excluded.running = 1 THEN NULL ELSE sessions.ended END, \
           blocked_since = CASE WHEN excluded.status = 'blocked' \
             THEN COALESCE(sessions.blocked_since, excluded.blocked_since) ELSE NULL END",
    )
    .bind(node)
    .bind(&s.session_id)
    .bind(enum_str(&s.agent))
    .bind(&s.workspace)
    .bind(&s.tab)
    .bind(&s.address)
    .bind(&s.cwd)
    .bind(&project)
    .bind(enum_str(&s.status))
    .bind(s.status_changed_at.as_millisecond())
    .bind(&s.resume_command)
    .bind(s.running as i64)
    .bind(now)
    .bind(blocked_since)
    .bind(&s.open_url)
    .bind(&s.opens_in)
    .execute(&mut *tx)
    .await?;
    refresh_fts(&mut tx, node, &s.session_id).await?;
    tx.commit().await?;
    Ok(())
}

/// A full snapshot: upsert each, and mark the node's other sessions as not running.
pub async fn replace_snapshot(
    db: &Db,
    config: &DeckConfig,
    node: &str,
    sessions: &[SessionInfo],
) -> anyhow::Result<()> {
    for s in sessions {
        upsert(db, config, node, s).await?;
    }
    let ids: Vec<&str> = sessions.iter().map(|s| s.session_id.as_str()).collect();
    let placeholders = vec!["?"; ids.len()].join(",");
    let sql = if ids.is_empty() {
        "UPDATE sessions SET running = 0 WHERE node = ? AND running = 1".to_string()
    } else {
        format!("UPDATE sessions SET running = 0 WHERE node = ? AND running = 1 AND session_id NOT IN ({placeholders})")
    };
    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(node);
    for id in ids {
        q = q.bind(id);
    }
    q.execute(db).await?;
    Ok(())
}

pub async fn closed(
    db: &Db,
    node: &str,
    session_id: &str,
    at: Timestamp,
    ended: Option<&SessionEnd>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE sessions SET running = 0, closed_at = ?, ended = ?, status = 'unknown', blocked_since = NULL \
         WHERE node = ? AND session_id = ?",
    )
    .bind(at.as_millisecond())
    .bind(ended.map(json))
    .bind(node)
    .bind(session_id)
    .execute(db)
    .await?;
    Ok(())
}

async fn refresh_fts(tx: &mut sqlx::SqliteConnection, node: &str, session_id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM sessions_fts WHERE node = ? AND session_id = ?")
        .bind(node)
        .bind(session_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO sessions_fts (node, session_id, tab, cwd, sentence, paragraph, page) \
         SELECT node, session_id, tab, cwd, sentence, paragraph, page FROM sessions WHERE node = ? AND session_id = ?",
    )
    .bind(node)
    .bind(session_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

fn period_parts(p: &SummaryPeriod) -> (&'static str, &str) {
    match p {
        SummaryPeriod::Hour(k) => ("hour", k),
        SummaryPeriod::Day(k) => ("day", k),
        SummaryPeriod::Week(k) => ("week", k),
    }
}

/// A summary event from a node.
pub async fn apply_summary(db: &Db, node: &str, event: &SummaryEvent) -> anyhow::Result<()> {
    match event {
        SummaryEvent::Summary(r) => store_record(db, node, r).await,
        SummaryEvent::Activity { session_id, last_activity } => {
            sqlx::query(
                "UPDATE sessions SET last_activity = MAX(COALESCE(last_activity, 0), ?) WHERE node = ? AND session_id = ?",
            )
            .bind(last_activity.as_millisecond())
            .bind(node)
            .bind(session_id)
            .execute(db)
            .await?;
            Ok(())
        }
        // Catch-ups are served on request; a pushed one has nowhere to go yet.
        SummaryEvent::Catchup(_) => Ok(()),
    }
}

async fn store_record(db: &Db, node: &str, r: &SummaryRecord) -> anyhow::Result<()> {
    let (kind, key) = period_parts(&r.period);
    let mut tx = db.begin().await?;
    sqlx::query(
        "INSERT INTO session_updates (node, session_id, period_kind, period_key, record, activity_to, written_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT (node, session_id, period_kind, period_key) DO UPDATE SET record = excluded.record, \
           activity_to = excluded.activity_to, written_at = excluded.written_at",
    )
    .bind(node)
    .bind(&r.session_id)
    .bind(kind)
    .bind(key)
    .bind(json(r))
    .bind(r.activity_to.as_millisecond())
    .bind(r.written_at.as_millisecond())
    .execute(&mut *tx)
    .await?;
    // The session's row may not exist yet (an imported or swept session that isn't running): make one.
    sqlx::query(
        "INSERT INTO sessions (node, session_id, agent, tab, cwd, status, status_changed_at, running, pushed_at) \
         VALUES (?, ?, ?, ?, ?, 'unknown', ?, 0, ?) ON CONFLICT (node, session_id) DO NOTHING",
    )
    .bind(node)
    .bind(&r.session_id)
    .bind(enum_str(&r.agent))
    .bind(&r.tab)
    .bind(&r.dir)
    .bind(r.activity_to.as_millisecond())
    .bind(now_ms())
    .execute(&mut *tx)
    .await?;
    // The newest record's tiers are the session's latest.
    sqlx::query(
        "UPDATE sessions SET sentence = ?, paragraph = ?, page = ?, \
           last_activity = MAX(COALESCE(last_activity, 0), ?) \
         WHERE node = ? AND session_id = ? AND NOT EXISTS ( \
           SELECT 1 FROM session_updates u WHERE u.node = ? AND u.session_id = ? AND u.activity_to > ?)",
    )
    .bind(&r.sentence)
    .bind(&r.paragraph)
    .bind(&r.page)
    .bind(r.activity_to.as_millisecond())
    .bind(node)
    .bind(&r.session_id)
    .bind(node)
    .bind(&r.session_id)
    .bind(r.activity_to.as_millisecond())
    .execute(&mut *tx)
    .await?;
    refresh_fts(&mut tx, node, &r.session_id).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn links_for(db: &Db, node: &str, session_id: &str) -> anyhow::Result<Vec<SessionLink>> {
    type LinkRow = (String, String, Option<String>, Option<String>, Option<String>, String, i64);
    let rows: Vec<LinkRow> = sqlx::query_as(
        "SELECT url, kind, ref, title, state, source, added_at FROM session_links \
         WHERE node = ? AND session_id = ? AND (hide_after IS NULL OR hide_after > ?) ORDER BY added_at",
    )
    .bind(node)
    .bind(session_id)
    .bind(now_ms())
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|(url, kind, r, title, state, source, added_at)| {
            Ok(SessionLink {
                url,
                kind: parse_enum::<LinkKind>(&kind)?,
                r#ref: r,
                title,
                state: state.as_deref().map(parse_enum::<LinkState>).transpose()?,
                source,
                added_at: ts(added_at),
            })
        })
        .collect()
}

async fn hydrate(db: &Db, hub: &crate::hub::Hub, rows: Vec<Row>) -> anyhow::Result<Vec<SessionRow>> {
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let online = hub.is_online(&r.node).await;
        let links = links_for(db, &r.node, &r.session_id).await?;
        out.push(r.into_row(online, links)?);
    }
    Ok(out)
}

/// For a session whose node sent no resume command (an imported one, or an older node): its directory, then the
/// agent's own resume, without the machine's `sessions:` arguments, which only the node knows.
pub fn resume_command(agent: quietdeck_types::AgentKind, cwd: &str, tab: Option<&str>, session_id: &str) -> String {
    let q = |w: &str| format!("'{}'", w.replace('\'', "'\\''"));
    let dir = q(cwd);
    match agent {
        quietdeck_types::AgentKind::Claude => match tab {
            Some(t) => format!("cd {dir} && claude --name {} --resume {session_id}", q(t)),
            None => format!("cd {dir} && claude --resume {session_id}"),
        },
        quietdeck_types::AgentKind::Codex => format!("cd {dir} && codex resume {session_id}"),
    }
}

#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct ListFilter {
    pub node: Option<String>,
    /// Include sessions that aren't running (closed or ended), most recent first.
    #[serde(default)]
    pub all: bool,
    /// Only sessions active since: a date (`2026-05-23`) or a number of days back (`180d`). Implies `all`.
    pub since: Option<String>,
    /// At most this many (default 200, or 5,000 with `since`).
    pub limit: Option<i64>,
}

/// `2026-05-23` (local midnight) or `180d`, as Unix milliseconds.
pub fn parse_since(s: &str) -> Result<i64, String> {
    let s = s.trim();
    if let Some(days) = s.strip_suffix('d') {
        let days: i64 = days.parse().map_err(|_| format!("{s:?} isn't a number of days like 180d"))?;
        return Ok(crate::db::now_ms() - days.clamp(0, 36_500) * 86_400_000);
    }
    let date: jiff::civil::Date =
        s.parse().map_err(|_| format!("{s:?} isn't a date like 2026-05-23 or a number of days like 180d"))?;
    let zoned = date.to_zoned_datetime(jiff::tz::TimeZone::system()).map_err(|e| e.to_string())?;
    Ok(zoned.timestamp().as_millisecond())
}

pub async fn list(db: &Db, hub: &crate::hub::Hub, f: &ListFilter) -> anyhow::Result<Vec<SessionRow>> {
    let mut sql = format!("SELECT {COLUMNS} FROM sessions WHERE 1 = 1");
    if f.node.is_some() {
        sql.push_str(" AND node = ?");
    }
    let since = f.since.as_deref().map(parse_since).transpose().map_err(anyhow::Error::msg)?;
    if since.is_some() {
        sql.push_str(" AND COALESCE(last_activity, status_changed_at) >= ?");
    } else if !f.all {
        sql.push_str(" AND running = 1");
    }
    sql.push_str(" ORDER BY COALESCE(last_activity, status_changed_at) DESC LIMIT ?");
    let mut q = sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(sql));
    if let Some(n) = &f.node {
        q = q.bind(n);
    }
    if let Some(ms) = since {
        q = q.bind(ms);
    }
    let limit = f.limit.unwrap_or(if since.is_some() { 5000 } else { 200 }).clamp(1, 5000);
    let rows = q.bind(limit).fetch_all(db).await?;
    hydrate(db, hub, rows).await
}

pub async fn get(db: &Db, hub: &crate::hub::Hub, node: &str, session_id: &str) -> anyhow::Result<Option<SessionRow>> {
    let rows = sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM sessions WHERE node = ? AND session_id = ?"
    )))
    .bind(node)
    .bind(session_id)
    .fetch_all(db)
    .await?;
    Ok(hydrate(db, hub, rows).await?.pop())
}

/// FTS5 query from free text: each word quoted, all of them required.
pub fn fts_query(text: &str) -> Option<String> {
    let terms: Vec<String> = text
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| c == '"'))
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{}\"", w.replace('"', "\"\"")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

pub async fn search(db: &Db, hub: &crate::hub::Hub, text: &str, limit: i64) -> anyhow::Result<Vec<SessionRow>> {
    let Some(q) = fts_query(text) else { return Ok(vec![]) };
    let prefixed: Vec<String> = COLUMNS.split(", ").map(|c| format!("s.{c}")).collect();
    let rows = sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(format!(
        "SELECT {} FROM sessions_fts f JOIN sessions s ON s.node = f.node AND s.session_id = f.session_id \
         WHERE sessions_fts MATCH ? ORDER BY bm25(sessions_fts), s.last_activity DESC LIMIT ?",
        prefixed.join(", ")
    )))
    .bind(q)
    .bind(limit.clamp(1, 200))
    .fetch_all(db)
    .await?;
    hydrate(db, hub, rows).await
}

/// "Who has this ID?" over the summaries: every session whose summaries mention it, with the newest mention.
pub async fn summary_hits(db: &Db, text: &str) -> anyhow::Result<Vec<WhoHasHit>> {
    let Some(q) = fts_query(text) else { return Ok(vec![]) };
    let rows: Vec<(String, String, Option<String>, Option<i64>)> = sqlx::query_as(
        "SELECT s.node, s.session_id, s.tab, s.last_activity FROM sessions_fts f \
         JOIN sessions s ON s.node = f.node AND s.session_id = f.session_id WHERE sessions_fts MATCH ? LIMIT 100",
    )
    .bind(&q)
    .fetch_all(db)
    .await?;
    let mut hits: Vec<WhoHasHit> = rows
        .into_iter()
        .map(|(node, session_id, tab, last)| WhoHasHit {
            node,
            session_id,
            tab,
            source: HitSource::Summary,
            last_mentioned_at: ts_opt(last),
            moved_to: None,
        })
        .collect();
    // Older timeline points mention IDs the latest tiers dropped.
    let needle = text.trim();
    let older: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT node, session_id, MAX(activity_to) FROM session_updates WHERE instr(record, ?) > 0 \
         GROUP BY node, session_id LIMIT 100",
    )
    .bind(needle)
    .fetch_all(db)
    .await?;
    for (node, session_id, at) in older {
        if let Some(h) = hits.iter_mut().find(|h| h.node == node && h.session_id == session_id) {
            if h.last_mentioned_at.is_none_or(|t| t.as_millisecond() < at) {
                h.last_mentioned_at = Some(ts(at));
            }
        } else {
            let tab: Option<(Option<String>,)> =
                sqlx::query_as("SELECT tab FROM sessions WHERE node = ? AND session_id = ?")
                    .bind(&node)
                    .bind(&session_id)
                    .fetch_optional(db)
                    .await?;
            hits.push(WhoHasHit {
                node,
                session_id,
                tab: tab.and_then(|t| t.0),
                source: HitSource::Summary,
                last_mentioned_at: Some(ts(at)),
                moved_to: None,
            });
        }
    }
    Ok(hits)
}

/// Every summary point a session has, oldest first: its timeline of one-line updates.
pub async fn timeline(db: &Db, node: &str, session_id: &str) -> anyhow::Result<Vec<SummaryRecord>> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT record FROM session_updates WHERE node = ? AND session_id = ? ORDER BY activity_to")
            .bind(node)
            .bind(session_id)
            .fetch_all(db)
            .await?;
    rows.into_iter().map(|(r,)| Ok(serde_json::from_str(&r)?)).collect()
}

/// The one session an address names, running or not: a session ID, `tab@node`, or a tab (running first, then the
/// one that ended last).
pub async fn find_any(db: &Db, hub: &crate::hub::Hub, address: &str) -> anyhow::Result<Vec<SessionRow>> {
    let running = resolve(db, hub, address, true).await?;
    if !running.is_empty() {
        return Ok(running);
    }
    resolve(db, hub, address, false).await
}

/// The running sessions an address (`tab` or `tab@node`) names.
pub async fn resolve_address(db: &Db, hub: &crate::hub::Hub, address: &str) -> anyhow::Result<Vec<SessionRow>> {
    resolve(db, hub, address, true).await
}

/// Sessions that aren't running, by the same addresses, most recently ended first: what Resume looks for.
pub async fn resolve_ended(db: &Db, hub: &crate::hub::Hub, address: &str) -> anyhow::Result<Vec<SessionRow>> {
    resolve(db, hub, address, false).await
}

async fn resolve(db: &Db, hub: &crate::hub::Hub, address: &str, running: bool) -> anyhow::Result<Vec<SessionRow>> {
    let (tab, node) = match address.rsplit_once('@') {
        Some((t, n)) if !t.is_empty() && !n.is_empty() => (t.trim(), Some(n.trim())),
        _ => (address.trim(), None),
    };
    let mut sql = format!("SELECT {COLUMNS} FROM sessions WHERE running = ? AND (tab = ? OR session_id = ?)");
    if node.is_some() {
        sql.push_str(" AND node = ?");
    }
    sql.push_str(" ORDER BY COALESCE(closed_at, status_changed_at) DESC");
    let mut q = sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(sql)).bind(running as i64).bind(tab).bind(tab);
    if let Some(n) = node {
        q = q.bind(n);
    }
    let rows = q.fetch_all(db).await?;
    hydrate(db, hub, rows).await
}

/// The node knows nothing about a session's tab beyond what it pushed: a lookup for tokens.
pub async fn tab_of(db: &Db, node: &str, session_id: &str) -> anyhow::Result<Option<String>> {
    let row: Option<(Option<String>,)> = sqlx::query_as("SELECT tab FROM sessions WHERE node = ? AND session_id = ?")
        .bind(node)
        .bind(session_id)
        .fetch_optional(db)
        .await?;
    Ok(row.and_then(|r| r.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::Hub;
    use quietdeck_types::AgentKind;
    use quietdeck_types::summary::RecordOrigin;

    fn info(id: &str, tab: &str, status: AgentStatus) -> SessionInfo {
        SessionInfo {
            session_id: id.into(),
            agent: AgentKind::Claude,
            workspace: Some("w1".into()),
            tab: Some(tab.into()),
            address: Some("w1:p1".into()),
            cwd: Some("/home/sam/code/app".into()),
            status,
            status_changed_at: Timestamp::now(),
            resume_command: None,
            running: true,
            open_url: None,
            opens_in: None,
        }
    }

    fn record(id: &str, hour: &str, sentence: &str, to: &str) -> SummaryRecord {
        SummaryRecord {
            session_id: id.into(),
            agent: AgentKind::Claude,
            period: SummaryPeriod::Hour(hour.into()),
            activity_from: to.parse().unwrap(),
            activity_to: to.parse().unwrap(),
            written_at: to.parse().unwrap(),
            sentence: sentence.into(),
            paragraph: format!("{sentence}, at length."),
            page: String::new(),
            tab: None,
            dir: None,
            origin: RecordOrigin::Node,
        }
    }

    #[tokio::test]
    async fn a_push_updates_rows_and_a_snapshot_marks_missing_ones_stopped() {
        let db = crate::db::memory().await;
        let hub = Hub::default();
        let cfg = DeckConfig::default();
        replace_snapshot(
            &db,
            &cfg,
            "laptop",
            &[info("s1", "billing", AgentStatus::Idle), info("s2", "web", AgentStatus::Working)],
        )
        .await
        .unwrap();
        assert_eq!(list(&db, &hub, &ListFilter::default()).await.unwrap().len(), 2);
        replace_snapshot(&db, &cfg, "laptop", &[info("s1", "billing", AgentStatus::Blocked)]).await.unwrap();
        let running = list(&db, &hub, &ListFilter::default()).await.unwrap();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].info.status, AgentStatus::Blocked);
        let all = list(&db, &hub, &ListFilter { all: true, ..Default::default() }).await.unwrap();
        assert_eq!(all.len(), 2);
        assert!(!all.iter().all(|s| s.node_online), "the node isn't connected");
    }

    #[tokio::test]
    async fn search_finds_words_in_summaries_and_the_newest_record_wins() {
        let db = crate::db::memory().await;
        let hub = Hub::default();
        upsert(&db, &DeckConfig::default(), "laptop", &info("s1", "certs", AgentStatus::Idle)).await.unwrap();
        apply_summary(
            &db,
            "laptop",
            &SummaryEvent::Summary(record(
                "s1",
                "2026-11-23T14",
                "Ordering the sc-1234 certificate",
                "2026-11-23T14:30:00Z",
            )),
        )
        .await
        .unwrap();
        apply_summary(
            &db,
            "laptop",
            &SummaryEvent::Summary(record("s1", "2026-11-23T10", "Older work on invoices", "2026-11-23T10:30:00Z")),
        )
        .await
        .unwrap();
        let found = search(&db, &hub, "certificate", 10).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].sentence.as_deref(),
            Some("Ordering the sc-1234 certificate"),
            "an older hour doesn't replace the latest"
        );
        let hits = summary_hits(&db, "sc-1234").await.unwrap();
        assert_eq!(hits.len(), 1);
        let hits = summary_hits(&db, "invoices").await.unwrap();
        assert_eq!(hits.len(), 1, "older timeline points are searched too");
    }

    #[tokio::test]
    async fn a_session_shows_its_timeline_oldest_first_and_how_to_resume_it() {
        let db = crate::db::memory().await;
        let hub = Hub::default();
        let mut s = info("s1", "certs", AgentStatus::Idle);
        s.cwd = Some("/work/it's here".into());
        s.running = false;
        upsert(&db, &DeckConfig::default(), "laptop", &s).await.unwrap();
        for (hour, text, to) in [
            ("2026-11-23T14", "Ordered the certificate", "2026-11-23T14:30:00Z"),
            ("2026-11-23T10", "Started on the certificate", "2026-11-23T10:30:00Z"),
        ] {
            apply_summary(&db, "laptop", &SummaryEvent::Summary(record("s1", hour, text, to))).await.unwrap();
        }
        let found = find_any(&db, &hub, "certs@laptop").await.unwrap();
        assert_eq!(found.len(), 1, "a tab finds the session that ended when none runs");
        assert_eq!(
            found[0].info.resume_command.as_deref(),
            Some("cd '/work/it'\\''s here' && claude --name 'certs' --resume s1")
        );
        let points: Vec<String> =
            timeline(&db, "laptop", "s1").await.unwrap().into_iter().map(|r| r.sentence).collect();
        assert_eq!(points, ["Started on the certificate", "Ordered the certificate"]);

        let f = ListFilter { since: Some("2000-01-01".into()), ..Default::default() };
        assert_eq!(list(&db, &hub, &f).await.unwrap().len(), 1, "since includes sessions that aren't running");
        let f = ListFilter { since: Some("1d".into()), ..Default::default() };
        assert!(list(&db, &hub, &f).await.unwrap().iter().all(|r| r.info.session_id == "s1"));
        assert!(parse_since("May").is_err());
    }

    #[tokio::test]
    async fn addresses_resolve_by_tab_and_node() {
        let db = crate::db::memory().await;
        let hub = Hub::default();
        let cfg = DeckConfig::default();
        upsert(&db, &cfg, "laptop", &info("s1", "billing", AgentStatus::Idle)).await.unwrap();
        upsert(&db, &cfg, "server", &info("s2", "billing", AgentStatus::Idle)).await.unwrap();
        assert_eq!(resolve_address(&db, &hub, "billing").await.unwrap().len(), 2);
        let one = resolve_address(&db, &hub, "billing@server").await.unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].info.session_id, "s2");
        assert!(resolve_address(&db, &hub, "nobody").await.unwrap().is_empty());
    }

    #[test]
    fn fts_queries_quote_every_word() {
        assert_eq!(fts_query("sc-1234 \"cert\"").unwrap(), "\"sc-1234\" \"cert\"");
        assert!(fts_query("  ").is_none());
    }
}
