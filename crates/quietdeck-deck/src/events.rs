//! The web app's event stream: server-sent events naming what changed, so the app refetches it.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::Stream;
pub use quietdeck_types::api::Changed;
use tokio::sync::broadcast::error::RecvError;

use crate::auth::Owner;
use crate::state::AppState;

pub async fn stream(
    State(state): State<AppState>,
    _owner: Owner,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.events.subscribe();
    let events = futures::stream::unfold(rx, |mut rx| async move {
        let what = match rx.recv().await {
            Ok(what) => what,
            Err(RecvError::Lagged(_)) => Changed::All,
            Err(RecvError::Closed) => return None,
        };
        let data = serde_json::to_string(&what).unwrap_or_else(|_| "\"all\"".into());
        Some((Ok(Event::default().event("changed").data(data)), rx))
    });
    Sse::new(events).keep_alive(KeepAlive::new().interval(Duration::from_secs(20)))
}
