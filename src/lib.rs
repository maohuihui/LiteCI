mod config;
mod db;

pub use config::{Config, StorageConfig};

use axum::{Json, Router, routing::get};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "autoci",
    })
}

pub fn app() -> Router {
    Router::new().route("/health", get(health))
}

pub use db::{connect, migrate};

pub fn prepare_storage(storage: &StorageConfig) -> std::io::Result<()> {
    for directory in [&storage.workspace, &storage.artifacts, &storage.logs] {
        std::fs::create_dir_all(directory)?;
    }
    Ok(())
}
