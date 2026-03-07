use llm_auth::openai_chatgpt_account_id;

#[test]
fn claim_path_exact_key_works() {
    let token =
        "x.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjdF8xIn19.y";
    let id = openai_chatgpt_account_id(token, "https://api.openai.com/auth");
    assert_eq!(id.as_deref(), Some("acct_1"));
}

#[test]
fn claim_path_fallback_finds_nested_custom_shape() {
    let token =
        "x.eyJjdXN0b20iOnsibmVzdGVkIjp7ImNoYXRncHRfYWNjb3VudF9pZCI6ImFjY3RfY3VzdG9tIn19fQ.y";
    let id = openai_chatgpt_account_id(token, "https://api.openai.com/auth");
    assert_eq!(id.as_deref(), Some("acct_custom"));
}

#[test]
fn claim_path_with_trailing_slash_still_works() {
    let token =
        "x.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjdF8yIn19.y";
    let id = openai_chatgpt_account_id(token, "https://api.openai.com/auth/");
    assert_eq!(id.as_deref(), Some("acct_2"));
}
