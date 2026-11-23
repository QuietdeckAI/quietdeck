//! The webhook receiver: a second listener on its own port, behind the tunnel. It serves `/healthz` and
//! `POST /hooks/<endpoint id>/<secret>`, and nothing else.

use axum::Router;
use axum::http::StatusCode;
use axum::routing::{get, post};

use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/hooks/{id}/{secret}", post(receive))
        .fallback(|| async { StatusCode::NOT_FOUND })
        .with_state(state)
}

/// Endpoints arrive with the webhook relay (build step 11); until then no endpoint exists, so every path is unknown.
async fn receive() -> StatusCode {
    StatusCode::NOT_FOUND
}
