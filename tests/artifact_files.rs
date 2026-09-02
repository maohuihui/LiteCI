use liteci::{ArtifactError, collect_file};

#[test]
fn artifact_paths_are_contained_regular_files_with_sha256_metadata() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("dist.txt"), b"artifact").unwrap();

    let (size, checksum, path) = collect_file(root.path(), "dist.txt").unwrap();

    assert_eq!(size, 8);
    assert_eq!(checksum.len(), 64);
    assert_eq!(
        path,
        std::fs::canonicalize(root.path().join("dist.txt")).unwrap()
    );
}

#[test]
fn artifact_paths_reject_parent_traversal_and_directories() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("dist")).unwrap();
    assert!(matches!(
        collect_file(root.path(), "../outside"),
        Err(ArtifactError::InvalidPath)
    ));
    assert!(matches!(
        collect_file(root.path(), "dist"),
        Err(ArtifactError::InvalidPath)
    ));
}
