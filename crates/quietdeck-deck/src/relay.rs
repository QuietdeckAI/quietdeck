//! The message relay: pass a delivery to the node that runs the target session, or fail at once. Nothing about a
//! message is stored; only per-node counts.

use std::time::Duration;

use quietdeck_types::directory::SessionRow;
use quietdeck_types::node::{DeckRequest, Delivered};
use quietdeck_types::relay::{Delivery, DeliveryError, DeliveryKind, DeliveryTarget};

use crate::db::{Db, now_ms};
use crate::hub::{Hub, HubError};
use crate::store::sessions;

/// How long a node gets to hand a delivery over. Delivery itself is quick (a socket write or one herdr call).
pub const DELIVERY_TIMEOUT: Duration = Duration::from_secs(20);

/// The one running session an address names, or why there isn't one.
pub async fn find_target(db: &Db, hub: &Hub, address: &str) -> Result<SessionRow, DeliveryError> {
    let fail = |e: anyhow::Error| DeliveryError::Failed { address: address.to_string(), error: e.to_string() };
    let rows = sessions::resolve_address(db, hub, address).await.map_err(fail)?;
    let mut nodes: Vec<&str> = rows.iter().map(|r| r.node.as_str()).collect();
    nodes.dedup();
    match rows.len() {
        0 => Err(DeliveryError::SessionOffline { address: address.to_string() }),
        _ if nodes.len() > 1 => {
            Err(DeliveryError::Ambiguous { candidates: rows.iter().map(SessionRow::address).collect() })
        }
        // Several sessions in one node's tabs with the same name: the most recently active one (rows are sorted).
        _ => {
            let row = rows.into_iter().next().expect("checked non-empty");
            if !hub.is_connected(&row.node).await {
                return Err(DeliveryError::NodeOffline { address: row.address() });
            }
            Ok(row)
        }
    }
}

pub fn target_of(row: &SessionRow) -> DeliveryTarget {
    DeliveryTarget {
        tab: row.info.tab.clone().unwrap_or_default(),
        session_id: Some(row.info.session_id.clone()),
        pane: row.info.pane.clone(),
        agent: row.info.agent,
    }
}

/// Deliver to a session found by address.
pub async fn send(
    db: &Db,
    hub: &Hub,
    address: &str,
    kind: DeliveryKind,
    body: String,
) -> Result<Delivered, DeliveryError> {
    let row = find_target(db, hub, address).await?;
    deliver_to(db, hub, &row, kind, body).await
}

pub async fn deliver_to(
    db: &Db,
    hub: &Hub,
    row: &SessionRow,
    kind: DeliveryKind,
    body: String,
) -> Result<Delivered, DeliveryError> {
    let delivery = Delivery { id: new_delivery_id(), target: target_of(row), kind, body };
    let address = row.address();
    let result = match hub.request(&row.node, DeckRequest::Deliver { delivery }, DELIVERY_TIMEOUT).await {
        Ok(reply) => reply.into_result::<Delivered>().map_err(|error| classify(&address, error)),
        Err(HubError::Offline(_)) => Err(DeliveryError::NodeOffline { address: address.clone() }),
        Err(HubError::Timeout(_)) => {
            Err(DeliveryError::Failed { address: address.clone(), error: "the node didn't answer in time".into() })
        }
    };
    count(db, &row.node, result.is_ok()).await;
    result
}

/// The node reports an offline session with this prefix; anything else is passed back as it is.
pub const NODE_OFFLINE_PREFIX: &str = "offline:";

fn classify(address: &str, error: String) -> DeliveryError {
    if error.starts_with(NODE_OFFLINE_PREFIX) {
        DeliveryError::SessionOffline { address: address.to_string() }
    } else {
        DeliveryError::Failed { address: address.to_string(), error }
    }
}

fn new_delivery_id() -> String {
    format!("d_{}", hex::encode(crate::auth::random_bytes::<12>()))
}

async fn count(db: &Db, node: &str, ok: bool) {
    let sql = if ok {
        "INSERT INTO relay_stats (node, delivered_count) VALUES (?, 1) \
         ON CONFLICT (node) DO UPDATE SET delivered_count = delivered_count + 1"
    } else {
        "INSERT INTO relay_stats (node, failed_count, last_failed_at) VALUES (?, 1, ?2) \
         ON CONFLICT (node) DO UPDATE SET failed_count = failed_count + 1, last_failed_at = ?2"
    };
    let mut q = sqlx::query(sql).bind(node);
    if !ok {
        q = q.bind(now_ms());
    }
    if let Err(e) = q.execute(db).await {
        tracing::warn!(error = %e, "counting a delivery");
    }
}
