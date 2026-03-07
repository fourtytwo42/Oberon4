use crate::AuthError;
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use serde::Deserialize;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
};
use tokio::{net::TcpSocket, sync::oneshot, task::JoinHandle};
use url::Url;

#[derive(Debug)]
pub struct OAuthCallbackServer {
    pub addr: SocketAddr,
    pub callback_rx: oneshot::Receiver<Result<OAuthCallbackPayload, AuthError>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join_handle: Option<JoinHandle<()>>,
}

impl OAuthCallbackServer {
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.await;
        }
    }
}

#[derive(Debug, Clone)]
pub struct OAuthCallbackPayload {
    pub state: String,
    pub code: String,
}

#[derive(Debug, Deserialize)]
struct CallbackQuery {
    #[serde(default)]
    state: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    return_to: Option<String>,
}

#[derive(Clone)]
struct CallbackState {
    tx: CallbackResultSender,
    default_return_to: Option<String>,
}

type CallbackResult = Result<OAuthCallbackPayload, AuthError>;
type CallbackResultSender = Arc<Mutex<Option<oneshot::Sender<CallbackResult>>>>;

pub async fn start_oauth_callback_server(
    host: Option<IpAddr>,
    port: Option<u16>,
    callback_path: Option<&str>,
    default_return_to: Option<&str>,
) -> Result<OAuthCallbackServer, AuthError> {
    let host = host.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    let port = port.unwrap_or(1455);
    let callback_path = callback_path.unwrap_or("/auth/callback");
    let addr = SocketAddr::new(host, port);

    let (tx, rx) = oneshot::channel::<CallbackResult>();
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let state = CallbackState {
        tx: Arc::new(Mutex::new(Some(tx))),
        default_return_to: sanitize_default_return_to(default_return_to),
    };

    let router = Router::new()
        .route(callback_path, get(handler))
        .with_state(state);

    let socket = TcpSocket::new_v4().map_err(|e| AuthError::CallbackBindFailed(e.to_string()))?;
    socket
        .set_reuseaddr(true)
        .map_err(|e| AuthError::CallbackBindFailed(e.to_string()))?;
    socket
        .bind(addr)
        .map_err(|e| AuthError::CallbackBindFailed(e.to_string()))?;
    let listener = socket
        .listen(1024)
        .map_err(|e| AuthError::CallbackBindFailed(e.to_string()))?;
    let local_addr = listener
        .local_addr()
        .map_err(|e| AuthError::CallbackBindFailed(e.to_string()))?;

    let join_handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await;
    });

    Ok(OAuthCallbackServer {
        addr: local_addr,
        callback_rx: rx,
        shutdown_tx: Some(shutdown_tx),
        join_handle: Some(join_handle),
    })
}

async fn handler(
    State(state): State<CallbackState>,
    Query(query): Query<CallbackQuery>,
) -> impl IntoResponse {
    let return_to = sanitize_return_to(query.return_to.as_deref())
        .or_else(|| state.default_return_to.clone())
        .unwrap_or_else(|| "http://localhost:11434/console".to_string());
    let outcome = if let Some(error) = query.error {
        Err(AuthError::CallbackError(error))
    } else if query.code.trim().is_empty() || query.state.trim().is_empty() {
        Err(AuthError::CallbackMissingFields)
    } else {
        Ok(OAuthCallbackPayload {
            state: query.state,
            code: query.code,
        })
    };

    if let Some(tx) = state
        .tx
        .lock()
        .expect("callback state mutex poisoned")
        .take()
    {
        let _ = tx.send(outcome.clone());
    }

    match outcome {
        Ok(_) => Html(render_callback_page(
            "You are being redirected, please wait.",
            "",
            &return_to,
            900,
            false,
            None,
        ))
        .into_response(),
        Err(err) => Html(render_callback_page(
            "OAuth sign-in failed",
            "Returning you to LLM Hotspot Console...",
            &return_to,
            1200,
            true,
            Some(&err.to_string()),
        ))
        .into_response(),
    }
}

fn sanitize_return_to(value: Option<&str>) -> Option<String> {
    let raw = value?.trim();
    if raw.is_empty() {
        return None;
    }
    let parsed = Url::parse(raw).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    if host != "localhost" && host != "127.0.0.1" {
        return None;
    }
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return None;
    }
    Some(parsed.to_string())
}

