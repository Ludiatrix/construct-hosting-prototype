use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use construct_registry::{App, Config, Record};
use http_body_util::BodyExt;
use std::path::PathBuf;
use tower::ServiceExt;

fn config(dir: &tempfile::TempDir) -> Config {
    Config {
        data_dir: dir.path().into(),
        python: std::env::var("CONSTRUCT_PYTHON").unwrap_or_else(|_| "python3".into()),
        validator: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("validate_usd.py"),
        api_key: "test-secret".into(),
    }
}
fn request(method: &str, uri: &str, bytes: &[u8]) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("Authorization", "Bearer test-secret")
        .header("Content-Type", "application/octet-stream")
        .body(Body::from(bytes.to_vec()))
        .unwrap()
}
async fn upload(router: &Router, name: &str, data: &[u8]) -> (StatusCode, Vec<u8>) {
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            &format!("/constructs?filename={name}"),
            data,
        ))
        .await
        .unwrap();
    (
        response.status(),
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
}

#[tokio::test]
async fn upload_deduplicate_download_list_and_reopen_database() {
    let dir = tempfile::tempdir().unwrap();
    let router = App::open(config(&dir)).unwrap().router();
    let data = include_bytes!("../fixtures/crate.usda");
    let (status, body) = upload(&router, "crate.usda", data).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let first: Record = serde_json::from_slice(&body).unwrap();
    assert_eq!(first.size_bytes, data.len() as u64);
    assert_eq!(first.default_prim, "/Crate");
    assert_eq!(first.format, "usda");
    let (status, body) = upload(&router, "renamed.usd", data).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let duplicate: Record = serde_json::from_slice(&body).unwrap();
    assert_eq!(duplicate.address, first.address);
    assert_eq!(duplicate.filename, "crate.usda");
    drop(router);
    let router = App::open(config(&dir)).unwrap().router();
    let response = router
        .clone()
        .oneshot(request(
            "GET",
            &format!("/constructs/{}", first.address),
            &[],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response
        .headers()
        .get("content-disposition")
        .unwrap()
        .to_str()
        .unwrap()
        .ends_with(".usda\""));
    assert_eq!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .as_ref(),
        data
    );
    let response = router
        .clone()
        .oneshot(request(
            "GET",
            &format!("/constructs/{}/metadata", first.address),
            &[],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = router
        .oneshot(request("GET", "/constructs?limit=1&offset=0", &[]))
        .await
        .unwrap();
    let records: Vec<Record> =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].address, first.address);
}

#[tokio::test]
async fn rejects_invalid_usd_and_external_dependencies_without_records() {
    let dir = tempfile::tempdir().unwrap();
    let router = App::open(config(&dir)).unwrap().router();
    for data in [b"not USD".as_slice(), b"#usda 1.0\n(defaultPrim=\"Crate\")\ndef Xform \"Crate\" { custom asset texture = @missing.png@ }\n"] {
        let (status, _) = upload(&router, "bad.usda", data).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }
    let response = router
        .oneshot(request("GET", "/constructs", &[]))
        .await
        .unwrap();
    assert_eq!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .as_ref(),
        b"[]"
    );
    assert_eq!(
        std::fs::read_dir(dir.path().join("objects"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        std::fs::read_dir(dir.path().join("incoming"))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn enforces_auth_paths_types_sizes_and_missing_addresses() {
    let dir = tempfile::tempdir().unwrap();
    let router = App::open(config(&dir)).unwrap().router();
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/constructs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/constructs?filename=crate.usda")
                .header("Authorization", "Bearer wrong")
                .body(Body::from("broken"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    for (name, expected) in [
        ("evil.exe", StatusCode::BAD_REQUEST),
        ("..%2Fcrate.usda", StatusCode::BAD_REQUEST),
        ("crate.usda", StatusCode::BAD_REQUEST),
    ] {
        assert_eq!(upload(&router, name, &[]).await.0, expected);
    }
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/constructs?filename=crate.usda")
                .header("Authorization", "Bearer test-secret")
                .body(Body::from("abc"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let mut oversized = request("POST", "/constructs?filename=crate.usda", &[]);
    oversized.headers_mut().insert(
        "Content-Length",
        (construct_registry::MAX_UPLOAD + 1)
            .to_string()
            .parse()
            .unwrap(),
    );
    assert_eq!(
        router.clone().oneshot(oversized).await.unwrap().status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request("GET", "/constructs/not-an-address", &[]))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request(
                "GET",
                &format!("/constructs/sha256:{}", "0".repeat(64)),
                &[]
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        router
            .oneshot(request("GET", "/constructs?limit=0", &[]))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn concurrent_duplicate_uploads_create_one_record() {
    let dir = tempfile::tempdir().unwrap();
    let router = App::open(config(&dir)).unwrap().router();
    let data = include_bytes!("../fixtures/crate.usda");
    let (first, second) = tokio::join!(
        upload(&router, "crate.usda", data),
        upload(&router, "crate.usda", data)
    );
    let mut statuses = [first.0.as_u16(), second.0.as_u16()];
    statuses.sort();
    assert_eq!(statuses, [200, 201]);
    assert_eq!(
        std::fs::read_dir(dir.path().join("objects"))
            .unwrap()
            .count(),
        1
    );
}
