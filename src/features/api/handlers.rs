use crate::{
    error::{Error, Result},
    features::{
        constructs::{model::UploadName, Record},
        storage::MAX_UPLOAD,
    },
    App,
};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use tokio_util::io::ReaderStream;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UploadQuery {
    filename: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ListQuery {
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default)]
    offset: u32,
}
fn default_limit() -> u32 {
    50
}

pub(super) async fn upload(
    State(app): State<App>,
    Query(query): Query<UploadQuery>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response> {
    let name = UploadName::parse(query.filename)?;
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        != Some("application/octet-stream")
    {
        return Err(Error::UnsupportedMediaType);
    }
    if headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|v| v > MAX_UPLOAD as u64)
    {
        return Err(Error::UploadTooLarge);
    }
    let result = app.registry.upload(name, body.into_data_stream()).await?;
    let status = if result.created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        [(
            header::LOCATION,
            format!("/constructs/{}", result.record.address),
        )],
        Json(result.record),
    )
        .into_response())
}

pub(super) async fn download(
    State(app): State<App>,
    Path(address): Path<String>,
) -> Result<Response> {
    let download = app.registry.download(address).await?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".into()),
            (
                header::CONTENT_LENGTH,
                download.record.size_bytes.to_string(),
            ),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", download.filename),
            ),
            (header::ETAG, format!("\"{}\"", download.record.address)),
            (
                header::CACHE_CONTROL,
                "private, max-age=31536000, immutable".into(),
            ),
        ],
        Body::from_stream(ReaderStream::new(download.body.into_async_read())),
    )
        .into_response())
}
pub(super) async fn metadata(
    State(app): State<App>,
    Path(address): Path<String>,
) -> Result<Json<Record>> {
    Ok(Json(app.registry.record(address).await?))
}
pub(super) async fn list(
    State(app): State<App>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<Record>>> {
    Ok(Json(app.registry.list(query.limit, query.offset).await?))
}
