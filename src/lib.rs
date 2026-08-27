mod auth;
mod config;
mod db;

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
        .with_state(AppState::new(pool, setup_token))
}

pub use db::{connect, migrate};

pub fn prepare_storage(storage: &StorageConfig) -> std::io::Result<()> {
    for directory in [&storage.workspace, &storage.artifacts, &storage.logs] {
        std::fs::create_dir_all(directory)?;
    }
    Ok(())
}
