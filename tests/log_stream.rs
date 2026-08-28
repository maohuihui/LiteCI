use std::{collections::BTreeMap, time::Duration};

use liteci::{CommandExecutor, CommandSpec, ExecutionStatus, LogStream};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn shell_spec(script: &str) -> CommandSpec {
    CommandSpec {
        program: "bash".into(),
        args: vec!["-lc".into(), script.into()],
        working_directory: None,
        environment: BTreeMap::new(),
        timeout: Duration::from_secs(5),
    }
}

#[tokio::test]
async fn emits_stdout_and_stderr_events_while_running() {
    let (sender, mut receiver) = mpsc::channel::<liteci::LogEvent>(16);
    let output = CommandExecutor::new()
        .execute_with_logs(
            shell_spec("printf 'out'; sleep 0.2; printf 'err' >&2"),
            CancellationToken::new(),
            sender,
        )
        .await
        .unwrap();

    let mut events = Vec::new();
    while let Ok(event) = receiver.try_recv() {
        events.push(event);
    }
    assert_eq!(output.status, ExecutionStatus::Success);
    assert!(
        events
            .iter()
            .any(|event| event.stream == LogStream::Stdout && event.data == b"out")
    );
    assert!(
        events
            .iter()
            .any(|event| event.stream == LogStream::Stderr && event.data == b"err")
    );
}

#[tokio::test]
async fn log_events_preserve_chunks_beyond_retained_output_limit() {
    let (sender, mut receiver) = mpsc::channel::<liteci::LogEvent>(16);
    let consumer = tokio::spawn(async move {
        let mut streamed_bytes = 0;
        while let Some(event) = receiver.recv().await {
            if event.stream == LogStream::Stdout {
                streamed_bytes += event.data.len();
            }
        }
        streamed_bytes
    });
    let output = CommandExecutor::new()
        .execute_with_logs(
            shell_spec("for i in $(seq 1 120000); do printf '0123456789'; done"),
            CancellationToken::new(),
            sender,
        )
        .await
        .unwrap();

    let streamed_bytes = consumer.await.unwrap();
    assert_eq!(output.status, ExecutionStatus::Success);
    assert_eq!(output.stdout.len(), 1024 * 1024);
    assert!(output.stdout_truncated);
    assert!(streamed_bytes > output.stdout.len());
}

#[tokio::test]
async fn captures_stdout_stderr_and_exit_code() {
    let output = CommandExecutor::new()
        .execute(
            shell_spec("printf 'out'; printf 'err' >&2; exit 7"),
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(output.status, ExecutionStatus::Failed);
    assert_eq!(output.exit_code, Some(7));
    assert_eq!(output.stdout, b"out");
    assert_eq!(output.stderr, b"err");
    assert!(!output.stdout_truncated);
    assert!(!output.stderr_truncated);
}
