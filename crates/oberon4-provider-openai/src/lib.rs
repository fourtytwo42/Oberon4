use std::{cell::RefCell, collections::VecDeque, time::Duration};

use serde::{Deserialize, Serialize};

use oberon4_core::{AppConfig, ChatMessage, ChatProvider, ChatRole, OberonError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub timeout_ms: u64,
    pub max_retries: u8,
}

impl RetryPolicy {
    pub fn from_config(config: &AppConfig) -> Self {
        Self {
            timeout_ms: config.runtime.request_timeout_ms,
            max_retries: config.runtime.max_retries,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiChatRequest {
    pub model: String,
    pub messages: Vec<OpenAiChatRequestMessage>,
    pub stream: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiChatRequestMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiChatResponse {
    pub choices: Vec<OpenAiChatChoice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiChatChoice {
    pub message: OpenAiChatResponseMessage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiChatResponseMessage {
    pub role: String,
    pub content: String,
}

pub trait OpenAiTransport {
    fn post_json(
        &self,
        url: &str,
        body: &str,
        timeout_ms: u64,
        bearer_token: Option<&str>,
    ) -> Result<String, OberonError>;
    fn get_text(
        &self,
        url: &str,
        timeout_ms: u64,
        bearer_token: Option<&str>,
    ) -> Result<String, OberonError>;
}

#[derive(Debug, Clone, Default)]
pub struct ReqwestTransport;

impl OpenAiTransport for ReqwestTransport {
    fn post_json(
        &self,
        url: &str,
        body: &str,
        timeout_ms: u64,
        bearer_token: Option<&str>,
    ) -> Result<String, OberonError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .map_err(|err| OberonError::Storage(format!("transport client build failed: {err}")))?;

        let mut request = client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::CONNECTION, "close");
        if let Some(token) = bearer_token {
            request = request.bearer_auth(token);
        }
        let response = request
            .body(body.to_string())
            .send()
            .map_err(|err| OberonError::Storage(format!("transport request failed: {err}")))?;

        let status = response.status();
        let text = response.text().map_err(|err| {
            OberonError::Storage(format!("transport response read failed: {err}"))
        })?;

        if !status.is_success() {
            return Err(OberonError::Validation(format!(
                "provider returned HTTP {}",
                status.as_u16()
            )));
        }

        Ok(text)
    }

    fn get_text(
        &self,
        url: &str,
        timeout_ms: u64,
        bearer_token: Option<&str>,
    ) -> Result<String, OberonError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .map_err(|err| OberonError::Storage(format!("transport client build failed: {err}")))?;

        let mut request = client.get(url).header(reqwest::header::CONNECTION, "close");
        if let Some(token) = bearer_token {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .map_err(|err| OberonError::Storage(format!("transport request failed: {err}")))?;

        let status = response.status();
        let text = response.text().map_err(|err| {
            OberonError::Storage(format!("transport response read failed: {err}"))
        })?;

        if !status.is_success() {
            return Err(OberonError::Validation(format!(
                "provider returned HTTP {}",
                status.as_u16()
            )));
        }

        Ok(text)
    }
}

#[derive(Debug, Clone)]
pub struct OpenAiCompatibleClient<T> {
    base_url: String,
    api_key_env: Option<String>,
    retry_policy: RetryPolicy,
    transport: T,
}

impl<T> OpenAiCompatibleClient<T> {
    pub fn new(
        base_url: impl Into<String>,
        api_key_env: Option<String>,
        retry_policy: RetryPolicy,
        transport: T,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            api_key_env,
            retry_policy,
            transport,
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn set_retry_policy(&mut self, retry_policy: RetryPolicy) {
        self.retry_policy = retry_policy;
    }

    pub fn set_api_key_env(&mut self, api_key_env: Option<String>) {
        self.api_key_env = api_key_env;
    }
}

impl<T> OpenAiCompatibleClient<T>
where
    T: OpenAiTransport,
{
    pub fn send_chat_messages(
        &self,
        endpoint_override: Option<&str>,
        model: &str,
        messages: &[ChatMessage],
    ) -> Result<ChatMessage, OberonError> {
        let request = OpenAiChatRequest {
            model: model.to_string(),
            messages: messages.iter().cloned().map(into_request_message).collect(),
            stream: false,
        };
        let body = serde_json::to_string(&request)
            .map_err(|err| OberonError::Storage(format!("request serialization failed: {err}")))?;
        let url = format!(
            "{}/v1/chat/completions",
            self.effective_base_url(endpoint_override)
        );
        let response_body = self.post_with_retry(&url, &body, self.bearer_token()?.as_deref())?;
        let response: OpenAiChatResponse = serde_json::from_str(&response_body).map_err(|err| {
            OberonError::Validation(format!("malformed provider response: {err}"))
        })?;

        let message = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| {
                OberonError::Validation("provider response missing choices".to_string())
            })?
            .message;

        into_chat_message(message)
    }

    pub fn endpoint_health(
        &self,
        endpoint_override: Option<&str>,
        _model: &str,
    ) -> Result<(), OberonError> {
        let url = format!("{}/v1/version", self.effective_base_url(endpoint_override));
        self.transport
            .get_text(&url, self.retry_policy.timeout_ms, self.bearer_token()?.as_deref())
            .map(|_| ())
    }

    fn effective_base_url(&self, endpoint_override: Option<&str>) -> String {
        endpoint_override
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(self.base_url())
            .trim_end_matches('/')
            .to_string()
    }

    fn post_with_retry(
        &self,
        url: &str,
        body: &str,
        bearer_token: Option<&str>,
    ) -> Result<String, OberonError> {
        let attempts = self.retry_policy.max_retries.saturating_add(1);
        let mut last_error: Option<OberonError> = None;

        for _ in 0..attempts {
            match self
                .transport
                .post_json(url, body, self.retry_policy.timeout_ms, bearer_token)
            {
                Ok(response) => return Ok(response),
                Err(err) => last_error = Some(err),
            }
        }

        Err(last_error.unwrap_or_else(|| {
            OberonError::Storage("provider request failed without an error".to_string())
        }))
    }

    fn bearer_token(&self) -> Result<Option<String>, OberonError> {
        match self.api_key_env.as_deref() {
            Some(name) if !name.trim().is_empty() => std::env::var(name)
                .map(Some)
                .map_err(|_| {
                    OberonError::Validation(format!(
                        "api key environment variable is not set: {name}"
                    ))
                }),
            _ => Ok(None),
        }
    }
}

impl<T> ChatProvider for OpenAiCompatibleClient<T>
where
    T: OpenAiTransport,
{
    fn send_message(
        &self,
        endpoint: &str,
        model: &str,
        messages: &[ChatMessage],
    ) -> Result<ChatMessage, OberonError> {
        let endpoint_override = if endpoint.trim().is_empty() {
            None
        } else {
            Some(endpoint)
        };
        self.send_chat_messages(endpoint_override, model, messages)
    }

    fn endpoint_health(&self, endpoint: &str, model: &str) -> Result<(), OberonError> {
        let endpoint_override = if endpoint.trim().is_empty() {
            None
        } else {
            Some(endpoint)
        };
        OpenAiCompatibleClient::endpoint_health(self, endpoint_override, model)
    }

    fn apply_runtime_config(&mut self, config: &AppConfig) -> Result<(), OberonError> {
        self.set_retry_policy(RetryPolicy::from_config(config));
        self.set_api_key_env(config.endpoint.api_key_env.clone());
        Ok(())
    }
}

pub fn endpoint_summary(config: &AppConfig) -> String {
    format!("{} -> {}", config.endpoint.base_url, config.endpoint.model)
}

fn into_request_message(message: ChatMessage) -> OpenAiChatRequestMessage {
    OpenAiChatRequestMessage {
        role: match message.role {
            ChatRole::System => "system",
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
        }
        .to_string(),
        content: message.content,
    }
}

fn into_chat_message(message: OpenAiChatResponseMessage) -> Result<ChatMessage, OberonError> {
    let role = match message.role.as_str() {
        "system" => ChatRole::System,
        "user" => ChatRole::User,
        "assistant" => ChatRole::Assistant,
        other => {
            return Err(OberonError::Validation(format!(
                "provider response contained unsupported role: {other}"
            )));
        }
    };

    Ok(ChatMessage {
        role,
        content: normalize_assistant_content(role, message.content),
    })
}

fn normalize_assistant_content(role: ChatRole, content: String) -> String {
    if role != ChatRole::Assistant {
        return content;
    }

    let mut normalized = content;
    loop {
        let len = normalized.len();
        if len < 16 || len % 2 != 0 {
            return normalized;
        }

        let midpoint = len / 2;
        if !normalized.is_char_boundary(midpoint) {
            return normalized;
        }

        let (left, right) = normalized.split_at(midpoint);
        if left != right {
            return normalized;
        }

        normalized = left.to_string();
    }
}

#[derive(Debug)]
pub struct FakeTransport {
    responses: RefCell<VecDeque<Result<String, OberonError>>>,
    requests: RefCell<Vec<(String, String, String, u64, Option<String>)>>,
}

impl FakeTransport {
    pub fn new(responses: Vec<Result<String, OberonError>>) -> Self {
        Self {
            responses: RefCell::new(responses.into()),
            requests: RefCell::new(Vec::new()),
        }
    }

    pub fn request_count(&self) -> usize {
        self.requests.borrow().len()
    }

    pub fn recorded_requests(&self) -> Vec<(String, String, String, u64, Option<String>)> {
        self.requests.borrow().clone()
    }
}

impl OpenAiTransport for FakeTransport {
    fn post_json(
        &self,
        url: &str,
        body: &str,
        timeout_ms: u64,
        bearer_token: Option<&str>,
    ) -> Result<String, OberonError> {
        self.requests.borrow_mut().push((
            "POST".to_string(),
            url.to_string(),
            body.to_string(),
            timeout_ms,
            bearer_token.map(ToOwned::to_owned),
        ));
        self.responses.borrow_mut().pop_front().unwrap_or_else(|| {
            Err(OberonError::Storage(
                "fake transport ran out of scripted responses".to_string(),
            ))
        })
    }

    fn get_text(
        &self,
        url: &str,
        timeout_ms: u64,
        bearer_token: Option<&str>,
    ) -> Result<String, OberonError> {
        self.requests.borrow_mut().push((
            "GET".to_string(),
            url.to_string(),
            String::new(),
            timeout_ms,
            bearer_token.map(ToOwned::to_owned),
        ));
        self.responses.borrow_mut().pop_front().unwrap_or_else(|| {
            Err(OberonError::Storage(
                "fake transport ran out of scripted responses".to_string(),
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    use super::{
        FakeTransport, OpenAiChatRequest, OpenAiCompatibleClient, ReqwestTransport, RetryPolicy,
        endpoint_summary,
    };
    use oberon4_core::{AppConfig, ChatMessage, ChatProvider, ChatRole, OberonError};

    fn ok_response(content: &str) -> String {
        format!(r#"{{"choices":[{{"message":{{"role":"assistant","content":"{content}"}}}}]}}"#)
    }

    #[test]
    fn endpoint_summary_uses_shared_config() {
        let summary = endpoint_summary(&AppConfig::default());
        assert!(summary.contains("https://api.openai.com"));
    }

    #[test]
    fn request_translation_matches_openai_chat_shape() {
        let transport = FakeTransport::new(vec![Ok(ok_response("hi"))]);
        let client = OpenAiCompatibleClient::new(
            "https://api.openai.com",
            None,
            RetryPolicy {
                timeout_ms: 30_000,
                max_retries: 0,
            },
            transport,
        );
        let messages = vec![
            ChatMessage {
                role: ChatRole::System,
                content: "be concise".to_string(),
            },
            ChatMessage {
                role: ChatRole::User,
                content: "hello".to_string(),
            },
        ];

        let reply = client
            .send_message("", "gpt-4.1-mini", &messages)
            .expect("provider request should succeed");

        assert_eq!(reply.role, ChatRole::Assistant);
        assert_eq!(reply.content, "hi");

        let requests = client.transport.recorded_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, "POST");
        assert_eq!(requests[0].1, "https://api.openai.com/v1/chat/completions");
        let decoded: OpenAiChatRequest = serde_json::from_str(&requests[0].2).unwrap();
        assert_eq!(decoded.model, "gpt-4.1-mini");
        assert!(!decoded.stream);
        assert_eq!(decoded.messages[0].role, "system");
        assert_eq!(decoded.messages[1].role, "user");
    }

    #[test]
    fn endpoint_override_is_used_when_present() {
        let transport = FakeTransport::new(vec![Ok(ok_response("hi"))]);
        let client = OpenAiCompatibleClient::new(
            "https://api.openai.com",
            None,
            RetryPolicy {
                timeout_ms: 10_000,
                max_retries: 0,
            },
            transport,
        );

        let _ = client
            .send_chat_messages(
                Some("https://gateway.example.test/"),
                "demo-model",
                &[ChatMessage {
                    role: ChatRole::User,
                    content: "ping".to_string(),
                }],
            )
            .unwrap();

        let requests = client.transport.recorded_requests();
        assert_eq!(
            requests[0].1,
            "https://gateway.example.test/v1/chat/completions"
        );
    }

    #[test]
    fn retry_policy_retries_before_succeeding() {
        let transport = FakeTransport::new(vec![
            Err(OberonError::Storage("timeout".to_string())),
            Err(OberonError::Storage("timeout".to_string())),
            Ok(ok_response("done")),
        ]);
        let client = OpenAiCompatibleClient::new(
            "https://api.openai.com",
            None,
            RetryPolicy {
                timeout_ms: 5_000,
                max_retries: 2,
            },
            transport,
        );

        let reply = client
            .send_message(
                "",
                "gpt-4.1-mini",
                &[ChatMessage {
                    role: ChatRole::User,
                    content: "hello".to_string(),
                }],
            )
            .unwrap();

        assert_eq!(reply.content, "done");
        assert_eq!(client.transport.request_count(), 3);
    }

    #[test]
    fn malformed_response_is_rejected() {
        let transport = FakeTransport::new(vec![Ok("{\"choices\":[".to_string())]);
        let client = OpenAiCompatibleClient::new(
            "https://api.openai.com",
            None,
            RetryPolicy {
                timeout_ms: 5_000,
                max_retries: 0,
            },
            transport,
        );

        let err = client
            .send_message(
                "",
                "gpt-4.1-mini",
                &[ChatMessage {
                    role: ChatRole::User,
                    content: "hello".to_string(),
                }],
            )
            .expect_err("malformed responses should fail");

        assert!(err.to_string().contains("malformed provider response"));
    }

    #[test]
    fn missing_choices_is_rejected() {
        let transport = FakeTransport::new(vec![Ok("{\"choices\":[]}".to_string())]);
        let client = OpenAiCompatibleClient::new(
            "https://api.openai.com",
            None,
            RetryPolicy {
                timeout_ms: 5_000,
                max_retries: 0,
            },
            transport,
        );

        let err = client
            .send_message(
                "",
                "gpt-4.1-mini",
                &[ChatMessage {
                    role: ChatRole::User,
                    content: "hello".to_string(),
                }],
            )
            .expect_err("empty choice list should fail");

        assert_eq!(
            err.to_string(),
            "validation failed: provider response missing choices"
        );
    }

    #[test]
    fn auth_failures_surface_after_retry_budget_exhaustion() {
        let transport = FakeTransport::new(vec![
            Err(OberonError::Validation("auth failed".to_string())),
            Err(OberonError::Validation("auth failed".to_string())),
        ]);
        let client = OpenAiCompatibleClient::new(
            "https://api.openai.com",
            None,
            RetryPolicy {
                timeout_ms: 5_000,
                max_retries: 1,
            },
            transport,
        );

        let err = client
            .send_message(
                "",
                "gpt-4.1-mini",
                &[ChatMessage {
                    role: ChatRole::User,
                    content: "hello".to_string(),
                }],
            )
            .expect_err("auth failure should surface");

        assert_eq!(err.to_string(), "validation failed: auth failed");
        assert_eq!(client.transport.request_count(), 2);
    }

    #[test]
    fn endpoint_health_uses_probe_request() {
        let transport = FakeTransport::new(vec![Ok(ok_response("healthy"))]);
        let client = OpenAiCompatibleClient::new(
            "https://api.openai.com",
            None,
            RetryPolicy::from_config(&AppConfig::default()),
            transport,
        );

        client.endpoint_health(None, "gpt-4.1-mini").unwrap();

        let requests = client.transport.recorded_requests();
        assert_eq!(requests[0].0, "GET");
        assert_eq!(requests[0].1, "https://api.openai.com/v1/version");
        assert_eq!(requests[0].2, "");
    }

    #[test]
    fn exact_duplicated_assistant_content_is_collapsed() {
        let duplicated = r#"{"choices":[{"message":{"role":"assistant","content":"Hello! How can I help you today?Hello! How can I help you today?"}}]}"#;
        let transport = FakeTransport::new(vec![Ok(duplicated.to_string())]);
        let client = OpenAiCompatibleClient::new(
            "https://api.openai.com",
            None,
            RetryPolicy {
                timeout_ms: 5_000,
                max_retries: 0,
            },
            transport,
        );

        let reply = client
            .send_message(
                "",
                "gpt-4.1-mini",
                &[ChatMessage {
                    role: ChatRole::User,
                    content: "hello".to_string(),
                }],
            )
            .unwrap();

        assert_eq!(reply.content, "Hello! How can I help you today?");
    }

    #[test]
    fn nonduplicated_assistant_content_is_preserved() {
        let transport = FakeTransport::new(vec![Ok(ok_response("Your name is Eric."))]);
        let client = OpenAiCompatibleClient::new(
            "https://api.openai.com",
            None,
            RetryPolicy {
                timeout_ms: 5_000,
                max_retries: 0,
            },
            transport,
        );

        let reply = client
            .send_message(
                "",
                "gpt-4.1-mini",
                &[ChatMessage {
                    role: ChatRole::User,
                    content: "hello".to_string(),
                }],
            )
            .unwrap();

        assert_eq!(reply.content, "Your name is Eric.");
    }

    #[test]
    fn request_uses_bearer_auth_from_configured_env() {
        unsafe {
            std::env::set_var("OBERON4_TEST_API_KEY", "hs_test_key");
        }
        let transport = FakeTransport::new(vec![Ok(ok_response("hi"))]);
        let client = OpenAiCompatibleClient::new(
            "https://api.openai.com",
            Some("OBERON4_TEST_API_KEY".to_string()),
            RetryPolicy {
                timeout_ms: 5_000,
                max_retries: 0,
            },
            transport,
        );

        let reply = client
            .send_message(
                "",
                "gpt-4.1-mini",
                &[ChatMessage {
                    role: ChatRole::User,
                    content: "hello".to_string(),
                }],
            )
            .unwrap();

        assert_eq!(reply.content, "hi");
        let requests = client.transport.recorded_requests();
        assert_eq!(requests[0].4.as_deref(), Some("hs_test_key"));
        unsafe {
            std::env::remove_var("OBERON4_TEST_API_KEY");
        }
    }

    #[test]
    fn missing_api_key_env_fails_fast() {
        unsafe {
            std::env::remove_var("OBERON4_TEST_API_KEY_MISSING");
        }
        let transport = FakeTransport::new(vec![Ok(ok_response("hi"))]);
        let client = OpenAiCompatibleClient::new(
            "https://api.openai.com",
            Some("OBERON4_TEST_API_KEY_MISSING".to_string()),
            RetryPolicy {
                timeout_ms: 5_000,
                max_retries: 0,
            },
            transport,
        );

        let err = client
            .send_message(
                "",
                "gpt-4.1-mini",
                &[ChatMessage {
                    role: ChatRole::User,
                    content: "hello".to_string(),
                }],
            )
            .expect_err("missing api key env should fail fast");

        assert_eq!(
            err.to_string(),
            "validation failed: api key environment variable is not set: OBERON4_TEST_API_KEY_MISSING"
        );
    }

    #[test]
    fn timeout_value_is_forwarded_to_transport() {
        let transport = FakeTransport::new(vec![Ok(ok_response("done"))]);
        let client = OpenAiCompatibleClient::new(
            "https://api.openai.com",
            None,
            RetryPolicy {
                timeout_ms: 12_345,
                max_retries: 0,
            },
            transport,
        );

        client
            .send_message(
                "",
                "gpt-4.1-mini",
                &[ChatMessage {
                    role: ChatRole::User,
                    content: "hello".to_string(),
                }],
            )
            .unwrap();

        let requests = client.transport.recorded_requests();
        assert_eq!(requests[0].3, 12_345);
    }

    #[test]
    fn reqwest_transport_posts_to_local_server() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let addr = listener.local_addr().expect("local addr");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept should succeed");
            let mut buffer = [0_u8; 4096];
            let bytes_read = stream.read(&mut buffer).expect("request should read");
            let request = String::from_utf8_lossy(&buffer[..bytes_read]).to_string();
            assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
            assert!(request.contains("\"model\":\"gpt-4.1-mini\""));

            let body = ok_response("from server");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("response should write");
        });

        let client = OpenAiCompatibleClient::new(
            format!("http://{}", addr),
            None,
            RetryPolicy {
                timeout_ms: 5_000,
                max_retries: 0,
            },
            ReqwestTransport,
        );

        let reply = client
            .send_message(
                "",
                "gpt-4.1-mini",
                &[ChatMessage {
                    role: ChatRole::User,
                    content: "hello".to_string(),
                }],
            )
            .expect("real transport request should succeed");

        server.join().expect("server thread should finish");
        assert_eq!(reply.content, "from server");
    }

    #[test]
    fn reqwest_transport_gets_version_from_local_server() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let addr = listener.local_addr().expect("local addr");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept should succeed");
            let mut buffer = [0_u8; 4096];
            let bytes_read = stream.read(&mut buffer).expect("request should read");
            let request = String::from_utf8_lossy(&buffer[..bytes_read]).to_string();
            assert!(request.starts_with("GET /v1/version HTTP/1.1"));

            let body = r#"{"version":"0.1.0"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("response should write");
        });

        let client = OpenAiCompatibleClient::new(
            format!("http://{}", addr),
            None,
            RetryPolicy {
                timeout_ms: 5_000,
                max_retries: 0,
            },
            ReqwestTransport,
        );

        client.endpoint_health(None, "ignored-model").unwrap();

        server.join().expect("server thread should finish");
    }
}
