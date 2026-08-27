use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{Mutex, OnceLock},
    time::{Duration as StdDuration, Instant},
};

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use rand::RngCore;
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::task::spawn_blocking;
use uuid::Uuid;

use crate::AppState;

const MIN_PASSWORD_BYTES: usize = 12;
const MAX_PASSWORD_BYTES: usize = 1024;
const SESSION_LIFETIME: Duration = Duration::days(7);
const LOGIN_FAILURE_LIMIT: u8 = 5;
const LOGIN_FAILURE_WINDOW: StdDuration = StdDuration::from_secs(5 * 60);
const MAX_LIMITER_ENTRIES: usize = 4096;

#[derive(Debug, Deserialize)]
pub struct SetupCredentials {
    username: String,
    password: String,
    setup_token: String,
}

#[derive(Debug, Deserialize)]
pub struct Credentials {
    username: String,
    password: String,
}

#[derive(Debug, Serialize)]
pub struct UserResponse {
    id: Uuid,
    username: String,
    role: &'static str,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    token: String,
    expires_at: String,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    code: &'static str,
    message: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LoginKey {
    ip: IpAddr,
    username: String,
}

#[derive(Debug, Clone, Copy)]
struct LoginEntry {
    failures: u8,
    in_flight: u8,
    window_started: Instant,
}

#[derive(Debug, Default)]
pub(crate) struct LoginLimiter {
    entries: Mutex<HashMap<LoginKey, LoginEntry>>,
}

pub(crate) struct LoginAttempt<'a> {
    limiter: &'a LoginLimiter,
    key: LoginKey,
    finished: bool,
}

impl LoginLimiter {
    fn begin(&self, ip: IpAddr, username: &str) -> Result<LoginAttempt<'_>, AuthError> {
        let key = LoginKey {
            ip,
            username: username.to_lowercase(),
        };
        let mut entries = self.entries.lock().map_err(|_| AuthError::InternalState)?;
        entries.retain(|_, entry| {
            entry.in_flight != 0 || entry.window_started.elapsed() < LOGIN_FAILURE_WINDOW
        });
        if !entries.contains_key(&key) && entries.len() >= MAX_LIMITER_ENTRIES {
            return Err(AuthError::RateLimited);
        }
        let entry = entries.entry(key.clone()).or_insert(LoginEntry {
            failures: 0,
            in_flight: 0,
            window_started: Instant::now(),
        });
        if entry.in_flight == 0 && entry.window_started.elapsed() >= LOGIN_FAILURE_WINDOW {
            *entry = LoginEntry {
                failures: 0,
                in_flight: 0,
                window_started: Instant::now(),
            };
        }
        if entry.failures.saturating_add(entry.in_flight) >= LOGIN_FAILURE_LIMIT {
            return Err(AuthError::RateLimited);
        }
        entry.in_flight = entry.in_flight.saturating_add(1);
        Ok(LoginAttempt {
            limiter: self,
            key,
            finished: false,
        })
    }

    fn finish(&self, key: &LoginKey, failed: bool) -> Result<(), AuthError> {
        let mut entries = self.entries.lock().map_err(|_| AuthError::InternalState)?;
        let entry = entries.get_mut(key).ok_or(AuthError::InternalState)?;
        entry.in_flight = entry.in_flight.saturating_sub(1);
        if failed {
            entry.failures = entry.failures.saturating_add(1);
        }
        Ok(())
    }
}

impl LoginAttempt<'_> {
    fn finish(mut self, failed: bool) -> Result<(), AuthError> {
        self.limiter.finish(&self.key, failed)?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for LoginAttempt<'_> {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.limiter.finish(&self.key, true);
        }
    }
}

pub async fn setup(
    State(state): State<AppState>,
    ConnectInfo(_peer): ConnectInfo<SocketAddr>,
    Json(credentials): Json<SetupCredentials>,
) -> Result<(StatusCode, Json<UserResponse>), AuthError> {
    if !constant_time_equal(&credentials.setup_token, &state.setup_token) {
        return Err(AuthError::InvalidSetupToken);
    }
    let username = validate_credentials(&credentials.username, &credentials.password)?.to_owned();
    let password_hash = hash_password_async(state.clone(), credentials.password).await?;

    let mut transaction = state.pool.begin().await?;
    let claim = sqlx::query(
        "UPDATE setup_state SET claimed = 1 WHERE singleton = 1 AND claimed = 0 AND NOT EXISTS (SELECT 1 FROM users)",
    )
    .execute(&mut *transaction)
    .await?;
    if claim.rows_affected() != 1 {
        return Err(AuthError::AlreadyInitialized);
    }

    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, role) VALUES (?1, ?2, ?3, 'admin')",
    )
    .bind(id.to_string())
    .bind(&username)
    .bind(password_hash)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;

    Ok((
        StatusCode::CREATED,
        Json(UserResponse {
            id,
            username,
            role: "admin",
        }),
    ))
}

pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(credentials): Json<Credentials>,
) -> Result<Response, AuthError> {
    let username = credentials.username.trim();
    if username.is_empty()
        || username.len() > 64
        || credentials.password.is_empty()
        || credentials.password.len() > MAX_PASSWORD_BYTES
    {
        return Err(AuthError::InvalidCredentials);
    }
    let attempt = state.login_limiter.begin(peer.ip(), username)?;

    let user: Option<(String, String)> =
        sqlx::query_as("SELECT id, password_hash FROM users WHERE username = ?1")
            .bind(username)
            .fetch_optional(&state.pool)
            .await?;
    let user_id = user.as_ref().map(|value| value.0.clone());
    let encoded = user.map(|value| value.1);
    let password_valid =
        verify_password_async(state.clone(), credentials.password, encoded).await?;
    if !password_valid || user_id.is_none() {
        attempt.finish(true)?;
        return Err(AuthError::InvalidCredentials);
    }
    attempt.finish(false)?;

    let mut token_bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut token_bytes);
    let token = token_bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let token_hash = hash_token(&token);
    let expires_at = OffsetDateTime::now_utc() + SESSION_LIFETIME;
    sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES (?1, ?2, ?3)")
        .bind(token_hash)
        .bind(user_id.expect("user id was checked above"))
        .bind(expires_at)
        .execute(&state.pool)
        .await?;

    let expires_at = expires_at
        .format(&Rfc3339)
        .map_err(|_| AuthError::SessionEncoding)?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(LoginResponse { token, expires_at }),
    )
        .into_response())
}

fn validate_credentials<'a>(username: &'a str, password: &str) -> Result<&'a str, AuthError> {
    let username = username.trim();
    if username.is_empty()
        || username.len() > 64
        || password.len() < MIN_PASSWORD_BYTES
        || password.len() > MAX_PASSWORD_BYTES
    {
        return Err(AuthError::InvalidInput);
    }
    Ok(username)
}

fn constant_time_equal(provided: &str, expected: &str) -> bool {
    let provided = Sha256::digest(provided.as_bytes());
    let expected = Sha256::digest(expected.as_bytes());
    bool::from(provided.ct_eq(&expected))
}

async fn hash_password_async(state: AppState, password: String) -> Result<String, AuthError> {
    let _permit = state
        .password_workers
        .try_acquire_owned()
        .map_err(|_| AuthError::RateLimited)?;
    spawn_blocking(move || hash_password(password.as_bytes()))
        .await
        .map_err(|_| AuthError::PasswordWorker)?
}

async fn verify_password_async(
    state: AppState,
    password: String,
    encoded: Option<String>,
) -> Result<bool, AuthError> {
    let _permit = state
        .password_workers
        .try_acquire_owned()
        .map_err(|_| AuthError::RateLimited)?;
    spawn_blocking(move || {
        let encoded = encoded.unwrap_or_else(|| dummy_password_hash().to_owned());
        verify_password(password.as_bytes(), &encoded)
    })
    .await
    .map_err(|_| AuthError::PasswordWorker)
}

fn hash_password(password: &[u8]) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password, &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| AuthError::PasswordHash)
}

fn verify_password(password: &[u8], encoded: &str) -> bool {
    PasswordHash::new(encoded).ok().is_some_and(|encoded| {
        Argon2::default()
            .verify_password(password, &encoded)
            .is_ok()
    })
}

fn dummy_password_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        hash_password(b"LiteCI dummy password")
            .expect("the built-in dummy password must be hashable")
    })
}

fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("LiteCI 已完成初始化")]
    AlreadyInitialized,
    #[error("初始化令牌无效")]
    InvalidSetupToken,
    #[error("用户名或密码不符合要求")]
    InvalidInput,
    #[error("用户名或密码错误")]
    InvalidCredentials,
    #[error("请求过于频繁")]
    RateLimited,
    #[error("无法安全处理密码")]
    PasswordHash,
    #[error("密码工作线程异常退出")]
    PasswordWorker,
    #[error("无法编码会话有效期")]
    SessionEncoding,
    #[error("内部状态不可用")]
    InternalState,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::AlreadyInitialized => (
                StatusCode::CONFLICT,
                "already_initialized",
                "LiteCI 已完成初始化",
            ),
            Self::InvalidSetupToken => (
                StatusCode::FORBIDDEN,
                "invalid_setup_token",
                "初始化令牌无效",
            ),
            Self::InvalidInput => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_input",
                "用户名不能为空，且密码长度必须在 12 到 1024 字节之间",
            ),
            Self::InvalidCredentials => (
                StatusCode::UNAUTHORIZED,
                "invalid_credentials",
                "用户名或密码错误",
            ),
            Self::RateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "请求过于频繁，请稍后重试",
            ),
            Self::PasswordHash
            | Self::PasswordWorker
            | Self::SessionEncoding
            | Self::InternalState
            | Self::Database(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "服务暂时无法完成请求",
            ),
        };
        (status, Json(ErrorResponse { code, message })).into_response()
    }
}
