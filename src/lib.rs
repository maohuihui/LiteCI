mod auth;
mod command_executor;
mod config;
mod db;
mod projects;

pub use command_executor::{
    CommandError, CommandExecutor, CommandOutput, CommandSpec, ExecutionStatus,
};
pub use config::{Config, StorageConfig};

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
}

impl AppState {
    fn new(pool: SqlitePool, setup_token: impl Into<Arc<str>>) -> Self {
        Self {
            pool,
            login_limiter: Arc::new(auth::LoginLimiter::default()),
            password_workers: Arc::new(Semaphore::new(4)),
            setup_token: setup_token.into(),
        }
    }
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
    app_with_setup_token(pool, "test-setup-token")
}

pub fn app_with_setup_token(pool: SqlitePool, setup_token: impl Into<Arc<str>>) -> Router {
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
        .with_state(AppState::new(pool, setup_token))
}

pub use db::{connect, migrate};

pub fn prepare_storage(storage: &StorageConfig) -> std::io::Result<()> {
    for directory in [&storage.workspace, &storage.artifacts, &storage.logs] {
        std::fs::create_dir_all(directory)?;
    }
    Ok(())
}
