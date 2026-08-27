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

async fn test_pool() -> SqlitePool {
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
            42000,
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

async fn login(pool: &SqlitePool) -> String {
    let service = app_with_state(pool.clone());
    service
        .clone()
        .oneshot(request(
            "POST",
            "/api/auth/setup",
            Some(json!({"username":"admin","password":"correct horse battery staple","setup_token":"test-setup-token"})),
            None,
        ))
        .await
        .unwrap();
    let response = service
        .oneshot(request(
            "POST",
            "/api/auth/login",
            Some(json!({"username":"admin","password":"correct horse battery staple"})),
            None,
        ))
        .await
        .unwrap();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice::<Value>(&body).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn current_user_requires_a_valid_bearer_session() {
    let pool = test_pool().await;
    let service = app_with_state(pool.clone());

    let missing = service
        .clone()
        .oneshot(request("GET", "/api/auth/me", None, None))
        .await
        .unwrap();
    let invalid = service
        .clone()
        .oneshot(request("GET", "/api/auth/me", None, Some("invalid")))
        .await
        .unwrap();
    let token = login(&pool).await;
    let valid = service
        .oneshot(request("GET", "/api/auth/me", None, Some(&token)))
        .await
        .unwrap();

    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(valid.status(), StatusCode::OK);
    let body = valid.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["username"], "admin");
    assert_eq!(body["role"], "admin");
}

#[tokio::test]
async fn expired_session_is_rejected() {
    let pool = test_pool().await;
    let token = login(&pool).await;
    sqlx::query("UPDATE sessions SET expires_at = '2000-01-01T00:00:00Z'")
        .execute(&pool)
        .await
        .unwrap();

    let response = app_with_state(pool)
        .oneshot(request("GET", "/api/auth/me", None, Some(&token)))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn logout_revokes_only_the_presented_session() {
    let pool = test_pool().await;
    let token = login(&pool).await;
    let service = app_with_state(pool);

    let logout = service
        .clone()
        .oneshot(request("POST", "/api/auth/logout", None, Some(&token)))
        .await
        .unwrap();
    let after = service
        .oneshot(request("GET", "/api/auth/me", None, Some(&token)))
        .await
        .unwrap();

    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    assert_eq!(after.status(), StatusCode::UNAUTHORIZED);
}
