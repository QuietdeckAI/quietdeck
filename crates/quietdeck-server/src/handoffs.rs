//! Handoffs (quietdeck_types::handoff): what a session being moved writes for the session that continues it on
//! another machine, and the link between the two.

use garde::Validate;
use quietdeck_types::action::SessionRef;
use quietdeck_types::handoff::{MoveLink, SessionHandoff, SessionHandoffAdd};

use crate::db::{Db, now_ms, ts};
use crate::error::{AppError, AppResult};

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    from_node: String,
    from_tab: String,
    from_session_id: Option<String>,
    handoff: String,
    manifest: String,
    created_at: i64,
    to_node: Option<String>,
    to_tab: Option<String>,
    to_session_id: Option<String>,
    continued_at: Option<i64>,
}

const COLUMNS: &str = "id, from_node, from_tab, from_session_id, handoff, manifest, created_at, to_node, to_tab, \
                       to_session_id, continued_at";

impl From<Row> for SessionHandoff {
    fn from(r: Row) -> SessionHandoff {
        let continued_by = match (r.to_node, r.to_tab) {
            (Some(node), Some(tab)) => Some(SessionRef { node, tab, session_id: r.to_session_id }),
            _ => None,
        };
        SessionHandoff {
            id: r.id,
            from: SessionRef { node: r.from_node, tab: r.from_tab, session_id: r.from_session_id },
            handoff: r.handoff,
            manifest: r.manifest,
            created_at: ts(r.created_at),
            continued_by,
            continued_at: r.continued_at.map(ts),
        }
    }
}

