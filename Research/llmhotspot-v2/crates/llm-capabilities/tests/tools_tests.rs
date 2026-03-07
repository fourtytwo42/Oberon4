use llm_capabilities::{ToolGuard, ToolRequest};

#[test]
fn guardrail_blocks_bad_command() {
    let guard = ToolGuard::new("/workspace");
    let req = ToolRequest {
        command: "rm -rf /".to_string(),
        path: "/workspace/app".to_string(),
    };
    assert!(guard.validate(&req).is_err());
}

#[test]
fn guardrail_blocks_outside_path() {
    let guard = ToolGuard::new("/workspace");
    let req = ToolRequest {
        command: "cat file".to_string(),
        path: "/etc/passwd".to_string(),
    };
    assert!(guard.validate(&req).is_err());
}

#[test]
fn guardrail_allows_safe_request() {
    let guard = ToolGuard::new("/workspace");
    let req = ToolRequest {
        command: "cat app.rs".to_string(),
        path: "/workspace/app.rs".to_string(),
    };
    assert!(guard.validate(&req).is_ok());
}
