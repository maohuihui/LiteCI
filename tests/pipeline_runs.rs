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
    let (service, token, _) = authenticated_service_with_pool().await;
    (service, token)
}

async fn authenticated_service_with_pool() -> (axum::Router, String, SqlitePool) {
    let pool = connect("sqlite::memory:").await.unwrap();
    migrate(&pool).await.unwrap();
    let service = app_with_state(pool.clone());
    assert_eq!(
        service
            .clone()
            .oneshot(request(
                "POST",
                "/api/auth/setup",
                Some(json!({"username":"admin","password":"correct horse battery staple","setup_token":"test-setup-token"})),
                None,
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
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

#[tokio::test]
async fn creates_and_lists_pending_manual_pipeline_runs() {
    let (service, token) = authenticated_service().await;
    let project = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/projects",
            Some(json!({"name":"run-project","git_url":"https://example.com/repo.git"})),
            Some(&token),
        ))
        .await
        .unwrap();
    let project: Value =
        serde_json::from_slice(&project.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let project_id = project["id"].as_str().unwrap();

    let created = service
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/projects/{project_id}/runs"),
            Some(json!({"branch":"main"})),
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let run: Value =
        serde_json::from_slice(&created.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(run["project_id"], project_id);
    assert_eq!(run["branch"], "main");
    assert_eq!(run["trigger_type"], "manual");
    assert_eq!(run["status"], "pending");
    assert!(run["run_number"].as_i64().unwrap() >= 1);

    let listed = service
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/projects/{project_id}/runs"),
            None,
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let runs: Value =
        serde_json::from_slice(&listed.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(runs.as_array().unwrap().len(), 1);
    assert_eq!(runs[0]["id"], run["id"]);
}

#[tokio::test]
async fn run_history_is_bounded_and_supports_pagination() {
    let (service, token) = authenticated_service().await;
    let project = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/projects",
            Some(json!({"name":"paginated-runs","git_url":"https://example.com/repo.git"})),
            Some(&token),
        ))
        .await
        .unwrap();
    let project: Value =
        serde_json::from_slice(&project.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let project_id = project["id"].as_str().unwrap();
    for _ in 0..3 {
        let created = service
            .clone()
            .oneshot(request(
                "POST",
                &format!("/api/projects/{project_id}/runs"),
                Some(json!({})),
                Some(&token),
            ))
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
    }

    let page = service
        .oneshot(request(
            "GET",
            &format!("/api/projects/{project_id}/runs?limit=1&offset=1"),
            None,
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let page: Value =
        serde_json::from_slice(&page.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(page.as_array().unwrap().len(), 1);
    assert_eq!(page[0]["run_number"], 2);
}

#[tokio::test]
async fn concurrent_run_creation_assigns_unique_monotonic_numbers() {
    let (service, token) = authenticated_service().await;
    let project = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/projects",
            Some(json!({"name":"concurrent-runs","git_url":"https://example.com/repo.git"})),
            Some(&token),
        ))
        .await
        .unwrap();
    let project: Value =
        serde_json::from_slice(&project.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let project_id = project["id"].as_str().unwrap().to_owned();

    let mut requests = tokio::task::JoinSet::new();
    for _ in 0..16 {
        let service = service.clone();
        let token = token.clone();
        let uri = format!("/api/projects/{project_id}/runs");
        requests.spawn(async move {
            service
                .oneshot(request(
                    "POST",
                    &uri,
                    Some(json!({"branch":"main"})),
                    Some(&token),
                ))
                .await
                .unwrap()
        });
    }

    let mut run_numbers = Vec::new();
    while let Some(response) = requests.join_next().await {
        let response = response.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let run: Value = serde_json::from_slice(&body).unwrap();
        run_numbers.push(run["run_number"].as_i64().unwrap());
    }
    run_numbers.sort_unstable();
    assert_eq!(run_numbers, (1..=16).collect::<Vec<_>>());
}

#[tokio::test]
async fn pipeline_run_endpoints_require_authentication() {
    let (service, token) = authenticated_service().await;
    let project = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/projects",
            Some(json!({"name":"protected-runs","git_url":"https://example.com/repo.git"})),
            Some(&token),
        ))
        .await
        .unwrap();
    let project: Value =
        serde_json::from_slice(&project.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let project_id = project["id"].as_str().unwrap();

    let response = service
        .oneshot(request(
            "GET",
            &format!("/api/projects/{project_id}/runs"),
            None,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn creating_a_run_for_a_missing_project_returns_not_found() {
    let (service, token) = authenticated_service().await;

    let response = service
        .oneshot(request(
            "POST",
            "/api/projects/missing-project/runs",
            Some(json!({})),
            Some(&token),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn creating_a_run_rejects_refs_that_git_would_reject() {
    let (service, token) = authenticated_service().await;
    let project = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/projects",
            Some(json!({"name":"invalid-run-refs","git_url":"https://example.com/repo.git"})),
            Some(&token),
        ))
        .await
        .unwrap();
    let project: Value =
        serde_json::from_slice(&project.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let project_id = project["id"].as_str().unwrap();

    for branch in ["/main", "main/", "main."] {
        let response = service
            .clone()
            .oneshot(request(
                "POST",
                &format!("/api/projects/{project_id}/runs"),
                Some(json!({"branch":branch})),
                Some(&token),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}

#[tokio::test]
async fn gets_cancels_and_retries_pipeline_runs() {
    let (service, token) = authenticated_service().await;
    let project = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/projects",
            Some(json!({"name":"run-lifecycle","git_url":"https://example.com/repo.git"})),
            Some(&token),
        ))
        .await
        .unwrap();
    let project: Value =
        serde_json::from_slice(&project.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let project_id = project["id"].as_str().unwrap();
    let created = service
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/projects/{project_id}/runs"),
            Some(json!({"branch":"main"})),
            Some(&token),
        ))
        .await
        .unwrap();
    let run: Value =
        serde_json::from_slice(&created.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let run_id = run["id"].as_str().unwrap();

    let fetched = service
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/runs/{run_id}"),
            None,
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(fetched.status(), StatusCode::OK);

    let cancelled = service
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/runs/{run_id}/cancel"),
            None,
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(cancelled.status(), StatusCode::OK);
    let cancelled: Value =
        serde_json::from_slice(&cancelled.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(cancelled["status"], "cancelled");
    assert!(!cancelled["finished_at"].is_null());

    let retried = service
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/runs/{run_id}/retry"),
            None,
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(retried.status(), StatusCode::CREATED);
    let retried: Value =
        serde_json::from_slice(&retried.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_ne!(retried["id"], run["id"]);
    assert_eq!(retried["project_id"], project_id);
    assert_eq!(retried["branch"], "main");
    assert_eq!(retried["status"], "pending");
    assert_eq!(retried["retry_of_run_id"], run_id);
}

#[tokio::test]
async fn a_terminal_run_supports_multiple_manual_retries_with_lineage() {
    let (service, token) = authenticated_service().await;
    let project = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/projects",
            Some(json!({"name":"multiple-retries","git_url":"https://example.com/repo.git"})),
            Some(&token),
        ))
        .await
        .unwrap();
    let project: Value =
        serde_json::from_slice(&project.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let project_id = project["id"].as_str().unwrap();
    let created = service
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/projects/{project_id}/runs"),
            Some(json!({})),
            Some(&token),
        ))
        .await
        .unwrap();
    let run: Value =
        serde_json::from_slice(&created.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let run_id = run["id"].as_str().unwrap();
    let cancelled = service
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/runs/{run_id}/cancel"),
            None,
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(cancelled.status(), StatusCode::OK);

    let mut retry_ids = Vec::new();
    for _ in 0..2 {
        let retried = service
            .clone()
            .oneshot(request(
                "POST",
                &format!("/api/runs/{run_id}/retry"),
                None,
                Some(&token),
            ))
            .await
            .unwrap();
        assert_eq!(retried.status(), StatusCode::CREATED);
        let retried: Value =
            serde_json::from_slice(&retried.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(retried["retry_of_run_id"], run_id);
        retry_ids.push(retried["id"].as_str().unwrap().to_owned());
    }
    assert_ne!(retry_ids[0], retry_ids[1]);
}

#[tokio::test]
async fn run_operations_reject_invalid_state_transitions() {
    let (service, token) = authenticated_service().await;
    let project = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/projects",
            Some(json!({"name":"invalid-transitions","git_url":"https://example.com/repo.git"})),
            Some(&token),
        ))
        .await
        .unwrap();
    let project: Value =
        serde_json::from_slice(&project.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let project_id = project["id"].as_str().unwrap();
    let created = service
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/projects/{project_id}/runs"),
            Some(json!({})),
            Some(&token),
        ))
        .await
        .unwrap();
    let run: Value =
        serde_json::from_slice(&created.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let run_id = run["id"].as_str().unwrap();

    let retry = service
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/runs/{run_id}/retry"),
            None,
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(retry.status(), StatusCode::CONFLICT);

    let cancel = service
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/runs/{run_id}/cancel"),
            None,
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(cancel.status(), StatusCode::OK);

    let second_cancel = service
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/runs/{run_id}/cancel"),
            None,
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(second_cancel.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn cancelling_a_pending_run_cancels_its_pending_stage_snapshots() {
    let (service, token) = authenticated_service().await;
    let project = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/projects",
            Some(json!({"name":"cancel-stage-snapshots","git_url":"https://example.com/repo.git"})),
            Some(&token),
        ))
        .await
        .unwrap();
    let project: Value =
        serde_json::from_slice(&project.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let project_id = project["id"].as_str().unwrap();
    let configured = service
        .clone()
        .oneshot(request(
            "PUT",
            &format!("/api/projects/{project_id}/pipeline"),
            Some(json!({"stages":[
                {"name":"build","command":{"mode":"process","program":"cargo","args":["build"]},"enabled":true,"timeout_seconds":600},
                {"name":"deploy","command":{"mode":"process","program":"deploy","args":[]},"enabled":false,"timeout_seconds":300}
            ]})),
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(configured.status(), StatusCode::OK);
    let created = service
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/projects/{project_id}/runs"),
            Some(json!({})),
            Some(&token),
        ))
        .await
        .unwrap();
    let run: Value =
        serde_json::from_slice(&created.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let run_id = run["id"].as_str().unwrap();
    let cancelled = service
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/runs/{run_id}/cancel"),
            None,
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(cancelled.status(), StatusCode::OK);

    let stages = service
        .oneshot(request(
            "GET",
            &format!("/api/runs/{run_id}/stages"),
            None,
            Some(&token),
        ))
        .await
        .unwrap();
    let stages: Value =
        serde_json::from_slice(&stages.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(stages[0]["status"], "cancelled");
    assert_eq!(stages[1]["status"], "skipped");
}

#[tokio::test]
async fn snapshots_configured_stages_when_a_run_is_created() {
    let (service, token) = authenticated_service().await;
    let project = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/projects",
            Some(json!({"name":"staged-pipeline","git_url":"https://example.com/repo.git"})),
            Some(&token),
        ))
        .await
        .unwrap();
    let project: Value =
        serde_json::from_slice(&project.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let project_id = project["id"].as_str().unwrap();

    let configured = service
        .clone()
        .oneshot(request(
            "PUT",
            &format!("/api/projects/{project_id}/pipeline"),
            Some(json!({"stages":[
                {"name":"build","command":{"mode":"process","program":"cargo","args":["build"]},"enabled":true,"timeout_seconds":600},
                {"name":"deploy","command":{"mode":"process","program":"deploy","args":[]},"enabled":false,"timeout_seconds":300}
            ]})),
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(configured.status(), StatusCode::OK);

    let created = service
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/projects/{project_id}/runs"),
            Some(json!({"branch":"main"})),
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let run: Value =
        serde_json::from_slice(&created.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let run_id = run["id"].as_str().unwrap();

    let stages = service
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/runs/{run_id}/stages"),
            None,
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(stages.status(), StatusCode::OK);
    let stages: Value =
        serde_json::from_slice(&stages.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(stages.as_array().unwrap().len(), 2);
    assert_eq!(stages[0]["name"], "build");
    assert_eq!(stages[0]["status"], "pending");
    assert_eq!(stages[0]["position"], 0);
    assert_eq!(stages[1]["name"], "deploy");
    assert_eq!(stages[1]["status"], "skipped");
    assert_eq!(stages[1]["position"], 1);
}

#[tokio::test]
async fn retry_copies_the_original_stage_snapshot() {
    let (service, token) = authenticated_service().await;
    let project = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/projects",
            Some(json!({"name":"retry-snapshot","git_url":"https://example.com/repo.git"})),
            Some(&token),
        ))
        .await
        .unwrap();
    let project: Value =
        serde_json::from_slice(&project.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let project_id = project["id"].as_str().unwrap();

    let response = service
        .clone()
        .oneshot(request(
            "PUT",
            &format!("/api/projects/{project_id}/pipeline"),
            Some(json!({"stages":[
                {"name":"build","command":{"mode":"process","program":"cargo","args":["build","--release"]},"enabled":true,"timeout_seconds":600},
                {"name":"deploy","command":{"mode":"process","program":"deploy-v1","args":[]},"enabled":false,"timeout_seconds":300}
            ]})),
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let created = service
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/projects/{project_id}/runs"),
            Some(json!({"branch":"main"})),
            Some(&token),
        ))
        .await
        .unwrap();
    let run: Value =
        serde_json::from_slice(&created.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let run_id = run["id"].as_str().unwrap();

    let response = service
        .clone()
        .oneshot(request(
            "PUT",
            &format!("/api/projects/{project_id}/pipeline"),
            Some(json!({"stages":[
                {"name":"replacement","command":{"mode":"process","program":"cargo","args":["test"]},"enabled":true,"timeout_seconds":42}
            ]})),
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let cancelled = service
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/runs/{run_id}/cancel"),
            None,
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(cancelled.status(), StatusCode::OK);
    let retried = service
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/runs/{run_id}/retry"),
            None,
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(retried.status(), StatusCode::CREATED);
    let retried: Value =
        serde_json::from_slice(&retried.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let retried_id = retried["id"].as_str().unwrap();

    let stages = service
        .oneshot(request(
            "GET",
            &format!("/api/runs/{retried_id}/stages"),
            None,
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(stages.status(), StatusCode::OK);
    let stages: Value =
        serde_json::from_slice(&stages.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(stages.as_array().unwrap().len(), 2);
    assert_eq!(stages[0]["name"], "build");
    assert_eq!(stages[0]["command"]["mode"], "process");
    assert_eq!(stages[0]["command"]["program"], "cargo");
    assert_eq!(stages[0]["command"]["args"], json!(["build", "--release"]));
    assert_eq!(stages[0]["status"], "pending");
    assert_eq!(stages[0]["enabled"], true);
    assert_eq!(stages[0]["timeout_seconds"], 600);
    assert_eq!(stages[1]["name"], "deploy");
    assert_eq!(stages[1]["command"]["mode"], "process");
    assert_eq!(stages[1]["command"]["program"], "deploy-v1");
    assert_eq!(stages[1]["command"]["args"], json!([]));
    assert_eq!(stages[1]["status"], "skipped");
    assert_eq!(stages[1]["enabled"], false);
    assert_eq!(stages[1]["timeout_seconds"], 300);
}

#[tokio::test]
async fn retry_uses_enabled_snapshot_instead_of_previous_runtime_status() {
    let (service, token, pool) = authenticated_service_with_pool().await;
    let project = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/projects",
            Some(json!({"name":"retry-enabled-snapshot","git_url":"https://example.com/repo.git"})),
            Some(&token),
        ))
        .await
        .unwrap();
    let project: Value =
        serde_json::from_slice(&project.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let project_id = project["id"].as_str().unwrap();
    let configured = service
        .clone()
        .oneshot(request(
            "PUT",
            &format!("/api/projects/{project_id}/pipeline"),
            Some(json!({"stages":[
                {"name":"build","command":{"mode":"process","program":"cargo","args":["build"]},"enabled":true,"timeout_seconds":600}
            ]})),
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(configured.status(), StatusCode::OK);
    let created = service
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/projects/{project_id}/runs"),
            Some(json!({})),
            Some(&token),
        ))
        .await
        .unwrap();
    let run: Value =
        serde_json::from_slice(&created.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let run_id = run["id"].as_str().unwrap();
    sqlx::query("UPDATE stage_runs SET status = 'skipped' WHERE run_id = ?1")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE pipeline_runs SET status = 'failed' WHERE id = ?1")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();

    let retried = service
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/runs/{run_id}/retry"),
            None,
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(retried.status(), StatusCode::CREATED);
    let retried: Value =
        serde_json::from_slice(&retried.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let stages = service
        .oneshot(request(
            "GET",
            &format!("/api/runs/{}/stages", retried["id"].as_str().unwrap()),
            None,
            Some(&token),
        ))
        .await
        .unwrap();
    let stages: Value =
        serde_json::from_slice(&stages.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(stages[0]["enabled"], true);
    assert_eq!(stages[0]["status"], "pending");
}

#[tokio::test]
async fn pipeline_commands_have_explicit_process_or_shell_boundaries() {
    let (service, token) = authenticated_service().await;
    let project = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/projects",
            Some(json!({"name":"command-boundary","git_url":"https://example.com/repo.git"})),
            Some(&token),
        ))
        .await
        .unwrap();
    let project: Value =
        serde_json::from_slice(&project.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let project_id = project["id"].as_str().unwrap();

    let configured = service
        .clone()
        .oneshot(request(
            "PUT",
            &format!("/api/projects/{project_id}/pipeline"),
            Some(json!({"stages":[
                {"name":"build","command":{"mode":"process","program":"cargo","args":["build","--release"]},"enabled":true,"timeout_seconds":600},
                {"name":"package","command":{"mode":"shell","script":"cargo build && package"},"enabled":true,"timeout_seconds":300}
            ]})),
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(configured.status(), StatusCode::OK);
    let configured: Value =
        serde_json::from_slice(&configured.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(configured[0]["command"]["mode"], "process");
    assert_eq!(configured[0]["command"]["program"], "cargo");
    assert_eq!(
        configured[0]["command"]["args"],
        json!(["build", "--release"])
    );
    assert_eq!(configured[1]["command"]["mode"], "shell");
    assert_eq!(configured[1]["command"]["script"], "cargo build && package");
}

#[tokio::test]
async fn replacing_pipeline_configuration_updates_its_timestamp() {
    let (service, token, pool) = authenticated_service_with_pool().await;
    let project = service
        .clone()
        .oneshot(request(
            "POST",
            "/api/projects",
            Some(json!({"name":"pipeline-timestamp","git_url":"https://example.com/repo.git"})),
            Some(&token),
        ))
        .await
        .unwrap();
    let project: Value =
        serde_json::from_slice(&project.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let project_id = project["id"].as_str().unwrap();
    let pipeline = json!({"stages":[
        {"name":"build","command":{"mode":"process","program":"cargo","args":["build"]},"enabled":true,"timeout_seconds":600}
    ]});
    let first = service
        .clone()
        .oneshot(request(
            "PUT",
            &format!("/api/projects/{project_id}/pipeline"),
            Some(pipeline.clone()),
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    sqlx::query(
        "UPDATE pipeline_configs SET updated_at = '2000-01-01 00:00:00' WHERE project_id = ?1",
    )
    .bind(project_id)
    .execute(&pool)
    .await
    .unwrap();
    let second = service
        .oneshot(request(
            "PUT",
            &format!("/api/projects/{project_id}/pipeline"),
            Some(pipeline),
            Some(&token),
        ))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    let updated_at: String =
        sqlx::query_scalar("SELECT updated_at FROM pipeline_configs WHERE project_id = ?1")
            .bind(project_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_ne!(updated_at, "2000-01-01 00:00:00");
}
