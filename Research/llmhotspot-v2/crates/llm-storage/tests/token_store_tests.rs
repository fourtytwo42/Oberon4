use llm_storage::{InMemoryTokenStore, StoredTokenRecord, TokenStore};

#[test]
fn token_store_crud() {
    let store = InMemoryTokenStore::default();
    let record = StoredTokenRecord::new("default", "openai", "refresh_1");

    store.save("default", "openai", &record).expect("save");
    let loaded = store
        .load("default", "openai")
        .expect("load")
        .expect("present");
    assert_eq!(loaded.refresh_token, "refresh_1");

    store.delete("default", "openai").expect("delete");
    assert!(store.load("default", "openai").expect("load").is_none());
}
