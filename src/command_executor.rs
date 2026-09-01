use std::{collections::BTreeMap, path::PathBuf, process::Stdio, time::Duration};

use command_group::{AsyncCommandGroup, AsyncGroupChild};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    sync::mpsc,
    task::JoinHandle,
    time::Instant,
};
use tokio_util::sync::CancellationToken;

const MAX_CAPTURED_STREAM_BYTES: usize = 1024 * 1024;
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(10);

type CapturedStream = std::io::Result<(Vec<u8>, bool)>;
type CaptureTask = JoinHandle<CapturedStream>;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEvent {
    pub stream: LogStream,
    pub data: Vec<u8>,
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
        self.execute_inner(spec, cancellation, None).await
    }

    pub async fn execute_with_logs(
        &self,
        spec: CommandSpec,
        cancellation: CancellationToken,
        logs: mpsc::Sender<LogEvent>,
    ) -> Result<CommandOutput, CommandError> {
        self.execute_inner(spec, cancellation, Some(logs)).await
    }

    async fn execute_inner(
        &self,
        spec: CommandSpec,
        cancellation: CancellationToken,
        logs: Option<mpsc::Sender<LogEvent>>,
    ) -> Result<CommandOutput, CommandError> {
        validate_spec(&spec)?;
        if cancellation.is_cancelled() {
            return Ok(cancelled_output());
        }

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
        let stdout = child
            .inner()
            .stdout
            .take()
            .ok_or(CommandError::MissingPipe)?;
        let stderr = child
            .inner()
            .stderr
            .take()
            .ok_or(CommandError::MissingPipe)?;
        let mut stdout_task = spawn_capture(stdout, logs.clone(), LogStream::Stdout);
        let mut stderr_task = spawn_capture(stderr, logs, LogStream::Stderr);
        let deadline = Instant::now() + spec.timeout;

        let process = wait_for_group(&mut child, deadline, &cancellation).await?;
        let captured =
            collect_output(&mut stdout_task, &mut stderr_task, deadline, &cancellation).await;

        match captured {
            OutputCollection::Complete(stdout, stderr) => {
                let (stdout, stdout_truncated) = stdout?;
                let (stderr, stderr_truncated) = stderr?;
                Ok(CommandOutput {
                    status: process.0,
                    exit_code: process.1,
                    stdout,
                    stderr,
                    stdout_truncated,
                    stderr_truncated,
                })
            }
            OutputCollection::TimedOut => {
                stdout_task.abort();
                stderr_task.abort();
                Ok(timed_out_output())
            }
            OutputCollection::Cancelled => {
                stdout_task.abort();
                stderr_task.abort();
                Ok(cancelled_output())
            }
        }
    }
}

fn spawn_capture(
    mut reader: impl AsyncRead + Unpin + Send + 'static,
    logs: Option<mpsc::Sender<LogEvent>>,
    stream: LogStream,
) -> CaptureTask {
    tokio::spawn(
        async move { read_bounded(&mut reader, MAX_CAPTURED_STREAM_BYTES, logs, stream).await },
    )
}

async fn wait_for_group(
    child: &mut AsyncGroupChild,
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<(ExecutionStatus, Option<i32>), CommandError> {
    let mut poll = tokio::time::interval(PROCESS_POLL_INTERVAL);
    loop {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                child.kill().await?;
                return Ok((ExecutionStatus::Cancelled, None));
            }
            _ = tokio::time::sleep_until(deadline) => {
                child.kill().await?;
                return Ok((ExecutionStatus::TimedOut, None));
            }
            _ = poll.tick() => {
                if let Some(process_status) = child.try_wait()? {
                    let status = if process_status.success() {
                        ExecutionStatus::Success
                    } else {
                        ExecutionStatus::Failed
                    };
                    return Ok((status, process_status.code()));
                }
            }
        }
    }
}

enum OutputCollection {
    Complete(
        Result<(Vec<u8>, bool), CommandError>,
        Result<(Vec<u8>, bool), CommandError>,
    ),
    TimedOut,
    Cancelled,
}

async fn collect_output(
    stdout_task: &mut CaptureTask,
    stderr_task: &mut CaptureTask,
    deadline: Instant,
    cancellation: &CancellationToken,
) -> OutputCollection {
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => OutputCollection::Cancelled,
        _ = tokio::time::sleep_until(deadline) => OutputCollection::TimedOut,
        (stdout, stderr) = async { tokio::join!(&mut *stdout_task, &mut *stderr_task) } => {
            OutputCollection::Complete(flatten_capture(stdout), flatten_capture(stderr))
        }
    }
}

fn flatten_capture(
    result: Result<CapturedStream, tokio::task::JoinError>,
) -> Result<(Vec<u8>, bool), CommandError> {
    result
        .map_err(CommandError::from)?
        .map_err(CommandError::from)
}

async fn read_bounded(
    reader: &mut (impl AsyncRead + Unpin),
    limit: usize,
    logs: Option<mpsc::Sender<LogEvent>>,
    stream: LogStream,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut retained = Vec::with_capacity(limit.min(8192));
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        if let Some(sender) = &logs {
            sender
                .send(LogEvent {
                    stream,
                    data: buffer[..count].to_vec(),
                })
                .await
                .map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "log sink closed")
                })?;
        }
        let remaining = limit.saturating_sub(retained.len());
        let retained_count = remaining.min(count);
        retained.extend_from_slice(&buffer[..retained_count]);
        truncated |= retained_count < count;
    }
    Ok((retained, truncated))
}

fn timed_out_output() -> CommandOutput {
    interrupted_output(ExecutionStatus::TimedOut)
}

fn cancelled_output() -> CommandOutput {
    interrupted_output(ExecutionStatus::Cancelled)
}

fn interrupted_output(status: ExecutionStatus) -> CommandOutput {
    CommandOutput {
        status,
        exit_code: None,
        stdout: Vec::new(),
        stderr: Vec::new(),
        stdout_truncated: true,
        stderr_truncated: true,
    }
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
