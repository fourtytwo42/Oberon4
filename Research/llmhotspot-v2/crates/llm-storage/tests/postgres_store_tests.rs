use llm_storage::{
    BillingEventAuditRecord, BrokerStore, CustomerRecord, DeviceBindingRecord, LicenseRecord,
    PostgresStore, SubscriptionRecord,
};

fn test_db_url() -> Option<String> {
    std::env::var("LLMHOTSPOT_TEST_POSTGRES_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
}

#[tokio::test]
async fn postgres_broker_store_smoke_crud() {
    let Some(url) = test_db_url() else {
        return;
    };
    let store = PostgresStore::connect(&url)
        .await
        .expect("connect postgres");
    let now = 1_700_000_000_u64;
    let tag = format!("{}", uuid::Uuid::new_v4().simple());
    let customer_id = format!("cust_pg_{tag}");
    let subscription_id = format!("sub_pg_{tag}");
    let stripe_subscription_id = format!("sub_stripe_pg_{tag}");
    let license_id = format!("lic_pg_{tag}");
    let license_key = format!("LLMH-pg-{tag}");
    let binding_id = format!("bind_pg_{tag}");
    let event_id = format!("evt_pg_{tag}");
    let email = format!("pg-{tag}@example.com");
    let stripe_customer_id = format!("cus_pg_{tag}");

    store
        .upsert_customer(&CustomerRecord {
            customer_id: customer_id.clone(),
            email: email.clone(),
            stripe_customer_id: Some(stripe_customer_id),
            created_unix: now,
            updated_unix: now,
        })
        .await
        .expect("upsert customer");

    let cust = store
        .get_customer_by_email(&email)
        .await
        .expect("get customer")
        .expect("customer");
    assert_eq!(cust.customer_id, customer_id);

    store
        .upsert_subscription(&SubscriptionRecord {
            subscription_id: subscription_id.clone(),
            customer_id: cust.customer_id.clone(),
            stripe_subscription_id: Some(stripe_subscription_id),
            status: "active".to_string(),
            seat_limit: 2,
            current_period_end_unix: None,
            grace_expires_unix: None,
            created_unix: now,
            updated_unix: now,
        })
        .await
        .expect("upsert subscription");

    store
        .upsert_license(&LicenseRecord {
            license_id: license_id.clone(),
            customer_id: cust.customer_id.clone(),
            subscription_id: Some(subscription_id),
            license_key,
            plan_name: "pro".to_string(),
            status: "active".to_string(),
            seat_limit: 2,
            source: "stripe".to_string(),
            grace_expires_unix: None,
            created_unix: now,
            updated_unix: now,
        })
        .await
        .expect("upsert license");

    store
        .upsert_device_binding(&DeviceBindingRecord {
            binding_id: binding_id.clone(),
            license_id: license_id.clone(),
            seat_index: 1,
            device_id: "dev_pg_smoke".to_string(),
            device_name: None,
            os: None,
            country_code: None,
            last_ip_hash: None,
            app_version: None,
            label: None,
            tunnel_id: None,
            tunnel_hostname: None,
            status: "active".to_string(),
            first_seen_unix: now,
            last_seen_unix: now,
            retired_unix: None,
        })
        .await
        .expect("upsert binding");

    let active = store
        .list_active_bindings_for_license(&license_id)
        .await
        .expect("list active");
    assert!(!active.is_empty());

    let ev = BillingEventAuditRecord {
        event_id,
        event_type: "checkout.session.completed".to_string(),
        created_unix: now,
        payload_json: "{}".to_string(),
        received_unix: now,
        processed_unix: None,
        process_status: "received".to_string(),
        error: None,
    };
    let inserted = store
        .insert_billing_event_if_new(&ev)
        .await
        .expect("insert event");
    assert!(inserted);
}
