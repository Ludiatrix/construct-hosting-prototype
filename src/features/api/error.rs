use crate::error::Error;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::UploadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::UploadCapacity => StatusCode::SERVICE_UNAVAILABLE,
            Self::UploadTimeout => StatusCode::REQUEST_TIMEOUT,
            Self::InvalidConstruct(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let message = if let Self::Internal(error) = &self {
            eprintln!("registry error: {error}");
            "storage service error".into()
        } else {
            self.to_string()
        };
        (status, Json(serde_json::json!({"error": message}))).into_response()
    }
}
