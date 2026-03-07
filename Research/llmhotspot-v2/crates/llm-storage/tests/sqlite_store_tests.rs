use llm_storage::{
    AdminSessionRecord, AuditLogRecord, BillingEventAuditRecord, BillingReminderRecord,
    CustomerRecord, GiftEntitlementRecord, OtpDeliveryEventRecord, SqliteStore, StorageError,
    StoredTokenRecord, SubscriptionRecord,
};

#[tokio::test]
async fn sqlite_state_crud_and_expiry() {
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");

    store
        .put_state("k1", "v1", Some(10))
        .await
        .expect("put state");
    let got = store.get_state("k1", 0).await.expect("get state");
    assert_eq!(got, "v1");

    store
        .put_state("k2", "v2", Some(1))
        .await
        .expect("put state 2");
    let err = store.get_state("k2", 1).await.expect_err("expired");
    assert!(matches!(err, StorageError::NotFound));
}

#[tokio::test]
async fn sqlite_token_crud() {
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let record = StoredTokenRecord::new("default", "openai", "refresh_abc");

    store.save_token(&record).await.expect("save token");
    let loaded = store
        .load_token("default", "openai")
        .await
        .expect("load token")
        .expect("token exists");
    assert_eq!(loaded.refresh_token, "refresh_abc");

    store
        .delete_token("default", "openai")
        .await
        .expect("delete token");
    let missing = store
        .load_token("default", "openai")
        .await
        .expect("load token again");
    assert!(missing.is_none());
}

#[tokio::test]
async fn sqlite_oauth_account_and_usage_crud() {
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");

    store
        .upsert_oauth_account(&llm_storage::OAuthAccountRecord {
            account_id: "acct_1".to_string(),
            provider_id: "openai".to_string(),
            display_name: "Primary".to_string(),
            email: "user@example.com".to_string(),
            priority: 0,
            enabled: true,
            last_used_unix: None,
            last_error: None,
            upstream_base_url: None,
            chat_path: None,
            responses_path: None,
            org_id: None,
            project_id: None,
            extra_headers_json: None,
        })
        .await
        .expect("upsert account");
    store
        .upsert_oauth_usage(&llm_storage::OAuthUsageRecord {
            account_id: "acct_1".to_string(),
            window_type: "5h".to_string(),
            used: 3,
            limit: 100,
            reset_unix: 1234,
        })
        .await
        .expect("upsert usage");

    let accounts = store.list_oauth_accounts().await.expect("list accounts");
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].email, "user@example.com");
    let usage = store.list_oauth_usage("acct_1").await.expect("list usage");
    assert_eq!(usage.len(), 1);
    assert_eq!(usage[0].used, 3);
}

#[tokio::test]
async fn sqlite_api_key_and_remote_assignment() {
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");

    store
        .create_api_key(&llm_storage::ApiKeyRecord {
            key_id: "k1".to_string(),
            label: "test".to_string(),
            key_hash: "abc".to_string(),
            tool_choice_policy: "auto".to_string(),
            created_unix: 10,
            expires_unix: Some(100),
            revoked_unix: None,
            last_used_unix: None,
        })
        .await
        .expect("create key");
    store
        .set_remote_assignment(&llm_storage::RemoteAssignment {
            remote_base_url: Some("https://remote.example.com".to_string()),
            remote_console_url: Some("https://remote.example.com/console".to_string()),
        })
        .await
        .expect("set remote");

    let keys = store.list_api_keys().await.expect("list keys");
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].key_id, "k1");
    assert_eq!(keys[0].tool_choice_policy, "auto");
    let remote = store.get_remote_assignment().await.expect("get remote");
    assert_eq!(
        remote.remote_console_url.as_deref(),
        Some("https://remote.example.com/console")
    );
}

