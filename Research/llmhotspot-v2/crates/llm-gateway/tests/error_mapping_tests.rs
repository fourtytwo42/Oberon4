use llm_foundation::contracts::ChatRequest;
use llm_gateway::{DefaultGatewayService, GatewayError, GatewayService, GatewayStatus};

#[test]
fn status_code_mapping() {
    assert_eq!(GatewayError::Auth.http_status(), 401);
    assert_eq!(GatewayError::RateLimit.http_status(), 429);
    assert_eq!(GatewayError::TenantOffline.http_status(), 503);
}

#[test]
fn deterministic_request_errors() {
    let svc = DefaultGatewayService {
        status: GatewayStatus::Offline,
    };
    let req = ChatRequest {
        model: "gpt-5.2".to_string(),
        input: "hello".to_string(),
        stream: false,
    };
    let err = svc.chat(req).expect_err("offline");
    assert_eq!(err, GatewayError::TenantOffline);
}
