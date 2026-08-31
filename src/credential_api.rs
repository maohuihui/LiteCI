use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;

use crate::{AppState, CredentialKind, CredentialStoreError, NewCredential, auth};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateCredential {
    name: String,
    kind: CredentialKind,
    payload: String,
}

pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::CredentialSummary>>, CredentialApiError> {
    auth::authenticated_user(&state, &headers).await?;
    let store = state
        .credentials
        .as_ref()
        .ok_or(CredentialApiError::Unavailable)?;
    Ok(Json(store.list().await?))
}

pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CreateCredential>,
) -> Result<(StatusCode, Json<crate::CredentialSummary>), CredentialApiError> {
    auth::authenticated_user(&state, &headers).await?;
    let store = state
        .credentials
        .as_ref()
        .ok_or(CredentialApiError::Unavailable)?;
    let summary = store
        .create(NewCredential {
            name: input.name,
            kind: input.kind,
            payload: input.payload.into_bytes(),
        })
        .await?;
    Ok((StatusCode::CREATED, Json(summary)))
}

pub async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, CredentialApiError> {
    auth::authenticated_user(&state, &headers).await?;
    let store = state
        .credentials
        .as_ref()
        .ok_or(CredentialApiError::Unavailable)?;
    store.delete(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialApiError {
    #[error(transparent)]
    Auth(#[from] auth::AuthError),
    #[error(transparent)]
    Store(#[from] CredentialStoreError),
    #[error("凭证存储不可用")]
    Unavailable,
}

impl axum::response::IntoResponse for CredentialApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, code, message) = match self {
            Self::Auth(auth::AuthError::InvalidSession) => (
                StatusCode::UNAUTHORIZED,
                "invalid_session",
                "会话无效或已过期",
            ),
            Self::Auth(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "服务暂时无法完成请求",
            ),
            Self::Store(CredentialStoreError::InvalidInput) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_input",
                "凭证数据无效",
            ),
            Self::Store(CredentialStoreError::NotFound) => {
                (StatusCode::NOT_FOUND, "not_found", "凭证不存在")
            }
            Self::Store(CredentialStoreError::Conflict) => {
                (StatusCode::CONFLICT, "conflict", "凭证名称已存在")
            }
            Self::Store(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "服务暂时无法完成请求",
            ),
            Self::Unavailable => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "credential_unavailable",
                "凭证存储不可用",
            ),
        };
        (
            status,
            Json(serde_json::json!({"code": code, "message": message})),
        )
            .into_response()
    }
}

#[allow(dead_code)]
fn _kind_is_exhaustive(kind: CredentialKind) -> CredentialKind {
    kind
}