#[tokio::test]
async fn sqlite_api_key_lookup_by_hash_and_count() {
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");

    assert_eq!(store.count_api_keys().await.expect("count before"), 0);
    store
        .create_api_key(&llm_storage::ApiKeyRecord {
            key_id: "k_lookup".to_string(),
            label: "lookup".to_string(),
            key_hash: "hash_lookup".to_string(),
            tool_choice_policy: "auto".to_string(),
            created_unix: 1,
            expires_unix: None,
            revoked_unix: None,
            last_used_unix: None,
        })
        .await
        .expect("create key");

    assert_eq!(store.count_api_keys().await.expect("count after"), 1);
    let found = store
        .get_api_key_by_hash("hash_lookup")
        .await
        .expect("lookup")
        .expect("key exists");
    assert_eq!(found.key_id, "k_lookup");
    assert_eq!(
        store
            .get_api_key_by_hash("hash_missing")
            .await
            .expect("missing"),
        None
    );
}

#[tokio::test]
async fn customer_upsert_by_email_preserves_existing_customer_id() {
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");

    let first = CustomerRecord {
        customer_id: "cust_a".to_string(),
        email: "test@example.com".to_string(),
        stripe_customer_id: None,
        created_unix: 10,
        updated_unix: 10,
    };
    store.upsert_customer(&first).await.expect("insert first");

    let second = CustomerRecord {
        customer_id: "cust_b".to_string(),
        email: "test@example.com".to_string(),
        stripe_customer_id: Some("cus_123".to_string()),
        created_unix: 20,
        updated_unix: 20,
    };
    store.upsert_customer(&second).await.expect("upsert second");

    let got = store
        .get_customer_by_email("test@example.com")
        .await
        .expect("get by email")
        .expect("row");
    assert_eq!(got.customer_id, "cust_a");
    assert_eq!(got.stripe_customer_id.as_deref(), Some("cus_123"));
    assert_eq!(got.updated_unix, 20);
}

#[tokio::test]
async fn billing_event_insert_is_idempotent_and_status_updates() {
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let row = BillingEventAuditRecord {
        event_id: "evt_1".to_string(),
        event_type: "checkout.session.completed".to_string(),
        created_unix: 100,
        payload_json: "{\"id\":\"evt_1\"}".to_string(),
        received_unix: 101,
        processed_unix: None,
        process_status: "received".to_string(),
        error: None,
    };
    let inserted = store
        .insert_billing_event_if_new(&row)
        .await
        .expect("insert");
    assert!(inserted);
    let inserted_again = store
        .insert_billing_event_if_new(&row)
        .await
        .expect("insert again");
    assert!(!inserted_again);

    let updated = store
        .set_billing_event_result("evt_1", 102, "processed", None)
        .await
        .expect("set result");
    assert!(updated);
    let got = store
        .get_billing_event("evt_1")
        .await
        .expect("get")
        .expect("row");
    assert_eq!(got.process_status, "processed");
    assert_eq!(got.processed_unix, Some(102));
}

#[tokio::test]
async fn subscription_lookup_by_stripe_subscription_id_works() {
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let row = SubscriptionRecord {
        subscription_id: "sub_internal_1".to_string(),
        customer_id: "cust_1".to_string(),
        stripe_subscription_id: Some("sub_stripe_1".to_string()),
        status: "active".to_string(),
        seat_limit: 5,
        current_period_end_unix: Some(200),
        grace_expires_unix: Some(300),
        created_unix: 100,
        updated_unix: 100,
    };
    store.upsert_subscription(&row).await.expect("upsert");
    let got = store
        .get_subscription_by_stripe_subscription_id("sub_stripe_1")
        .await
        .expect("lookup")
        .expect("row");
    assert_eq!(got.subscription_id, "sub_internal_1");
    assert_eq!(got.customer_id, "cust_1");
    assert_eq!(got.seat_limit, 5);
}

