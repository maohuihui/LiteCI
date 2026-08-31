use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{AppState, auth};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineInput {
    stages: Vec<StageInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StageInput {
    name: String,
    command: StageCommand,
    enabled: bool,
    timeout_seconds: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum StageCommand {
    Process { program: String, args: Vec<String> },
    Shell { script: String },
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PipelineStage {
    id: String,
    position: i64,
    name: String,
    command: StageCommand,
    enabled: bool,
    timeout_seconds: i64,
}

#[derive(sqlx::FromRow)]
struct StoredPipelineStage {
    id: String,
    position: i64,
    name: String,
    command: String,
    enabled: bool,
    timeout_seconds: i64,
}

pub async fn put(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(input): Json<PipelineInput>,
) -> Result<Json<Vec<PipelineStage>>, PipelineError> {
    auth::authenticated_user(&state, &headers).await?;
    if input.stages.is_empty() || input.stages.len() > 64 {
        return Err(PipelineError::InvalidInput);
    }
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?1)")
        .bind(&project_id)
        .fetch_one(&state.pool)
        .await?;
    if !exists {
        return Err(PipelineError::NotFound);
    }
    for stage in &input.stages {
        if stage.name.trim().is_empty()
            || stage.name.len() > 64
            || !stage.command.is_valid()
            || !(1..=86_400).contains(&stage.timeout_seconds)
        {
            return Err(PipelineError::InvalidInput);
        }
    }

    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO pipeline_configs (project_id) VALUES (?1) \
         ON CONFLICT(project_id) DO UPDATE SET updated_at = CURRENT_TIMESTAMP",
    )
    .bind(&project_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM pipeline_stages WHERE project_id = ?1")
        .bind(&project_id)
        .execute(&mut *tx)
        .await?;
    for (position, stage) in input.stages.into_iter().enumerate() {
        sqlx::query("INSERT INTO pipeline_stages (id, project_id, position, name, command, enabled, timeout_seconds) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)")
            .bind(Uuid::new_v4().to_string())
            .bind(&project_id)
            .bind(i64::try_from(position).map_err(|_| PipelineError::InvalidInput)?)
            .bind(stage.name.trim())
            .bind(serde_json::to_string(&stage.command).map_err(|_| PipelineError::InvalidInput)?)
            .bind(stage.enabled)
            .bind(stage.timeout_seconds)
            .execute(&mut *tx)
            .await
            .map_err(map_db_error)?;
    }
    tx.commit().await?;
    Ok(Json(load(&state, &project_id).await?))
}

pub async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> Result<Json<Vec<PipelineStage>>, PipelineError> {
    auth::authenticated_user(&state, &headers).await?;
    Ok(Json(load(&state, &project_id).await?))
}

async fn load(state: &AppState, project_id: &str) -> Result<Vec<PipelineStage>, PipelineError> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?1)")
        .bind(project_id)
        .fetch_one(&state.pool)
        .await?;
    if !exists {
        return Err(PipelineError::NotFound);
    }
    let stages = sqlx::query_as::<_, StoredPipelineStage>(
        "SELECT id, position, name, command, enabled, timeout_seconds FROM pipeline_stages WHERE project_id = ?1 ORDER BY position",
    )
    .bind(project_id)
    .fetch_all(&state.pool)
    .await?;
    stages.into_iter().map(PipelineStage::try_from).collect()
}

impl StageCommand {
    fn is_valid(&self) -> bool {
        match self {
            Self::Process { program, args } => {
                !program.trim().is_empty()
                    && program.len() <= 4_096
                    && !program.contains('\0')
                    && args.len() <= 256
                    && args
                        .iter()
                        .all(|arg| arg.len() <= 16_384 && !arg.contains('\0'))
                    && program.len() + args.iter().map(String::len).sum::<usize>() <= 16_384
            }
            Self::Shell { script } => {
                !script.trim().is_empty() && script.len() <= 16_384 && !script.contains('\0')
            }
        }
    }
}

impl TryFrom<StoredPipelineStage> for PipelineStage {
    type Error = PipelineError;

    fn try_from(stage: StoredPipelineStage) -> Result<Self, Self::Error> {
        Ok(Self {
            id: stage.id,
            position: stage.position,
            name: stage.name,
            command: serde_json::from_str(&stage.command)
                .map_err(|error| PipelineError::CorruptCommand(error.to_string()))?,
            enabled: stage.enabled,
            timeout_seconds: stage.timeout_seconds,
        })
    }
}

fn map_db_error(error: sqlx::Error) -> PipelineError {
    if matches!(&error, sqlx::Error::Database(database) if database.message().contains("UNIQUE")) {
        PipelineError::InvalidInput
    } else {
        PipelineError::Database(error)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("会话无效")]
    Auth(#[from] auth::AuthError),
    #[error("项目不存在")]
    NotFound,
    #[error("Pipeline 配置无效")]
    InvalidInput,
    #[error("数据库中的 Stage 命令无效: {0}")]
    CorruptCommand(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

impl axum::response::IntoResponse for PipelineError {
    fn into_response(self) -> axum::response::Response {
        let (status, code, message) = match self {
            Self::Auth(auth::AuthError::InvalidSession) => (
                StatusCode::UNAUTHORIZED,
                "invalid_session",
                "会话无效或已过期",
            ),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", "项目不存在"),
            Self::InvalidInput => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_input",
                "Pipeline 配置无效",
            ),
            Self::Auth(_) | Self::CorruptCommand(_) | Self::Database(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "服务暂时无法完成请求",
            ),
        };
        (
            status,
            Json(serde_json::json!({"code":code,"message":message})),
        )
            .into_response()
    }
}
