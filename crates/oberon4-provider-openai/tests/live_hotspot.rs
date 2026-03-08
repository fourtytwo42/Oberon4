use oberon4_core::{ChatMessage, ChatRole};
use oberon4_provider_openai::{OpenAiCompatibleClient, ReqwestTransport, RetryPolicy};

fn live_enabled() -> bool {
    std::env::var("OBERON4_LIVE_HOTSPOT").as_deref() == Ok("1")
}

fn normalized_base_url() -> String {
    let raw = std::env::var("OBERON4_LIVE_BASE_URL")
        .expect("OBERON4_LIVE_BASE_URL must be set for live hotspot tests");
    raw.trim_end_matches('/')
        .trim_end_matches("/v1")
        .to_string()
}

fn openai_model() -> String {
    std::env::var("OBERON4_LIVE_OPENAI_MODEL").unwrap_or_else(|_| "gpt-5.1-codex-mini".to_string())
}

#[test]
#[ignore = "requires OBERON4_LIVE_HOTSPOT=1 and a reachable live LLM Hotspot endpoint"]
fn live_hotspot_openai_chat_completions_round_trip() {
    if !live_enabled() {
        return;
    }

    let client = OpenAiCompatibleClient::new(
        normalized_base_url(),
        std::env::var("OBERON4_LIVE_API_KEY_ENV").ok(),
        RetryPolicy {
            timeout_ms: 45_000,
            max_retries: 1,
        },
        ReqwestTransport,
    );

    let reply = client
        .send_chat_messages(
            None,
            &openai_model(),
            &[
                ChatMessage {
                    role: ChatRole::System,
                    content: "Reply in under six words.".to_string(),
                    tool_calls: Vec::new(),
                },
                ChatMessage {
                    role: ChatRole::User,
                    content: "Say live endpoint ok".to_string(),
                    tool_calls: Vec::new(),
                },
            ],
        )
        .expect("live hotspot chat request should succeed");

    assert_eq!(reply.role, ChatRole::Assistant);
    assert!(!reply.content.trim().is_empty());
}
