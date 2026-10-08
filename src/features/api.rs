mod auth;
mod error;
mod handlers;

use crate::App;
use axum::{middleware, routing::get, Json, Router};

pub(crate) fn router(app: App) -> Router {
    let protected = Router::new()
        .route("/constructs", get(handlers::list).post(handlers::upload))
        .route("/constructs/{address}", get(handlers::download))
        .route("/constructs/{address}/metadata", get(handlers::metadata))
        .route_layer(middleware::from_fn_with_state(app.clone(), auth::authorize));
    Router::new()
        .route(
            "/health",
            get(|| async { Json(serde_json::json!({"status": "ok"})) }),
        )
        .merge(protected)
        .with_state(app)
}
