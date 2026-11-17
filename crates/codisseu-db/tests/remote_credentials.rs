//! Explicit own-entry roundtrip in the normal user's OS credential store.
//! Never enumerates, reads, updates or deletes other applications' credentials.
use codisseu_db::{remote::ConnectionConfig, remote_credentials::CredentialStore};

#[test]
#[ignore = "explicit normal desktop keyring test; CODISSEU_TEST_OS_KEYRING=1"]
fn own_system_keyring_entry_survives_store_recreation_and_is_deleted() {
    assert_eq!(
        std::env::var("CODISSEU_TEST_OS_KEYRING").as_deref(),
        Ok("1")
    );
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let config = ConnectionConfig {
        id: format!("owned-keyring-test-{}-{stamp}", std::process::id()),
        name: "Disposable Codisseu keyring validation".into(),
        host: "keyring-fixture.invalid".into(),
        username: "synthetic-reader".into(),
        database: "synthetic-database".into(),
        ..Default::default()
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let store = CredentialStore::new();
        assert!(
            store
                .load(&config)
                .await
                .expect("own entry lookup")
                .is_none()
        );
        store
            .store(&config, "synthetic-only-codisseu-keyring-test".into())
            .await
            .expect("own entry storage");
        // Separate store/worker demonstrates the secret is OS persisted, not in the wrapper.
        let reopened = CredentialStore::new();
        let result = reopened.load(&config).await;
        let cleanup = reopened.delete(&config).await;
        assert!(
            cleanup.is_ok(),
            "own entry cleanup failed; no other credential was touched"
        );
        let secret = result
            .expect("own persisted entry lookup")
            .expect("own entry exists");
        assert_eq!(secret.expose(), "synthetic-only-codisseu-keyring-test");
        assert!(
            reopened
                .load(&config)
                .await
                .expect("own deleted entry lookup")
                .is_none()
        );
    });
}
