use std::{collections::BTreeMap, path::PathBuf, process::Stdio, time::Duration};

use command_group::AsyncCommandGroup;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub environment: BTreeMap<String, String>,
    pub timeout: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStatus {
    Success,
    Failed,
    TimedOut,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub status: ExecutionStatus,
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

const MAX_CAPTURED_STREAM_BYTES: usize = 1024 * 1024;

#[derive(Debug, Default, Clone, Copy)]
pub struct CommandExecutor;

impl CommandExecutor {
    pub fn new() -> Self {
        Self
    }

    pub async fn execute(
        &self,
        spec: CommandSpec,
        cancellation: CancellationToken,
    ) -> Result<CommandOutput, CommandError> {
        validate_spec(&spec)?;
        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .envs(&spec.environment)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(directory) = &spec.working_directory {
            command.current_dir(directory);
        }
        let mut child = command.group().kill_on_drop(true).spawn()?;
        let mut stdout = child
            .inner()
            .stdout
            .take()
            .ok_or(CommandError::MissingPipe)?;
        let mut stderr = child
            .inner()
            .stderr
            .take()
            .ok_or(CommandError::MissingPipe)?;
        let stdout_task =
            tokio::spawn(async move { read_bounded(&mut stdout, MAX_CAPTURED_STREAM_BYTES).await });
        let stderr_task =
            tokio::spawn(async move { read_bounded(&mut stderr, MAX_CAPTURED_STREAM_BYTES).await });

        let deadline = tokio::time::sleep(spec.timeout);
        tokio::pin!(deadline);
        let (status, exit_code) = tokio::select! {
            result = child.wait() => {
                let process_status = result?;
                let status = if process_status.success() {
                    ExecutionStatus::Success
                } else {
                    ExecutionStatus::Failed
                };
                (status, process_status.code())
            }
            _ = &mut deadline => {
                child.kill().await?;
                (ExecutionStatus::TimedOut, None)
            }
            _ = cancellation.cancelled() => {
                child.kill().await?;
                (ExecutionStatus::Cancelled, None)
            }
        };
        let (stdout, stdout_truncated) = stdout_task.await??;
        let (stderr, stderr_truncated) = stderr_task.await??;
        Ok(CommandOutput {
            status,
            exit_code,
            stdout,
            stderr,
            stdout_truncated,
            stderr_truncated,
        })
    }
}

async fn read_bounded(
    reader: &mut (impl AsyncRead + Unpin),
    limit: usize,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut retained = Vec::with_capacity(limit.min(8192));
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let remaining = limit.saturating_sub(retained.len());
        let retained_count = remaining.min(count);
        retained.extend_from_slice(&buffer[..retained_count]);
        truncated |= retained_count < count;
    }
    Ok((retained, truncated))
}

fn validate_spec(spec: &CommandSpec) -> Result<(), CommandError> {
    if spec.program.trim().is_empty() || spec.timeout.is_zero() {
        return Err(CommandError::InvalidSpec);
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("命令配置无效")]
    InvalidSpec,
    #[error("命令输出管道初始化失败")]
    MissingPipe,
    #[error("命令执行 I/O 失败")]
    Io(#[from] std::io::Error),
    #[error("命令输出任务失败")]
    Join(#[from] tokio::task::JoinError),
}
