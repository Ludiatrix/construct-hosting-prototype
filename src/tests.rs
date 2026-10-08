use crate::{
    features::{
        constructs::{model::UploadName, Registry},
        database::Database,
        storage::Storage,
        validation::Validator,
    },
    App,
};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;

fn sdk() -> aws_config::SdkConfig {
    aws_config::SdkConfig::builder()
        .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
        .region(aws_sdk_s3::config::Region::new("us-east-2"))
        .credentials_provider(aws_sdk_s3::config::SharedCredentialsProvider::new(
            aws_sdk_s3::config::Credentials::new("test", "test", None, None, "test"),
        ))
        .build()
}
fn app() -> App {
    let sdk = sdk();
    let database = Database {
        pool: sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://test:test@127.0.0.1:1/test")
            .unwrap(),
    };
    App {
        registry: Registry::new(
            database,
            Storage::new(aws_sdk_s3::Client::new(&sdk), "test".into()),
            Validator::new(aws_sdk_lambda::Client::new(&sdk), "test".into()),
        ),
        upload_key: "upload-key".into(),
        read_key: "read-key".into(),
    }
}
fn request(method: &str, uri: &str, key: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("Authorization", format!("Bearer {key}"))
        .header("Content-Type", "application/octet-stream")
        .body(Body::empty())
        .unwrap()
}
#[tokio::test]
async fn separates_read_and_upload_authorization() {
    let router = app().router();
    for (method, uri, key, expected) in [
        (
            "POST",
            "/constructs?filename=bad.exe",
            "read-key",
            StatusCode::UNAUTHORIZED,
        ),
        (
            "POST",
            "/constructs?filename=bad.exe",
            "upload-key",
            StatusCode::BAD_REQUEST,
        ),
        (
            "GET",
            "/constructs/not-an-address",
            "upload-key",
            StatusCode::UNAUTHORIZED,
        ),
        (
            "GET",
            "/constructs/not-an-address",
            "read-key",
            StatusCode::BAD_REQUEST,
        ),
        (
            "GET",
            "/constructs?limit=0",
            "read-key",
            StatusCode::BAD_REQUEST,
        ),
        ("GET", "/constructs", "wrong", StatusCode::UNAUTHORIZED),
    ] {
        assert_eq!(
            router
                .clone()
                .oneshot(request(method, uri, key))
                .await
                .unwrap()
                .status(),
            expected
        );
    }
    assert_eq!(
        router
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
}
#[tokio::test]
async fn rejects_content_type_length_and_empty_upload() {
    let router = app().router();
    let mut req = request("POST", "/constructs?filename=crate.usda", "upload-key");
    req.headers_mut().remove("Content-Type");
    assert_eq!(
        router.clone().oneshot(req).await.unwrap().status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    let mut req = request("POST", "/constructs?filename=crate.usda", "upload-key");
    req.headers_mut().insert(
        "Content-Length",
        (crate::MAX_UPLOAD + 1).to_string().parse().unwrap(),
    );
    assert_eq!(
        router.clone().oneshot(req).await.unwrap().status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        router
            .oneshot(request(
                "POST",
                "/constructs?filename=crate.usda",
                "upload-key"
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
}
#[tokio::test]
async fn stages_hashes_and_cleans_local_uploads() {
    let storage = Storage::new(aws_sdk_s3::Client::new(&sdk()), "test".into());
    let data = include_bytes!("../samples/crate.usda");
    let stream = futures_util::stream::iter([Ok::<_, std::io::Error>(data)]);
    let staged = storage
        .stage(&UploadName::parse("crate.usda".into()).unwrap(), stream)
        .await
        .unwrap();
    use sha2::{Digest, Sha256};
    assert_eq!(
        staged.address.as_str(),
        format!("sha256:{:x}", Sha256::digest(data))
    );
    assert_eq!(staged.size_bytes, data.len() as u64);
    let path = staged.path.clone();
    assert_eq!(tokio::fs::read(&path).await.unwrap(), data);
    drop(staged);
    assert!(!path.exists());
}
#[tokio::test]
async fn streaming_upload_limit_does_not_depend_on_content_length() {
    let storage = Storage::new(aws_sdk_s3::Client::new(&sdk()), "test".into());
    let chunk = vec![0u8; 1024 * 1024];
    let stream =
        futures_util::stream::iter((0..129).map(|_| Ok::<_, std::io::Error>(chunk.as_slice())));
    assert!(matches!(
        storage
            .stage(&UploadName::parse("crate.usda".into()).unwrap(), stream)
            .await,
        Err(crate::error::Error::UploadTooLarge)
    ));
}
