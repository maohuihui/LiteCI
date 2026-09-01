use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};

use crate::{AppState, auth, pipeline_runs::RunError};

#[derive(Debug, Deserialize)]
pub struct LogPage {
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default)]
    offset: u32,
}

const fn default_limit() -> u32 {
    500
}

#[derive(Debug, Serialize)]
pub struct StageLog {
    pub id: String,
    pub stage_run_id: String,
    pub sequence: i64,
    pub stream: String,
    pub data: String,
    pub created_at: String,
}

#[derive(Debug, sqlx::FromRow)]
struct StoredStageLog {
    id: String,
    stage_run_id: String,
    sequence: i64,
    stream: String,
    data: Vec<u8>,
    created_at: String,
}

pub async fn logs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run_id): Path<String>,
    Query(page): Query<LogPage>,
) -> Result<Json<Vec<StageLog>>, RunError> {
    auth::authenticated_user(&state, &headers).await?;
    if page.limit == 0 || page.limit > 1000 || page.offset > i32::MAX as u32 {
        return Err(RunError::InvalidInput);
    }
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pipeline_runs WHERE id = ?1)")
            .bind(&run_id)
            .fetch_one(&state.pool)
            .await?;
    if !exists {
        return Err(RunError::NotFound);
    }
    let rows = sqlx::query_as::<_, StoredStageLog>(
        "SELECT l.id, l.stage_run_id, l.sequence, l.stream, l.data, l.created_at FROM pipeline_stage_logs l JOIN stage_runs s ON s.id = l.stage_run_id WHERE s.run_id = ?1 ORDER BY s.position, l.sequence, l.id LIMIT ?2 OFFSET ?3",
    )
    .bind(run_id)
    .bind(i64::from(page.limit))
    .bind(i64::from(page.offset))
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|log| StageLog {
                id: log.id,
                stage_run_id: log.stage_run_id,
                sequence: log.sequence,
                stream: log.stream,
                data: String::from_utf8_lossy(&log.data).into_owned(),
                created_at: log.created_at,
            })
            .collect(),
    ))
}
