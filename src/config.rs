use std::{
    net::IpAddr,
    path::{Path, PathBuf},
};

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    pub setup_token: Option<String>,
    pub credential_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub host: IpAddr,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StorageConfig {
    pub workspace: PathBuf,
    pub artifacts: PathBuf,
    pub logs: PathBuf,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: [127, 0, 0, 1].into(),
            port: 3000,
        }
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: default_database_url(),
        }
    }
}

fn default_database_url() -> String {
    default_database_url_for(Path::new("liteci.db"), Path::new("autoci.db"))
}

fn default_database_url_for(liteci: &Path, autoci: &Path) -> String {
    if !liteci.exists() && autoci.exists() {
        "sqlite://autoci.db".into()
    } else {
        "sqlite://liteci.db".into()
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            workspace: "./data/workspaces".into(),
            artifacts: "./data/artifacts".into(),
            logs: "./data/logs".into(),
        }
    }
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let mut config = Self::default();
        if let Some(value) = env_with_legacy("LITECI_HOST", "AUTOCI_HOST") {
            let value = value.to_string_lossy();
            config.server.host = value
                .parse()
                .map_err(|_| ConfigError::InvalidHost(value.into_owned()))?;
        }
        if let Some(value) = env_with_legacy("LITECI_PORT", "AUTOCI_PORT") {
            let value = value.to_string_lossy();
            config.server.port = value
                .parse()
                .map_err(|_| ConfigError::InvalidPort(value.into_owned()))?;
        }
        if let Some(value) = env_with_legacy("LITECI_DATABASE_URL", "AUTOCI_DATABASE_URL") {
            config.database.url = value.to_string_lossy().into_owned();
        }
        config.setup_token = std::env::var("LITECI_SETUP_TOKEN").ok();
        config.credential_key = std::env::var("LITECI_CREDENTIAL_KEY").ok();
        Ok(config)
    }
}

fn env_with_legacy(current: &str, legacy: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(current).or_else(|| std::env::var_os(legacy))
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("LITECI_HOST 不是有效的 IP 地址: {0}")]
    InvalidHost(String),
    #[error("LITECI_PORT 不是有效的端口: {0}")]
    InvalidPort(String),
}

#[cfg(test)]
mod tests {
    use super::default_database_url_for;
    use std::{env, fs};
    use uuid::Uuid;

    #[test]
    fn legacy_database_is_used_when_liteci_database_does_not_exist() {
        let root = env::temp_dir().join(format!("liteci-config-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("autoci.db"), []).unwrap();

        let url = default_database_url_for(&root.join("liteci.db"), &root.join("autoci.db"));

        fs::remove_dir_all(root).unwrap();
        assert_eq!(url, "sqlite://autoci.db");
    }
}
