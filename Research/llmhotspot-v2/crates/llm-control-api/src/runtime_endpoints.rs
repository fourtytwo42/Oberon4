use super::*;

#[derive(Debug, Clone, Serialize)]
struct EndpointsPayload {
    local_endpoint_url: String,
    remote_endpoint_url: Option<String>,
    remote_console_url: Option<String>,
    remote_assignment_stale: bool,
    remote_assignment_last_error: Option<String>,
    remote_assignment_lease_expires_unix: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
struct TrayStatusPayload {
    native_supported: bool,
    native_enabled: bool,
    fallback_mode: String,
    last_native_error: Option<String>,
    tray_host_mode: String,
    tray_host_pid: Option<String>,
    daemon_pid: Option<String>,
}
pub(crate) async fn get_endpoints(State(state): State<AppState>) -> impl IntoResponse {
    let remote = read_remote_assignment(&state).await.unwrap_or_default();
    let mut stale = false;
    let mut last_error = None;
    let mut lease_expires = None;
    if let Some(store) = &state.store {
        let raw_lease = store
            .get_app_state("remote_assignment_lease_expires_unix")
            .await
            .unwrap_or_default();
        lease_expires = raw_lease.parse::<u64>().ok();
        stale = lease_expires.map(|v| v <= now_unix()).unwrap_or(false);
        let raw_error = store
            .get_app_state("remote_assignment_last_error")
            .await
            .unwrap_or_default();
        if !raw_error.is_empty() {
            last_error = Some(raw_error);
        }
    }
    let remote_endpoint_url = remote
        .remote_base_url
        .as_ref()
        .map(|base| format!("{}/v1", normalized_base(base)));
    Json(EndpointsPayload {
        local_endpoint_url: format!("{}/v1", normalized_base(&state.local_base_url)),
        remote_endpoint_url,
        remote_console_url: remote.remote_console_url,
        remote_assignment_stale: stale,
        remote_assignment_last_error: last_error,
        remote_assignment_lease_expires_unix: lease_expires,
    })
}

pub(crate) async fn tray_status(State(state): State<AppState>) -> impl IntoResponse {
    let Some(store) = &state.store else {
        return Json(TrayStatusPayload {
            native_supported: false,
            native_enabled: false,
            fallback_mode: "ui_cli".to_string(),
            last_native_error: None,
            tray_host_mode: "daemon_embedded".to_string(),
            tray_host_pid: None,
            daemon_pid: None,
        });
    };
    let native_supported = store
        .get_app_state("tray_native_supported")
        .await
        .unwrap_or_default()
        == "1";
    let native_enabled = store
        .get_app_state("tray_native_enabled")
        .await
        .unwrap_or_default()
        == "1";
    let fallback_mode = store
        .get_app_state("tray_fallback_mode")
        .await
        .unwrap_or_else(|_| "ui_cli".to_string());
    let last_native_error = store
        .get_app_state("tray_last_native_error")
        .await
        .ok()
        .filter(|v| !v.trim().is_empty());
    let tray_host_mode = store
        .get_app_state("tray_host_mode")
        .await
        .unwrap_or_else(|_| "daemon_embedded".to_string());
    let tray_host_pid = store
        .get_app_state("tray_host_pid")
        .await
        .ok()
        .filter(|v| !v.trim().is_empty());
    let daemon_pid = store
        .get_app_state("runtime_pid")
        .await
        .ok()
        .filter(|v| !v.trim().is_empty());
    Json(TrayStatusPayload {
        native_supported,
        native_enabled,
        fallback_mode,
        last_native_error,
        tray_host_mode,
        tray_host_pid,
        daemon_pid,
    })
}
