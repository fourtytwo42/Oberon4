use llm_storage::SqliteStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub(crate) const RUNTIME_IPC_PROTOCOL_VERSION: u32 = 1;
pub(crate) const RUNTIME_IPC_CAPABILITIES: &[&str] = &[
    "status",
    "console_url",
    "console_admin_key",
    "console_admin_key_auth_v1",
    "client_metrics_v1",
];

#[derive(Debug, Deserialize)]
struct RuntimeIpcRequest {
    op: String,
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    remote: Option<bool>,
    #[serde(default)]
    auth: Option<String>,
    #[serde(default)]
    protocol_version: Option<u32>,
    #[serde(default)]
    client_metrics: Option<ClientMetricsDelta>,
}

#[derive(Debug, Serialize)]
struct RuntimeIpcResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClientMetricsDelta {
    #[serde(default)]
    ipc_hit_delta: u64,
    #[serde(default)]
    fallback_delta: u64,
    #[serde(default)]
    ipc_fail_delta: u64,
}

fn fnv1a64(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub(crate) fn runtime_pipe_name(db_path: &str) -> String {
    let db_norm = db_path.replace('\\', "/").to_ascii_lowercase();
    format!(r"\\.\pipe\llmhotspot-{:016x}-runtime", fnv1a64(&db_norm))
}

#[cfg(target_os = "windows")]
pub(crate) fn spawn_runtime_ipc_server(store: SqliteStore, db_path: String) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::windows::named_pipe::ServerOptions;

    let pipe_name = runtime_pipe_name(&db_path);
    tokio::spawn(async move {
        loop {
            let server = match ServerOptions::new().create(&pipe_name) {
                Ok(v) => v,
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                    continue;
                }
            };
            if server.connect().await.is_err() {
                continue;
            }
            let mut server = server;
            let mut line = String::new();
            {
                let mut reader = BufReader::new(&mut server);
                let _ = reader.read_line(&mut line).await;
            }
            let reply = handle_runtime_ipc_request(&store, line.trim()).await;
            let _ = server.write_all(reply.as_bytes()).await;
            let _ = server.write_all(b"\n").await;
            let _ = server.flush().await;
        }
    });
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn spawn_runtime_ipc_server(_store: SqliteStore, _db_path: String) {}

async fn handle_runtime_ipc_request(store: &SqliteStore, raw: &str) -> String {
    let req = match serde_json::from_str::<RuntimeIpcRequest>(raw) {
        Ok(v) => v,
        Err(e) => {
            let _ = increment_counter(store, "runtime_ipc_server_errors_total").await;
            eprintln!("ipc reject id=unknown error=invalid_request");
            return serde_json::to_string(&RuntimeIpcResponse {
                ok: false,
                data: None,
                error: Some(format!("invalid request: {e}")),
            })
            .unwrap_or_else(|_| r#"{"ok":false,"error":"invalid request"}"#.to_string());
        }
    };
    let request_id = req.request_id.as_deref().unwrap_or("ipc_unknown");
    eprintln!("ipc recv id={request_id} op={}", req.op);

    let _ = increment_counter(store, "runtime_ipc_server_requests_total").await;
    if req.op == "hello" {
        let resp = RuntimeIpcResponse {
            ok: true,
            data: Some(json!({
                "protocol_version": RUNTIME_IPC_PROTOCOL_VERSION,
                "capabilities": RUNTIME_IPC_CAPABILITIES,
            })),
            error: None,
        };
        return serde_json::to_string(&resp)
            .unwrap_or_else(|_| json!({"ok":false,"error":"encode error"}).to_string());
    }
    if req.protocol_version != Some(RUNTIME_IPC_PROTOCOL_VERSION) {
        let _ = increment_counter(store, "runtime_ipc_server_errors_total").await;
        eprintln!("ipc reject id={request_id} error=protocol_mismatch");
        let resp = RuntimeIpcResponse {
            ok: false,
            data: None,
            error: Some(format!(
                "protocol mismatch: client={:?} server={}",
                req.protocol_version, RUNTIME_IPC_PROTOCOL_VERSION
            )),
        };
        return serde_json::to_string(&resp)
            .unwrap_or_else(|_| json!({"ok":false,"error":"encode error"}).to_string());
    }
    if let Some(delta) = req.client_metrics.as_ref() {
        let _ = add_counter(store, "runtime_ipc_shell_hits_total", delta.ipc_hit_delta).await;
        let _ = add_counter(
            store,
            "runtime_ipc_shell_fallback_total",
            delta.fallback_delta,
        )
        .await;
        let _ = add_counter(store, "runtime_ipc_shell_fail_total", delta.ipc_fail_delta).await;
    }

    let out = match req.op.as_str() {
        "status" => super::status_command(store).await,
        "console_url" => {
            super::app_command(
                store,
                super::AppCommands::ConsoleUrl {
                    remote: req.remote.unwrap_or(false),
                },
            )
            .await
        }
        "console_admin_key" => {
            super::app_command(
                store,
                super::AppCommands::ConsoleAdminKey {
                    local_auth: req.auth.clone(),
                },
            )
            .await
        }
        _ => Err(anyhow::anyhow!("unsupported op")),
    };

    let resp = match out {
        Ok(v) => RuntimeIpcResponse {
            ok: true,
            data: Some(v),
            error: None,
        },
        Err(e) => RuntimeIpcResponse {
            ok: false,
            data: None,
            error: Some(e.to_string()),
        },
    };
    if !resp.ok {
        let _ = increment_counter(store, "runtime_ipc_server_errors_total").await;
        eprintln!("ipc reject id={request_id} error=handler_failed");
    } else {
        eprintln!("ipc done id={request_id} op={}", req.op);
    }
    serde_json::to_string(&resp)
        .unwrap_or_else(|_| json!({"ok":false,"error":"encode error"}).to_string())
}

async fn increment_counter(store: &SqliteStore, key: &str) -> Result<(), String> {
    add_counter(store, key, 1).await
}

async fn add_counter(store: &SqliteStore, key: &str, delta: u64) -> Result<(), String> {
    if delta == 0 {
        return Ok(());
    }
    let current = store
        .get_app_state(key)
        .await
        .map_err(|e| e.to_string())?
        .parse::<u64>()
        .unwrap_or(0);
    let next = current.saturating_add(delta);
    store
        .set_app_state(key, &next.to_string())
        .await
        .map_err(|e| e.to_string())
}