#[tokio::test]
async fn billing_event_list_by_status_filters_and_limits() {
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    for i in 0..3_u64 {
        let row = BillingEventAuditRecord {
            event_id: format!("evt_{i}"),
            event_type: "invoice.paid".to_string(),
            created_unix: 100 + i,
            payload_json: "{}".to_string(),
            received_unix: 100 + i,
            processed_unix: None,
            process_status: if i == 2 {
                "processed".to_string()
            } else {
                "failed".to_string()
            },
            error: None,
        };
        store
            .insert_billing_event_if_new(&row)
            .await
            .expect("insert");
    }
    let failed = store
        .list_billing_events_by_status("failed", 10)
        .await
        .expect("list failed");
    assert_eq!(failed.len(), 2);
    let limited = store
        .list_billing_events_by_status("failed", 1)
        .await
        .expect("list limited");
    assert_eq!(limited.len(), 1);
}

#[tokio::test]
async fn billing_reminder_insert_is_idempotent_and_due_query_filters() {
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let row = BillingReminderRecord {
        reminder_id: "brm_1".to_string(),
        subscription_id: "sub_1".to_string(),
        license_id: Some("lic_1".to_string()),
        customer_email: "buyer@example.com".to_string(),
        reason: "payment_failed_immediate".to_string(),
        due_unix: 100,
        sent_unix: None,
        revoked_unix: None,
        attempt_count: 0,
        last_error: None,
        created_unix: 90,
        updated_unix: 90,
        idempotency_key: "sub_1:payment_failed_immediate:100".to_string(),
    };
    assert!(store
        .insert_billing_reminder_if_absent(&row)
        .await
        .expect("insert"));
    assert!(!store
        .insert_billing_reminder_if_absent(&row)
        .await
        .expect("insert again"));

    let due = store
        .list_due_unsent_billing_reminders(100, 10)
        .await
        .expect("list due");
    assert_eq!(due.len(), 1);

    store
        .mark_billing_reminder_sent("brm_1", 101)
        .await
        .expect("mark sent");
    let due_after_send = store
        .list_due_unsent_billing_reminders(200, 10)
        .await
        .expect("list due after send");
    assert_eq!(due_after_send.len(), 0);
}

#[tokio::test]
async fn billing_reminder_revoke_and_count_behaviors_work() {
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");

    for (id, reason, due) in [
        ("brm_1", "payment_failed_immediate", 50_u64),
        ("brm_2", "payment_failed_3d", 80_u64),
        ("brm_3", "cancel_3d", 90_u64),
    ] {
        store
            .insert_billing_reminder_if_absent(&BillingReminderRecord {
                reminder_id: id.to_string(),
                subscription_id: "sub_1".to_string(),
                license_id: None,
                customer_email: "buyer@example.com".to_string(),
                reason: reason.to_string(),
                due_unix: due,
                sent_unix: None,
                revoked_unix: None,
                attempt_count: 0,
                last_error: None,
                created_unix: 10,
                updated_unix: 10,
                idempotency_key: format!("sub_1:{reason}:{due}"),
            })
            .await
            .expect("insert");
    }
    assert_eq!(
        store
            .count_due_billing_reminders(100)
            .await
            .expect("count due"),
        3
    );

    let revoked = store
        .revoke_pending_billing_reminders("sub_1", &["payment_failed_"], 120)
        .await
        .expect("revoke");
    assert_eq!(revoked, 2);
    assert_eq!(
        store
            .list_due_unsent_billing_reminders(200, 10)
            .await
            .expect("list due")
            .len(),
        1
    );

    store
        .mark_billing_reminder_failed("brm_3", 130, Some("smtp_failed"))
        .await
        .expect("mark failed");
    assert_eq!(
        store
            .count_failed_billing_reminders_since(120)
            .await
            .expect("count failed"),
        1
    );
}