/// `quietdeck handoff add`: the calling session's handoff about itself.
pub async fn add(db: &Db, me: &SessionRef, req: SessionHandoffAdd) -> AppResult<SessionHandoff> {
    req.validate().map_err(|e| AppError::BadRequest(e.to_string()))?;
    let id: (i64,) = sqlx::query_as(
        "INSERT INTO handoffs (from_node, from_tab, from_session_id, handoff, manifest, created_at) \
         VALUES (?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(&me.node)
    .bind(&me.tab)
    .bind(&me.session_id)
    .bind(&req.handoff)
    .bind(&req.manifest)
    .bind(now_ms())
    .fetch_one(db)
    .await?;
    get(db, id.0).await
}

pub async fn get(db: &Db, id: i64) -> AppResult<SessionHandoff> {
    let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {COLUMNS} FROM handoffs WHERE id = ?")))
        .bind(id)
        .fetch_optional(db)
        .await?;
    row.map(SessionHandoff::from).ok_or_else(|| AppError::NotFound(format!("no handoff h_{id}")))
}

/// A handoff a new session can start from: one nobody has continued yet.
pub async fn unclaimed(db: &Db, id: i64) -> AppResult<SessionHandoff> {
    let h = get(db, id).await?;
    if let Some(by) = &h.continued_by {
        return Err(AppError::Conflict(format!(
            "h_{id} was already continued by {}: a handoff starts one session",
            by.address()
        )));
    }
    Ok(h)
}

/// The session started from the handoff now continues the one that wrote it, and takes over its unfinished work on
/// the deck: its open cards (the runner carries out approvals only on its own agent's cards, so a moved triage tab
/// would otherwise never see them) and its open report (the Inbox sends answers to the report's tab).
pub async fn continued(db: &Db, id: i64, by: &SessionRef) -> AppResult<()> {
    let mut tx = db.begin().await?;
    let done = sqlx::query(
        "UPDATE handoffs SET to_node = ?, to_tab = ?, to_session_id = ?, continued_at = ? \
         WHERE id = ? AND to_node IS NULL",
    )
    .bind(&by.node)
    .bind(&by.tab)
    .bind(&by.session_id)
    .bind(now_ms())
    .bind(id)
    .execute(&mut *tx)
    .await?;
    if done.rows_affected() == 1 {
        let from: (String, String) = sqlx::query_as("SELECT from_node, from_tab FROM handoffs WHERE id = ?")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE cards SET posted_node = ?, posted_tab = ?, posted_session_id = ? \
             WHERE posted_node = ? AND posted_tab = ? AND state NOT IN ('done', 'handled_elsewhere', 'automatic')",
        )
        .bind(&by.node)
        .bind(&by.tab)
        .bind(&by.session_id)
        .bind(&from.0)
        .bind(&from.1)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE reports SET posted_node = ?, posted_tab = ?, posted_session_id = ? \
             WHERE posted_node = ? AND posted_tab = ? AND closed_at IS NULL",
        )
        .bind(&by.node)
        .bind(&by.tab)
        .bind(&by.session_id)
        .bind(&from.0)
        .bind(&from.1)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// `quietdeck handoff claim`: the calling session continues the handoff's session. For a start that stopped before
/// the deck could record it (a prompt in the new tab while the agent started): the new session claims it once it's
/// running. Claiming again is fine; a handoff someone else continues, or your own, isn't yours to claim.
pub async fn claim(db: &Db, id: i64, me: &SessionRef) -> AppResult<SessionHandoff> {
    let h = get(db, id).await?;
    if let Some(by) = &h.continued_by {
        if by.node == me.node && by.tab == me.tab {
            return Ok(h);
        }
        return Err(AppError::Conflict(format!(
            "h_{id} is already continued by {}: a handoff starts one session",
            by.address()
        )));
    }
    if h.from.node == me.node && h.from.tab == me.tab && h.from.session_id == me.session_id {
        return Err(AppError::BadRequest(format!(
            "h_{id} is your own handoff: the session that continues it claims it"
        )));
    }
    continued(db, id, me).await?;
    get(db, id).await
}

/// The first prompt of a session started from a handoff, when the starter gave none.
pub fn first_prompt(h: &SessionHandoff) -> String {
    format!(
        "You continue the session {} (session ID {}), which was moved to this machine. Read its handoff and access \
         manifest first: `quietdeck handoff show h_{}`. Then check you can reach what the manifest lists, and reply \
         with one line saying you're ready or what's missing.",
        h.from.address(),
        h.from.session_id.as_deref().unwrap_or("unknown"),
        h.id
    )
}

/// Both ends of the moves a session took part in: the session it continues, and the one that continues it.
pub async fn links(db: &Db, node: &str, session_id: &str) -> AppResult<(Option<MoveLink>, Option<MoveLink>)> {
    let continues: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM handoffs WHERE to_node = ? AND to_session_id = ? ORDER BY id DESC LIMIT 1"
    )))
    .bind(node)
    .bind(session_id)
    .fetch_optional(db)
    .await?;
    let moved: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM handoffs WHERE from_node = ? AND from_session_id = ? AND to_node IS NOT NULL \
         ORDER BY id DESC LIMIT 1"
    )))
    .bind(node)
    .bind(session_id)
    .fetch_optional(db)
    .await?;
    let continues = continues.map(SessionHandoff::from).map(|h| MoveLink {
        handoff_id: h.id,
        at: h.continued_at.unwrap_or(h.created_at),
        session: h.from,
    });
    let moved_to = moved.map(SessionHandoff::from).and_then(|h| {
        let at = h.continued_at.unwrap_or(h.created_at);
        h.continued_by.map(|session| MoveLink { handoff_id: h.id, session, at })
    });
    Ok((continues, moved_to))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;

    fn old() -> SessionRef {
        SessionRef { node: "laptop".into(), tab: "infra".into(), session_id: Some("s-old".into()) }
    }

    fn add_req() -> SessionHandoffAdd {
        SessionHandoffAdd { handoff: "# For\nThe home server.".into(), manifest: "- ssh to the VM".into() }
    }

    #[tokio::test]
    async fn a_handoff_links_the_new_session_to_the_old_one_once() {
        let state = AppState::for_tests().await;
        let h = add(&state.db, &old(), add_req()).await.unwrap();
        assert_eq!(h.from, old());
        assert!(h.continued_by.is_none());
        assert!(first_prompt(&h).contains(&format!("quietdeck handoff show h_{}", h.id)));
        assert_eq!(links(&state.db, "laptop", "s-old").await.unwrap(), (None, None));

        let new = SessionRef { node: "vm".into(), tab: "infra".into(), session_id: Some("s-new".into()) };
        unclaimed(&state.db, h.id).await.unwrap();
        continued(&state.db, h.id, &new).await.unwrap();
        let again = unclaimed(&state.db, h.id).await.unwrap_err();
        assert!(again.to_string().contains("already continued by infra@vm"), "{again}");

        let (continues, moved_to) = links(&state.db, "laptop", "s-old").await.unwrap();
        assert!(continues.is_none());
        assert_eq!(moved_to.map(|m| m.session), Some(new.clone()));
        let (continues, moved_to) = links(&state.db, "vm", "s-new").await.unwrap();
        assert_eq!(continues.map(|m| (m.handoff_id, m.session)), Some((h.id, old())));
        assert!(moved_to.is_none());
    }

    #[tokio::test]
    async fn the_continuing_session_takes_over_open_cards_and_the_report() {
        let state = AppState::for_tests().await;
        let card = |s: &'static str| {
            let db = state.db.clone();
            async move {
                sqlx::query_scalar::<_, i64>(
                    "INSERT INTO cards (posted_node, posted_tab, account, thread_key, section, state, input, \
                     prepared_at, created_at, updated_at) VALUES ('laptop', 'infra', 'me', 't', 'needs_you', ?, '{}', \
                     0, 0, 0) RETURNING id",
                )
                .bind(s)
                .fetch_one(&db)
                .await
                .unwrap()
            }
        };
        let (open, done) = (card("open").await, card("done").await);
        let report =
            quietdeck_types::report::ReportSet { markdown: "# Round".into(), base: None, new: false, change: None };
        crate::reports::set(&state, &old(), report).await.unwrap();

        let h = add(&state.db, &old(), add_req()).await.unwrap();
        let new = SessionRef { node: "vm".into(), tab: "infra".into(), session_id: Some("s-new".into()) };
        continued(&state.db, h.id, &new).await.unwrap();

        let node = |id: i64| {
            let db = state.db.clone();
            async move {
                sqlx::query_scalar::<_, String>("SELECT posted_node FROM cards WHERE id = ?")
                    .bind(id)
                    .fetch_one(&db)
                    .await
                    .unwrap()
            }
        };
        assert_eq!(node(open).await, "vm");
        assert_eq!(node(done).await, "laptop", "finished cards stay with the session that did them");
        let r = crate::reports::current(&state.db).await.unwrap().unwrap();
        assert_eq!(r.posted_by, new);
    }

    #[tokio::test]
    async fn a_running_session_claims_a_handoff_its_start_never_recorded() {
        let state = AppState::for_tests().await;
        let h = add(&state.db, &old(), add_req()).await.unwrap();
        let err = claim(&state.db, h.id, &old()).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "{err}");
        let new = SessionRef { node: "vm".into(), tab: "infra".into(), session_id: Some("s-new".into()) };
        assert_eq!(claim(&state.db, h.id, &new).await.unwrap().continued_by, Some(new.clone()));
        claim(&state.db, h.id, &new).await.unwrap();
        let other = SessionRef { node: "vm2".into(), tab: "infra".into(), session_id: None };
        let err = claim(&state.db, h.id, &other).await.unwrap_err();
        assert!(err.to_string().contains("already continued by infra@vm"), "{err}");
    }

    #[tokio::test]
    async fn a_handoff_needs_both_parts() {
        let state = AppState::for_tests().await;
        let err = add(&state.db, &old(), SessionHandoffAdd { manifest: String::new(), ..add_req() }).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "{err}");
        let err = get(&state.db, 99).await.unwrap_err();
        assert!(err.to_string().contains("no handoff h_99"), "{err}");
    }
}
