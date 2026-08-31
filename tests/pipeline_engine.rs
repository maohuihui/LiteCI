use liteci::{PipelineEngine, connect, migrate};
use serde_json::json;
use sqlx::SqlitePool;
use tempfile::TempDir;
use tokio::time::{Duration, sleep, timeout};

async fn fixture(stages: &[(&str, serde_json::Value, bool, i64)]) -> (SqlitePool, TempDir, String) {
    let pool = connect("sqlite::memory:").await.unwrap();
    migrate(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash) VALUES ('user-1', 'admin', 'hash')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO projects (id, name, git_url, default_branch, workspace_path) VALUES ('11111111-1111-4111-8111-111111111111', 'engine', 'https://example.com/repo.git', 'main', '11111111-1111-4111-8111-111111111111')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO pipeline_runs (id, project_id, run_number, branch, created_by) VALUES ('22222222-2222-4222-8222-222222222222', '11111111-1111-4111-8111-111111111111', 1, 'main', 'user-1')")
        .execute(&pool)
        .await
        .unwrap();
    for (position, (name, command, enabled, timeout)) in stages.iter().enumerate() {
        sqlx::query("INSERT INTO stage_runs (id, run_id, position, name, command, enabled, status, timeout_seconds) VALUES (?1, '22222222-2222-4222-8222-222222222222', ?2, ?3, ?4, ?5, ?6, ?7)")
            .bind(format!("stage-{position}"))
            .bind(i64::try_from(position).unwrap())
            .bind(*name)
            .bind(command.to_string())
            .bind(*enabled)
            .bind(if *enabled { "pending" } else { "skipped" })
            .bind(*timeout)
            .execute(&pool)
            .await
            .unwrap();
    }
    (
        pool,
        tempfile::tempdir().unwrap(),
        "22222222-2222-4222-8222-222222222222".to_owned(),
    )
}

fn process(program: &str, args: &[&str]) -> serde_json::Value {
    json!({"mode":"process", "program":program, "args":args})
}

fn shell(script: &str) -> serde_json::Value {
    json!({"mode":"shell", "script":script})
}

