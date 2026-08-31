mod auth;
mod command_executor;
mod config;
mod credential_api;
mod credential_store;
mod credentials;
mod db;
mod git;
mod projects;

pub use command_executor::{
    CommandError, CommandExecutor, CommandOutput, CommandSpec, ExecutionStatus, LogEvent, LogStream,
};
pub use config::{Config, StorageConfig};
pub use credential_store::{
    CredentialKind, CredentialStore, CredentialStoreError, CredentialSummary, NewCredential,
};
pub use credentials::{CredentialCipher, CredentialError};
pub use git::{GitCredential, GitError, GitService, GitSnapshot};

use axum::{
    Json, Router,
    routing::{get, post},
};
use serde::Serialize;
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct AppState {
    pool: SqlitePool,
    login_limiter: Arc<auth::LoginLimiter>,
    password_workers: Arc<Semaphore>,
    setup_token: Arc<str>,
    workspace_root: Arc<std::path::PathBuf>,
    pub(crate) credentials: Option<Arc<CredentialStore>>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "liteci",
    })
}

pub fn app() -> Router {
    Router::new().route("/health", get(health))
}

pub fn app_with_state(pool: SqlitePool) -> Router {
    app_with_setup_token_and_workspace(pool, "test-setup-token", ".")
}

pub fn app_with_setup_token(pool: SqlitePool, setup_token: impl Into<Arc<str>>) -> Router {
    app_with_setup_token_and_workspace(pool, setup_token, ".")
}

pub fn app_with_setup_token_and_workspace(
    pool: SqlitePool,
    setup_token: impl Into<Arc<str>>,
    workspace_root: impl Into<std::path::PathBuf>,
) -> Router {
    let state = AppState {
        pool,
        login_limiter: Arc::new(auth::LoginLimiter::default()),
        password_workers: Arc::new(Semaphore::new(4)),
        setup_token: setup_token.into(),
        workspace_root: Arc::new(workspace_root.into()),
        credentials: None,
    };
    Router::new()
        .route("/health", get(health))
        .route("/api/auth/setup", post(auth::setup))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/me", get(auth::current_user))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/projects", get(projects::list).post(projects::create))
        .route(
            "/api/projects/{id}",
            get(projects::get)
                .put(projects::update)
                .delete(projects::delete),
        )
        .route("/api/projects/{id}/sync", post(projects::sync))
        .with_state(state)
}

pub fn app_with_setup_token_workspace_and_cipher(
    pool: SqlitePool,
    setup_token: impl Into<Arc<str>>,
    workspace_root: impl Into<std::path::PathBuf>,
    cipher: CredentialCipher,
) -> Router {
    let credentials = Arc::new(CredentialStore::new(pool.clone(), cipher));
    Router::new()
        .route("/health", get(health))
        .route("/api/auth/setup", post(auth::setup))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/me", get(auth::current_user))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/projects", get(projects::list).post(projects::create))
        .route(
            "/api/projects/{id}",
            get(projects::get)
                .put(projects::update)
                .delete(projects::delete),
        )
        .route("/api/projects/{id}/sync", post(projects::sync))
        .route(
            "/api/credentials",
            get(credential_api::list).post(credential_api::create),
        )
        .route(
            "/api/credentials/{id}",
            axum::routing::delete(credential_api::delete),
        )
        .with_state(AppState {
            pool,
            login_limiter: Arc::new(auth::LoginLimiter::default()),
            password_workers: Arc::new(Semaphore::new(4)),
            setup_token: setup_token.into(),
            workspace_root: Arc::new(workspace_root.into()),
            credentials: Some(credentials),
        })
}

pub use db::{connect, migrate};

pub fn prepare_storage(storage: &StorageConfig) -> std::io::Result<()> {
    for directory in [&storage.workspace, &storage.artifacts, &storage.logs] {
        std::fs::create_dir_all(directory)?;
    }
    Ok(())
}
