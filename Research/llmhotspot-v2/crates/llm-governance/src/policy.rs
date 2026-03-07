#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyInput {
    pub action: String,
    pub trust_tier: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
    RequireApproval,
}

#[derive(Debug, Clone, Default)]
pub struct PolicyEngine;

impl PolicyEngine {
    pub fn evaluate(&self, input: &PolicyInput) -> Decision {
        if input.trust_tier == "blocked" {
            return Decision::Deny;
        }

        if input.action.contains("tool.invoke") && input.trust_tier != "user-approved" {
            return Decision::RequireApproval;
        }

        Decision::Allow
    }
}
