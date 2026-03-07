use llm_storage::{InMemoryStateStore, StateStore, StorageError};

#[test]
fn state_store_crud() {
    let store = InMemoryStateStore::default();
    store.put("a", "b", None).expect("put");
    let got = store.get("a").expect("get");
    assert_eq!(got, "b");
    store.delete("a").expect("delete");
    assert!(matches!(store.get("a"), Err(StorageError::NotFound)));
}

#[test]
fn state_store_ttl_expiry() {
    let store = InMemoryStateStore::default();
    store.put("ttl", "v", Some(0)).expect("put");
    assert!(matches!(store.get("ttl"), Err(StorageError::NotFound)));
}
