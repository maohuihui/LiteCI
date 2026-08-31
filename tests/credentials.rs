use liteci::{CredentialCipher, CredentialError};

#[test]
fn encrypts_and_decrypts_without_exposing_plaintext() {
    let cipher = CredentialCipher::from_key_bytes(&[7_u8; 32]).unwrap();
    let encrypted = cipher.encrypt(b"username=git\ntoken=top-secret").unwrap();

    assert!(!encrypted.contains("top-secret"));
    assert_ne!(encrypted, "username=git\ntoken=top-secret");
    assert_eq!(
        cipher.decrypt(&encrypted).unwrap(),
        b"username=git\ntoken=top-secret"
    );
}

#[test]
fn encryption_is_randomized_and_tampering_is_rejected() {
    let cipher = CredentialCipher::from_key_bytes(&[9_u8; 32]).unwrap();
    let first = cipher.encrypt(b"private-key").unwrap();
    let second = cipher.encrypt(b"private-key").unwrap();
    assert_ne!(first, second);

    let mut bytes = second.into_bytes();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    let tampered = String::from_utf8(bytes).unwrap();
    assert!(matches!(
        cipher.decrypt(&tampered),
        Err(CredentialError::InvalidCiphertext)
    ));
}

#[test]
fn rejects_wrong_key_and_invalid_key_size() {
    assert!(matches!(
        CredentialCipher::from_key_bytes(&[1_u8; 16]),
        Err(CredentialError::InvalidKey)
    ));
    let cipher = CredentialCipher::from_key_bytes(&[1_u8; 32]).unwrap();
    let encrypted = cipher.encrypt(b"secret").unwrap();
    let other = CredentialCipher::from_key_bytes(&[2_u8; 32]).unwrap();
    assert!(matches!(
        other.decrypt(&encrypted),
        Err(CredentialError::InvalidCiphertext)
    ));
}
