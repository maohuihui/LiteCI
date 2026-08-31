use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use liteci::{CommandExecutor, GitCredential, GitService, GitSnapshot};
use tokio_util::sync::CancellationToken;

fn spec(program: &str, args: &[&str], directory: Option<PathBuf>) -> liteci::CommandSpec {
    liteci::CommandSpec {
        program: program.into(),
        args: args.iter().map(|arg| (*arg).into()).collect(),
        working_directory: directory,
        environment: BTreeMap::new(),
        timeout: Duration::from_secs(10),
    }
}

async fn run(
    executor: &CommandExecutor,
    program: &str,
    args: &[&str],
    directory: Option<PathBuf>,
) -> liteci::CommandOutput {
    executor
        .execute(spec(program, args, directory), CancellationToken::new())
        .await
        .unwrap()
}

#[tokio::test]
async fn sync_fetches_commit_metadata_into_a_clean_workspace() {
    let executor = CommandExecutor::new();
    let root = std::env::temp_dir().join(format!("liteci-git-{}", uuid::Uuid::new_v4()));
    let source = root.join("source");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&source).unwrap();
    run(
        &executor,
        "git",
        &["init", "-b", "main"],
        Some(source.clone()),
    )
    .await;
    run(
        &executor,
        "git",
        &["config", "user.email", "test@example.invalid"],
        Some(source.clone()),
    )
    .await;
    run(
        &executor,
        "git",
        &["config", "user.name", "LiteCI Test"],
        Some(source.clone()),
    )
    .await;
    std::fs::write(source.join("README.md"), "hello\n").unwrap();
    run(
        &executor,
        "git",
        &["add", "README.md"],
        Some(source.clone()),
    )
    .await;
    run(
        &executor,
        "git",
        &["commit", "-m", "initial commit"],
        Some(source.clone()),
    )
    .await;

    let source_url = source.to_string_lossy().into_owned();
    let snapshot = GitService::new(executor)
        .sync(
            &source_url,
            "main",
            workspace.clone(),
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert!(workspace.join("README.md").is_file());
    assert_eq!(snapshot.branch, "main");
    assert_eq!(snapshot.commit_message, "initial commit");
    assert_eq!(snapshot.author, "LiteCI Test");
    assert_eq!(snapshot.commit_sha.len(), 40);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn sync_rejects_unsafe_refs_before_running_git() {
    let result = GitService::new(CommandExecutor::new())
        .sync(
            "https://example.com/repo.git",
            "main; touch hacked",
            std::env::temp_dir(),
            CancellationToken::new(),
        )
        .await;

    assert!(matches!(result, Err(liteci::GitError::InvalidRef)));
}

#[tokio::test]
async fn sync_rejects_repository_values_that_look_like_git_options() {
    let result = GitService::new(CommandExecutor::new())
        .sync(
            "--upload-pack=touch hacked",
            "main",
            std::env::temp_dir(),
            CancellationToken::new(),
        )
        .await;

    assert!(matches!(result, Err(liteci::GitError::InvalidRepository)));
}

#[tokio::test]
async fn https_credentials_are_rejected_for_non_https_repositories() {
    let root = std::env::temp_dir().join(format!("liteci-git-auth-{}", uuid::Uuid::new_v4()));
    let source = root.join("source");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&source).unwrap();
    let executor = CommandExecutor::new();
    run(
        &executor,
        "git",
        &["init", "-b", "main"],
        Some(source.clone()),
    )
    .await;
    run(
        &executor,
        "git",
        &["config", "user.email", "test@example.invalid"],
        Some(source.clone()),
    )
    .await;
    run(
        &executor,
        "git",
        &["config", "user.name", "LiteCI Test"],
        Some(source.clone()),
    )
    .await;
    std::fs::write(source.join("README.md"), "credentialed\n").unwrap();
    run(
        &executor,
        "git",
        &["add", "README.md"],
        Some(source.clone()),
    )
    .await;
    run(
        &executor,
        "git",
        &["commit", "-m", "credentialed"],
        Some(source.clone()),
    )
    .await;
    let result = GitService::new(executor)
        .sync_with_credential(
            &source.to_string_lossy(),
            "main",
            workspace,
            CancellationToken::new(),
            GitCredential::HttpsToken {
                username: "ci".into(),
                token: "secret".into(),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(result, liteci::GitError::InvalidCredential));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshot_exposes_the_required_commit_fields() {
    let snapshot = GitSnapshot {
        repository: "https://example.com/repo.git".into(),
        branch: "main".into(),
        commit_sha: "a".repeat(40),
        commit_message: "message".into(),
        author: "author".into(),
    };
    assert_eq!(snapshot.repository, "https://example.com/repo.git");
}
