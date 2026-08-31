use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{AppState, auth};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRun {
    branch: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunPage {
    #[serde(default = "default_page_limit")]
    limit: u32,
    #[serde(default)]
    offset: u32,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PipelineRun {
    pub id: String,
    pub project_id: String,
    pub run_number: i64,
    pub branch: String,
    pub commit_sha: Option<String>,
    pub trigger_type: String,
    pub status: String,
    pub retry_of_run_id: Option<String>,
    pub created_by: String,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StageRun {
    pub id: String,
    pub run_id: String,
    pub position: i64,
    pub name: String,
    pub command: serde_json::Value,
    pub enabled: bool,
    pub status: String,
    pub timeout_seconds: i64,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(sqlx::FromRow)]
struct StoredStageRun {
    id: String,
    run_id: String,
    position: i64,
    name: String,
    command: String,
    enabled: bool,
    status: String,
    timeout_seconds: i64,
    created_at: String,
    started_at: Option<String>,
    finished_at: Option<String>,
}

pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(input): Json<CreateRun>,
) -> Result<(StatusCode, Json<PipelineRun>), RunError> {
    let (user, _) = auth::authenticated_user(&state, &headers).await?;
    let default_branch: Option<String> =
        sqlx::query_scalar("SELECT default_branch FROM projects WHERE id = ?1")
            .bind(&project_id)
            .fetch_optional(&state.pool)
            .await?;
    let default_branch = default_branch.ok_or(RunError::ProjectNotFound)?;
    let branch = input.branch.as_deref().unwrap_or(&default_branch);
    if !valid_ref(branch) {
        return Err(RunError::InvalidInput);
    }

    let mut transaction = state.pool.begin().await?;
    let run_number = allocate_run_number(&mut transaction, &project_id).await?;
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO pipeline_runs (id, project_id, run_number, branch, created_by) VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(&id)
    .bind(&project_id)
    .bind(run_number)
    .bind(branch)
    .bind(&user.id)
    .execute(&mut *transaction)
    .await?;
    snapshot_stages(&mut transaction, &project_id, &id).await?;
    transaction.commit().await?;
    Ok((StatusCode::CREATED, Json(find(&state, &id).await?)))
}

pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Query(page): Query<RunPage>,
) -> Result<Json<Vec<PipelineRun>>, RunError> {
    auth::authenticated_user(&state, &headers).await?;
    if page.limit == 0 || page.limit > 100 || page.offset > i32::MAX as u32 {
        return Err(RunError::InvalidInput);
    }
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?1)")
        .bind(&project_id)
        .fetch_one(&state.pool)
        .await?;
    if !exists {
        return Err(RunError::ProjectNotFound);
    }
    let runs = sqlx::query_as::<_, PipelineRun>(
        "SELECT id, project_id, run_number, branch, commit_sha, trigger_type, status, retry_of_run_id, created_by, created_at, started_at, finished_at FROM pipeline_runs WHERE project_id = ?1 ORDER BY run_number DESC LIMIT ?2 OFFSET ?3",
    )
    .bind(project_id)
    .bind(i64::from(page.limit))
    .bind(i64::from(page.offset))
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(runs))
}

pub async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<PipelineRun>, RunError> {
    auth::authenticated_user(&state, &headers).await?;
    Ok(Json(find(&state, &id).await?))
}

pub async fn stages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Vec<StageRun>>, RunError> {
    auth::authenticated_user(&state, &headers).await?;
    find(&state, &id).await?;
    let stages = sqlx::query_as::<_, StoredStageRun>(
        "SELECT id, run_id, position, name, command, enabled, status, timeout_seconds, created_at, started_at, finished_at FROM stage_runs WHERE run_id = ?1 ORDER BY position",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(StageRun::try_from)
    .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(stages))
}

impl TryFrom<StoredStageRun> for StageRun {
    type Error = RunError;

    fn try_from(stage: StoredStageRun) -> Result<Self, Self::Error> {
        Ok(Self {
            id: stage.id,
            run_id: stage.run_id,
            position: stage.position,
            name: stage.name,
            command: serde_json::from_str(&stage.command)
                .map_err(|error| RunError::CorruptCommand(error.to_string()))?,
            enabled: stage.enabled,
            status: stage.status,
            timeout_seconds: stage.timeout_seconds,
            created_at: stage.created_at,
            started_at: stage.started_at,
            finished_at: stage.finished_at,
        })
    }
}

pub async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<PipelineRun>, RunError> {
    auth::authenticated_user(&state, &headers).await?;
    let mut transaction = state.pool.begin().await?;
    let result = sqlx::query(
        "UPDATE pipeline_runs SET status = 'cancelled', finished_at = CURRENT_TIMESTAMP WHERE id = ?1 AND status = 'pending'",
    )
    .bind(&id)
    .execute(&mut *transaction)
    .await?;
    if result.rows_affected() == 0 {
        transaction.rollback().await?;
        return match find(&state, &id).await {
            Err(RunError::NotFound) => Err(RunError::NotFound),
            _ => Err(RunError::InvalidTransition),
        };
    }
    sqlx::query(
        "UPDATE stage_runs SET status = 'cancelled', finished_at = CURRENT_TIMESTAMP WHERE run_id = ?1 AND status = 'pending'",
    )
    .bind(&id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(Json(find(&state, &id).await?))
}

pub async fn retry(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<PipelineRun>), RunError> {
    let (user, _) = auth::authenticated_user(&state, &headers).await?;
    let source = find(&state, &id).await?;
    if !matches!(source.status.as_str(), "failed" | "cancelled") {
        return Err(RunError::InvalidTransition);
    }
    let mut transaction = state.pool.begin().await?;
    let run_number = allocate_run_number(&mut transaction, &source.project_id).await?;
    let new_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO pipeline_runs (id, project_id, run_number, branch, commit_sha, trigger_type, retry_of_run_id, created_by) VALUES (?1, ?2, ?3, ?4, ?5, 'manual', ?6, ?7)",
    )
    .bind(&new_id)
    .bind(&source.project_id)
    .bind(run_number)
    .bind(&source.branch)
    .bind(&source.commit_sha)
    .bind(&source.id)
    .bind(&user.id)
    .execute(&mut *transaction)
    .await?;
    copy_stage_snapshot(&mut transaction, &source.id, &new_id).await?;
    transaction.commit().await?;
    Ok((StatusCode::CREATED, Json(find(&state, &new_id).await?)))
}

