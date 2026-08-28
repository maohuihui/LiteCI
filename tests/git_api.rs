use std::net::{Ipv4Addr, SocketAddr};

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use liteci::{app_with_state, connect, migrate};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tower::ServiceExt;

async fn pool() -> SqlitePool {
    let pool = connect("sqlite::memory:").await.unwrap();
    migrate(&pool).await.unwrap();
    pool
}

fn request(method: &str, uri: &str, body: Option<Value>, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .extension(ConnectInfo(SocketAddr::new(
            Ipv4Addr::LOCALHOST.into(),
            43000,
        )));
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    builder
        .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
        .unwrap()
}

async fn authenticated_service() -> (axum::Router, String, SqlitePool) {
    let pool = pool().await;
    let service = app_with_state(pool.clone());
    let setup = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/auth/setup",
            Some(json!({"username":"admin","password":"correct horse battery staple","setup_token":"test-setup-token"})),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(setup.status(), StatusCode::CREATED);
    let login = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/auth/login",
            Some(json!({"username":"admin","password":"correct horse battery staple"})),
            None,
        ))
        .await
        .unwrap();
    let body = login.into_body().collect().await.unwrap().to_bytes();
    let token = serde_json::from_slice::<Value>(&body).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    (service, token, pool)
}

async fn git(source: &std::path::Path, args: &[&str]) {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(source)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn project_sync_requires_authentication_and_returns_commit_metadata() {
    let (service, token, pool) = authenticated_service().await;
    let root = std::env::temp_dir().join(format!("liteci-api-git-{}", uuid::Uuid::new_v4()));
    let source = root.join("source");
    std::fs::create_dir_all(&source).unwrap();
    git(&source, &["init", "-b", "main"]).await;
    git(&source, &["config", "user.email", "test@example.invalid"]).await;
    git(&source, &["config", "user.name", "LiteCI Test"]).await;
    std::fs::write(source.join("README.md"), "hello\n").unwrap();
    git(&source, &["add", "README.md"]).await;
    git(&source, &["commit", "-m", "api sync"]).await;

    let id = uuid::Uuid::new_v4().to_string();
    let workspace_path = format!("workspaces/{}", uuid::Uuid::new_v4());
    sqlx::query("INSERT INTO projects (id, name, git_url, default_branch, workspace_path) VALUES (?1, ?2, ?3, 'main', ?4)")
        .bind(&id)
        .bind("sync-site")
        .bind(source.to_string_lossy().as_ref())
        .bind(&workspace_path)
        .execute(&pool)
        .await
        .unwrap();

    let unauthorized = service
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/projects/{id}/sync"),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let synced = service
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/projects/{id}/sync"),
            None,
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(synced.status(), StatusCode::OK);
    let snapshot: Value =
        serde_json::from_slice(&synced.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(snapshot["branch"], "main");
    assert_eq!(snapshot["commit_message"], "api sync");
    assert_eq!(snapshot["author"], "LiteCI Test");
    assert_eq!(snapshot["commit_sha"].as_str().unwrap().len(), 40);

    std::fs::remove_dir_all(root).unwrap();
}