#[tokio::test]
async fn executes_enabled_stages_serially_and_marks_run_success() {
    let (pool, workspace, run_id) = fixture(&[
        ("first", shell(success_script("first.txt")), true, 30),
        ("disabled", shell(success_script("disabled.txt")), false, 30),
        ("second", shell(success_script("second.txt")), true, 30),
    ])
    .await;
    let engine = PipelineEngine::new(pool.clone(), workspace.path(), 1);

    engine.execute(&run_id).await.unwrap();

    let status: String = sqlx::query_scalar("SELECT status FROM pipeline_runs WHERE id = ?1")
        .bind(&run_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "success");
    let statuses: Vec<String> =
        sqlx::query_scalar("SELECT status FROM stage_runs WHERE run_id = ?1 ORDER BY position")
            .bind(&run_id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(statuses, ["success", "skipped", "success"]);
    let run_workspace = workspace
        .path()
        .join("11111111-1111-4111-8111-111111111111")
        .join("22222222-2222-4222-8222-222222222222");
    assert!(run_workspace.join("first.txt").exists());
    assert!(!run_workspace.join("disabled.txt").exists());
    assert!(run_workspace.join("second.txt").exists());
}

#[tokio::test]
async fn stops_after_a_failed_stage_and_skips_remaining_stages() {
    let (pool, workspace, run_id) = fixture(&[
        ("failure", process(test_program(), failure_args()), true, 30),
        ("never", shell(success_script("never.txt")), true, 30),
    ])
    .await;
    let engine = PipelineEngine::new(pool.clone(), workspace.path(), 1);

    engine.execute(&run_id).await.unwrap();

    let status: String = sqlx::query_scalar("SELECT status FROM pipeline_runs WHERE id = ?1")
        .bind(&run_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "failed");
    let statuses: Vec<String> =
        sqlx::query_scalar("SELECT status FROM stage_runs WHERE run_id = ?1 ORDER BY position")
            .bind(&run_id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(statuses, ["failed", "skipped"]);
    assert!(!workspace.path().join("11111111-1111-4111-8111-111111111111/22222222-2222-4222-8222-222222222222/never.txt").exists());
}

#[tokio::test]
async fn cancellation_stops_the_running_stage_and_converges_statuses() {
    let (pool, workspace, run_id) = fixture(&[
        ("slow", shell(slow_script()), true, 30),
        ("never", shell(success_script("never.txt")), true, 30),
    ])
    .await;
    let engine = PipelineEngine::new(pool.clone(), workspace.path(), 1);
    let running = {
        let engine = engine.clone();
        let run_id = run_id.clone();
        tokio::spawn(async move { engine.execute(&run_id).await })
    };
    timeout(Duration::from_secs(5), async {
        loop {
            let status: String =
                sqlx::query_scalar("SELECT status FROM stage_runs WHERE id = 'stage-0'")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            if status == "running" {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    assert!(engine.cancel(&run_id).await);
    running.await.unwrap().unwrap();

    let run_status: String = sqlx::query_scalar("SELECT status FROM pipeline_runs WHERE id = ?1")
        .bind(&run_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(run_status, "cancelled");
    let statuses: Vec<String> =
        sqlx::query_scalar("SELECT status FROM stage_runs WHERE run_id = ?1 ORDER BY position")
            .bind(&run_id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(statuses, ["cancelled", "skipped"]);
    assert!(!workspace.path().join("11111111-1111-4111-8111-111111111111/22222222-2222-4222-8222-222222222222/never.txt").exists());
}

#[tokio::test]
async fn runs_from_the_same_project_do_not_execute_concurrently() {
    let (pool, workspace, first_id) = fixture(&[("slow", shell(slow_script()), true, 30)]).await;
    sqlx::query("INSERT INTO pipeline_runs (id, project_id, run_number, branch, created_by) VALUES ('33333333-3333-4333-8333-333333333333', '11111111-1111-4111-8111-111111111111', 2, 'main', 'user-1')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO stage_runs (id, run_id, position, name, command, enabled, status, timeout_seconds) VALUES ('stage-2', '33333333-3333-4333-8333-333333333333', 0, 'slow', ?1, 1, 'pending', 30)")
        .bind(shell(slow_script()).to_string())
        .execute(&pool).await.unwrap();
    let engine = PipelineEngine::new(pool.clone(), workspace.path(), 2);
    let first = {
        let engine = engine.clone();
        let first_id = first_id.clone();
        tokio::spawn(async move { engine.execute(&first_id).await })
    };
    wait_for_status(&pool, "stage-0", "running").await;
    let second = {
        let engine = engine.clone();
        tokio::spawn(async move { engine.execute("33333333-3333-4333-8333-333333333333").await })
    };
    sleep(Duration::from_millis(150)).await;
    let second_status: String = sqlx::query_scalar(
        "SELECT status FROM pipeline_runs WHERE id = '33333333-3333-4333-8333-333333333333'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(second_status, "pending");

    assert!(engine.cancel(&first_id).await);
    first.await.unwrap().unwrap();
    wait_for_status(&pool, "stage-2", "running").await;
    assert!(engine.cancel("33333333-3333-4333-8333-333333333333").await);
    second.await.unwrap().unwrap();
}

async fn wait_for_status(pool: &SqlitePool, stage_id: &str, expected: &str) {
    timeout(Duration::from_secs(5), async {
        loop {
            let status: String = sqlx::query_scalar("SELECT status FROM stage_runs WHERE id = ?1")
                .bind(stage_id)
                .fetch_one(pool)
                .await
                .unwrap();
            if status == expected {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn invalid_stage_command_fails_the_run_and_skips_remaining_stages() {
    let (pool, workspace, run_id) = fixture(&[
        ("invalid", json!({"mode":"unknown"}), true, 30),
        ("never", shell(success_script("never.txt")), true, 30),
    ])
    .await;
    let engine = PipelineEngine::new(pool.clone(), workspace.path(), 1);

    assert!(engine.execute(&run_id).await.is_err());

    let run_status: String = sqlx::query_scalar("SELECT status FROM pipeline_runs WHERE id = ?1")
        .bind(&run_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(run_status, "failed");
    let statuses: Vec<String> =
        sqlx::query_scalar("SELECT status FROM stage_runs WHERE run_id = ?1 ORDER BY position")
            .bind(&run_id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(statuses, ["failed", "skipped"]);
}

#[cfg(windows)]
fn test_program() -> &'static str {
    "cmd"
}
#[cfg(not(windows))]
fn test_program() -> &'static str {
    "sh"
}

#[cfg(windows)]
fn success_script(file: &str) -> &str {
    Box::leak(format!("type nul > {file}").into_boxed_str())
}
#[cfg(not(windows))]
fn success_script(file: &str) -> &str {
    Box::leak(format!("touch {file}").into_boxed_str())
}

#[cfg(windows)]
fn failure_args() -> &'static [&'static str] {
    &["/C", "exit", "7"]
}
#[cfg(not(windows))]
fn failure_args() -> &'static [&'static str] {
    &["-c", "exit 7"]
}

#[cfg(windows)]
fn slow_script() -> &'static str {
    "ping -n 30 127.0.0.1 >nul"
}
#[cfg(not(windows))]
fn slow_script() -> &'static str {
    "sleep 30"
}
