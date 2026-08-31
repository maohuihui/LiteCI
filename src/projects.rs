use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{AppState, GitService, auth};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateProject {
    name: String,
    description: Option<String>,
    git_url: String,
    default_branch: Option<String>,
    git_auth_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateProject {
    name: Option<String>,
    description: Option<String>,
    git_url: Option<String>,
    default_branch: Option<String>,
    #[serde(default, deserialize_with = "deserialize_nullable_field")]
    git_auth_id: Option<Option<String>>,
    status: Option<String>,
}

fn deserialize_nullable_field<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub description: String,
    pub git_url: String,
    pub default_branch: String,
    pub status: String,
    pub workspace_path: String,
    pub created_at: String,
    pub updated_at: String,
    pub git_auth_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    code: &'static str,
    message: &'static str,
}

pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Project>>, ProjectError> {
    auth::authenticated_user(&state, &headers).await?;
    let projects = sqlx::query_as::<_, Project>(
        "SELECT id, name, description, git_url, default_branch, status, workspace_path, created_at, updated_at, git_auth_id
         FROM projects ORDER BY name ASC",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(projects))
}

pub async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Project>, ProjectError> {
    auth::authenticated_user(&state, &headers).await?;
    let project = find(&state.pool, &id)
        .await?
        .ok_or(ProjectError::NotFound)?;
    Ok(Json(project))
}

pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateProject>,
) -> Result<(StatusCode, Json<Project>), ProjectError> {
    auth::authenticated_user(&state, &headers).await?;
    let name = validate_name(&input.name)?;
    let git_url = validate_git_url(&input.git_url)?;
    let branch = validate_branch(input.default_branch.as_deref().unwrap_or("main"))?;
    let description = input.description.unwrap_or_default();
    if description.len() > 2000 {
        return Err(ProjectError::InvalidInput);
    }
    let id = Uuid::new_v4().to_string();
    let workspace_path = format!("workspaces/{id}");
    sqlx::query(
        "INSERT INTO projects (id, name, description, git_url, default_branch, workspace_path, git_auth_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(&id)
    .bind(name)
    .bind(description)
    .bind(git_url)
    .bind(branch)
    .bind(workspace_path)
    .bind(input.git_auth_id)
    .execute(&state.pool)
    .await
    .map_err(map_db_error)?;
    Ok((
        StatusCode::CREATED,
        Json(
            find(&state.pool, &id)
                .await?
                .ok_or(ProjectError::NotFound)?,
        ),
    ))
}

pub async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<UpdateProject>,
) -> Result<Json<Project>, ProjectError> {
    auth::authenticated_user(&state, &headers).await?;
    if input.name.is_none()
        && input.description.is_none()
        && input.git_url.is_none()
        && input.default_branch.is_none()
        && input.status.is_none()
        && input.git_auth_id.is_none()
    {
        return Err(ProjectError::InvalidInput);
    }
    let current = find(&state.pool, &id)
        .await?
        .ok_or(ProjectError::NotFound)?;
    let name = input
        .name
        .as_deref()
        .map(validate_name)
        .transpose()?
        .unwrap_or(current.name.as_str());
    let git_url = input
        .git_url
        .as_deref()
        .map(validate_git_url)
        .transpose()?
        .unwrap_or(current.git_url.as_str());
    let branch = input
        .default_branch
        .as_deref()
        .map(validate_branch)
        .transpose()?
        .unwrap_or(current.default_branch.as_str());
    let description = input.description.as_deref().unwrap_or(&current.description);
    let status = input.status.as_deref().unwrap_or(&current.status);
    let git_auth_id = input
        .git_auth_id
        .as_ref()
        .map(|value| value.as_deref())
        .unwrap_or(current.git_auth_id.as_deref());
    if description.len() > 2000 || !matches!(status, "active" | "disabled") {
        return Err(ProjectError::InvalidInput);
    }
    sqlx::query(
        "UPDATE projects SET name = ?1, description = ?2, git_url = ?3, default_branch = ?4, status = ?5, git_auth_id = ?6, updated_at = CURRENT_TIMESTAMP WHERE id = ?7",
    )
    .bind(name)
    .bind(description)
    .bind(git_url)
    .bind(branch)
    .bind(status)
    .bind(git_auth_id)
    .bind(&id)
    .execute(&state.pool)
    .await
    .map_err(map_db_error)?;
    Ok(Json(
        find(&state.pool, &id)
            .await?
            .ok_or(ProjectError::NotFound)?,
    ))
}

