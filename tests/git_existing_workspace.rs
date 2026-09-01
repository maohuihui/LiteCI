use liteci::{CommandExecutor, GitService};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn git_service_clones_into_an_existing_empty_run_workspace() {
    let source = tempfile::tempdir().unwrap();
    let workspace_root = tempfile::tempdir().unwrap();
    let workspace = workspace_root.path().join("run");
    std::fs::create_dir(&workspace).unwrap();
    git(source.path(), &["init", "-b", "main"]).await;
    git(
        source.path(),
        &["config", "user.email", "test@example.invalid"],
    )
    .await;
    git(source.path(), &["config", "user.name", "LiteCI Test"]).await;
    std::fs::write(source.path().join("README.md"), "hello\n").unwrap();
    git(source.path(), &["add", "README.md"]).await;
    git(source.path(), &["commit", "-m", "initial"]).await;

    let workspace = std::fs::canonicalize(&workspace).unwrap();
    let snapshot = GitService::new(CommandExecutor::new())
        .sync(
            &source.path().to_string_lossy(),
            "main",
            workspace.clone(),
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(snapshot.commit_message, "initial");
    assert!(
        std::fs::read_to_string(workspace.join("README.md"))
            .unwrap()
            .starts_with("hello")
    );
}

#[tokio::test]
async fn git_service_does_not_modify_a_non_empty_workspace() {
    let source = tempfile::tempdir().unwrap();
    let workspace_root = tempfile::tempdir().unwrap();
    let workspace = workspace_root.path().join("run");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(workspace.join("keep.txt"), "keep").unwrap();
    git(source.path(), &["init", "-b", "main"]).await;
    git(
        source.path(),
        &["config", "user.email", "test@example.invalid"],
    )
    .await;
    git(source.path(), &["config", "user.name", "LiteCI Test"]).await;
    std::fs::write(source.path().join("README.md"), "hello\n").unwrap();
    git(source.path(), &["add", "README.md"]).await;
    git(source.path(), &["commit", "-m", "initial"]).await;

    let result = GitService::new(CommandExecutor::new())
        .sync(
            &source.path().to_string_lossy(),
            "main",
            workspace.clone(),
            CancellationToken::new(),
        )
        .await;

    assert!(result.is_err());
    assert_eq!(
        std::fs::read_to_string(workspace.join("keep.txt")).unwrap(),
        "keep"
    );
    assert!(!workspace.join(".git").exists());
}

#[tokio::test]
async fn git_service_rejects_a_symlinked_git_metadata_path() {
    let source = tempfile::tempdir().unwrap();
    let workspace_root = tempfile::tempdir().unwrap();
    let workspace = workspace_root.path().join("run");
    std::fs::create_dir(&workspace).unwrap();
    let external = workspace_root.path().join("external-git");
    std::fs::create_dir(&external).unwrap();
    std::fs::write(
        workspace.join(".git"),
        format!("gitdir: {}\n", external.display()),
    )
    .unwrap();

    let result = GitService::new(CommandExecutor::new())
        .sync(
            &source.path().to_string_lossy(),
            "main",
            workspace,
            CancellationToken::new(),
        )
        .await;

    assert!(result.is_err());
}

async fn git(directory: &std::path::Path, args: &[&str]) {
    let status = tokio::process::Command::new("git")
        .args(args)
        .current_dir(directory)
        .status()
        .await
        .unwrap();
    assert!(status.success());
}
