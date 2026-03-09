use std::collections::BTreeMap;

use oberon4_core::{
    BrokeredServiceResult, GithubRequest, GithubResponse, HandleOperation, HttpHeader,
    OauthRequest, OauthResponse, OberonError, RedactionPipeline, RedactionProfile, RedactionResult,
    RedactionTargetKind, SecretBroker, SecretClass, SecretMetadata, SecretStore, SecretUseGrant,
    SecurityLevel, ServiceBroker, ServiceMetadata, SshSignRequest, WalletSignMessageRequest,
    WalletSignTxRequest,
};

pub trait SecretBackend: Send + Sync {
    fn get_secret(&self, key: &str) -> Result<String, OberonError>;
    fn security_level(&self) -> SecurityLevel;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretBinding {
    pub secret_handle: String,
    pub store_key: String,
    pub secret_class: SecretClass,
    pub allowed_operations: Vec<HandleOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceBinding {
    pub service_handle: String,
    pub service_name: String,
    pub secret_handle: String,
    pub base_url: String,
    pub allowed_operations: Vec<HandleOperation>,
}

pub struct EncryptedLocalSecretBackend<S> {
    store: S,
}

impl<S> EncryptedLocalSecretBackend<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl<S> SecretBackend for EncryptedLocalSecretBackend<S>
where
    S: SecretStore + Send + Sync,
{
    fn get_secret(&self, key: &str) -> Result<String, OberonError> {
        self.store.get_secret(key)
    }

    fn security_level(&self) -> SecurityLevel {
        SecurityLevel::EncryptedLocal
    }
}

pub struct MacOsSecretBackend<S> {
    store: S,
}

impl<S> MacOsSecretBackend<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl<S> SecretBackend for MacOsSecretBackend<S>
where
    S: SecretStore + Send + Sync,
{
    fn get_secret(&self, key: &str) -> Result<String, OberonError> {
        self.store.get_secret(key)
    }

    fn security_level(&self) -> SecurityLevel {
        SecurityLevel::OsProtected
    }
}

pub struct WindowsSecretBackend<S> {
    store: S,
}

impl<S> WindowsSecretBackend<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl<S> SecretBackend for WindowsSecretBackend<S>
where
    S: SecretStore + Send + Sync,
{
    fn get_secret(&self, key: &str) -> Result<String, OberonError> {
        self.store.get_secret(key)
    }

    fn security_level(&self) -> SecurityLevel {
        SecurityLevel::OsProtected
    }
}

pub struct LinuxSecretBackend<S> {
    store: S,
}

impl<S> LinuxSecretBackend<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl<S> SecretBackend for LinuxSecretBackend<S>
where
    S: SecretStore + Send + Sync,
{
    fn get_secret(&self, key: &str) -> Result<String, OberonError> {
        self.store.get_secret(key)
    }

    fn security_level(&self) -> SecurityLevel {
        SecurityLevel::OsProtected
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NativeSecretBackendUnavailable;

impl SecretBackend for NativeSecretBackendUnavailable {
    fn get_secret(&self, _key: &str) -> Result<String, OberonError> {
        Err(OberonError::PolicyDenied(
            "native secure backend is not configured".to_string(),
        ))
    }

    fn security_level(&self) -> SecurityLevel {
        SecurityLevel::OsProtected
    }
}

pub struct PluginRuntimeSecretBroker<B> {
    backend: B,
    secrets: BTreeMap<String, SecretBinding>,
}

impl<B> PluginRuntimeSecretBroker<B> {
    pub fn new(backend: B, secrets: Vec<SecretBinding>) -> Self {
        let secrets = secrets
            .into_iter()
            .map(|binding| (binding.secret_handle.clone(), binding))
            .collect();
        Self { backend, secrets }
    }

    pub fn secret_value(&self, secret_handle: &str) -> Result<String, OberonError>
    where
        B: SecretBackend,
    {
        let binding = self
            .secrets
            .get(secret_handle)
            .ok_or_else(|| OberonError::NotFound(format!("secret_handle:{secret_handle}")))?;
        self.backend.get_secret(&binding.store_key)
    }

    pub fn security_level(&self) -> SecurityLevel
    where
        B: SecretBackend,
    {
        self.backend.security_level()
    }
}

impl<B> SecretBroker for PluginRuntimeSecretBroker<B>
where
    B: SecretBackend,
{
    fn resolve_secret(&self, secret_handle: &str) -> Result<SecretMetadata, OberonError> {
        let binding = self
            .secrets
            .get(secret_handle)
            .ok_or_else(|| OberonError::NotFound(format!("secret_handle:{secret_handle}")))?;
        Ok(SecretMetadata {
            secret_handle: binding.secret_handle.clone(),
            secret_class: binding.secret_class.clone(),
            security_level: self.backend.security_level(),
            allowed_operations: binding.allowed_operations.clone(),
        })
    }

    fn grant_secret_use(
        &self,
        secret_handle: &str,
        operation: HandleOperation,
    ) -> Result<SecretUseGrant, OberonError> {
        let binding = self
            .secrets
            .get(secret_handle)
            .ok_or_else(|| OberonError::NotFound(format!("secret_handle:{secret_handle}")))?;
        if !binding.allowed_operations.contains(&operation) {
            return Err(OberonError::PolicyDenied(format!(
                "secret handle '{}' does not allow {:?}",
                secret_handle, operation
            )));
        }
        Ok(SecretUseGrant {
            secret_use_handle: format!("use_{}_{}", secret_handle, operation_name(&operation)),
            secret_handle: binding.secret_handle.clone(),
            secret_class: binding.secret_class.clone(),
            security_level: self.backend.security_level(),
            allowed_operations: vec![operation],
        })
    }
}

pub trait ServiceTransport: Send + Sync {
    fn execute(
        &self,
        request: &ServiceExecutionRequest<'_>,
    ) -> Result<(u16, Vec<HttpHeader>, serde_json::Value), OberonError>;
}

pub struct ServiceExecutionRequest<'a> {
    pub service_name: &'a str,
    pub base_url: &'a str,
    pub method: &'a str,
    pub path: &'a str,
    pub headers: &'a [HttpHeader],
    pub body_json: Option<&'a serde_json::Value>,
    pub bearer_token: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StubbedServiceResponse {
    pub status_code: u16,
    pub headers: Vec<HttpHeader>,
    pub body: serde_json::Value,
}

#[derive(Default)]
pub struct InMemoryServiceTransport {
    responses: BTreeMap<String, StubbedServiceResponse>,
}

impl InMemoryServiceTransport {
    pub fn with_response(
        mut self,
        service_name: &str,
        method: &str,
        path: &str,
        response: StubbedServiceResponse,
    ) -> Self {
        self.responses
            .insert(transport_key(service_name, method, path), response);
        self
    }
}

impl ServiceTransport for InMemoryServiceTransport {
    fn execute(
        &self,
        request: &ServiceExecutionRequest<'_>,
    ) -> Result<(u16, Vec<HttpHeader>, serde_json::Value), OberonError> {
        let response = self
            .responses
            .get(&transport_key(
                request.service_name,
                request.method,
                request.path,
            ))
            .ok_or_else(|| {
                OberonError::NotFound(format!(
                    "service_response:{}:{}:{}",
                    request.service_name, request.method, request.path
                ))
            })?;
        Ok((
            response.status_code,
            response.headers.clone(),
            response.body.clone(),
        ))
    }
}

pub struct PluginRuntimeServiceBroker<B, T> {
    secret_broker: PluginRuntimeSecretBroker<B>,
    services: BTreeMap<String, ServiceBinding>,
    transport: T,
}

impl<B, T> PluginRuntimeServiceBroker<B, T> {
    pub fn new(
        secret_broker: PluginRuntimeSecretBroker<B>,
        services: Vec<ServiceBinding>,
        transport: T,
    ) -> Self {
        let services = services
            .into_iter()
            .map(|binding| (binding.service_handle.clone(), binding))
            .collect();
        Self {
            secret_broker,
            services,
            transport,
        }
    }

    pub fn secret_values_for_service(
        &self,
        service_handle: &str,
    ) -> Result<Vec<String>, OberonError>
    where
        B: SecretBackend,
    {
        let binding = self
            .services
            .get(service_handle)
            .ok_or_else(|| OberonError::NotFound(format!("service_handle:{service_handle}")))?;
        Ok(vec![
            self.secret_broker.secret_value(&binding.secret_handle)?,
        ])
    }

    pub fn execute_service(
        &self,
        service_handle: &str,
        method: &str,
        path: &str,
        headers: &[HttpHeader],
        body_json: Option<&serde_json::Value>,
    ) -> Result<BrokeredServiceResult, OberonError>
    where
        B: SecretBackend,
        T: ServiceTransport,
    {
        let metadata = self.resolve_service(service_handle)?;
        let binding = self
            .services
            .get(service_handle)
            .ok_or_else(|| OberonError::NotFound(format!("service_handle:{service_handle}")))?;
        if !binding
            .allowed_operations
            .contains(&HandleOperation::Invoke)
        {
            return Err(OberonError::PolicyDenied(format!(
                "service '{}' does not allow invoke",
                binding.service_handle
            )));
        }
        let token = self.secret_broker.secret_value(&binding.secret_handle)?;
        let request = ServiceExecutionRequest {
            service_name: &binding.service_name,
            base_url: &binding.base_url,
            method,
            path,
            headers,
            body_json,
            bearer_token: &token,
        };
        let (status_code, response_headers, body) = self.transport.execute(&request)?;
        Ok(BrokeredServiceResult {
            summary: format!(
                "{} {} -> {} {}",
                method.to_uppercase(),
                path,
                metadata.service_name,
                status_code
            ),
            data: serde_json::json!({
                "serviceHandle": service_handle,
                "serviceName": metadata.service_name,
                "statusCode": status_code,
                "headers": response_headers,
                "body": body,
            }),
            next_suggested_action: None,
        })
    }
}

impl<B, T> ServiceBroker for PluginRuntimeServiceBroker<B, T>
where
    B: SecretBackend,
    T: ServiceTransport,
{
    fn resolve_service(&self, service_handle: &str) -> Result<ServiceMetadata, OberonError> {
        let binding = self
            .services
            .get(service_handle)
            .ok_or_else(|| OberonError::NotFound(format!("service_handle:{service_handle}")))?;
        let secret = self.secret_broker.resolve_secret(&binding.secret_handle)?;
        Ok(ServiceMetadata {
            service_handle: binding.service_handle.clone(),
            service_name: binding.service_name.clone(),
            secret_class: Some(secret.secret_class),
            security_level: Some(secret.security_level),
            allowed_operations: binding.allowed_operations.clone(),
        })
    }

    fn redaction_values_for_service(
        &self,
        service_handle: &str,
    ) -> Result<Vec<String>, OberonError> {
        self.secret_values_for_service(service_handle)
    }

    fn execute_github_request(
        &self,
        request: &GithubRequest,
    ) -> Result<GithubResponse, OberonError> {
        let binding = self.services.get(&request.service_handle).ok_or_else(|| {
            OberonError::NotFound(format!("service_handle:{}", request.service_handle))
        })?;
        if !matches!(
            binding.service_name.as_str(),
            "github.request" | "service.github.request"
        ) {
            return Err(OberonError::Validation(format!(
                "service '{}' is not a github broker",
                binding.service_name
            )));
        }
        let result = self.execute_service(
            &request.service_handle,
            &request.method,
            &request.path,
            &request.headers,
            request.body_json.as_ref(),
        )?;
        Ok(GithubResponse {
            status_code: result.data["statusCode"].as_u64().unwrap_or_default() as u16,
            headers: serde_json::from_value(result.data["headers"].clone()).unwrap_or_default(),
            body: result.data["body"].clone(),
        })
    }

    fn execute_oauth_request(&self, request: &OauthRequest) -> Result<OauthResponse, OberonError> {
        let binding = self.services.get(&request.service_handle).ok_or_else(|| {
            OberonError::NotFound(format!("service_handle:{}", request.service_handle))
        })?;
        if !matches!(
            binding.service_name.as_str(),
            "oauth.request" | "service.oauth.request"
        ) {
            return Err(OberonError::Validation(format!(
                "service '{}' is not an oauth broker",
                binding.service_name
            )));
        }
        let result = self.execute_service(
            &request.service_handle,
            &request.method,
            &request.path,
            &request.headers,
            request.body_json.as_ref(),
        )?;
        Ok(OauthResponse {
            status_code: result.data["statusCode"].as_u64().unwrap_or_default() as u16,
            headers: serde_json::from_value(result.data["headers"].clone()).unwrap_or_default(),
            body: result.data["body"].clone(),
        })
    }

    fn sign_ssh(&self, _request: &SshSignRequest) -> Result<String, OberonError> {
        Err(OberonError::PolicyDenied(
            "ssh signing is not configured in WP3".to_string(),
        ))
    }

    fn sign_wallet_tx(&self, _request: &WalletSignTxRequest) -> Result<String, OberonError> {
        Err(OberonError::PolicyDenied(
            "wallet signing is not configured in WP3".to_string(),
        ))
    }

    fn sign_wallet_message(
        &self,
        _request: &WalletSignMessageRequest,
    ) -> Result<String, OberonError> {
        Err(OberonError::PolicyDenied(
            "wallet signing is not configured in WP3".to_string(),
        ))
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PluginRuntimeRedactor;

impl RedactionPipeline for PluginRuntimeRedactor {
    fn redact_text(
        &self,
        _target: RedactionTargetKind,
        text: &str,
        profile: &RedactionProfile,
    ) -> Result<RedactionResult, OberonError> {
        let mut markers = Vec::new();
        let mut output = text.to_string();
        let mut applied = false;

        for value in &profile.exact_values {
            if !value.is_empty() && output.contains(value) {
                output = output.replace(value, "<redacted>");
                applied = true;
                markers.push("exact_value".to_string());
            }
        }

        let (next_output, next_applied) = redact_common_token_shapes(&output);
        if next_applied {
            applied = true;
            markers.push("token_shape".to_string());
            output = next_output;
        }

        let blocked = profile.fail_closed && contains_blocked_secret_shape(&output);
        if blocked {
            markers.push("blocked_secret_shape".to_string());
        }

        Ok(RedactionResult {
            redacted_text: output,
            applied,
            blocked,
            markers,
        })
    }

    fn redact_json(
        &self,
        target: RedactionTargetKind,
        value: &serde_json::Value,
        profile: &RedactionProfile,
    ) -> Result<(serde_json::Value, RedactionResult), OberonError> {
        let mut markers = Vec::new();
        let mut applied = false;
        let redacted = redact_json_value(self, target, value, profile, &mut applied, &mut markers)?;
        let blocked = profile.fail_closed
            && serde_json::to_string(&redacted)
                .map(|encoded| contains_blocked_secret_shape(&encoded))
                .unwrap_or(false);
        if blocked {
            markers.push("blocked_secret_shape".to_string());
        }
        Ok((
            redacted,
            RedactionResult {
                redacted_text: String::new(),
                applied,
                blocked,
                markers,
            },
        ))
    }
}

fn redact_json_value(
    redactor: &PluginRuntimeRedactor,
    target: RedactionTargetKind,
    value: &serde_json::Value,
    profile: &RedactionProfile,
    applied: &mut bool,
    markers: &mut Vec<String>,
) -> Result<serde_json::Value, OberonError> {
    match value {
        serde_json::Value::String(text) => {
            let result = redactor.redact_text(target, text, profile)?;
            *applied |= result.applied;
            markers.extend(result.markers);
            Ok(serde_json::Value::String(result.redacted_text))
        }
        serde_json::Value::Array(items) => {
            let mut next = Vec::with_capacity(items.len());
            for item in items {
                next.push(redact_json_value(
                    redactor,
                    target.clone(),
                    item,
                    profile,
                    applied,
                    markers,
                )?);
            }
            Ok(serde_json::Value::Array(next))
        }
        serde_json::Value::Object(map) => {
            let mut next = serde_json::Map::new();
            for (key, item) in map {
                next.insert(
                    key.clone(),
                    redact_json_value(redactor, target.clone(), item, profile, applied, markers)?,
                );
            }
            Ok(serde_json::Value::Object(next))
        }
        _ => Ok(value.clone()),
    }
}

fn operation_name(operation: &HandleOperation) -> &'static str {
    match operation {
        HandleOperation::Read => "read",
        HandleOperation::Write => "write",
        HandleOperation::List => "list",
        HandleOperation::Stat => "stat",
        HandleOperation::Create => "create",
        HandleOperation::Delete => "delete",
        HandleOperation::Invoke => "invoke",
        HandleOperation::Sign => "sign",
    }
}

fn transport_key(service_name: &str, method: &str, path: &str) -> String {
    format!("{}:{}:{}", service_name, method.to_uppercase(), path)
}

fn redact_common_token_shapes(input: &str) -> (String, bool) {
    let mut output = input.to_string();
    let mut applied = false;
    for marker in [
        "authorization: bearer ",
        "bearer ",
        "\"access_token\":\"",
        "\"refresh_token\":\"",
        "access_token=",
        "refresh_token=",
        "api_key=",
        "token=",
    ] {
        let mut cursor = 0usize;
        loop {
            let lower = output.to_ascii_lowercase();
            let Some(relative_start) = lower[cursor..].find(marker) else {
                break;
            };
            let start = cursor + relative_start;
            let value_start = start + marker.len();
            let value_end = if marker.ends_with('"') {
                output[value_start..]
                    .find('"')
                    .map(|offset| value_start + offset)
                    .unwrap_or(output.len())
            } else {
                output[value_start..]
                    .find(|c: char| c.is_whitespace() || c == '&' || c == '"' || c == ',')
                    .map(|offset| value_start + offset)
                    .unwrap_or(output.len())
            };
            if value_end <= value_start {
                cursor = value_start.saturating_add(1);
                continue;
            }
            output.replace_range(value_start..value_end, "<redacted>");
            applied = true;
            cursor = value_start + "<redacted>".len();
        }
    }
    (output, applied)
}

fn contains_blocked_secret_shape(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    lower.contains("-----begin private key-----")
        || lower.contains("-----begin openssh private key-----")
        || (lower.contains("authorization: bearer ")
            && !lower.contains("authorization: bearer <redacted>"))
        || (lower.contains("\"access_token\":\"")
            && !lower.contains("\"access_token\":\"<redacted>"))
        || (lower.contains("\"refresh_token\":\"")
            && !lower.contains("\"refresh_token\":\"<redacted>"))
}

#[cfg(test)]
mod tests {
    use super::{
        EncryptedLocalSecretBackend, InMemoryServiceTransport, PluginRuntimeRedactor,
        PluginRuntimeSecretBroker, PluginRuntimeServiceBroker, SecretBackend, SecretBinding,
        ServiceBinding, StubbedServiceResponse,
    };
    use oberon4_core::{
        GithubRequest, HandleOperation, HttpHeader, InMemorySecretStore, OauthRequest,
        RedactionPipeline, RedactionProfile, RedactionTargetKind, SecretBroker, SecretClass,
        SecretStore, SecurityLevel, ServiceBroker,
    };

    #[test]
    fn backends_report_stable_security_level() {
        let mut store = InMemorySecretStore::default();
        store
            .set_secret("oauth.github.main", "secret-token")
            .unwrap();
        let backend = EncryptedLocalSecretBackend::new(store);
        assert_eq!(backend.security_level(), SecurityLevel::EncryptedLocal);
    }

    #[test]
    fn secret_broker_never_returns_raw_secret_value() {
        let mut store = InMemorySecretStore::default();
        store
            .set_secret("oauth.github.main", "secret-token")
            .unwrap();
        let broker = PluginRuntimeSecretBroker::new(
            EncryptedLocalSecretBackend::new(store),
            vec![SecretBinding {
                secret_handle: "sec_github".to_string(),
                store_key: "oauth.github.main".to_string(),
                secret_class: SecretClass::OauthCredential,
                allowed_operations: vec![HandleOperation::Invoke],
            }],
        );
        let metadata = broker.resolve_secret("sec_github").unwrap();
        assert_eq!(metadata.secret_handle, "sec_github");
        let encoded = serde_json::to_string(&metadata).unwrap();
        assert!(!encoded.contains("secret-token"));
    }

    #[test]
    fn service_broker_executes_without_exposing_secret() {
        let mut store = InMemorySecretStore::default();
        store
            .set_secret("oauth.github.main", "secret-token")
            .unwrap();
        let secret_broker = PluginRuntimeSecretBroker::new(
            EncryptedLocalSecretBackend::new(store),
            vec![SecretBinding {
                secret_handle: "sec_github".to_string(),
                store_key: "oauth.github.main".to_string(),
                secret_class: SecretClass::OauthCredential,
                allowed_operations: vec![HandleOperation::Invoke],
            }],
        );
        let broker = PluginRuntimeServiceBroker::new(
            secret_broker,
            vec![ServiceBinding {
                service_handle: "svc_github".to_string(),
                service_name: "github.request".to_string(),
                secret_handle: "sec_github".to_string(),
                base_url: "https://api.github.test".to_string(),
                allowed_operations: vec![HandleOperation::Invoke],
            }],
            InMemoryServiceTransport::default().with_response(
                "github.request",
                "GET",
                "/repos/oberon4/status",
                StubbedServiceResponse {
                    status_code: 200,
                    headers: vec![HttpHeader {
                        name: "authorization".to_string(),
                        value: "Bearer secret-token".to_string(),
                    }],
                    body: serde_json::json!({"ok": true, "echo": "secret-token"}),
                },
            ),
        );

        let response = broker
            .execute_github_request(&GithubRequest {
                service_handle: "svc_github".to_string(),
                method: "GET".to_string(),
                path: "/repos/oberon4/status".to_string(),
                headers: Vec::new(),
                body_json: None,
            })
            .unwrap();
        assert_eq!(response.status_code, 200);
        assert_eq!(response.body["echo"], "secret-token");
    }

    #[test]
    fn oauth_request_uses_same_broker_contract() {
        let mut store = InMemorySecretStore::default();
        store
            .set_secret("oauth.openai.primary", "oauth-secret")
            .unwrap();
        let secret_broker = PluginRuntimeSecretBroker::new(
            EncryptedLocalSecretBackend::new(store),
            vec![SecretBinding {
                secret_handle: "sec_openai".to_string(),
                store_key: "oauth.openai.primary".to_string(),
                secret_class: SecretClass::OauthCredential,
                allowed_operations: vec![HandleOperation::Invoke],
            }],
        );
        let broker = PluginRuntimeServiceBroker::new(
            secret_broker,
            vec![ServiceBinding {
                service_handle: "svc_oauth".to_string(),
                service_name: "oauth.request".to_string(),
                secret_handle: "sec_openai".to_string(),
                base_url: "https://oauth.test".to_string(),
                allowed_operations: vec![HandleOperation::Invoke],
            }],
            InMemoryServiceTransport::default().with_response(
                "oauth.request",
                "GET",
                "/userinfo",
                StubbedServiceResponse {
                    status_code: 200,
                    headers: Vec::new(),
                    body: serde_json::json!({"sub": "acct-1"}),
                },
            ),
        );

        let response = broker
            .execute_oauth_request(&OauthRequest {
                service_handle: "svc_oauth".to_string(),
                method: "GET".to_string(),
                path: "/userinfo".to_string(),
                headers: Vec::new(),
                body_json: None,
            })
            .unwrap();
        assert_eq!(response.body["sub"], "acct-1");
    }

    #[test]
    fn redactor_removes_exact_values_and_common_shapes() {
        let redactor = PluginRuntimeRedactor;
        let result = redactor
            .redact_text(
                RedactionTargetKind::ToolResult,
                "Authorization: Bearer secret-token access_token=secret-token",
                &RedactionProfile {
                    exact_values: vec!["secret-token".to_string()],
                    fail_closed: true,
                },
            )
            .unwrap();
        assert!(result.applied);
        assert!(!result.redacted_text.contains("secret-token"));
        assert!(!result.blocked);
    }

    #[test]
    fn redactor_fail_closes_private_keys() {
        let redactor = PluginRuntimeRedactor;
        let result = redactor
            .redact_text(
                RedactionTargetKind::ErrorPayload,
                "-----BEGIN PRIVATE KEY----- abc",
                &RedactionProfile {
                    exact_values: Vec::new(),
                    fail_closed: true,
                },
            )
            .unwrap();
        assert!(result.blocked);
    }
}
