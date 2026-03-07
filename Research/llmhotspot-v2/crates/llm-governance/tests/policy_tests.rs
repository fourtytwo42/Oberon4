use llm_governance::{Decision, PolicyEngine, PolicyInput};

#[test]
fn policy_matrix_allow_deny_approval() {
    let engine = PolicyEngine;

    let allow = engine.evaluate(&PolicyInput {
        action: "auth.connect.start".to_string(),
        trust_tier: "user-approved".to_string(),
    });
    assert_eq!(allow, Decision::Allow);

    let approval = engine.evaluate(&PolicyInput {
        action: "tool.invoke".to_string(),
        trust_tier: "basic".to_string(),
    });
    assert_eq!(approval, Decision::RequireApproval);

    let deny = engine.evaluate(&PolicyInput {
        action: "anything".to_string(),
        trust_tier: "blocked".to_string(),
    });
    assert_eq!(deny, Decision::Deny);
}
