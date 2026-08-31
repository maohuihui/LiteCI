use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use crate::{CommandError, CommandExecutor, CommandSpec, ExecutionStatus};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GitSnapshot {
    pub repository: String,
    pub branch: String,
    pub commit_sha: String,
    pub commit_message: String,
    pub author: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitCredential {
    HttpsToken { username: String, token: String },
    SshKey { private_key: String },
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
        self.sync_internal(repository, branch, workspace, cancellation, None)
            .await
    }

    pub async fn sync_with_credential(
        &self,
        repository: &str,
        branch: &str,
        workspace: PathBuf,
        cancellation: CancellationToken,
        credential: GitCredential,
    ) -> Result<GitSnapshot, GitError> {
        self.sync_internal(
            repository,
            branch,
            workspace,
            cancellation,
            Some(credential),
        )
        .await
    }

    async fn sync_internal(
        &self,
        repository: &str,
        branch: &str,
        workspace: PathBuf,
        cancellation: CancellationToken,
        credential: Option<GitCredential>,
    ) -> Result<GitSnapshot, GitError> {
        validate_ref(branch)?;
        if repository.trim().is_empty() || repository.trim_start().starts_with('-') {
            return Err(GitError::InvalidRepository);
        }
        let workspace = absolute_path(&workspace)?;
        std::fs::create_dir_all(&workspace)?;
        let auth = GitAuth::prepare(repository, credential)?;
        let workspace_is_repository = workspace.join(".git").exists();
        if workspace_is_repository {
            self.verify_origin(
                repository,
                &workspace,
                cancellation.clone(),
                &auth.environment,
            )
            .await?;
            self.run_git(
                &["fetch", "--prune", "origin", branch],
                &workspace,
                cancellation.clone(),
                &auth.environment,
            )
            .await?;
            self.run_git(
                &["checkout", "--force", branch],
                &workspace,
                cancellation.clone(),
                &auth.environment,
            )
            .await?;
            self.run_git(
                &["reset", "--hard", &format!("origin/{branch}")],
                &workspace,
                cancellation.clone(),
                &auth.environment,
            )
            .await?;
            self.run_git(
                &["clean", "-ffdx"],
                &workspace,
                cancellation.clone(),
                &auth.environment,
            )
            .await?;
        } else {
            let clone_target = ".";
            self.run_git(
                &[
                    "clone",
                    "--no-tags",
                    "--branch",
                    branch,
                    "--single-branch",
                    repository,
                    clone_target,
                ],
                &workspace,
                cancellation.clone(),
                &auth.environment,
            )
            .await?;
        }
        let sha = self
            .git_output(
                &["rev-parse", "HEAD"],
                &workspace,
                cancellation.clone(),
                &auth.environment,
            )
            .await?;
        let message = self
            .git_output(
                &["log", "-1", "--format=%B"],
                &workspace,
                cancellation.clone(),
                &auth.environment,
            )
            .await?;
        let author = self
            .git_output(
                &["log", "-1", "--format=%an"],
                &workspace,
                cancellation,
                &auth.environment,
            )
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
        environment: &BTreeMap<String, String>,
    ) -> Result<(), GitError> {
        let output = self
            .executor
            .execute(
                CommandSpec {
                    program: "git".into(),
                    args: args.iter().map(|arg| (*arg).into()).collect(),
                    working_directory: Some(directory.to_path_buf()),
                    environment: environment.clone(),
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
        environment: &BTreeMap<String, String>,
    ) -> Result<String, GitError> {
        let output = self
            .executor
            .execute(
                CommandSpec {
                    program: "git".into(),
                    args: args.iter().map(|arg| (*arg).into()).collect(),
                    working_directory: Some(directory.to_path_buf()),
                    environment: environment.clone(),
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
        environment: &BTreeMap<String, String>,
    ) -> Result<(), GitError> {
        let origin = self
            .git_output(
                &["remote", "get-url", "origin"],
                directory,
                cancellation,
                environment,
            )
            .await?;
        if !same_repository(repository, origin.trim()) {
            return Err(GitError::RepositoryMismatch);
        }
        Ok(())
    }
}

struct GitAuth {
    _temporary: Option<TempDir>,
    environment: BTreeMap<String, String>,
}

impl GitAuth {
    fn prepare(repository: &str, credential: Option<GitCredential>) -> Result<Self, GitError> {
        let Some(credential) = credential else {
            return Ok(Self {
                _temporary: None,
                environment: BTreeMap::new(),
            });
        };
        let temporary = tempfile::tempdir()?;
        let mut environment = BTreeMap::new();
        match credential {
            GitCredential::HttpsToken { username, token } => {
                if username.is_empty()
                    || token.is_empty()
                    || url::Url::parse(repository)
                        .map(|url| url.scheme() != "https")
                        .unwrap_or(true)
                {
                    return Err(GitError::InvalidCredential);
                }
                let helper = if cfg!(windows) {
                    temporary.path().join("askpass.cmd")
                } else {
                    temporary.path().join("askpass")
                };
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mut file = std::fs::File::create(&helper)?;
                    file.set_permissions(std::fs::Permissions::from_mode(0o700))?;
                    writeln!(file, "#!/bin/sh")?;
                    writeln!(file, "case \"$1\" in")?;
                    writeln!(
                        file,
                        "*Username*) printf '%s\\n' '{}' ;;",
                        shell_quote(&username)
                    )?;
                    writeln!(file, "*) printf '%s\\n' '{}' ;;", shell_quote(&token))?;
                    writeln!(file, "esac")?;
                }
                #[cfg(windows)]
                {
                    std::fs::write(temporary.path().join("username"), username)?;
                    std::fs::write(temporary.path().join("token"), token)?;
                    let mut file = std::fs::File::create(&helper)?;
                    writeln!(file, "@echo off")?;
                    writeln!(
                        file,
                        "echo(%~1| %SystemRoot%\\System32\\findstr.exe /I /C:\"Username\" >nul"
                    )?;
                    writeln!(
                        file,
                        "if errorlevel 1 (type \"%~dp0token\") else (type \"%~dp0username\")"
                    )?;
                }
                environment.insert("GIT_ASKPASS".into(), path_string(&helper));
                environment.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
            }
            GitCredential::SshKey { private_key } => {
                if private_key.is_empty()
                    || !(repository.starts_with("git@") || repository.starts_with("ssh://"))
                {
                    return Err(GitError::InvalidCredential);
                }
                let key = temporary.path().join("id_key");
                let mut file = std::fs::File::create(&key)?;
                file.write_all(private_key.as_bytes())?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
                }
                environment.insert(
                    "GIT_SSH_COMMAND".into(),
                    format!(
                        "ssh -o BatchMode=yes -o IdentitiesOnly=yes -i {}",
                        quote_command_path(&key)
                    ),
                );
            }
        }
        Ok(Self {
            _temporary: Some(temporary),
            environment,
        })
    }
}

#[cfg(unix)]
fn shell_quote(value: &str) -> String {
    value.replace('\'', "'\\''")
}

fn path_string(path: &Path) -> String {
    #[cfg(windows)]
    {
        let value = path.to_string_lossy();
        value.strip_prefix(r"\\?\").unwrap_or(&value).to_owned()
    }
    #[cfg(not(windows))]
    path.to_string_lossy().into_owned()
}

#[cfg(unix)]
fn quote_command_path(path: &Path) -> String {
    format!("'{}'", shell_quote(&path_string(path)))
}

#[cfg(windows)]
fn quote_command_path(path: &Path) -> String {
    format!("\"{}\"", path_string(path).replace('"', "\\\""))
}

fn absolute_path(path: &Path) -> Result<PathBuf, std::io::Error> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
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
    #[error("Git 凭证类型或仓库不匹配")]
    InvalidCredential,
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
