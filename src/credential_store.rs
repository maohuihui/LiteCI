use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{CredentialCipher, CredentialError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    HttpsToken,
    SshKey,
}

impl CredentialKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::HttpsToken => "https_token",
            Self::SshKey => "ssh_key",
        }
    }

    fn parse(value: &str) -> Result<Self, CredentialStoreError> {
        match value {
            "https_token" => Ok(Self::HttpsToken),
            "ssh_key" => Ok(Self::SshKey),
            _ => Err(CredentialStoreError::InvalidKind),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewCredential {
    pub name: String,
    pub kind: CredentialKind,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CredentialSummary {
    pub id: String,
    pub name: String,
    pub kind: CredentialKind,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone)]
pub struct CredentialStore {
    pool: SqlitePool,
    cipher: CredentialCipher,
}

impl CredentialStore {
    pub fn new(pool: SqlitePool, cipher: CredentialCipher) -> Self {
        Self { pool, cipher }
    }

    pub async fn create(
        &self,
        input: NewCredential,
    ) -> Result<CredentialSummary, CredentialStoreError> {
        if input.name.trim().is_empty() || input.name.len() > 128 || input.payload.is_empty() {
            return Err(CredentialStoreError::InvalidInput);
        }
        let id = Uuid::new_v4().to_string();
        let encrypted_payload = self.cipher.encrypt(&input.payload)?;
        sqlx::query(
            "INSERT INTO credentials (id, name, kind, encrypted_payload) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(&id)
        .bind(input.name.trim())
        .bind(input.kind.as_str())
        .bind(encrypted_payload)
        .execute(&self.pool)
        .await
        .map_err(map_db_error)?;
        self.find(&id).await?.ok_or(CredentialStoreError::NotFound)
    }

    pub async fn list(&self) -> Result<Vec<CredentialSummary>, CredentialStoreError> {
        let rows = sqlx::query_as::<_, (String, String, String, String, String)>(
            "SELECT id, name, kind, created_at, updated_at FROM credentials ORDER BY name ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|(id, name, kind, created_at, updated_at)| {
                let kind = CredentialKind::parse(&kind)?;
                Ok(CredentialSummary {
                    id,
                    name,
                    kind,
                    created_at,
                    updated_at,
                })
            })
            .collect()
    }

    pub async fn delete(&self, id: &str) -> Result<(), CredentialStoreError> {
        let result = sqlx::query("DELETE FROM credentials WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(CredentialStoreError::NotFound);
        }
        Ok(())
    }

    pub async fn decrypt(&self, id: &str) -> Result<Vec<u8>, CredentialStoreError> {
        let payload: String =
            sqlx::query_scalar("SELECT encrypted_payload FROM credentials WHERE id = ?1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?
                .ok_or(CredentialStoreError::NotFound)?;
        Ok(self.cipher.decrypt(&payload)?)
    }

    pub async fn kind(&self, id: &str) -> Result<CredentialKind, CredentialStoreError> {
        let value: String = sqlx::query_scalar("SELECT kind FROM credentials WHERE id = ?1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(CredentialStoreError::NotFound)?;
        CredentialKind::parse(&value)
    }

    async fn find(&self, id: &str) -> Result<Option<CredentialSummary>, CredentialStoreError> {
        let row = sqlx::query_as::<_, (String, String, String, String, String)>(
            "SELECT id, name, kind, created_at, updated_at FROM credentials WHERE id = ?1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|(id, name, kind, created_at, updated_at)| {
            let kind = CredentialKind::parse(&kind)?;
            Ok(CredentialSummary {
                id,
                name,
                kind,
                created_at,
                updated_at,
            })
        })
        .transpose()
    }
}

fn map_db_error(error: sqlx::Error) -> CredentialStoreError {
    if matches!(&error, sqlx::Error::Database(database) if database.message().contains("UNIQUE")) {
        CredentialStoreError::Conflict
    } else {
        CredentialStoreError::Database(error)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialStoreError {
    #[error("凭证输入无效")]
    InvalidInput,
    #[error("凭证不存在")]
    NotFound,
    #[error("凭证名称已存在")]
    Conflict,
    #[error("凭证类型无效")]
    InvalidKind,
    #[error(transparent)]
    Cipher(#[from] CredentialError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}
