use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::Serialize;

use crate::{AppState, auth, pipeline_runs::RunError};

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PipelineArtifact {
    pub id: String,
    pub run_id: String,
    pub project_id: String,
    pub name: String,
    pub relative_path: String,
    pub file_name: String,
    pub size_bytes: i64,
    pub checksum_sha256: String,
    pub created_at: String,
}

pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run_id): Path<String>,
) -> Result<Json<Vec<PipelineArtifact>>, RunError> {
    auth::authenticated_user(&state, &headers).await?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pipeline_runs WHERE id = ?1)")
            .bind(&run_id)
            .fetch_one(&state.pool)
            .await?;
    if !exists {
        return Err(RunError::NotFound);
    }
    let artifacts = sqlx::query_as::<_, PipelineArtifact>(
        "SELECT id, run_id, project_id, name, relative_path, file_name, size_bytes, checksum_sha256, created_at FROM pipeline_artifacts WHERE run_id = ?1 ORDER BY name, id",
    )
    .bind(run_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(artifacts))
}