pub async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, ProjectError> {
    auth::authenticated_user(&state, &headers).await?;
    let result = sqlx::query("DELETE FROM projects WHERE id = ?1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ProjectError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[axum::debug_handler]
pub async fn sync(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<crate::GitSnapshot>, ProjectError> {
    auth::authenticated_user(&state, &headers).await?;
    let project = find(&state.pool, &id)
        .await?
        .ok_or(ProjectError::NotFound)?;
    let workspace_relative = safe_workspace_path(&project.workspace_path)?;
    let workspace = state.workspace_root.join(workspace_relative);
    let credential = if let Some(id) = project.git_auth_id.as_deref() {
        let store = state
            .credentials
            .as_ref()
            .ok_or(ProjectError::CredentialUnavailable)?;
        let kind = store.kind(id).await.map_err(ProjectError::Credential)?;
        let payload = store.decrypt(id).await.map_err(ProjectError::Credential)?;
        if project.git_url.starts_with("git@") || project.git_url.starts_with("ssh://") {
            if kind != crate::CredentialKind::SshKey {
                return Err(ProjectError::CredentialInvalid);
            }
            Some(crate::GitCredential::SshKey {
                private_key: String::from_utf8(payload)
                    .map_err(|_| ProjectError::CredentialInvalid)?,
            })
        } else {
            if kind != crate::CredentialKind::HttpsToken {
                return Err(ProjectError::CredentialInvalid);
            }
            let payload =
                String::from_utf8(payload).map_err(|_| ProjectError::CredentialInvalid)?;
            let (username, token) = parse_https_credential(&payload)?;
            Some(crate::GitCredential::HttpsToken { username, token })
        }
    } else {
        None
    };
    let snapshot = if let Some(credential) = credential {
        GitService::new(crate::CommandExecutor::new())
            .sync_with_credential(
                &project.git_url,
                &project.default_branch,
                workspace,
                CancellationToken::new(),
                credential,
            )
            .await?
    } else {
        GitService::new(crate::CommandExecutor::new())
            .sync(
                &project.git_url,
                &project.default_branch,
                workspace,
                CancellationToken::new(),
            )
            .await?
    };
    Ok(Json(snapshot))
}

fn parse_https_credential(value: &str) -> Result<(String, String), ProjectError> {
    let mut username = None;
    let mut token = None;
    for line in value.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "username" => username = Some(value.to_owned()),
            "token" => token = Some(value.to_owned()),
            _ => {}
        }
    }
    match (username, token) {
        (Some(username), Some(token)) if !username.is_empty() && !token.is_empty() => {
            Ok((username, token))
        }
        _ => Err(ProjectError::CredentialInvalid),
    }
}

fn safe_workspace_path(value: &str) -> Result<&std::path::Path, ProjectError> {
    let path = std::path::Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            !matches!(
                component,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    {
        return Err(ProjectError::InvalidWorkspace);
    }
    Ok(path)
}

async fn find(pool: &SqlitePool, id: &str) -> Result<Option<Project>, sqlx::Error> {
    sqlx::query_as::<_, Project>(
        "SELECT id, name, description, git_url, default_branch, status, workspace_path, created_at, updated_at, git_auth_id FROM projects WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

fn validate_name(value: &str) -> Result<&str, ProjectError> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 64
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
        || value.starts_with('-')
    {
        return Err(ProjectError::InvalidInput);
    }
    Ok(value)
}

fn validate_git_url(value: &str) -> Result<&str, ProjectError> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 2048
        || value.chars().any(char::is_control)
        || value.chars().any(char::is_whitespace)
    {
        return Err(ProjectError::InvalidInput);
    }
    if let Some(scp) = value.strip_prefix("git@") {
        let Some((host, path)) = scp.split_once(':') else {
            return Err(ProjectError::InvalidInput);
        };
        return if valid_host(host) && valid_repo_path(path) {
            Ok(value)
        } else {
            Err(ProjectError::InvalidInput)
        };
    }
    let parsed = url::Url::parse(value).map_err(|_| ProjectError::InvalidInput)?;
    let userinfo_valid = if parsed.scheme() == "ssh" {
        parsed.username() == "git" && parsed.password().is_none()
    } else {
        parsed.username().is_empty() && parsed.password().is_none()
    };
    if !matches!(parsed.scheme(), "https" | "ssh")
        || !userinfo_valid
        || !valid_host(parsed.host_str().unwrap_or_default())
        || !valid_repo_path(parsed.path().trim_start_matches('/'))
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(ProjectError::InvalidInput);
    }
    Ok(value)
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && !host.starts_with('.')
        && !host.ends_with('.')
        && host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b':'))
}

