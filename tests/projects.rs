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

async fn authenticated_service() -> (axum::Router, String) {
    let pool = pool().await;
    let service = app_with_state(pool);
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
    let response = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/auth/login",
            Some(json!({"username":"admin","password":"correct horse battery staple"})),
            None,
        ))
        .await
        .unwrap();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let token = serde_json::from_slice::<Value>(&body).unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    (service, token)
}

#[tokio::test]
async fn project_crud_requires_authentication_and_round_trips() {
    let (service, token) = authenticated_service().await;
    let unauthorized = service
        .clone()
        .oneshot(request("GET", "/api/projects", None, None))
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let created = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/projects",
            Some(json!({"name":"lite-site","description":"LiteCI site","git_url":"https://example.com/lite-site.git","default_branch":"main"})),
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let body = created.into_body().collect().await.unwrap().to_bytes();
    let project: Value = serde_json::from_slice(&body).unwrap();
    let id = project["id"].as_str().unwrap().to_owned();
    assert_eq!(project["workspace_path"], format!("workspaces/{id}"));

    let listed = service
        .clone()
        .oneshot(request("GET", "/api/projects", None, Some(&token)))
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let projects: Value =
        serde_json::from_slice(&listed.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(projects.as_array().unwrap().len(), 1);

    let updated = service
        .clone()
        .oneshot(request(
            "PUT",
            &format!("/api/projects/{id}"),
            Some(json!({"description":"updated","status":"disabled"})),
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(updated.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&updated.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["description"], "updated");
    assert_eq!(body["status"], "disabled");

    let deleted = service
        .clone()
        .oneshot(request(
            "DELETE",
            &format!("/api/projects/{id}"),
            None,
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn project_validation_rejects_unsafe_or_duplicate_data() {
    let (service, token) = authenticated_service().await;
    let invalid = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/projects",
            Some(json!({"name":"../escape","git_url":"javascript:bad"})),
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let payload = json!({"name":"duplicate","git_url":"ssh://git@example.com/repo.git"});
    let first = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/projects",
            Some(payload.clone()),
            Some(&token),
        ))
        .await
        .unwrap();
    let second = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/projects",
            Some(payload),
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::CREATED);
    assert_eq!(second.status(), StatusCode::CONFLICT);

    for git_url in [
        "http://example.com/repo.git",
        "file:///tmp/repo",
        "https://user:token@example.com/repo.git",
        "https://",
    ] {
        let response = service
            .clone()
            .oneshot(request(
                "POST",
                "/api/projects",
                Some(json!({"name":format!("bad-{}", git_url.len()),"git_url":git_url})),
                Some(&token),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    let bad_branch = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/projects",
            Some(json!({"name":"bad-branch","git_url":"https://example.com/repo.git","default_branch":"--detach"})),
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(bad_branch.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let empty_update = service
        .clone()
        .oneshot(request(
            "PUT",
            "/api/projects/missing",
            Some(json!({})),
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(empty_update.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
