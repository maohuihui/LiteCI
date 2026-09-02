use std::{
    fs,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;

pub fn collect_file(
    workspace: &Path,
    relative_path: &str,
) -> Result<(u64, String, PathBuf), ArtifactError> {
    let relative = Path::new(relative_path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(ArtifactError::InvalidPath);
    }
    let workspace = fs::canonicalize(workspace).map_err(|_| ArtifactError::InvalidPath)?;
    let path = workspace.join(relative);
    let metadata = fs::symlink_metadata(&path).map_err(|_| ArtifactError::NotFound)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_ARTIFACT_BYTES
    {
        return Err(ArtifactError::InvalidPath);
    }
    let canonical = fs::canonicalize(&path).map_err(|_| ArtifactError::InvalidPath)?;
    if !canonical.starts_with(&workspace) {
        return Err(ArtifactError::InvalidPath);
    }
    let bytes = fs::read(&canonical).map_err(|_| ArtifactError::ReadFailed)?;
    let checksum = format!("{:x}", Sha256::digest(&bytes));
    Ok((metadata.len(), checksum, canonical))
}

#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("Artifact 路径无效")]
    InvalidPath,
    #[error("Artifact 文件不存在")]
    NotFound,
    #[error("Artifact 文件读取失败")]
    ReadFailed,
}
