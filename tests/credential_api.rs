use std::net::{Ipv4Addr, SocketAddr};

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use liteci::{CredentialCipher, app_with_setup_token_workspace_and_cipher, connect, migrate};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tower::ServiceExt;

async fn setup() -> (axum::Router, String) {
    let pool: SqlitePool = connect("sqlite::memory:").await.unwrap();
    migrate(&pool).await.unwrap();
    let service = app_with_setup_token_workspace_and_cipher(
        pool,
        "test-setup-token",
        ".",
        CredentialCipher::from_key_bytes(&[0_u8; 32]).unwrap(),
    );
    let request = Request::builder()
        .method("POST")
        .uri("/api/auth/setup")
        .extension(ConnectInfo(SocketAddr::new(
            Ipv4Addr::LOCALHOST.into(),
            43000,
        )))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "username": "admin",
                "password": "correct horse battery staple",
                "setup_token": "test-setup-token"
            })
            .to_string(),
        ))
        .unwrap();
    assert_eq!(
        service.clone().oneshot(request).await.unwrap().status(),
        StatusCode::CREATED
    );
    let request = Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .extension(ConnectInfo(SocketAddr::new(
            Ipv4Addr::LOCALHOST.into(),
            43000,
        )))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"username":"admin","password":"correct horse battery staple"}).to_string(),
        ))
        .unwrap();
    let body = service
        .clone()
        .oneshot(request)
        .await
        .unwrap()
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    (
        service,
        serde_json::from_slice::<Value>(&body).unwrap()["token"]
            .as_str()
            .unwrap()
            .to_owned(),
    )
}

fn request(method: &str, uri: &str, body: Option<Value>, token: &str) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .extension(ConnectInfo(SocketAddr::new(
            Ipv4Addr::LOCALHOST.into(),
            43000,
        )))
        .header(header::AUTHORIZATION, format!("Bearer {token}"));
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    builder
        .body(body.map_or_else(Body::empty, |v| Body::from(v.to_string())))
        .unwrap()
}

#[tokio::test]
async fn credential_api_requires_auth_and_never_returns_payload() {
    let (service, token) = setup().await;
    let unauthorized = service
        .clone()
        .oneshot(request("GET", "/api/credentials", None, "invalid"))
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let created = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/credentials",
            Some(
                json!({"name":"gitee","kind":"https_token","payload":"username=ci\\ntoken=secret"}),
            ),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let body: Value =
        serde_json::from_slice(&created.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["name"], "gitee");
    assert!(body.get("payload").is_none());
    let listed = service
        .clone()
        .oneshot(request("GET", "/api/credentials", None, &token))
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&listed.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(!body.to_string().contains("secret"));
    assert_eq!(body.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn project_round_trips_and_can_clear_a_credential_binding() {
    let (service, token) = setup().await;
    let credential = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/credentials",
            Some(json!({"name":"project-token","kind":"https_token","payload":"username=ci\ntoken=secret"})),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(credential.status(), StatusCode::CREATED);
    let credential: Value =
        serde_json::from_slice(&credential.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    let credential_id = credential["id"].as_str().unwrap().to_owned();

    let project = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/projects",
            Some(json!({"name":"credential-project","git_url":"https://example.com/repo.git","git_auth_id":credential_id})),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(project.status(), StatusCode::CREATED);
    let project: Value =
        serde_json::from_slice(&project.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(project["git_auth_id"], credential_id);
    let project_id = project["id"].as_str().unwrap();

    let cleared = service
        .clone()
        .oneshot(request(
            "PUT",
            &format!("/api/projects/{project_id}"),
            Some(json!({"git_auth_id":null})),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(cleared.status(), StatusCode::OK);
    let project: Value =
        serde_json::from_slice(&cleared.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(project["git_auth_id"].is_null());
}
