use axum::{
    body::{Body, Bytes},
    extract::{Path, Query, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use futures_util::StreamExt;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{io::AsyncWriteExt, sync::Semaphore};
use tokio_util::io::ReaderStream;

pub const MAX_UPLOAD: usize = 128 * 1024 * 1024;

#[derive(Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub python: String,
    pub validator: PathBuf,
    pub api_key: String,
}

struct Inner {
    config: Config,
    db: Mutex<Connection>,
    uploads: Semaphore,
}
#[derive(Clone)]
pub struct App(Arc<Inner>);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub address: String,
    pub filename: String,
    pub format: String,
    pub size_bytes: u64,
    pub created_at: String,
    pub default_prim: String,
    pub prim_count: u64,
}

#[derive(Debug)]
pub struct ApiError(StatusCode, String);
impl ApiError {
    fn bad(message: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, message.into())
    }
    fn internal(error: impl std::fmt::Display) -> Self {
        eprintln!("registry error: {error}");
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage service error".into(),
        )
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({"error": self.1}))).into_response()
    }
}

type Result<T> = std::result::Result<T, ApiError>;

impl App {
    pub fn open(config: Config) -> std::result::Result<Self, Box<dyn std::error::Error>> {
        if config.api_key.is_empty() {
            return Err("CONSTRUCT_API_KEY must not be empty".into());
        }
        std::fs::create_dir_all(config.data_dir.join("objects"))?;
        std::fs::create_dir_all(config.data_dir.join("incoming"))?;
        let db = Connection::open(config.data_dir.join("registry.sqlite3"))?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS constructs (
               address TEXT PRIMARY KEY,
               filename TEXT NOT NULL,
               format TEXT NOT NULL,
               size_bytes INTEGER NOT NULL,
               created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
               default_prim TEXT NOT NULL,
               prim_count INTEGER NOT NULL
             );",
        )?;
        Ok(Self(Arc::new(Inner {
            config,
            db: Mutex::new(db),
            uploads: Semaphore::new(2),
        })))
    }
    pub fn router(self) -> Router {
        let protected = Router::new()
            .route("/constructs", get(list).post(upload))
            .route("/constructs/{address}", get(download))
            .route("/constructs/{address}/metadata", get(metadata))
            .route_layer(middleware::from_fn_with_state(self.clone(), authorize));
        Router::new()
            .route(
                "/health",
                get(|| async { Json(serde_json::json!({"status": "ok"})) }),
            )
            .merge(protected)
            .with_state(self)
    }
    async fn record(&self, address: String) -> Result<Record> {
        validate_address(&address)?;
        let app = self.clone();
        tokio::task::spawn_blocking(move || {
            let db = app.0.db.lock().map_err(ApiError::internal)?;
            query_record(&db, &address)?
                .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "construct not found".into()))
        })
        .await
        .map_err(ApiError::internal)?
    }
}

async fn authorize(State(app): State<App>, request: Request, next: Next) -> Response {
    let expected = format!("Bearer {}", app.0.config.api_key);
    let provided = request
        .headers()
        .get(header::AUTHORIZATION)
        .map(|v| v.as_bytes());
    // Compare the complete fixed-length credential without early exit.
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadQuery {
    filename: String,
}

async fn upload(
    State(app): State<App>,
    Query(query): Query<UploadQuery>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response> {
    let filename = query.filename;
    if filename.is_empty()
        || filename.len() > 160
        || !filename
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    {
        return Err(ApiError::bad(
            "filename must be 1..160 ASCII letters, digits, dots, underscores or hyphens",
        ));
    }
    let extension = filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if !["usd", "usda", "usdc", "usdz"].contains(&extension.as_str()) {
        return Err(ApiError::bad("upload must be .usd, .usda, .usdc or .usdz"));
    }
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        != Some("application/octet-stream")
    {
        return Err(ApiError(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "use application/octet-stream".into(),
        ));
    }
    if headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|v| v > MAX_UPLOAD as u64)
    {
        return Err(ApiError(
            StatusCode::PAYLOAD_TOO_LARGE,
            "upload exceeds 128 MiB".into(),
        ));
    }
    let _permit = app.0.uploads.try_acquire().map_err(|_| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "upload capacity reached; retry later".into(),
        )
    })?;
    tokio::time::timeout(
        Duration::from_secs(120),
        receive(app.clone(), filename, extension, body),
    )
    .await
    .map_err(|_| ApiError(StatusCode::REQUEST_TIMEOUT, "upload timed out".into()))?
}

#[derive(Deserialize)]
struct Validation {
    format: String,
    default_prim: String,
    prim_count: u64,
}

