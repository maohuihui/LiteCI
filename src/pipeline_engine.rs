use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

use sqlx::SqlitePool;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::{CommandExecutor, CommandSpec, ExecutionStatus, GitService};

#[derive(Clone)]
pub struct PipelineEngine {
    pool: SqlitePool,
    workspace_root: Arc<PathBuf>,
    executor: CommandExecutor,
    credentials: Option<crate::CredentialStore>,
    concurrency: Arc<Semaphore>,
    running: Arc<Mutex<HashMap<String, CancellationToken>>>,
    project_locks: Arc<Mutex<HashMap<String, Weak<Semaphore>>>>,
}

impl PipelineEngine {
    pub fn new(
        pool: SqlitePool,
        workspace_root: impl Into<PathBuf>,
        max_concurrency: usize,
    ) -> Self {
        Self {
            pool,
            workspace_root: Arc::new(workspace_root.into()),
            executor: CommandExecutor::new(),
            credentials: None,
            concurrency: Arc::new(Semaphore::new(max_concurrency.max(1))),
            running: Arc::new(Mutex::new(HashMap::new())),
            project_locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn cancel(&self, run_id: &str) -> bool {
        if let Some(cancellation) = self
            .running
            .lock()
            .ok()
            .and_then(|running| running.get(run_id).cloned())
        {
            cancellation.cancel();
            return true;
        }
        let mut transaction = match self.pool.begin().await {
            Ok(transaction) => transaction,
            Err(_) => return false,
        };
        let result = match sqlx::query(
            "UPDATE pipeline_runs SET status = 'cancelled', finished_at = CURRENT_TIMESTAMP WHERE id = ?1 AND status = 'pending'",
        )
        .bind(run_id)
        .execute(&mut *transaction)
        .await
        {
            Ok(result) => result,
            Err(_) => return false,
        };
        if result.rows_affected() == 0 {
            let _ = transaction.rollback().await;
            return false;
        }
        let stages = sqlx::query(
            "UPDATE stage_runs SET status = 'cancelled', finished_at = CURRENT_TIMESTAMP WHERE run_id = ?1 AND status = 'pending'",
        )
        .bind(run_id)
        .execute(&mut *transaction)
        .await;
        if stages.is_err() || transaction.commit().await.is_err() {
            return false;
        }
        true
    }

    pub async fn execute(&self, run_id: &str) -> Result<(), PipelineEngineError> {
        self.execute_mode(run_id, false).await
    }

    pub async fn execute_with_git(&self, run_id: &str) -> Result<(), PipelineEngineError> {
        self.execute_mode(run_id, true).await
    }

    pub fn with_credentials(self, credentials: crate::CredentialStore) -> Self {
        Self {
            credentials: Some(credentials),
            ..self
        }
    }

    async fn execute_mode(&self, run_id: &str, sync_git: bool) -> Result<(), PipelineEngineError> {
        let execution = self.execute_inner(run_id, sync_git).await;
        let result = match execution {
            Err(PipelineEngineError::NotPending) => {
                let _ = self.remove_running(run_id);
                return Err(PipelineEngineError::NotPending);
            }
            Err(PipelineEngineError::Cancelled) => {
                let _ = self.cancel_running_run(run_id).await;
                Ok(())
            }
            Err(error) => {
                let _ = self.fail_running_run(run_id).await;
                Err(error)
            }
            Ok(()) => Ok(()),
        };
        let _ = self.remove_running(run_id);
        result
    }

    async fn execute_inner(&self, run_id: &str, sync_git: bool) -> Result<(), PipelineEngineError> {
        let project_id: String = sqlx::query_scalar(
            "SELECT project_id FROM pipeline_runs WHERE id = ?1 AND status = 'pending'",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(PipelineEngineError::NotPending)?;
        let _project_permit = self.project_permit(&project_id).await?;
        let _permit = self
            .concurrency
            .acquire()
            .await
            .map_err(|_| PipelineEngineError::Closed)?;
        let cancellation = CancellationToken::new();
        {
            let mut running = self
                .running
                .lock()
                .map_err(|_| PipelineEngineError::Closed)?;
            if running.contains_key(run_id) {
                return Err(PipelineEngineError::AlreadyRunning);
            }
            running.insert(run_id.to_owned(), cancellation.clone());
        }
        let claimed = sqlx::query("UPDATE pipeline_runs SET status = 'running', started_at = CURRENT_TIMESTAMP WHERE id = ?1 AND status = 'pending'")
            .bind(run_id)
            .execute(&self.pool)
            .await?;
        if claimed.rows_affected() == 0 {
            self.remove_running(run_id)?;
            return Err(PipelineEngineError::NotPending);
        }
        if cancellation.is_cancelled() {
            return Err(PipelineEngineError::Cancelled);
        }
        let stages = sqlx::query_as::<_, StageRow>("SELECT id, position, command, enabled, timeout_seconds FROM stage_runs WHERE run_id = ?1 ORDER BY position")
            .bind(run_id)
            .fetch_all(&self.pool)
            .await?;
        let workspace = prepare_workspace(&self.workspace_root, &project_id, run_id).await?;
        if sync_git {
            let (repository, branch, credential_id): (String, String, Option<String>) =
                sqlx::query_as(
                    "SELECT projects.git_url, pipeline_runs.branch, projects.git_auth_id
                 FROM pipeline_runs
                 JOIN projects ON projects.id = pipeline_runs.project_id
                 WHERE pipeline_runs.id = ?1",
                )
                .bind(run_id)
                .fetch_one(&self.pool)
                .await?;
            let git = GitService::new(self.executor);
            if cancellation.is_cancelled() {
                return Err(PipelineEngineError::Cancelled);
            }
            let snapshot = match credential_id {
                Some(id) => {
                    let store = self
                        .credentials
                        .as_ref()
                        .ok_or(PipelineEngineError::CredentialsUnavailable)?;
                    let kind = store
                        .kind(&id)
                        .await
                        .map_err(PipelineEngineError::CredentialStore)?;
                    let payload = store
                        .decrypt(&id)
                        .await
                        .map_err(PipelineEngineError::CredentialStore)?;
                    let credential = match kind {
                        crate::CredentialKind::HttpsToken => parse_https_credential(&payload)?,
                        crate::CredentialKind::SshKey => crate::GitCredential::SshKey {
                            private_key: String::from_utf8(payload)
                                .map_err(|_| PipelineEngineError::InvalidCredential)?,
                        },
                    };
                    git.sync_with_credential(
                        &repository,
                        &branch,
                        workspace.clone(),
                        cancellation.clone(),
                        credential,
                    )
                    .await
                    .map_err(map_git_error)?
                }
                None => git
                    .sync(
                        &repository,
                        &branch,
                        workspace.clone(),
                        cancellation.clone(),
                    )
                    .await
                    .map_err(map_git_error)?,
            };
            if cancellation.is_cancelled() {
                return Err(PipelineEngineError::Cancelled);
            }
            let updated = sqlx::query(
                "UPDATE pipeline_runs SET commit_sha = ?1 WHERE id = ?2 AND status = 'running'",
            )
            .bind(snapshot.commit_sha)
            .bind(run_id)
            .execute(&self.pool)
            .await?;
            if updated.rows_affected() != 1 {
                return Err(PipelineEngineError::StateConflict);
            }
        }
        for stage in stages {
            if !stage.enabled {
                continue;
            }
            if cancellation.is_cancelled() {
                return Err(PipelineEngineError::Cancelled);
            }
            let _position = stage.position;
            let stage_claim = sqlx::query("UPDATE stage_runs SET status = 'running', started_at = CURRENT_TIMESTAMP WHERE id = ?1 AND status = 'pending'")
                .bind(&stage.id).execute(&self.pool).await?;
            if stage_claim.rows_affected() != 1 {
                return Err(PipelineEngineError::StateConflict);
            }
            let command: StageCommand = serde_json::from_str(&stage.command)
                .map_err(|error| PipelineEngineError::InvalidCommand(error.to_string()))?;
            let timeout = u64::try_from(stage.timeout_seconds)
                .map_err(|_| PipelineEngineError::InvalidTimeout)?;
            let command = command.into_spec(workspace.clone(), Duration::from_secs(timeout));
            let output = self.executor.execute(command, cancellation.clone()).await?;
            let (status, failed) = match output.status {
                ExecutionStatus::Success => ("success", false),
                ExecutionStatus::Cancelled => return Err(PipelineEngineError::Cancelled),
                ExecutionStatus::TimedOut | ExecutionStatus::Failed => ("failed", true),
            };
            sqlx::query(
                "UPDATE stage_runs SET status = ?1, finished_at = CURRENT_TIMESTAMP WHERE id = ?2",
            )
            .bind(status)
            .bind(&stage.id)
            .execute(&self.pool)
            .await?;
            if failed {
                if status == "cancelled" {
                    return Err(PipelineEngineError::Cancelled);
                }
                sqlx::query("UPDATE stage_runs SET status = 'skipped', finished_at = CURRENT_TIMESTAMP WHERE run_id = ?1 AND status = 'pending'")
                    .bind(run_id).execute(&self.pool).await?;
                sqlx::query("UPDATE pipeline_runs SET status = ?1, finished_at = CURRENT_TIMESTAMP WHERE id = ?2")
                    .bind(status).bind(run_id).execute(&self.pool).await?;
                return Ok(());
            }
        }
        if cancellation.is_cancelled() {
            return Err(PipelineEngineError::Cancelled);
        }
        let updated = sqlx::query("UPDATE pipeline_runs SET status = 'success', finished_at = CURRENT_TIMESTAMP WHERE id = ?1 AND status = 'running'")
            .bind(run_id).execute(&self.pool).await?;
        if updated.rows_affected() != 1 {
            return Err(PipelineEngineError::StateConflict);
        }
        Ok(())
    }

    async fn fail_running_run(&self, run_id: &str) -> Result<(), sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("UPDATE stage_runs SET status = 'failed', finished_at = CURRENT_TIMESTAMP WHERE run_id = ?1 AND status = 'running'")
            .bind(run_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE stage_runs SET status = 'skipped', finished_at = CURRENT_TIMESTAMP WHERE run_id = ?1 AND status = 'pending'")
            .bind(run_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE pipeline_runs SET status = 'failed', finished_at = CURRENT_TIMESTAMP WHERE id = ?1 AND status = 'running'")
            .bind(run_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await
    }

    async fn cancel_running_run(&self, run_id: &str) -> Result<(), sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("UPDATE stage_runs SET status = 'cancelled', finished_at = CURRENT_TIMESTAMP WHERE run_id = ?1 AND status = 'running'")
            .bind(run_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE stage_runs SET status = 'skipped', finished_at = CURRENT_TIMESTAMP WHERE run_id = ?1 AND status = 'pending'")
            .bind(run_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE pipeline_runs SET status = 'cancelled', finished_at = CURRENT_TIMESTAMP WHERE id = ?1 AND status = 'running'")
            .bind(run_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await
    }

    async fn project_permit(
        &self,
        project_id: &str,
    ) -> Result<OwnedSemaphorePermit, PipelineEngineError> {
        let lock = {
            let mut locks = self
                .project_locks
                .lock()
                .map_err(|_| PipelineEngineError::Closed)?;
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = locks.get(project_id).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(Semaphore::new(1));
                locks.insert(project_id.to_owned(), Arc::downgrade(&lock));
                lock
            }
        };
        lock.acquire_owned()
            .await
            .map_err(|_| PipelineEngineError::Closed)
    }

    fn remove_running(&self, run_id: &str) -> Result<(), PipelineEngineError> {
        let mut running = self
            .running
            .lock()
            .map_err(|_| PipelineEngineError::Closed)?;
        running.remove(run_id);
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct StageRow {
    id: String,
    position: i64,
    command: String,
    enabled: bool,
    timeout_seconds: i64,
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "mode", deny_unknown_fields)]
enum StageCommand {
    #[serde(rename = "process")]
    Process { program: String, args: Vec<String> },
    #[serde(rename = "shell")]
    Shell { script: String },
}

impl StageCommand {
    fn into_spec(self, workspace: PathBuf, timeout: Duration) -> CommandSpec {
        let (program, args) = match self {
            Self::Process { program, args } => (program, args),
            Self::Shell { script } => shell_command(script),
        };
        CommandSpec {
            program,
            args,
            working_directory: Some(workspace),
            environment: Default::default(),
            timeout,
        }
    }
}

#[cfg(windows)]
fn shell_command(script: String) -> (String, Vec<String>) {
    ("cmd".to_owned(), vec!["/C".to_owned(), script])
}

#[cfg(not(windows))]
fn shell_command(script: String) -> (String, Vec<String>) {
    ("sh".to_owned(), vec!["-c".to_owned(), script])
}

#[derive(Debug, thiserror::Error)]
pub enum PipelineEngineError {
    #[error("运行不存在或不是 pending 状态")]
    NotPending,
    #[error("执行器已关闭")]
    Closed,
    #[error("Stage 命令无效: {0}")]
    InvalidCommand(String),
    #[error("Stage 超时时间无效")]
    InvalidTimeout,
    #[error("Pipeline 工作目录无效")]
    InvalidWorkspace,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Executor(#[from] crate::CommandError),
    #[error(transparent)]
    Git(#[from] crate::GitError),
    #[error(transparent)]
    CredentialStore(#[from] crate::CredentialStoreError),
    #[error("凭证存储不可用")]
    CredentialsUnavailable,
    #[error("Git 凭证格式无效")]
    InvalidCredential,
    #[error("运行已被执行")]
    AlreadyRunning,
    #[error("运行状态冲突")]
    StateConflict,
    #[error("运行已取消")]
    Cancelled,
}

fn map_git_error(error: crate::GitError) -> PipelineEngineError {
    match error {
        crate::GitError::CommandFailed {
            status: crate::ExecutionStatus::Cancelled,
            ..
        } => PipelineEngineError::Cancelled,
        other => PipelineEngineError::Git(other),
    }
}

fn parse_https_credential(payload: &[u8]) -> Result<crate::GitCredential, PipelineEngineError> {
    let value =
        String::from_utf8(payload.to_vec()).map_err(|_| PipelineEngineError::InvalidCredential)?;
    let mut username = None;
    let mut token = None;
    for line in value.lines() {
        let Some((key, value)) = line.split_once('=') else {
            return Err(PipelineEngineError::InvalidCredential);
        };
        match key {
            "username" if username.is_none() => username = Some(value.to_owned()),
            "token" if token.is_none() => token = Some(value.to_owned()),
            _ => return Err(PipelineEngineError::InvalidCredential),
        }
    }
    match (username, token) {
        (Some(username), Some(token)) if !username.is_empty() && !token.is_empty() => {
            Ok(crate::GitCredential::HttpsToken { username, token })
        }
        _ => Err(PipelineEngineError::InvalidCredential),
    }
}

async fn prepare_workspace(
    root: &Path,
    project_id: &str,
    run_id: &str,
) -> Result<PathBuf, PipelineEngineError> {
    if uuid::Uuid::parse_str(project_id).is_err() || uuid::Uuid::parse_str(run_id).is_err() {
        return Err(PipelineEngineError::InvalidWorkspace);
    }
    let root = match tokio::fs::symlink_metadata(root).await {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(PipelineEngineError::InvalidWorkspace);
            }
            tokio::fs::canonicalize(root).await?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(PipelineEngineError::InvalidWorkspace);
        }
        Err(error) => return Err(error.into()),
    };
    let project_workspace = root.join(project_id);
    create_contained_directory(&root, &project_workspace).await?;
    let workspace = project_workspace.join(run_id);
    create_contained_directory(&root, &workspace).await?;
    let workspace = tokio::fs::canonicalize(workspace).await?;
    if !workspace.starts_with(&root) {
        return Err(PipelineEngineError::InvalidWorkspace);
    }
    Ok(workspace)
}

async fn create_contained_directory(
    root: &Path,
    directory: &Path,
) -> Result<(), PipelineEngineError> {
    match tokio::fs::symlink_metadata(directory).await {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(PipelineEngineError::InvalidWorkspace);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tokio::fs::create_dir(directory).await?;
        }
        Err(error) => return Err(error.into()),
    }
    let directory = tokio::fs::canonicalize(directory).await?;
    if !directory.starts_with(root) {
        return Err(PipelineEngineError::InvalidWorkspace);
    }
    Ok(())
}
