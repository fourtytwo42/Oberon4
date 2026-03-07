#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ToolError {
    #[error("path denied")]
    PathDenied,
    #[error("command denied")]
    CommandDenied,
}

#[derive(Debug, Clone)]
pub struct ToolRequest {
    pub command: String,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct ToolGuard {
    pub allowed_prefix: String,
}

impl ToolGuard {
    pub fn new(allowed_prefix: &str) -> Self {
        Self {
            allowed_prefix: allowed_prefix.to_string(),
        }
    }

    pub fn validate(&self, req: &ToolRequest) -> Result<(), ToolError> {
        if req.command.contains("rm -rf /") || req.command.contains("shutdown") {
            return Err(ToolError::CommandDenied);
        }

        if !req.path.starts_with(&self.allowed_prefix) {
            return Err(ToolError::PathDenied);
        }

        Ok(())
    }
}
