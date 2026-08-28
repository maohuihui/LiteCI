use std::{collections::BTreeMap, time::Duration};

use liteci::{CommandExecutor, CommandSpec, ExecutionStatus};
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

#[tokio::test]
async fn drains_large_stdout_and_stderr_without_deadlock() {
    let output = CommandExecutor::new()
        .execute(
            shell_spec(
                "for i in $(seq 1 10000); do printf 'stdout-line-%s\\n' \"$i\"; printf 'stderr-line-%s\\n' \"$i\" >&2; done",
            ),
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(output.status, ExecutionStatus::Success);
    assert!(output.stdout.len() > 100_000);
    assert!(output.stderr.len() > 100_000);
}

#[tokio::test]
async fn bounds_retained_output_while_draining_the_process() {
    let output = CommandExecutor::new()
        .execute(
            shell_spec(
                "for i in $(seq 1 120000); do printf '0123456789'; printf 'abcdefghij' >&2; done",
            ),
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(output.status, ExecutionStatus::Success);
    assert_eq!(output.stdout.len(), 1024 * 1024);
    assert_eq!(output.stderr.len(), 1024 * 1024);
    assert!(output.stdout_truncated);
    assert!(output.stderr_truncated);
}

#[tokio::test]
async fn applies_working_directory_and_environment() {
    let directory = std::env::temp_dir().join(format!("liteci-command-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("marker.txt"), "marker").unwrap();
    let mut spec = shell_spec("printf '%s|%s' \"$(cat marker.txt)\" \"$LITECI_TEST_VALUE\"");
    spec.working_directory = Some(directory.clone());
    spec.environment
        .insert("LITECI_TEST_VALUE".into(), "configured".into());

    let output = CommandExecutor::new()
        .execute(spec, CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(output.status, ExecutionStatus::Success);
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(output.stdout, b"marker|configured");
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn timeout_terminates_the_command() {
    let mut spec = shell_spec("sleep 5");
    spec.timeout = Duration::from_millis(100);

    let started = std::time::Instant::now();
    let output = CommandExecutor::new()
        .execute(spec, CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(output.status, ExecutionStatus::TimedOut);
    assert_eq!(output.exit_code, None);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn cancellation_terminates_the_command() {
    let cancellation = CancellationToken::new();
    let trigger = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        trigger.cancel();
    });

    let started = std::time::Instant::now();
    let output = CommandExecutor::new()
        .execute(shell_spec("sleep 5"), cancellation)
        .await
        .unwrap();

    assert_eq!(output.status, ExecutionStatus::Cancelled);
    assert_eq!(output.exit_code, None);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn cancellation_terminates_descendant_processes() {
    let directory =
        std::env::temp_dir().join(format!("liteci-descendant-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let marker = directory.join("descendant-finished");
    let script = format!(
        "(sleep 0.4; touch '{}') & wait",
        marker.to_string_lossy().replace('\\', "/")
    );
    let cancellation = CancellationToken::new();
    let trigger = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        trigger.cancel();
    });

    let output = CommandExecutor::new()
        .execute(shell_spec(&script), cancellation)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;

    assert_eq!(output.status, ExecutionStatus::Cancelled);
    assert!(!marker.exists(), "descendant process survived cancellation");
    std::fs::remove_dir_all(directory).unwrap();
}
