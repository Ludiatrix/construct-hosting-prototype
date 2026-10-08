use crate::App;
use axum::{
    extract::{Request, State},
    http::{header, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};

pub(super) async fn authorize(State(app): State<App>, request: Request, next: Next) -> Response {
    let key = if request.method() == Method::POST {
        &app.upload_key
    } else {
        &app.read_key
    };
    let expected = format!("Bearer {key}");
    let provided = request
        .headers()
        .get(header::AUTHORIZATION)
        .map(|v| v.as_bytes());
    let authenticated = provided.is_some_and(|value| {
        value.len() == expected.len()
            && value
                .iter()
                .zip(expected.as_bytes())
                .fold(0u8, |diff, (a, b)| diff | (a ^ b))
                == 0
    });
    if !authenticated {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
            Json(serde_json::json!({"error": "invalid API key"})),
        )
            .into_response();
    }
    next.run(request).await
}