fn valid_repo_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('-')
        && !path.contains("..")
        && path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'-' | b'_'))
}

fn validate_branch(value: &str) -> Result<&str, ProjectError> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 255
        || value.starts_with('-')
        || value.contains("..")
        || value.contains("@{")
        || value.contains("//")
        || value.contains('\\')
        || value.chars().any(char::is_whitespace)
        || value.chars().any(char::is_control)
        || value
            .chars()
            .any(|character| matches!(character, '~' | '^' | ':' | '?' | '*' | '['))
        || value.starts_with('/')
        || value.ends_with('/')
        || value.ends_with('.')
    {
        return Err(ProjectError::InvalidInput);
    }
    Ok(value)
}

fn map_db_error(error: sqlx::Error) -> ProjectError {
    if matches!(&error, sqlx::Error::Database(database) if database.message().contains("UNIQUE")) {
        ProjectError::Conflict
    } else {
        ProjectError::Database(error)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("会话无效或已过期")]
    InvalidSession,
    #[error("项目不存在")]
    NotFound,
    #[error("项目数据无效")]
    InvalidInput,
    #[error("项目名称已存在")]
    Conflict,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Git(#[from] crate::GitError),
    #[error(transparent)]
    Credential(#[from] crate::CredentialStoreError),
    #[error("凭证存储尚未配置")]
    CredentialUnavailable,
    #[error("凭证内容无效")]
    CredentialInvalid,
    #[error("项目工作目录无效")]
    InvalidWorkspace,
}

impl From<auth::AuthError> for ProjectError {
    fn from(error: auth::AuthError) -> Self {
        match error {
            auth::AuthError::InvalidSession => Self::InvalidSession,
            other => Self::Database(sqlx::Error::Protocol(other.to_string())),
        }
    }
}

impl axum::response::IntoResponse for ProjectError {
    fn into_response(self) -> axum::response::Response {
        let (status, code, message) = match self {
            Self::InvalidSession => (
                StatusCode::UNAUTHORIZED,
                "invalid_session",
                "会话无效或已过期",
            ),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", "项目不存在"),
            Self::InvalidInput => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_input",
                "项目数据无效",
            ),
            Self::Conflict => (StatusCode::CONFLICT, "conflict", "项目名称已存在"),
            Self::Database(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "服务暂时无法完成请求",
            ),
            Self::Git(_) => (StatusCode::BAD_GATEWAY, "git_error", "Git 操作失败"),
            Self::Credential(_) | Self::CredentialUnavailable | Self::CredentialInvalid => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "credential_error",
                "凭证不可用",
            ),
            Self::InvalidWorkspace => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "invalid_workspace",
                "项目工作目录无效",
            ),
        };
        (status, Json(ErrorResponse { code, message })).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::safe_workspace_path;

    #[test]
    fn workspace_path_must_remain_relative_and_normal() {
        assert!(safe_workspace_path("workspaces/project").is_ok());
        assert!(safe_workspace_path("../outside").is_err());
        assert!(safe_workspace_path("C:/outside").is_err());
        assert!(safe_workspace_path("/outside").is_err());
    }
}
