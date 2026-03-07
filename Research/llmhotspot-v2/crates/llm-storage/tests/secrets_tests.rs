use llm_storage::{InMemorySecretStore, SecretStore, StorageError};

#[test]
fn secret_store_crud() {
    let store = InMemorySecretStore::default();
    store.put_secret("k", "s").expect("put");
    assert_eq!(store.get_secret("k").expect("get"), "s");
    store.delete_secret("k").expect("delete");
    assert!(matches!(store.get_secret("k"), Err(StorageError::NotFound)));
}

#[test]
fn secret_store_invalid_key() {
    let store = InMemorySecretStore::default();
    assert!(matches!(
        store.put_secret("", "x"),
        Err(StorageError::InvalidKey)
    ));
}
