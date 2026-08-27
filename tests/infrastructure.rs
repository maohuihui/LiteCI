use std::path::PathBuf;

use autoci::{StorageConfig, connect, migrate, prepare_storage};
use sqlx::Row;
use uuid::Uuid;

fn temp_root() -> PathBuf {
    std::env::temp_dir().join(format!("autoci-test-{}", Uuid::new_v4()))
}

#[tokio::test]
async fn every_pool_connection_enables_foreign_keys_and_wal() {
    let root = temp_root();
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("autoci.db");
    let url = format!("sqlite://{}?mode=rwc", database.display());
    let pool = connect(&url).await.unwrap();
    migrate(&pool).await.unwrap();

    let mut connections = Vec::new();
    for _ in 0..5 {
        connections.push(pool.acquire().await.unwrap());
    }
    for connection in &mut connections {
        let foreign_keys: i64 = sqlx::query("PRAGMA foreign_keys")
            .fetch_one(&mut **connection)
            .await
            .unwrap()
            .get(0);
        let journal_mode: String = sqlx::query("PRAGMA journal_mode")
            .fetch_one(&mut **connection)
            .await
            .unwrap()
            .get(0);
        assert_eq!(foreign_keys, 1);
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
    }

    drop(connections);
    pool.close().await;
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn storage_directories_are_created_before_startup() {
    let root = temp_root();
    let storage = StorageConfig {
        workspace: root.join("workspaces"),
        artifacts: root.join("artifacts"),
        logs: root.join("logs"),
    };

    prepare_storage(&storage).unwrap();

    assert!(storage.workspace.is_dir());
    assert!(storage.artifacts.is_dir());
    assert!(storage.logs.is_dir());
    std::fs::remove_dir_all(root).unwrap();
}
