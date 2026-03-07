use llm_foundation::contracts::{ChatRequest, ErrorCode, ErrorEnvelope, OpenAiRequestEnvelope};

#[test]
fn contract_roundtrip_from_fixture() {
    let fixture = include_str!("fixtures/chat_request.json");
    let parsed: ChatRequest = serde_json::from_str(fixture).expect("fixture parse");
    assert_eq!(parsed.model, "gpt-5.2");
    assert!(parsed.stream);

    let re = serde_json::to_string(&parsed).expect("serialize");
    let parsed_again: ChatRequest = serde_json::from_str(&re).expect("reparse");
    assert_eq!(parsed_again, parsed);
}

#[test]
fn error_envelope_roundtrip() {
    let err = ErrorEnvelope {
        code: ErrorCode::AuthError,
        message: "invalid key".to_string(),
    };

    let raw = serde_json::to_string(&err).expect("serialize");
    let parsed: ErrorEnvelope = serde_json::from_str(&raw).expect("parse");
    assert_eq!(parsed, err);
}

#[test]
fn openai_envelope_preserves_unknown_fields() {
    let parsed: OpenAiRequestEnvelope = serde_json::from_str(
        r#"{
          "model":"gpt-5.2",
          "stream":true,
          "messages":[{"role":"user","content":"hello"}],
          "metadata":{"k":"v"},
          "custom_field":{"x":1}
        }"#,
    )
    .expect("parse");
    assert_eq!(parsed.model(), Some("gpt-5.2"));
    assert!(parsed.stream());
    assert!(parsed.messages().is_some());
    assert!(parsed.metadata().is_some());
    assert!(parsed.raw.contains_key("custom_field"));
}
