use std::net::{Ipv4Addr, SocketAddr};

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use liteci::{PipelineEngine, app_with_state, connect, migrate};
use serde_json::Value;
use tempfile::TempDir;
use tower::ServiceExt;

fn request(uri: &str, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method("GET")
        .uri(uri)
        .extension(ConnectInfo(SocketAddr::new(
            Ipv4Addr::LOCALHOST.into(),
            43000,
        )));
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder.body(Body::empty()).unwrap()
}

async fn authenticated() -> (axum::Router, String, sqlx::SqlitePool) {
    let pool = connect("sqlite::memory:").await.unwrap();
    migrate(&pool).await.unwrap();
    let service = app_with_state(pool.clone());
    let setup = Request::builder()
        .method("POST")
        .uri("/api/auth/setup")
        .header("content-type", "application/json")
        .extension(ConnectInfo(SocketAddr::new(
            Ipv4Addr::LOCALHOST.into(),
            43000,
        )))
        .body(Body::from(
            r#"{"username":"admin","password":"correct horse battery staple","setup_token":"test-setup-token"}"#,
        ))
        .unwrap();
    assert_eq!(
        service.clone().oneshot(setup).await.unwrap().status(),
        StatusCode::CREATED
    );
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
    (service, token, pool)
}

#[tokio::test]
async fn run_logs_require_authentication() {
    let pool = connect("sqlite::memory:").await.unwrap();
    migrate(&pool).await.unwrap();
    let response = app_with_state(pool)
        .oneshot(request(
            "/api/runs/11111111-1111-4111-8111-111111111111/logs",
            None,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn pipeline_engine_persists_stdout_and_stderr_chunks() {
    let (service, token, pool) = authenticated().await;
    let workspace = TempDir::new().unwrap();
    sqlx::query("INSERT INTO projects (id, name, git_url, workspace_path) VALUES ('11111111-1111-4111-8111-111111111111', 'logs-project', 'https://example.invalid/repo.git', 'logs-project')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO pipeline_runs (id, project_id, run_number, branch, created_by) VALUES ('22222222-2222-4222-8222-222222222222', '11111111-1111-4111-8111-111111111111', 1, 'main', (SELECT id FROM users WHERE username = 'admin'))")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO stage_runs (id, run_id, position, name, command, enabled, status, timeout_seconds) VALUES ('stage-logs', '22222222-2222-4222-8222-222222222222', 0, 'logs', '{\"mode\":\"process\",\"program\":\"cmd\",\"args\":[\"/C\",\"echo out && echo err 1>&2\"]}', 1, 'pending', 30)")
        .execute(&pool)
        .await
        .unwrap();

    let engine = PipelineEngine::new(pool.clone(), workspace.path(), 1);
    engine
        .execute("22222222-2222-4222-8222-222222222222")
        .await
        .unwrap();

    let response = service
        .clone()
        .oneshot(request(
            "/api/runs/22222222-2222-4222-8222-222222222222/logs",
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let logs: Value = serde_json::from_slice(&body).unwrap();
    assert!(!logs.as_array().unwrap().is_empty());
    let rendered = logs
        .as_array()
        .unwrap()
        .iter()
        .map(|log| log["data"].as_str().unwrap_or_default())
        .collect::<String>();
    assert!(rendered.contains("out"));
    let response = service
        .clone()
        .oneshot(request(
            "/api/runs/22222222-2222-4222-8222-222222222222/logs?limit=1&offset=1",
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let page: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(page.as_array().unwrap().len(), 1);
}
