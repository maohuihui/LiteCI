use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use crate::{CommandError, CommandExecutor, CommandSpec, ExecutionStatus};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GitSnapshot {
    pub repository: String,
    pub branch: String,
    pub commit_sha: String,
    pub commit_message: String,
    pub author: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct GitService {
    executor: CommandExecutor,
}

impl GitService {
    pub fn new(executor: CommandExecutor) -> Self {
        Self { executor }
    }

    pub async fn sync(
        &self,
        repository: &str,
        branch: &str,
        workspace: PathBuf,
        cancellation: CancellationToken,
    ) -> Result<GitSnapshot, GitError> {
        validate_ref(branch)?;
        if repository.trim().is_empty() || repository.trim_start().starts_with('-') {
            return Err(GitError::InvalidRepository);
        }
        let workspace = absolute_path(&workspace)?;
        std::fs::create_dir_all(&workspace)?;
        let workspace_is_repository = workspace.join(".git").exists();
        if workspace_is_repository {
            self.run_git(
                &["fetch", "--prune", "origin", branch],
                &workspace,
                cancellation.clone(),
            )
            .await?;
        } else {
            self.run_git(
                &[
                    "clone",
                    "--no-tags",
                    "--branch",
                    branch,
                    "--single-branch",
                    repository,
                    &path_string(&workspace),
                ],
                &workspace_parent(&workspace),
                cancellation.clone(),
            )
            .await?;
        }
        if workspace_is_repository {
            self.verify_origin(repository, &workspace, cancellation.clone())
                .await?;
            self.run_git(
                &["checkout", "--force", branch],
                &workspace,
                cancellation.clone(),
            )
            .await?;
            self.run_git(
                &["reset", "--hard", &format!("origin/{branch}")],
                &workspace,
                cancellation.clone(),
            )
            .await?;
            self.run_git(&["clean", "-ffdx"], &workspace, cancellation.clone())
                .await?;
        }
        let sha = self
            .git_output(&["rev-parse", "HEAD"], &workspace, cancellation.clone())
            .await?;
        let message = self
            .git_output(
                &["log", "-1", "--format=%B"],
                &workspace,
                cancellation.clone(),
            )
            .await?;
        let author = self
            .git_output(&["log", "-1", "--format=%an"], &workspace, cancellation)
            .await?;
        Ok(GitSnapshot {
            repository: repository.into(),
            branch: branch.into(),
            commit_sha: sha.trim().into(),
            commit_message: message.trim().into(),
            author: author.trim().into(),
        })
    }

    async fn run_git(
        &self,
        args: &[&str],
        directory: &Path,
        cancellation: CancellationToken,
    ) -> Result<(), GitError> {
        let output = self
            .executor
            .execute(
                CommandSpec {
                    program: "git".into(),
                    args: args.iter().map(|arg| (*arg).into()).collect(),
                    working_directory: Some(directory.to_path_buf()),
                    environment: Default::default(),
                    timeout: Duration::from_secs(10 * 60),
                },
                cancellation,
            )
            .await?;
        if output.status != ExecutionStatus::Success {
            return Err(GitError::CommandFailed {
                status: output.status,
                stderr: String::from_utf8_lossy(&output.stderr).trim().into(),
            });
        }
        Ok(())
    }

    async fn git_output(
        &self,
        args: &[&str],
        directory: &Path,
        cancellation: CancellationToken,
    ) -> Result<String, GitError> {
        let output = self
            .executor
            .execute(
                CommandSpec {
                    program: "git".into(),
                    args: args.iter().map(|arg| (*arg).into()).collect(),
                    working_directory: Some(directory.to_path_buf()),
                    environment: Default::default(),
                    timeout: Duration::from_secs(60),
                },
                cancellation,
            )
            .await?;
        if output.status != ExecutionStatus::Success {
            return Err(GitError::CommandFailed {
                status: output.status,
                stderr: String::from_utf8_lossy(&output.stderr).trim().into(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    async fn verify_origin(
        &self,
        repository: &str,
        directory: &Path,
        cancellation: CancellationToken,
    ) -> Result<(), GitError> {
        let origin = self
            .git_output(&["remote", "get-url", "origin"], directory, cancellation)
            .await?;
        if !same_repository(repository, origin.trim()) {
            return Err(GitError::RepositoryMismatch);
        }
        Ok(())
    }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn absolute_path(path: &Path) -> Result<PathBuf, std::io::Error> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn workspace_parent(path: &Path) -> PathBuf {
    path.parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn same_repository(expected: &str, actual: &str) -> bool {
    if expected == actual {
        return true;
    }
    if let (Ok(expected), Ok(actual)) = (
        Path::new(expected).canonicalize(),
        Path::new(actual).canonicalize(),
    ) {
        return expected == actual;
    }
    expected.trim_end_matches(".git") == actual.trim_end_matches(".git")
}

fn validate_ref(value: &str) -> Result<(), GitError> {
    if value.trim().is_empty()
        || value.len() > 255
        || value.starts_with('-')
        || value.contains("..")
        || value.contains("//")
        || value.contains("@{")
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
        return Err(GitError::InvalidRef);
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("Git 仓库地址无效")]
    InvalidRepository,
    #[error("Git 引用无效")]
    InvalidRef,
    #[error("Git 工作区来源与项目仓库不一致")]
    RepositoryMismatch,
    #[error("Git 命令执行失败: {stderr}")]
    CommandFailed {
        status: ExecutionStatus,
        stderr: String,
    },
    #[error("Git 文件操作失败")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Executor(#[from] CommandError),
}
