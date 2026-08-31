use liteci::{CredentialCipher, CredentialKind, CredentialStore, NewCredential};

#[tokio::test]
async fn stores_only_encrypted_credential_payload_and_lists_metadata() {
    let pool = liteci::connect("sqlite::memory:").await.unwrap();
    liteci::migrate(&pool).await.unwrap();
    let store = CredentialStore::new(
        pool.clone(),
        CredentialCipher::from_key_bytes(&[3_u8; 32]).unwrap(),
    );

    let created = store
        .create(NewCredential {
            name: "gitee-main".into(),
            kind: CredentialKind::HttpsToken,
            payload: b"username=ci\ntoken=secret-value".to_vec(),
        })
        .await
        .unwrap();
    assert_eq!(created.name, "gitee-main");
    assert_eq!(created.kind, CredentialKind::HttpsToken);
    assert!(!created.id.is_empty());

    let raw: String = sqlx::query_scalar("SELECT encrypted_payload FROM credentials WHERE id = ?1")
        .bind(&created.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!raw.contains("secret-value"));

    let listed = store.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "gitee-main");
    assert_eq!(listed[0].kind, CredentialKind::HttpsToken);
}

#[tokio::test]
async fn decrypts_a_credential_only_when_explicitly_requested() {
    let pool = liteci::connect("sqlite::memory:").await.unwrap();
    liteci::migrate(&pool).await.unwrap();
    let store = CredentialStore::new(pool, CredentialCipher::from_key_bytes(&[4_u8; 32]).unwrap());
    let created = store
        .create(NewCredential {
            name: "deploy-key".into(),
            kind: CredentialKind::SshKey,
            payload: b"private-key".to_vec(),
        })
        .await
        .unwrap();

    assert_eq!(store.decrypt(&created.id).await.unwrap(), b"private-key");
}