#[tokio::test]
async fn otp_rate_limit_and_verify_attempt_counters_work() {
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let c1 = store
        .otp_rate_limit_attempt("email:test@example.com", 100, 60)
        .await
        .expect("attempt 1");
    let c2 = store
        .otp_rate_limit_attempt("email:test@example.com", 120, 60)
        .await
        .expect("attempt 2");
    assert_eq!(c1, 1);
    assert_eq!(c2, 2);
    let c3 = store
        .otp_rate_limit_attempt("email:test@example.com", 200, 60)
        .await
        .expect("new window");
    assert_eq!(c3, 1);

    let a1 = store
        .otp_verify_attempt("ch_1", 100)
        .await
        .expect("verify 1");
    let a2 = store
        .otp_verify_attempt("ch_1", 101)
        .await
        .expect("verify 2");
    assert_eq!(a1, 1);
    assert_eq!(a2, 2);
    store
        .clear_otp_verify_attempts("ch_1")
        .await
        .expect("clear");
    let a3 = store
        .otp_verify_attempt("ch_1", 102)
        .await
        .expect("verify after clear");
    assert_eq!(a3, 1);
}

#[tokio::test]
async fn otp_delivery_event_insert_works() {
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let row = OtpDeliveryEventRecord {
        event_id: "od_1".to_string(),
        challenge_id: "ch_1".to_string(),
        email: "test@example.com".to_string(),
        provider: "resend".to_string(),
        status: "sent".to_string(),
        error: None,
        created_unix: 100,
    };
    store.insert_otp_delivery_event(&row).await.expect("insert");
}

#[tokio::test]
async fn admin_sessions_lifecycle_works() {
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let row = AdminSessionRecord {
        session_id: "as_1".to_string(),
        issued_unix: 100,
        expires_unix: 200,
        revoked_unix: None,
        source_ip: Some("127.0.0.1".to_string()),
        user_agent_hash: Some("abc".to_string()),
        created_by: "bootstrap_secret".to_string(),
    };
    store.create_admin_session(&row).await.expect("create");
    let got = store
        .get_admin_session("as_1")
        .await
        .expect("get")
        .expect("row");
    assert_eq!(got.source_ip.as_deref(), Some("127.0.0.1"));
    assert!(got.revoked_unix.is_none());

    let revoked = store
        .revoke_admin_session("as_1", 150)
        .await
        .expect("revoke");
    assert!(revoked);
    let got2 = store
        .get_admin_session("as_1")
        .await
        .expect("get 2")
        .expect("row");
    assert_eq!(got2.revoked_unix, Some(150));

    let deleted = store
        .prune_expired_admin_sessions(250)
        .await
        .expect("prune");
    assert_eq!(deleted, 1);
}

#[tokio::test]
async fn gift_entitlement_insert_list_and_status_work() {
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let row = GiftEntitlementRecord {
        gift_id: "gift_1".to_string(),
        license_id: "lic_1".to_string(),
        recipient_email: "gift@example.com".to_string(),
        seat_count: 2,
        starts_unix: 100,
        expires_unix: 200,
        created_unix: 100,
        created_by_admin_session: Some("as_1".to_string()),
        status: "active".to_string(),
    };
    store.insert_gift_entitlement(&row).await.expect("insert");
    let rows = store
        .list_gift_entitlements_for_license("lic_1")
        .await
        .expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].recipient_email, "gift@example.com");

    let changed = store
        .set_gift_entitlement_status("gift_1", "expired")
        .await
        .expect("status");
    assert!(changed);
    let rows2 = store
        .list_gift_entitlements_for_license("lic_1")
        .await
        .expect("list 2");
    assert_eq!(rows2[0].status, "expired");
}

#[tokio::test]
async fn audit_log_insert_and_list_work() {
    let store = SqliteStore::connect("sqlite::memory:")
        .await
        .expect("connect");
    let row = AuditLogRecord {
        audit_id: "aud_1".to_string(),
        actor_type: "admin_session".to_string(),
        actor_id: "as_1".to_string(),
        action: "license.rotate".to_string(),
        target_type: "license".to_string(),
        target_id: "lic_1".to_string(),
        payload_json: "{\"ok\":true}".to_string(),
        created_unix: 100,
        ip: Some("127.0.0.1".to_string()),
        outcome: "success".to_string(),
    };
    store.insert_audit_log(&row).await.expect("insert");
    let rows = store.list_audit_logs(10, 0).await.expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].action, "license.rotate");
    assert_eq!(rows[0].target_id, "lic_1");
}