fn sanitize_default_return_to(value: Option<&str>) -> Option<String> {
    let raw = value?.trim();
    if raw.is_empty() {
        return None;
    }
    let parsed = Url::parse(raw).ok()?;
    let _ = parsed.host_str()?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return None;
    }
    Some(parsed.to_string())
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn render_callback_page(
    title: &str,
    subtitle: &str,
    return_to: &str,
    redirect_ms: u64,
    is_error: bool,
    error_detail: Option<&str>,
) -> String {
    use base64::{engine::general_purpose, Engine as _};

    let title = html_escape(title);
    let subtitle = html_escape(subtitle);
    let return_to = html_escape(return_to);
    let detail = error_detail.map(html_escape).unwrap_or_default();
    let detail_html = if detail.is_empty() {
        String::new()
    } else {
        format!(r#"<div class="detail">{detail}</div>"#)
    };
    let subtitle_html = if subtitle.trim().is_empty() {
        String::new()
    } else {
        format!(r#"<p class="subtitle">{subtitle}</p>"#)
    };
    let status_class = if is_error { "error" } else { "ok" };
    let is_error_js = if is_error { "true" } else { "false" };

    // Theme-aware, offline-safe branding for the localhost:1455 callback page.
    // Light mode: dark logo. Dark mode: light logo.
    let light_mode_logo = format!(
        "data:image/png;base64,{}",
        general_purpose::STANDARD.encode(include_bytes!("../../../dark-logo.png"))
    );
    let dark_mode_logo = format!(
        "data:image/png;base64,{}",
        general_purpose::STANDARD.encode(include_bytes!("../../../light-logo.png"))
    );
    format!(
        r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <meta name="color-scheme" content="light dark" />
  <link rel="icon" id="cbFavicon" type="image/png" href="{light_mode_logo}" />
  <title>Sign into Codex</title>
  <style>
    :root {{
      --bg:#f3f4f6;
      --text:#0d0d0d;
      --sub:#5d5d5d;
      --line:rgba(13,13,13,.10);
      --card:#fff;
    }}
    @media (prefers-color-scheme: dark) {{
      :root {{
        --bg:#0b0f14;
        --text:#e5e7eb;
        --sub:#9ca3af;
        --line:#243041;
        --card:#0f172a;
      }}
    }}
    * {{ box-sizing:border-box; }}
    html {{ color-scheme: light dark; }}
    body {{
      margin:0;
      font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Oxygen, Ubuntu, Cantarell, "Open Sans", "Helvetica Neue", sans-serif;
      background:var(--bg);
      color:var(--text);
    }}
    .container {{
      min-height:100vh;
      display:flex;
      align-items:center;
      justify-content:center;
      padding:24px;
    }}
    .inner {{
      width: min(660px, 100%);
      display:flex;
      flex-direction:column;
      align-items:center;
      gap:18px;
      text-align:center;
    }}
    .logo {{
      width:96px;
      height:96px;
      object-fit:contain;
      filter: drop-shadow(0 4px 16px rgba(0,0,0,.08));
    }}
    h1 {{
      margin:0;
      font-weight:400;
      font-size:50px;
      line-height:58px;
      letter-spacing:-0.02em;
    }}
    .subtitle {{
      color:var(--sub);
      font-size:36px;
      line-height:42px;
      font-weight:400;
      margin:0;
    }}
    .detail {{
      color:#991b1b;
      font-size:14px;
      max-width:600px;
      word-break:break-word;
    }}
    .hint {{
      margin-top:8px;
      color:#6b7280;
      font-size:13px;
    }}
    .status.ok .subtitle {{ color:var(--sub); }}
    .status.error .subtitle {{ color:#991b1b; }}
    @media (max-width:720px) {{
      h1 {{ font-size:36px; line-height:42px; }}
      .subtitle {{ font-size:24px; line-height:30px; }}
    }}
  </style>
</head>
<body>
  <div class="container">
    <div class="inner status {status_class}">
      <img id="cbLogo" class="logo" src="{light_mode_logo}" alt="LLM Hotspot logo" />
      <h1>{title}</h1>
      {subtitle_html}
      {detail_html}
    </div>
  </div>
  <script>
    (function() {{
      const LIGHT_LOGO = "{light_mode_logo}";
      const DARK_LOGO = "{dark_mode_logo}";
      const mq = window.matchMedia ? window.matchMedia('(prefers-color-scheme: dark)') : null;
      function apply() {{
        const uri = (mq && mq.matches) ? DARK_LOGO : LIGHT_LOGO;
        const img = document.getElementById('cbLogo');
        const fav = document.getElementById('cbFavicon');
        if(img) img.src = uri;
        if(fav) fav.href = uri;
      }}
      apply();
      if(mq && mq.addEventListener) mq.addEventListener('change', apply);
    }})();
    (function() {{
      // If this callback runs in a popup opened by the console, notify opener and close.
      // Fallback to redirect for normal tabs.
      try {{
        if (window.opener && !window.opener.closed) {{
          window.opener.postMessage({{ type: 'llmhotspot-oauth-callback', ok: !{is_error_js} }}, '*');
          window.close();
        }}
      }} catch (_) {{}}
      setTimeout(function() {{
        window.location.replace("{return_to}");
      }}, {redirect_ms});
    }})();
  </script>
</body>
</html>"##
    )
}

#[cfg(test)]
mod callback_page_tests {
    use super::render_callback_page;

    #[test]
    fn callback_page_includes_theme_aware_inline_logos_and_no_external_fetch() {
        let html =
            render_callback_page("t", "s", "http://localhost:11434/console", 700, false, None);
        assert!(html.contains("data:image/png;base64,"));
        assert!(html.contains("prefers-color-scheme"));
        assert!(!html.contains("llmhotspot.com/logo.png"));
    }
}
