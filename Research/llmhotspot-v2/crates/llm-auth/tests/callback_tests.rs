use llm_auth::{start_oauth_callback_server, AuthError};

#[tokio::test]
async fn callback_success() {
    let server = start_oauth_callback_server(None, Some(0), Some("/auth/callback"), None)
        .await
        .expect("server");

    let url = format!("http://{}/auth/callback?state=abc&code=xyz", server.addr);
    let resp = reqwest::get(url).await.expect("req");
    assert!(resp.status().is_success());

    let payload = server.callback_rx.await.expect("rx").expect("payload");
    assert_eq!(payload.state, "abc");
    assert_eq!(payload.code, "xyz");
}

#[tokio::test]
async fn callback_missing_fields() {
    let server = start_oauth_callback_server(None, Some(0), Some("/auth/callback"), None)
        .await
        .expect("server");

    let url = format!("http://{}/auth/callback?state=abc", server.addr);
    let resp = reqwest::get(url).await.expect("req");
    assert!(resp.status().is_success());

    let err = server.callback_rx.await.expect("rx").expect_err("err");
    assert!(matches!(err, AuthError::CallbackMissingFields));
}
