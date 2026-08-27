use std::net::{IpAddr, Ipv4Addr, SocketAddr};

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

async fn test_pool() -> SqlitePool {
    let pool = connect("sqlite::memory:").await.unwrap();
    migrate(&pool).await.unwrap();
    pool
}

async fn json_request(
    pool: &SqlitePool,
    method: &str,
    uri: &str,
    body: Value,
) -> axum::response::Response {
    json_request_from(pool, method, uri, body, Ipv4Addr::LOCALHOST.into()).await
}

async fn json_request_from(
    pool: &SqlitePool,
    method: &str,
    uri: &str,
    body: Value,
    ip: IpAddr,
) -> axum::response::Response {
    app_with_state(pool.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/json")
                .extension(ConnectInfo(SocketAddr::new(ip, 41000)))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn setup_creates_first_admin_without_storing_plaintext_password() {
    let pool = test_pool().await;
    let response = json_request(
        &pool,
        "POST",
        "/api/auth/setup",
        json!({"username":"admin","password":"correct horse battery staple","setup_token":"test-setup-token"}),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CREATED);
    let (username, password_hash, role): (String, String, String) =
        sqlx::query_as("SELECT username, password_hash, role FROM users")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(username, "admin");
    assert_eq!(role, "admin");
    assert!(password_hash.starts_with("$argon2id$"));
    assert!(!password_hash.contains("correct horse battery staple"));
}

#[tokio::test]
async fn setup_accepts_valid_token_through_a_remote_proxy_address() {
    let pool = test_pool().await;
    let response = json_request_from(
        &pool,
        "POST",
        "/api/auth/setup",
        json!({"username":"admin","password":"correct horse battery staple","setup_token":"test-setup-token"}),
        Ipv4Addr::new(192, 0, 2, 1).into(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn setup_is_disabled_after_the_first_user() {
    let pool = test_pool().await;
    let payload = json!({"username":"admin","password":"correct horse battery staple","setup_token":"test-setup-token"});
    let first = json_request(&pool, "POST", "/api/auth/setup", payload.clone()).await;
    let second = json_request(&pool, "POST", "/api/auth/setup", payload).await;

    assert_eq!(first.status(), StatusCode::CREATED);
    assert_eq!(second.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn login_returns_a_session_token_only_for_valid_credentials() {
    let pool = test_pool().await;
    let credentials = json!({"username":"admin","password":"correct horse battery staple","setup_token":"test-setup-token"});
    json_request(&pool, "POST", "/api/auth/setup", credentials.clone()).await;

    let success = json_request(&pool, "POST", "/api/auth/login", credentials).await;
    assert_eq!(success.status(), StatusCode::OK);
    assert_eq!(success.headers()[header::CACHE_CONTROL], "no-store");
    let body = success.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let token = body["token"].as_str().unwrap();
    assert_eq!(token.len(), 64);
    assert!(body["expires_at"].as_str().is_some());

    let stored: (String,) = sqlx::query_as("SELECT token_hash FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored.0.len(), 64);
    assert_ne!(stored.0, token);

    let failure = json_request(
        &pool,
        "POST",
        "/api/auth/login",
        json!({"username":"admin","password":"wrong password"}),
    )
    .await;
    assert_eq!(failure.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn setup_rejects_invalid_input() {
    let pool = test_pool().await;
    let response = json_request(
        &pool,
        "POST",
        "/api/auth/setup",
        json!({"username":" ","password":"short","setup_token":"test-setup-token"}),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn login_rate_limits_repeated_failures_by_client() {
    let pool = test_pool().await;
    let service = app_with_state(pool.clone());
    let setup = Request::builder()
        .method("POST")
        .uri("/api/auth/setup")
        .header(header::CONTENT_TYPE, "application/json")
        .extension(ConnectInfo(SocketAddr::new(
            Ipv4Addr::LOCALHOST.into(),
            41000,
        )))
        .body(Body::from(
            json!({"username":"admin","password":"correct horse battery staple","setup_token":"test-setup-token"}).to_string(),
        ))
        .unwrap();
    service.clone().oneshot(setup).await.unwrap();

    let mut last_status = StatusCode::UNAUTHORIZED;
    for _ in 0..6 {
        let request = Request::builder()
            .method("POST")
            .uri("/api/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .extension(ConnectInfo(SocketAddr::new(
                Ipv4Addr::LOCALHOST.into(),
                41000,
            )))
            .body(Body::from(
                json!({"username":"admin","password":"wrong password"}).to_string(),
            ))
            .unwrap();
        last_status = service.clone().oneshot(request).await.unwrap().status();
    }

    assert_eq!(last_status, StatusCode::TOO_MANY_REQUESTS);
}