async fn receive(app: App, filename: String, extension: String, body: Body) -> Result<Response> {
    let dir =
        tempfile::tempdir_in(app.0.config.data_dir.join("incoming")).map_err(ApiError::internal)?;
    let path = dir.path().join(format!("asset.{extension}"));
    let mut file = tokio::fs::File::create(&path)
        .await
        .map_err(ApiError::internal)?;
    let mut stream = body.into_data_stream();
    let mut hasher = Sha256::new();
    let mut size = 0usize;
    while let Some(chunk) = stream.next().await {
        let chunk: Bytes = chunk.map_err(|_| ApiError::bad("could not read upload body"))?;
        size = size
            .checked_add(chunk.len())
            .ok_or_else(|| ApiError::bad("invalid upload size"))?;
        if size > MAX_UPLOAD {
            return Err(ApiError(
                StatusCode::PAYLOAD_TOO_LARGE,
                "upload exceeds 128 MiB".into(),
            ));
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(ApiError::internal)?;
    }
    if size == 0 {
        return Err(ApiError::bad("empty upload"));
    }
    file.sync_all().await.map_err(ApiError::internal)?;
    drop(file);
    let address = format!("sha256:{:x}", hasher.finalize());
    let output = tokio::time::timeout(
        Duration::from_secs(20),
        tokio::process::Command::new(&app.0.config.python)
            .arg(&app.0.config.validator)
            .arg(&path)
            .env("OMP_NUM_THREADS", "1")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| {
        ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "USD validation timed out".into(),
        )
    })?
    .map_err(ApiError::internal)?;
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(ApiError::internal)?;
    if !output.status.success() {
        return Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            value
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("invalid USD")
                .into(),
        ));
    }
    let validation: Validation = serde_json::from_value(value).map_err(ApiError::internal)?;
    if !["usda", "usdc", "usdz"].contains(&validation.format.as_str()) {
        return Err(ApiError::internal(
            "validator returned an unsupported format",
        ));
    }
    // Serialize publication and SQL writes. Bytes become visible before their record.
    // A crash may leave an unlisted object, but never a committed record before its file.
    let (created, record) = tokio::task::spawn_blocking(move || {
        let db = app.0.db.lock().map_err(ApiError::internal)?;
        if let Some(record) = query_record(&db, &address)? {
            return Ok::<_, ApiError>((false, record));
        }
        let target = object_path(&app, &address, &validation.format);
        std::fs::rename(&path, &target).map_err(ApiError::internal)?;
        #[cfg(unix)]
        std::fs::File::open(app.0.config.data_dir.join("objects"))
            .and_then(|directory| directory.sync_all())
            .map_err(ApiError::internal)?;
        db.execute(
            "INSERT INTO constructs(address,filename,format,size_bytes,default_prim,prim_count)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                address,
                filename,
                validation.format,
                size as u64,
                validation.default_prim,
                validation.prim_count
            ],
        )
        .map_err(ApiError::internal)?;
        let record = query_record(&db, &address)?
            .ok_or_else(|| ApiError::internal("inserted record missing"))?;
        drop(dir);
        Ok((true, record))
    })
    .await
    .map_err(ApiError::internal)??;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        [(header::LOCATION, format!("/constructs/{}", record.address))],
        Json(record),
    )
        .into_response())
}

fn validate_address(address: &str) -> Result<()> {
    let hash = address
        .strip_prefix("sha256:")
        .ok_or_else(|| ApiError::bad("invalid content address"))?;
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(ApiError::bad("invalid content address"));
    }
    Ok(())
}

fn object_path(app: &App, address: &str, format: &str) -> PathBuf {
    app.0
        .config
        .data_dir
        .join("objects")
        .join(format!("{}.{format}", &address[7..]))
}

fn read_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<Record> {
    Ok(Record {
        address: row.get(0)?,
        filename: row.get(1)?,
        format: row.get(2)?,
        size_bytes: row.get(3)?,
        created_at: row.get(4)?,
        default_prim: row.get(5)?,
        prim_count: row.get(6)?,
    })
}
fn query_record(db: &Connection, address: &str) -> Result<Option<Record>> {
    db.query_row(
        "SELECT address,filename,format,size_bytes,created_at,default_prim,prim_count
                  FROM constructs WHERE address=?1",
        [address],
        read_record,
    )
    .optional()
    .map_err(ApiError::internal)
}

async fn metadata(State(app): State<App>, Path(address): Path<String>) -> Result<Json<Record>> {
    Ok(Json(app.record(address).await?))
}

async fn download(State(app): State<App>, Path(address): Path<String>) -> Result<Response> {
    let record = app.record(address).await?;
    let file = tokio::fs::File::open(object_path(&app, &record.address, &record.format))
        .await
        .map_err(ApiError::internal)?;
    // Use a content-derived filename so a caller's original extension cannot mislead importers.
    let filename = format!("{}.{}", &record.address[7..], record.format);
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".into()),
            (header::CONTENT_LENGTH, record.size_bytes.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
            (header::ETAG, format!("\"{}\"", record.address)),
            (
                header::CACHE_CONTROL,
                "private, max-age=31536000, immutable".into(),
            ),
        ],
        Body::from_stream(ReaderStream::new(file)),
    )
        .into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default)]
    offset: u32,
}
fn default_limit() -> u32 {
    50
}

async fn list(State(app): State<App>, Query(query): Query<ListQuery>) -> Result<Json<Vec<Record>>> {
    if !(1..=200).contains(&query.limit) {
        return Err(ApiError::bad("limit must be 1..200"));
    }
    let records = tokio::task::spawn_blocking(move || {
        let db = app.0.db.lock().map_err(ApiError::internal)?;
        let mut statement = db
            .prepare(
                "SELECT address,filename,format,size_bytes,created_at,default_prim,prim_count
            FROM constructs ORDER BY created_at DESC,address DESC LIMIT ?1 OFFSET ?2",
            )
            .map_err(ApiError::internal)?;
        let rows = statement
            .query_map(params![query.limit, query.offset], read_record)
            .map_err(ApiError::internal)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(ApiError::internal)
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(Json(records))
}
