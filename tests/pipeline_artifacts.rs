use std::net::{Ipv4Addr, SocketAddr};

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use liteci::{app_with_state, connect, migrate};
use serde_json::Value;
use tower::ServiceExt;

fn get(uri: &str, token: Option<&str>) -> Request<Body> {
    let mut request = Request::builder()
        .method("GET")
        .uri(uri)
        .extension(ConnectInfo(SocketAddr::new(
            Ipv4Addr::LOCALHOST.into(),
            43000,
        )))
        .body(Body::empty())
        .unwrap();
    if let Some(token) = token {
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
    }
    request
}

#[tokio::test]
async fn artifact_list_returns_metadata_for_a_run() {
    let pool = connect("sqlite::memory:").await.unwrap();
    migrate(&pool).await.unwrap();
    let service = app_with_state(pool.clone());
    let setup = Request::builder().method("POST").uri("/api/auth/setup")
        .header("content-type", "application/json")
        .extension(ConnectInfo(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 43000)))
        .body(Body::from(r#"{"username":"admin","password":"correct horse battery staple","setup_token":"test-setup-token"}"#)).unwrap();
    service.clone().oneshot(setup).await.unwrap();
    let login = Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header("content-type", "application/json")
        .extension(ConnectInfo(SocketAddr::new(
            Ipv4Addr::LOCALHOST.into(),
            43000,
        )))
        .body(Body::from(
            r#"{"username":"admin","password":"correct horse battery staple"}"#,
        ))
        .unwrap();
    let body = service
        .clone()
        .oneshot(login)
        .await
        .unwrap()
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let token = serde_json::from_slice::<Value>(&body).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();

    sqlx::query("INSERT INTO projects (id, name, git_url, workspace_path) VALUES ('p', 'artifacts', 'https://example.invalid/repo.git', 'p')").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO pipeline_runs (id, project_id, run_number, branch, created_by) SELECT 'r', 'p', 1, 'main', id FROM users WHERE username = 'admin'").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO pipeline_artifacts (id, run_id, project_id, name, relative_path, file_name, size_bytes, checksum_sha256) VALUES ('a', 'r', 'p', 'dist', 'dist/app.tar', 'app.tar', 42, '0123456789012345678901234567890123456789012345678901234567890123')").execute(&pool).await.unwrap();

    let response = service
        .oneshot(get("/api/runs/r/artifacts", Some(&token)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let artifacts: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(artifacts[0]["name"], "dist");
    assert_eq!(artifacts[0]["size_bytes"], 42);
    assert_eq!(
        artifacts[0]["checksum_sha256"],
        "0123456789012345678901234567890123456789012345678901234567890123"
    );
}
