use std::{net::IpAddr, path::PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub storage: StorageConfig,
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
            url: "sqlite://autoci.db".into(),
        }
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
        if let Some(value) = std::env::var_os("AUTOCI_HOST") {
            let value = value.to_string_lossy();
            config.server.host = value
                .parse()
                .map_err(|_| ConfigError::InvalidHost(value.into_owned()))?;
        }
        if let Some(value) = std::env::var_os("AUTOCI_PORT") {
            let value = value.to_string_lossy();
            config.server.port = value
                .parse()
                .map_err(|_| ConfigError::InvalidPort(value.into_owned()))?;
        }
        if let Some(value) = std::env::var_os("AUTOCI_DATABASE_URL") {
            config.database.url = value.to_string_lossy().into_owned();
        }
        Ok(config)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("AUTOCI_HOST 不是有效的 IP 地址: {0}")]
    InvalidHost(String),
    #[error("AUTOCI_PORT 不是有效的端口: {0}")]
    InvalidPort(String),
}