async fn snapshot_stages(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    project_id: &str,
    run_id: &str,
) -> Result<(), sqlx::Error> {
    let stages = sqlx::query_as::<_, (i64, String, String, bool, i64)>(
        "SELECT position, name, command, enabled, timeout_seconds FROM pipeline_stages WHERE project_id = ?1 ORDER BY position",
    )
    .bind(project_id)
    .fetch_all(&mut **transaction)
    .await?;
    for (position, name, command, enabled, timeout_seconds) in stages {
        sqlx::query("INSERT INTO stage_runs (id, run_id, position, name, command, enabled, status, timeout_seconds) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)")
            .bind(Uuid::new_v4().to_string())
            .bind(run_id)
            .bind(position)
            .bind(name)
            .bind(command)
            .bind(enabled)
            .bind(if enabled { "pending" } else { "skipped" })
            .bind(timeout_seconds)
            .execute(&mut **transaction)
            .await?;
    }
    Ok(())
}

async fn allocate_run_number(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    project_id: &str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO pipeline_run_counters (project_id, last_run_number) VALUES (?1, 1) \
         ON CONFLICT(project_id) DO UPDATE SET last_run_number = last_run_number + 1 \
         RETURNING last_run_number",
    )
    .bind(project_id)
    .fetch_one(&mut **transaction)
    .await
}

async fn copy_stage_snapshot(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    source_run_id: &str,
    target_run_id: &str,
) -> Result<(), sqlx::Error> {
    let stages = sqlx::query_as::<_, (i64, String, String, bool, i64)>(
        "SELECT position, name, command, enabled, timeout_seconds FROM stage_runs WHERE run_id = ?1 ORDER BY position",
    )
    .bind(source_run_id)
    .fetch_all(&mut **transaction)
    .await?;
    for (position, name, command, enabled, timeout_seconds) in stages {
        sqlx::query("INSERT INTO stage_runs (id, run_id, position, name, command, enabled, status, timeout_seconds) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)")
            .bind(Uuid::new_v4().to_string())
            .bind(target_run_id)
            .bind(position)
            .bind(name)
            .bind(command)
            .bind(enabled)
            .bind(if enabled { "pending" } else { "skipped" })
            .bind(timeout_seconds)
            .execute(&mut **transaction)
            .await?;
    }
    Ok(())
}

async fn find(state: &AppState, id: &str) -> Result<PipelineRun, RunError> {
    sqlx::query_as::<_, PipelineRun>(
        "SELECT id, project_id, run_number, branch, commit_sha, trigger_type, status, retry_of_run_id, created_by, created_at, started_at, finished_at FROM pipeline_runs WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(RunError::NotFound)
}

const fn default_page_limit() -> u32 {
    50
}

fn valid_ref(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && !value.starts_with('-')
        && !value.starts_with('/')
        && !value.ends_with('/')
        && !value.ends_with('.')
        && !value.contains("..")
        && !value.contains("@{")
        && !value.contains("//")
        && !value.contains('\\')
        && !value.chars().any(char::is_whitespace)
        && !value.chars().any(char::is_control)
        && !value
            .chars()
            .any(|character| matches!(character, '~' | '^' | ':' | '?' | '*' | '['))
}

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("会话无效")]
    Auth(#[from] auth::AuthError),
    #[error("项目不存在")]
    ProjectNotFound,
    #[error("运行不存在")]
    NotFound,
    #[error("运行参数无效")]
    InvalidInput,
    #[error("运行状态不允许该操作")]
    InvalidTransition,
    #[error("数据库中的 Stage 命令无效: {0}")]
    CorruptCommand(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

impl axum::response::IntoResponse for RunError {
    fn into_response(self) -> axum::response::Response {
        let (status, code, message) = match self {
            Self::Auth(auth::AuthError::InvalidSession) => (
                StatusCode::UNAUTHORIZED,
                "invalid_session",
                "会话无效或已过期",
            ),
            Self::ProjectNotFound => (StatusCode::NOT_FOUND, "not_found", "项目不存在"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", "运行不存在"),
            Self::InvalidInput => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_input",
                "运行参数无效",
            ),
            Self::InvalidTransition => (
                StatusCode::CONFLICT,
                "invalid_transition",
                "运行状态不允许该操作",
            ),
            Self::Auth(_) | Self::CorruptCommand(_) | Self::Database(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "服务暂时无法完成请求",
            ),
        };
        (
            status,
            Json(serde_json::json!({"code": code, "message": message})),
        )
            .into_response()
    }
}
