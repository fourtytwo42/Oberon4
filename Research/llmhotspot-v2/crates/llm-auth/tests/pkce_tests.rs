use llm_auth::pkce_challenge_s256;

#[test]
fn pkce_output_is_urlsafe() {
    let out = pkce_challenge_s256("verifier");
    assert!(out
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
}
