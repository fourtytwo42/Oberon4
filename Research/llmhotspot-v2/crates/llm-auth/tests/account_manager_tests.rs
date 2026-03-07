use llm_auth::OAuthAccountManager;
use llm_storage::SqliteStore;

#[tokio::test]
async fn account_priority_and_usage_tracking() {
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let mgr = OAuthAccountManager::new(store.clone());

    mgr.connect_account("acct_1", "Primary", "one@example.com")
        .await
        .expect("connect 1");
    mgr.connect_account("acct_2", "Secondary", "two@example.com")
        .await
        .expect("connect 2");

    mgr.activate("acct_2").await.expect("activate");
    let chain = mgr.select_failover_chain().await.expect("chain");
    assert_eq!(chain[0].account_id, "acct_2");

    mgr.mark_success("acct_2").await.expect("mark success");
    let listed = mgr.list_accounts().await.expect("list");
    let acct2 = listed
        .into_iter()
        .find(|x| x.account.account_id == "acct_2")
        .expect("acct2 exists");
    assert!(!acct2.usage.is_empty());
}
