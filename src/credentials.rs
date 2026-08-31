use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use chacha20poly1305::{
    AeadCore, KeyInit, XChaCha20Poly1305,
    aead::{Aead, OsRng},
};

const VERSION: u8 = 1;
const NONCE_BYTES: usize = 24;

#[derive(Clone)]
pub struct CredentialCipher {
    cipher: XChaCha20Poly1305,
}

impl CredentialCipher {
    pub fn from_key_bytes(key: &[u8]) -> Result<Self, CredentialError> {
        if key.len() != 32 {
            return Err(CredentialError::InvalidKey);
        }
        Ok(Self {
            cipher: XChaCha20Poly1305::new_from_slice(key)
                .map_err(|_| CredentialError::InvalidKey)?,
        })
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<String, CredentialError> {
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher
            .encrypt(&nonce, plaintext)
            .map_err(|_| CredentialError::EncryptionFailed)?;
        let mut payload = Vec::with_capacity(1 + NONCE_BYTES + ciphertext.len());
        payload.push(VERSION);
        payload.extend_from_slice(&nonce);
        payload.extend_from_slice(&ciphertext);
        Ok(STANDARD_NO_PAD.encode(payload))
    }

    pub fn decrypt(&self, encoded: &str) -> Result<Vec<u8>, CredentialError> {
        let payload = STANDARD_NO_PAD
            .decode(encoded)
            .map_err(|_| CredentialError::InvalidCiphertext)?;
        if payload.len() <= 1 + NONCE_BYTES || payload[0] != VERSION {
            return Err(CredentialError::InvalidCiphertext);
        }
        let nonce = chacha20poly1305::XNonce::from_slice(&payload[1..1 + NONCE_BYTES]);
        self.cipher
            .decrypt(nonce, &payload[1 + NONCE_BYTES..])
            .map_err(|_| CredentialError::InvalidCiphertext)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("凭证主密钥无效")]
    InvalidKey,
    #[error("凭证加密失败")]
    EncryptionFailed,
    #[error("凭证密文无效")]
    InvalidCiphertext,
}
