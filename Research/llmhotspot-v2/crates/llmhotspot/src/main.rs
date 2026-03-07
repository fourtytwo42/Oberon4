use anyhow::{anyhow, Context, Result};
use axum::{extract::Request, middleware, middleware::Next, response::IntoResponse, Json, Router};
use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand};
use llm_auth::{OpenAiChatGptOAuthClient, OpenAiOAuthConfig};
use llm_control_api::{app_router, AppState};
use llm_storage::{
    ApiKeyRecord, BrokerStore, OAuthAccountRecord, PostgresStore, RemoteAssignment, SqliteStore,
    StoredTokenRecord,
};
use llm_tray::{TrayAction, TrayCommandHandler, TrayRuntime};
use rand::seq::SliceRandom;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    process,
    process::Command,
    sync::mpsc,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    net::TcpListener,
    process::Command as TokioCommand,
    signal,
    time::{sleep, Duration},
};
use url::Url;

const DEFAULT_DB_PATH: &str = "./llmhotspot.db";
const DEFAULT_LISTEN: &str = "127.0.0.1:11434";
const PID_FILE: &str = ".llmhotspot.pid";
const STARTUP_CONSOLE_URL_FILE: &str = ".llmhotspot.console.url";
const CONSOLE_ADMIN_BOOTSTRAP_TTL_SECS: u64 = 30 * 24 * 60 * 60;
const RUNTIME_LOCK_KEY: &str = "runtime_pid";
const RUNTIME_STARTED_KEY: &str = "runtime_started_unix";
const BROKER_OFFLINE_ENFORCEMENT_SECS: u64 = 24 * 60 * 60;
const APP_BROKER_LAST_OK_UNIX_KEY: &str = "app_broker_last_ok_unix";
const APP_BROKER_OFFLINE_SINCE_UNIX_KEY: &str = "app_broker_offline_since_unix";
const APP_BROKER_LAST_ERROR_KEY: &str = "app_broker_last_error";
const APP_BROKER_WARNING_KEY: &str = "app_broker_warning";
const APP_MANAGED_TUNNEL_ID_KEY: &str = "app_managed_tunnel_id";
const APP_MANAGED_TUNNEL_HOSTNAME_KEY: &str = "app_managed_tunnel_hostname";
const APP_MANAGED_TUNNEL_TOKEN_KEY: &str = "app_managed_tunnel_token";
const APP_MANAGED_TUNNEL_CACHED_UNIX_KEY: &str = "app_managed_tunnel_cached_unix";

mod runtime_ipc;
mod tunnel_broker;

trait Clock: Send + Sync {
    fn now_unix(&self) -> u64;
}

struct SystemClock;

impl Clock for SystemClock {
    fn now_unix(&self) -> u64 {
        now_unix()
    }
}

#[derive(Debug, Parser)]
#[command(name = "llmhotspot", about = "LLM Hotspot v2 control CLI", version)]
struct Cli {
    #[arg(long, global = true, help = "Emit machine-readable JSON")]
    json: bool,

    #[arg(long, global = true, default_value = DEFAULT_DB_PATH, help = "SQLite database path")]
    db_path: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    #[command(
        about = "Start daemon + API + UI (+ tray unless disabled)",
        after_help = "Examples:\n  llmhotspot serve\n  llmhotspot serve --listen 0.0.0.0:11434 --no-tray"
    )]
    Serve(ServeArgs),
    #[command(
        about = "Show app and endpoint status",
        after_help = "Examples:\n  llmhotspot status\n  llmhotspot --json status"
    )]
    Status,
    #[command(subcommand, about = "OAuth account commands")]
    Oauth(OAuthCommands),
    #[command(subcommand, about = "API key lifecycle commands")]
    Keys(KeyCommands),
    #[command(subcommand, about = "Endpoint URL and test commands")]
    Endpoint(EndpointCommands),
    #[command(subcommand, about = "Application control commands")]
    App(AppCommands),
    #[command(subcommand, about = "Session controls")]
    Session(SessionCommands),
    #[command(about = "Generate CLI docs at docs/CLI_REFERENCE.md")]
    GenerateCliDocs,
    #[command(
        about = "Run tunnel broker API (Cloudflare provisioning service)",
        after_help = "This is intended to be deployed centrally.\n\nRequires:\n  LLMHOTSPOT_TUNNEL_BROKER=1\n  CF_API_TOKEN / CF_ACCOUNT_ID / CF_ZONE_ID\n  CRON_SECRET (for /api/jobs/tunnels-reap)\n"
    )]
    Broker(BrokerArgs),
}

#[derive(Debug, Args)]
struct ServeArgs {
    #[arg(long, default_value = DEFAULT_LISTEN)]
    listen: String,
    #[arg(long)]
    no_tray: bool,
}

#[derive(Debug, Clone)]
struct RuntimeEnvConfig {
    open_console_on_start: bool,
    local_advertise_host: Option<String>,
    advertise_host: Option<String>,
}

impl RuntimeEnvConfig {
    fn from_env() -> Result<Self> {
        let local_advertise_host = env_nonempty("LLMHOTSPOT_LOCAL_ADVERTISE_HOST");
        let advertise_host = env_nonempty("LLMHOTSPOT_ADVERTISE_HOST");
        let tunnel_broker_url = env_nonempty("LLMHOTSPOT_TUNNEL_BROKER_URL");
        let website_base_url = env_nonempty("LLMHOTSPOT_WEBSITE_BASE_URL");
        let app_base_url = env_nonempty("APP_BASE_URL");
        let control_plane_url = env_nonempty("LLMHOTSPOT_CONTROL_PLANE_URL");
        let remote_base_url = env_nonempty("LLMHOTSPOT_REMOTE_BASE_URL");
        let remote_console_url = env_nonempty("LLMHOTSPOT_REMOTE_CONSOLE_URL");

        for (name, value) in [
            ("LLMHOTSPOT_TUNNEL_BROKER_URL", tunnel_broker_url.as_deref()),
            ("LLMHOTSPOT_WEBSITE_BASE_URL", website_base_url.as_deref()),
            ("APP_BASE_URL", app_base_url.as_deref()),
            ("LLMHOTSPOT_CONTROL_PLANE_URL", control_plane_url.as_deref()),
            ("LLMHOTSPOT_REMOTE_BASE_URL", remote_base_url.as_deref()),
            (
                "LLMHOTSPOT_REMOTE_CONSOLE_URL",
                remote_console_url.as_deref(),
            ),
        ] {
            if let Some(v) = value {
                let parsed = Url::parse(v).with_context(|| format!("invalid {name}"))?;
                if !matches!(parsed.scheme(), "https" | "http") {
                    return Err(anyhow!("{name} must use http/https"));
                }
            }
        }

        Ok(Self {
            open_console_on_start: std::env::var("LLMHOTSPOT_OPEN_CONSOLE_ON_START")
                .map(|v| v != "0")
                .unwrap_or(true),
            local_advertise_host,
            advertise_host,
        })
    }
}

#[derive(Debug, Args)]
struct BrokerArgs {
    #[arg(long, default_value = "0.0.0.0:3001")]
    listen: String,
    #[arg(long, help = "Broker database backend (default: postgres)")]
    db_backend: Option<String>,
    #[arg(
        long,
        help = "Broker database URL (postgres expected for db-backend=postgres)"
    )]
    database_url: Option<String>,
}

#[derive(Debug, Subcommand)]
enum OAuthCommands {
    #[command(
        about = "Start OAuth flow URL generation",
        after_help = "Examples:\n  llmhotspot oauth start"
    )]
    Start,
    #[command(
        about = "Complete OAuth exchange using callback state + code",
        after_help = "Examples:\n  llmhotspot oauth complete --state abc --code xyz"
    )]
    Complete {
        #[arg(long)]
        state: String,
        #[arg(long)]
        code: String,
        #[arg(long)]
        redirect_uri: Option<String>,
    },
    #[command(
        about = "Connect/register an OAuth account",
        after_help = "Examples:\n  llmhotspot oauth connect --account acct_1 --email user@example.com --name Primary"
    )]
    Connect {
        #[arg(long)]
        account: String,
        #[arg(long)]
        email: String,
        #[arg(long)]
        name: String,
    },
    #[command(
        about = "List connected OAuth accounts",
        after_help = "Examples:\n  llmhotspot oauth list"
    )]
    List,
    #[command(
        about = "Activate account by id",
        after_help = "Examples:\n  llmhotspot oauth activate --account acct_1"
    )]
    Activate {
        #[arg(long)]
        account: String,
    },
    #[command(subcommand, about = "Priority operations")]
    Priority(OAuthPriorityCommands),
    #[command(
        about = "Disable account",
        after_help = "Examples:\n  llmhotspot oauth disable --account acct_1"
    )]
    Disable {
        #[arg(long)]
        account: String,
    },
    #[command(
        about = "Enable account",
        after_help = "Examples:\n  llmhotspot oauth enable --account acct_1"
    )]
    Enable {
        #[arg(long)]
        account: String,
    },
    #[command(
        about = "Disconnect account",
        after_help = "Examples:\n  llmhotspot oauth disconnect --account acct_1"
    )]
    Disconnect {
        #[arg(long)]
        account: String,
    },
}

#[derive(Debug, Subcommand)]
enum OAuthPriorityCommands {
    #[command(
        about = "Set account priority index",
        after_help = "Examples:\n  llmhotspot oauth priority set --account acct_1 --index 0"
    )]
    Set {
        #[arg(long)]
        account: String,
        #[arg(long)]
        index: usize,
    },
}

#[derive(Debug, Subcommand)]
enum KeyCommands {
    #[command(
        about = "Create API key",
        after_help = "Examples:\n  llmhotspot keys create --label ci-key\n  llmhotspot keys create --label temp --expires 2026-02-13T20:00:00Z"
    )]
    Create {
        #[arg(long)]
        label: String,
        #[arg(long)]
        expires: Option<String>,
    },
    #[command(
        about = "List API keys",
        after_help = "Examples:\n  llmhotspot keys list"
    )]
    List,
    #[command(
        about = "Revoke API key",
        after_help = "Examples:\n  llmhotspot keys revoke --key-id <id>"
    )]
    Revoke {
        #[arg(long = "key-id")]
        key_id: String,
    },
    #[command(
        about = "Expire API key immediately",
        after_help = "Examples:\n  llmhotspot keys expire --key-id <id>"
    )]
    Expire {
        #[arg(long = "key-id")]
        key_id: String,
    },
    #[command(
        about = "Copy/show raw API key",
        after_help = "Examples:\n  llmhotspot keys copy --key-id <id>"
    )]
    Copy {
        #[arg(long = "key-id")]
        key_id: String,
    },
}

#[derive(Debug, Subcommand)]
enum EndpointCommands {
    #[command(about = "Show local endpoint URL")]
    Local,
    #[command(about = "Show remote endpoint URL")]
    Remote,
    #[command(subcommand, about = "Run endpoint tests")]
    Test(EndpointTestCommands),
}

#[derive(Debug, Subcommand)]
enum EndpointTestCommands {
    #[command(about = "Test local endpoint")]
    Local,
    #[command(about = "Test remote endpoint")]
    Remote,
}

#[derive(Debug, Subcommand)]
enum AppCommands {
    #[command(about = "Open console URL in browser")]
    OpenConsole,
    #[command(about = "Print sanitized console URL")]
    ConsoleUrl {
        #[arg(long, help = "Prefer remote console URL from assignment if available")]
        remote: bool,
    },
    #[command(about = "Return current local console admin bootstrap key")]
    ConsoleAdminKey {
        #[arg(long, hide = true, help = "Local auth token for sensitive retrieval")]
        local_auth: Option<String>,
    },
    #[command(about = "Run local runtime/env/packaging diagnostics")]
    Doctor,
    #[command(about = "Ingest shell IPC metric deltas", hide = true)]
    IpcMetrics {
        #[arg(long, default_value_t = 0)]
        hits: u64,
        #[arg(long, default_value_t = 0)]
        fallbacks: u64,
        #[arg(long, default_value_t = 0)]
        fails: u64,
    },
    #[command(about = "Persist pending shell IPC metric deltas", hide = true)]
    IpcMetricsPending {
        #[arg(long, default_value_t = 0)]
        hits: u64,
        #[arg(long, default_value_t = 0)]
        fallbacks: u64,
        #[arg(long, default_value_t = 0)]
        fails: u64,
        #[arg(long, default_value_t = 0)]
        updated_unix: u64,
    },
    #[command(about = "Mark app restart request")]
    Restart,
    #[command(about = "Stop running app")]
    Stop,
}

#[derive(Debug, Subcommand)]
enum SessionCommands {
    #[command(about = "Start hotspot session")]
    Start,
    #[command(about = "Stop hotspot session")]
    Stop,
    #[command(about = "Get hotspot session status")]
    Status,
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("{err:#}");
        process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let payload = match cli.command {
        Commands::GenerateCliDocs => generate_cli_docs().await?,
        Commands::Broker(args) => broker_command(&cli.db_path, args).await?,
        other => {
            let store = open_store(&cli.db_path).await?;
            match other {
                Commands::Serve(args) => serve_command(store, &cli.db_path, args).await?,
                Commands::Status => status_command(&store).await?,
                Commands::Oauth(cmd) => oauth_command(&store, cmd).await?,
                Commands::Keys(cmd) => keys_command(&store, cmd).await?,
                Commands::Endpoint(cmd) => endpoint_command(&store, cmd).await?,
                Commands::App(cmd) => app_command(&store, cmd).await?,
                Commands::Session(cmd) => session_command(&store, cmd).await?,
                Commands::Broker(_) => unreachable!(),
                Commands::GenerateCliDocs => unreachable!(),
            }
        }
    };

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        print_human(&payload);
    }

    Ok(())
}

async fn broker_command(db_path: &str, args: BrokerArgs) -> Result<Value> {
    if !tunnel_broker::enabled() {
        return Ok(json!({
            "ok": false,
            "error": "broker disabled (set LLMHOTSPOT_TUNNEL_BROKER=1)"
        }));
    }
    let listen: SocketAddr = args
        .listen
        .parse()
        .context("invalid --listen socket address")?;
    let listener = TcpListener::bind(listen).await?;
    let actual_listen = listener.local_addr()?;

    let backend = args
        .db_backend
        .or_else(|| std::env::var("LLMHOTSPOT_BROKER_DB_BACKEND").ok())
        .unwrap_or_else(|| "postgres".to_string())
        .to_ascii_lowercase();
    let broker_store: Arc<dyn BrokerStore> = match backend.as_str() {
        "sqlite" => Arc::new(open_store(db_path).await?),
        "postgres" => {
            let database_url = args
                .database_url
                .or_else(|| std::env::var("LLMHOTSPOT_BROKER_DATABASE_URL").ok())
                .or_else(|| std::env::var("DATABASE_URL").ok())
                .ok_or_else(|| {
                    anyhow!(
                        "missing broker database URL; set --database-url or LLMHOTSPOT_BROKER_DATABASE_URL"
                    )
                })?;
            Arc::new(
                PostgresStore::connect(&database_url)
                    .await
                    .context("failed to connect broker postgres database")?,
            )
        }
        other => {
            return Err(anyhow!(
                "invalid broker db backend '{other}', expected postgres or sqlite"
            ));
        }
    };

    let app = Router::new()
        .route(
            "/health",
            axum::routing::get(|| async { Json(json!({"ok": true})) }),
        )
        .route(
            "/keys/health",
            axum::routing::get(|| async { Json(json!({"ok": true})) }),
        )
        .merge(tunnel_broker::router(broker_store))
        .layer(middleware::from_fn(request_log_middleware));

    let serve_url = format!("http://{}", actual_listen);
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = signal::ctrl_c().await;
        })
        .await;
    result?;

    Ok(json!({"ok":true,"serve_url":serve_url}))
}

async fn request_log_middleware(req: Request, next: Next) -> impl IntoResponse {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let res = next.run(req).await;
    if std::env::var("LLMHOTSPOT_TRACE_BROKER")
        .ok()
        .map(|v| v.trim() == "1")
        .unwrap_or(false)
    {
        eprintln!("broker {method} {path} -> {}", res.status().as_u16());
    }
    res
}

async fn open_store(db_path: &str) -> Result<SqliteStore> {
    let db_url = if db_path == ":memory:" {
        "sqlite::memory:".to_string()
    } else {
        format!("sqlite://{db_path}?mode=rwc")
    };
    SqliteStore::connect(&db_url)
        .await
        .with_context(|| format!("failed to open sqlite database at {db_path}"))
}

#[derive(Clone)]
struct RuntimeTrayHandler {
    tx: mpsc::Sender<TrayAction>,
}

impl TrayCommandHandler for RuntimeTrayHandler {
    fn handle(&self, action: TrayAction) {
        let _ = self.tx.send(action);
    }
}

fn restart_current_process(db_path: &str, listen: &str, no_tray: bool) -> Result<()> {
    let exe = std::env::current_exe().context("resolve current executable for restart")?;
    let mut cmd = Command::new(exe);
    cmd.arg("--db-path")
        .arg(db_path)
        .arg("serve")
        .arg("--listen")
        .arg(listen);
    if no_tray {
        cmd.arg("--no-tray");
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn().context("spawn restarted process")?;
    Ok(())
}

async fn serve_command(store: SqliteStore, db_path: &str, args: ServeArgs) -> Result<Value> {
    let env_cfg = RuntimeEnvConfig::from_env()?;
    let listen: SocketAddr = args
        .listen
        .parse()
        .context("invalid --listen socket address")?;
    acquire_runtime_lock(&store).await?;

    let remote = resolve_remote_assignment(&store).await;
    store.set_remote_assignment(&remote).await?;
    let _ = ensure_console_admin_bootstrap_key_fresh(&store).await?;
    ensure_default_api_key_exists(&store).await?;
    store
        .set_app_state("session_active", "1")
        .await
        .context("set session active")?;
    let tray_host_mode = std::env::var("LLMHOTSPOT_TRAY_HOST_MODE")
        .unwrap_or_else(|_| "daemon_embedded".to_string());
    store
        .set_app_state("tray_host_mode", &tray_host_mode)
        .await
        .context("set tray host mode")?;
    let tray_host_pid = std::env::var("LLMHOTSPOT_TRAY_HOST_PID").unwrap_or_default();
    store
        .set_app_state("tray_host_pid", &tray_host_pid)
        .await
        .context("set tray host pid")?;
    store
        .set_app_state(RUNTIME_STARTED_KEY, &now_unix().to_string())
        .await
        .context("set runtime started")?;
    if let Ok(ipc_secret) = std::env::var("LLMHOTSPOT_RUNTIME_IPC_SECRET") {
        let secret_hash = hash_key(&ipc_secret);
        store
            .set_app_state("runtime_ipc_secret_hash", &secret_hash)
            .await
            .context("set runtime ipc secret hash")?;
    } else {
        store.set_app_state("runtime_ipc_secret_hash", "").await?;
    }
    fs::write(PID_FILE, process::id().to_string())?;

    let listener = TcpListener::bind(listen).await?;
    let actual_listen = listener.local_addr()?;
    let local_base_url = local_client_base_url(actual_listen, &env_cfg);
    store
        .set_app_state("local_base_url", &local_base_url)
        .await?;
    store.set_app_state("runtime_db_path", db_path).await?;

    runtime_ipc::spawn_runtime_ipc_server(store.clone(), db_path.to_string());

    let mut app_state = AppState::with_store(store.clone(), local_base_url.clone());
    app_state.oauth_client = resolve_oauth_client_from_env();
    let tunnel_mode_notify = app_state.tunnel_mode_notify.clone();
    let mut app = app_router(app_state);
    if tunnel_broker::enabled() {
        app = app.merge(tunnel_broker::router(Arc::new(store.clone())));
    }

    let base_console_url = format!("{local_base_url}/console");
    let startup_console_url = resolve_console_open_url(&store, &base_console_url).await;
    let startup_console_public_url = resolve_console_public_url(&base_console_url);
    println!("Console URL: {startup_console_public_url}");
    if let Err(err) = fs::write(
        STARTUP_CONSOLE_URL_FILE,
        format!("{startup_console_public_url}\n"),
    ) {
        eprintln!(
            "Warning: failed to write startup console URL file {STARTUP_CONSOLE_URL_FILE}: {err}"
        );
    }

    let (tray_tx, tray_rx) = mpsc::channel::<TrayAction>();
    let tray_runtime = if !args.no_tray {
        let base_console_url = format!("{local_base_url}/console");
        let console_url = resolve_console_open_url(&store, &base_console_url).await;
        let handler = Arc::new(RuntimeTrayHandler {
            tx: tray_tx.clone(),
        });
        Some(TrayRuntime::new(console_url, handler))
    } else {
        None
    };

    let tray_supported = llm_tray::TrayRuntime::native_supported();
    store
        .set_app_state(
            "tray_native_supported",
            if tray_supported { "1" } else { "0" },
        )
        .await?;
    store
        .set_app_state(
            "tray_native_enabled",
            if tray_runtime.is_some() && tray_supported {
                "1"
            } else {
                "0"
            },
        )
        .await?;
    store
        .set_app_state(
            "tray_fallback_mode",
            if tray_supported { "none" } else { "ui_cli" },
        )
        .await?;
    store.set_app_state("tray_last_native_error", "").await?;

    let serve_url = format!("http://{}", actual_listen);
    if env_cfg.open_console_on_start {
        let _ = webbrowser::open(&startup_console_url);
    }

    if let Some(tray) = &tray_runtime {
        #[cfg(feature = "native-tray")]
        {
            let tray_clone = tray.clone();
            let tray_store = store.clone();
            let handle = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                let run = tray_clone.run_native();
                if let Err(err) = run {
                    handle.block_on(async {
                        let _ = tray_store
                            .set_app_state("tray_last_native_error", &err)
                            .await;
                        let _ = tray_store
                            .set_app_state("tray_fallback_mode", "ui_cli")
                            .await;
                        let _ = tray_store.set_app_state("tray_native_enabled", "0").await;
                    });
                }
            });
        }
    }
    if tray_runtime.is_some() {
        let restart_db_path = db_path.to_string();
        let restart_listen = args.listen.clone();
        let restart_no_tray = args.no_tray;
        tokio::task::spawn_blocking(move || loop {
            match tray_rx.recv() {
                Ok(TrayAction::OpenConsole) => {}
                Ok(TrayAction::RestartApp) => {
                    if let Err(err) =
                        restart_current_process(&restart_db_path, &restart_listen, restart_no_tray)
                    {
                        eprintln!("Failed to restart app: {err:#}");
                    }
                    process::exit(0);
                }
                Ok(TrayAction::Exit) => {
                    process::exit(0);
                }
                Err(_) => break,
            }
        });
    }

    let refresh_store = store.clone();
    tokio::spawn(async move {
        loop {
            let sleep_secs = compute_assignment_refresh_interval_secs(&refresh_store)
                .await
                .unwrap_or(60);
            sleep(Duration::from_secs(sleep_secs)).await;
            let remote = resolve_remote_assignment(&refresh_store).await;
            let _ = refresh_store.set_remote_assignment(&remote).await;
            let _ = refresh_store
                .set_app_state("remote_assignment_force_refresh", "0")
                .await;
        }
    });

    // Per-client Cloudflare tunnel lifecycle owner (cloudflared sidecar).
    let entitlement_notify = tunnel_mode_notify.clone();
    let tunnel_store = store.clone();
    tokio::spawn(async move {
        tunnel_manager_loop(tunnel_store, tunnel_mode_notify).await;
    });

    let entitlement_store = store.clone();
    tokio::spawn(async move {
        loop {
            if sync_app_session_entitlement(&entitlement_store).await {
                entitlement_notify.notify_one();
            }
            sleep(Duration::from_secs(15)).await;
        }
    });

    tokio::spawn(async move {
        let _ = signal::ctrl_c().await;
    });

    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = signal::ctrl_c().await;
        })
        .await;

    let _ = fs::remove_file(PID_FILE);
    let _ = release_runtime_lock(&store).await;
    result?;

    Ok(json!({"status":"stopped","serve_url":serve_url}))
}

fn local_client_base_url(listen: SocketAddr, cfg: &RuntimeEnvConfig) -> String {
    // When binding to all interfaces (0.0.0.0 / ::), advertise a concrete host so the UI shows a
    // usable LAN URL. Allow override for environments with multiple NICs / NAT.
    let host = match listen.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => cfg
            .local_advertise_host
            .clone()
            .or_else(|| cfg.advertise_host.clone())
            .or_else(detect_lan_ipv4)
            .unwrap_or_else(|| "localhost".to_string()),
        IpAddr::V6(ip) if ip.is_unspecified() => cfg
            .local_advertise_host
            .clone()
            .or_else(|| cfg.advertise_host.clone())
            .or_else(detect_lan_ipv4)
            .unwrap_or_else(|| "localhost".to_string()),
        _ => listen.ip().to_string(),
    };
    if host.contains(':') && !host.starts_with('[') && !host.ends_with(']') {
        format!("http://[{host}]:{}", listen.port())
    } else {
        format!("http://{host}:{}", listen.port())
    }
}

fn detect_lan_ipv4() -> Option<String> {
    // UDP connect doesn't send packets but lets the OS pick a default route and local IP.
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    let _ = sock.connect("8.8.8.8:80");
    let addr = sock.local_addr().ok()?;
    match addr.ip() {
        IpAddr::V4(ip) => (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip.to_string()),
        _ => None,
    }
}

fn env_truthy(name: &str) -> bool {
    env_nonempty(name)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes"))
        .unwrap_or(false)
}

async fn status_command(store: &SqliteStore) -> Result<Value> {
    let local = store.get_app_state("local_base_url").await?;
    let remote = store.get_remote_assignment().await?;
    let accounts = store.list_oauth_accounts().await?;
    let session_active = store.get_app_state("session_active").await? == "1";
    let runtime_pid = store.get_app_state(RUNTIME_LOCK_KEY).await?;
    let started = store.get_app_state(RUNTIME_STARTED_KEY).await?;
    let refresh_unix = store
        .get_app_state("remote_assignment_last_refresh_unix")
        .await?;
    let assignment_error = store.get_app_state("remote_assignment_last_error").await?;
    let lease_expires_unix = store
        .get_app_state("remote_assignment_lease_expires_unix")
        .await?;
    let tray_host_mode = store
        .get_app_state("tray_host_mode")
        .await
        .unwrap_or_else(|_| "daemon_embedded".to_string());
    let tray_host_pid = store
        .get_app_state("tray_host_pid")
        .await
        .unwrap_or_default();
    let clock = SystemClock;
    let stale = lease_expires_unix
        .parse::<u64>()
        .map(|v| is_lease_stale(v, &clock))
        .unwrap_or(false);

    Ok(json!({
        "local_url": if local.is_empty() { Value::Null } else { json!(local) },
        "remote_url": remote.remote_base_url,
        "remote_console_url": remote.remote_console_url,
        "connector_status": if session_active { "running" } else { "stopped" },
        "accounts_total": accounts.len(),
        "accounts_enabled": accounts.iter().filter(|a| a.enabled).count(),
        "runtime_pid": if runtime_pid.is_empty() { Value::Null } else { json!(runtime_pid) },
        "daemon_pid": if runtime_pid.is_empty() { Value::Null } else { json!(runtime_pid) },
        "runtime_started_unix": if started.is_empty() { Value::Null } else { json!(started) },
        "tray_host_mode": tray_host_mode,
        "tray_host_pid": if tray_host_pid.is_empty() { Value::Null } else { json!(tray_host_pid) },
        "remote_assignment_last_refresh_unix": if refresh_unix.is_empty() { Value::Null } else { json!(refresh_unix) },
        "remote_assignment_last_error": if assignment_error.is_empty() { Value::Null } else { json!(assignment_error) },
        "remote_assignment_lease_expires_unix": if lease_expires_unix.is_empty() { Value::Null } else { json!(lease_expires_unix) },
        "remote_assignment_stale": stale,
    }))
}

async fn oauth_command(store: &SqliteStore, cmd: OAuthCommands) -> Result<Value> {
    match cmd {
        OAuthCommands::Start => {
            let client = resolve_oauth_client_from_env();
            let flow = client.start_flow(None)?;
            if let Some(state_rec) = client.state_store.consume(&flow.state) {
                let payload = format!(
                    "{}|{}|{}",
                    state_rec.redirect_uri, state_rec.code_verifier, state_rec.expires_unix
                );
                store
                    .set_app_state(&format!("oauth_state:{}", flow.state), &payload)
                    .await?;
            }
            Ok(json!({
                "ok": true,
                "code": "oauth_start_ready",
                "message": "oauth flow initialized",
                "authorization_url":flow.authorization_url,
                "state":flow.state,
                "redirect_uri":flow.redirect_uri
            }))
        }
        OAuthCommands::Complete {
            state,
            code,
            redirect_uri,
        } => {
            let raw = store.get_app_state(&format!("oauth_state:{state}")).await?;
            if raw.is_empty() {
                return Err(anyhow!("oauth state not found; run oauth start first"));
            }
            let parts: Vec<&str> = raw.split('|').collect();
            if parts.len() != 3 {
                return Err(anyhow!("stored oauth state is invalid"));
            }
            let redirect = redirect_uri.unwrap_or_else(|| parts[0].to_string());
            let code_verifier = parts[1].to_string();
            let token = exchange_with_state_record(&code, &redirect, &code_verifier).await?;
            let account_id = token
                .account_id
                .clone()
                .unwrap_or_else(|| format!("acct_{}", now_unix()));
            let mut rec = StoredTokenRecord::new("default", &account_id, &token.refresh_token);
            rec.account_id = token.account_id.clone();
            store.save_token(&rec).await?;
            let current = store.list_oauth_accounts().await?;
            let row = OAuthAccountRecord {
                account_id: account_id.clone(),
                provider_id: "openai".to_string(),
                display_name: token.email.clone().unwrap_or_else(|| account_id.clone()),
                email: token
                    .email
                    .clone()
                    .unwrap_or_else(|| format!("{account_id}@unknown.local")),
                priority: current.len() as i64,
                enabled: true,
                last_used_unix: None,
                last_error: None,
                upstream_base_url: None,
                chat_path: None,
                responses_path: None,
                org_id: None,
                project_id: None,
                extra_headers_json: None,
            };
            store.upsert_oauth_account(&row).await?;
            Ok(json!({
                "ok": true,
                "code": "oauth_connected",
                "message": "oauth callback exchange completed",
                "account_id": account_id,
                "email": token.email,
                "expires_at_unix": token.expires_at_unix
            }))
        }
        OAuthCommands::Connect {
            account,
            email,
            name,
        } => {
            let current = store.list_oauth_accounts().await?;
            let row = OAuthAccountRecord {
                account_id: account,
                provider_id: "openai".to_string(),
                display_name: name,
                email,
                priority: current.len() as i64,
                enabled: true,
                last_used_unix: None,
                last_error: None,
                upstream_base_url: None,
                chat_path: None,
                responses_path: None,
                org_id: None,
                project_id: None,
                extra_headers_json: None,
            };
            store.upsert_oauth_account(&row).await?;
            Ok(json!({"ok":true,"code":"oauth_connected","message":"account connected"}))
        }
        OAuthCommands::List => {
            let accounts = store.list_oauth_accounts().await?;
            let mut rows = Vec::new();
            for account in accounts {
                let usage = store.list_oauth_usage(&account.account_id).await?;
                rows.push(json!({"account":account,"usage":usage}));
            }
            Ok(json!({"accounts":rows}))
        }
        OAuthCommands::Activate { account } => {
            reorder_priority(store, &account, 0).await?;
            Ok(json!({"ok":true,"code":"oauth_activated","message":"account activated"}))
        }
        OAuthCommands::Priority(OAuthPriorityCommands::Set { account, index }) => {
            reorder_priority(store, &account, index).await?;
            Ok(json!({"ok":true,"code":"oauth_priority_updated","message":"priority updated"}))
        }
        OAuthCommands::Disable { account } => {
            store.update_oauth_enabled(&account, false).await?;
            Ok(json!({"ok":true,"code":"oauth_disabled","message":"account disabled"}))
        }
        OAuthCommands::Enable { account } => {
            store.update_oauth_enabled(&account, true).await?;
            Ok(json!({"ok":true,"code":"oauth_enabled","message":"account enabled"}))
        }
        OAuthCommands::Disconnect { account } => {
            store.delete_oauth_account(&account).await?;
            Ok(json!({"ok":true,"code":"oauth_disconnected","message":"account disconnected"}))
        }
    }
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn resolve_oauth_client_from_env() -> OpenAiChatGptOAuthClient {
    let mut cfg = OpenAiOAuthConfig::default();
    if let Some(v) = env_nonempty("LLMHOTSPOT_OAUTH_CLIENT_ID") {
        cfg.client_id = v;
    }
    if let Some(v) = env_nonempty("LLMHOTSPOT_OAUTH_AUTHORIZE_URL") {
        cfg.authorize_url = v;
    }
    if let Some(v) = env_nonempty("LLMHOTSPOT_OAUTH_TOKEN_URL") {
        cfg.token_url = v;
    }
    if let Some(v) = env_nonempty("LLMHOTSPOT_OAUTH_DEFAULT_REDIRECT_URI") {
        cfg.redirect_uri = v;
    }
    if let Some(v) = env_nonempty("LLMHOTSPOT_OAUTH_SCOPE") {
        cfg.scope = v;
    }
    if let Some(v) = env_nonempty("LLMHOTSPOT_OAUTH_JWT_CLAIM_PATH") {
        cfg.jwt_claim_path = v;
    }
    OpenAiChatGptOAuthClient::new(cfg)
}

async fn reorder_priority(store: &SqliteStore, account_id: &str, index: usize) -> Result<()> {
    let mut accounts = store.list_oauth_accounts().await?;
    let pos = accounts
        .iter()
        .position(|a| a.account_id == account_id)
        .ok_or_else(|| anyhow!("account not found: {account_id}"))?;
    let item = accounts.remove(pos);
    let idx = index.min(accounts.len());
    accounts.insert(idx, item);
    for (priority, account) in accounts.into_iter().enumerate() {
        store
            .set_oauth_priority(&account.account_id, priority as i64)
            .await?;
    }
    Ok(())
}

async fn keys_command(store: &SqliteStore, cmd: KeyCommands) -> Result<Value> {
    match cmd {
        KeyCommands::Create { label, expires } => {
            let now = now_unix();
            let expires_unix = parse_optional_rfc3339(expires.as_deref())?;
            let raw_key = format!("hs_{}", uuid::Uuid::new_v4().simple());
            let key = ApiKeyRecord {
                key_id: uuid::Uuid::new_v4().to_string(),
                label,
                key_hash: hash_key(&raw_key),
                tool_choice_policy: "auto".to_string(),
                created_unix: now,
                expires_unix,
                revoked_unix: None,
                last_used_unix: None,
            };
            store.create_api_key(&key).await?;
            if set_api_key_raw_secret("default", &key.key_id, &raw_key).is_err() {
                store
                    .set_app_state(&format!("key_raw:{}", key.key_id), &raw_key)
                    .await?;
            } else {
                let _ = store
                    .delete_app_state(&format!("key_raw:{}", key.key_id))
                    .await;
            }
            Ok(json!({
                "ok": true,
                "code": "key_created",
                "message": "api key created",
                "key_id":key.key_id,
                "api_key":raw_key,
                "expires_unix":expires_unix
            }))
        }
        KeyCommands::List => {
            let keys = store.list_api_keys().await?;
            Ok(json!({"keys": keys}))
        }
        KeyCommands::Revoke { key_id } => {
            store.revoke_api_key(&key_id, now_unix()).await?;
            Ok(json!({"ok":true,"code":"key_revoked","message":"api key revoked"}))
        }
        KeyCommands::Expire { key_id } => {
            let keys = store.list_api_keys().await?;
            let target = keys
                .into_iter()
                .find(|k| k.key_id == key_id)
                .ok_or_else(|| anyhow!("unknown key id: {key_id}"))?;
            let mut updated = target.clone();
            updated.expires_unix = Some(now_unix());
            store.revoke_api_key(&target.key_id, now_unix()).await?;
            Ok(
                json!({"ok":true,"code":"key_expired","message":"api key expired","expired":updated.key_id}),
            )
        }
        KeyCommands::Copy { key_id } => {
            let raw = get_api_key_raw_secret_with_legacy_migration(store, "default", &key_id)
                .await?
                .unwrap_or_default();
            if raw.trim().is_empty() {
                return Err(anyhow!("raw key unavailable for key_id={key_id}"));
            }
            Ok(json!({"key_id":key_id,"api_key":raw}))
        }
    }
}

async fn endpoint_command(store: &SqliteStore, cmd: EndpointCommands) -> Result<Value> {
    match cmd {
        EndpointCommands::Local => {
            let base = ensure_local_base_url(store).await?;
            Ok(json!({"local_endpoint": format!("{}/v1", base)}))
        }
        EndpointCommands::Remote => {
            let remote = store.get_remote_assignment().await?;
            Ok(
                json!({"remote_endpoint": remote.remote_base_url, "remote_console_url": remote.remote_console_url}),
            )
        }
        EndpointCommands::Test(EndpointTestCommands::Local) => test_endpoint(store, true).await,
        EndpointCommands::Test(EndpointTestCommands::Remote) => test_endpoint(store, false).await,
    }
}

async fn test_endpoint(store: &SqliteStore, local: bool) -> Result<Value> {
    let target = if local {
        format!("{}/v1/models", ensure_local_base_url(store).await?)
    } else {
        let remote = store.get_remote_assignment().await?;
        let Some(base) = remote.remote_base_url else {
            return Ok(json!({
                "ok": false,
                "reason": "unassigned",
                "message": "remote endpoint unavailable"
            }));
        };
        format!("{}/v1/models", base.trim_end_matches('/'))
    };

    let resp = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?
        .get(&target)
        .send()
        .await
    {
        Ok(v) => v,
        Err(err) => {
            let reason = if err.is_timeout() {
                "timeout"
            } else {
                "request_failed"
            };
            return Ok(json!({
                "ok": false,
                "reason": reason,
                "message": err.to_string(),
                "target": target
            }));
        }
    };

    let status = resp.status().as_u16();
    if resp.status().is_success() {
        return Ok(json!({"target":target,"status":status,"ok":true}));
    }
    let reason = if status == 401 {
        "auth_failed"
    } else {
        "non_2xx"
    };
    Ok(json!({"target":target,"status":status,"ok":false,"reason":reason}))
}

async fn app_command(store: &SqliteStore, cmd: AppCommands) -> Result<Value> {
    match cmd {
        AppCommands::OpenConsole => {
            let remote = store.get_remote_assignment().await?;
            let local = ensure_local_base_url(store).await?;
            let base_url = remote
                .remote_console_url
                .unwrap_or_else(|| format!("{local}/console"));
            let url = resolve_console_open_url(store, &base_url).await;
            let _ = webbrowser::open(&url);
            Ok(json!({"ok":true,"code":"console_opened","message":"console opened","opened":url}))
        }
        AppCommands::ConsoleUrl { remote } => {
            let assigned = store.get_remote_assignment().await?;
            let local = ensure_local_base_url(store).await?;
            let base_url = if remote {
                assigned
                    .remote_console_url
                    .unwrap_or_else(|| format!("{local}/console"))
            } else {
                format!("{local}/console")
            };
            let url = resolve_console_public_url(&base_url);
            Ok(json!({
                "ok": true,
                "code": "console_url_ready",
                "message": "console url generated",
                "url": url,
                "target": if remote { "remote_or_fallback_local" } else { "local" }
            }))
        }
        AppCommands::ConsoleAdminKey { local_auth } => {
            validate_console_admin_key_access(store, local_auth.as_deref()).await?;
            let key = ensure_console_admin_bootstrap_key(store)
                .await
                .unwrap_or_default();
            Ok(json!({
                "ok": true,
                "code": "console_admin_key_ready",
                "message": "console admin key ready",
                "admin_key": key
            }))
        }
        AppCommands::Doctor => doctor_command(store).await,
        AppCommands::IpcMetrics {
            hits,
            fallbacks,
            fails,
        } => {
            add_app_state_counter(store, "runtime_ipc_shell_hits_total", hits).await?;
            add_app_state_counter(store, "runtime_ipc_shell_fallback_total", fallbacks).await?;
            add_app_state_counter(store, "runtime_ipc_shell_fail_total", fails).await?;
            Ok(json!({
                "ok": true,
                "code": "ipc_metrics_ingested",
                "hits_added": hits,
                "fallbacks_added": fallbacks,
                "fails_added": fails
            }))
        }
        AppCommands::IpcMetricsPending {
            hits,
            fallbacks,
            fails,
            updated_unix,
        } => {
            let payload = json!({
                "hits": hits,
                "fallbacks": fallbacks,
                "fails": fails,
                "updated_unix": if updated_unix == 0 { Value::Null } else { Value::from(updated_unix) }
            });
            store
                .set_app_state("runtime_ipc_shell_pending_json", &payload.to_string())
                .await?;
            Ok(json!({
                "ok": true,
                "code": "ipc_metrics_pending_updated"
            }))
        }
        AppCommands::Restart => {
            store.set_app_state("restart_requested", "1").await?;
            Ok(json!({"ok":true,"code":"app_restart_requested","message":"restart requested"}))
        }
        AppCommands::Stop => {
            store.set_app_state("session_active", "0").await?;
            let runtime_pid = store
                .get_app_state(RUNTIME_LOCK_KEY)
                .await
                .unwrap_or_default();
            if let Ok(pid) = runtime_pid.parse::<i32>() {
                #[cfg(unix)]
                {
                    let _ = process_kill_unix(pid);
                    for _ in 0..10 {
                        if !process_is_alive(pid) {
                            break;
                        }
                        sleep(Duration::from_millis(200)).await;
                    }
                }
                #[cfg(not(unix))]
                {
                    let _ = pid;
                }
            }
            let _ = clear_runtime_lock(store).await;
            Ok(json!({"ok":true,"code":"app_stopped","message":"stop signal sent"}))
        }
    }
}

async fn resolve_console_open_url(store: &SqliteStore, base_url: &str) -> String {
    let admin_key = ensure_console_admin_bootstrap_key(store).await;
    with_admin_key_query(base_url, admin_key.as_deref())
}

fn resolve_console_public_url(base_url: &str) -> String {
    match Url::parse(base_url) {
        Ok(mut parsed) => {
            parsed.set_query(None);
            parsed.to_string()
        }
        Err(_) => base_url.to_string(),
    }
}

async fn ensure_console_admin_bootstrap_key_fresh(store: &SqliteStore) -> Result<String> {
    let existing = store
        .get_app_state("console_admin_bootstrap_key")
        .await
        .unwrap_or_default();
    let created_unix = store
        .get_app_state("console_admin_bootstrap_key_created_unix")
        .await
        .unwrap_or_default()
        .parse::<u64>()
        .unwrap_or(0);
    let now = now_unix();

    if !existing.is_empty() {
        if created_unix == 0 {
            store
                .set_app_state("console_admin_bootstrap_key_created_unix", &now.to_string())
                .await?;
            return Ok(existing);
        }
        if now.saturating_sub(created_unix) < CONSOLE_ADMIN_BOOTSTRAP_TTL_SECS {
            return Ok(existing);
        }
    }

    let previous = existing;
    let key = format!("adm_{}", uuid::Uuid::new_v4().simple());
    store
        .set_app_state("console_admin_bootstrap_key", &key)
        .await?;
    store
        .set_app_state("console_admin_previous_bootstrap_key", &previous)
        .await?;
    store
        .set_app_state("console_admin_bootstrap_key_created_unix", &now.to_string())
        .await?;
    store
        .set_app_state("console_admin_session_token_hash", "")
        .await?;
    store
        .set_app_state("console_admin_session_expires_unix", "0")
        .await?;
    Ok(key)
}

#[cfg(test)]
async fn rotate_console_admin_bootstrap_key(store: &SqliteStore) -> Result<()> {
    let previous = store
        .get_app_state("console_admin_bootstrap_key")
        .await
        .unwrap_or_default();
    let key = format!("adm_{}", uuid::Uuid::new_v4().simple());
    store
        .set_app_state("console_admin_bootstrap_key", &key)
        .await?;
    store
        .set_app_state("console_admin_previous_bootstrap_key", &previous)
        .await?;
    store
        .set_app_state(
            "console_admin_bootstrap_key_created_unix",
            &now_unix().to_string(),
        )
        .await?;
    store
        .set_app_state("console_admin_session_token_hash", "")
        .await?;
    store
        .set_app_state("console_admin_session_expires_unix", "0")
        .await?;
    Ok(())
}

async fn ensure_console_admin_bootstrap_key(store: &SqliteStore) -> Option<String> {
    let existing = store
        .get_app_state("console_admin_bootstrap_key")
        .await
        .ok()?;
    if !existing.is_empty() {
        return Some(existing);
    }
    let key = format!("adm_{}", uuid::Uuid::new_v4().simple());
    let _ = store
        .set_app_state("console_admin_bootstrap_key", &key)
        .await;
    let _ = store
        .set_app_state("console_admin_previous_bootstrap_key", "")
        .await;
    let _ = store
        .set_app_state(
            "console_admin_bootstrap_key_created_unix",
            &now_unix().to_string(),
        )
        .await;
    Some(key)
}

fn with_admin_key_query(base_url: &str, admin_key: Option<&str>) -> String {
    let Some(admin_key) = admin_key else {
        return base_url.to_string();
    };
    match Url::parse(base_url) {
        Ok(mut url) => {
            {
                let mut pairs = url.query_pairs_mut();
                pairs.clear().append_pair("admin_key", admin_key);
            }
            url.to_string()
        }
        Err(_) => base_url.to_string(),
    }
}

fn api_key_secret_account(workspace_id: &str, key_id: &str) -> String {
    format!("api_key_raw:{workspace_id}:{key_id}")
}

fn is_missing_secret_error(msg: &str) -> bool {
    msg.contains("No entry")
        || msg.contains("NoPassword")
        || msg.contains("Item not found")
        || msg.contains("The system cannot find the file specified")
}

fn set_api_key_raw_secret(workspace_id: &str, key_id: &str, raw: &str) -> Result<()> {
    let entry = keyring::Entry::new("llmhotspot", &api_key_secret_account(workspace_id, key_id))
        .map_err(|e| anyhow!("keyring error: {e}"))?;
    entry
        .set_password(raw)
        .map_err(|e| anyhow!("keyring error: {e}"))
}

fn get_api_key_raw_secret(workspace_id: &str, key_id: &str) -> Result<Option<String>> {
    let entry = keyring::Entry::new("llmhotspot", &api_key_secret_account(workspace_id, key_id))
        .map_err(|e| anyhow!("keyring error: {e}"))?;
    match entry.get_password() {
        Ok(raw) => Ok(Some(raw)),
        Err(e) => {
            let msg = e.to_string();
            if is_missing_secret_error(&msg) {
                Ok(None)
            } else {
                Err(anyhow!("keyring error: {msg}"))
            }
        }
    }
}

async fn get_api_key_raw_secret_with_legacy_migration(
    store: &SqliteStore,
    workspace_id: &str,
    key_id: &str,
) -> Result<Option<String>> {
    if let Ok(Some(raw)) = get_api_key_raw_secret(workspace_id, key_id) {
        return Ok(Some(raw));
    }

    let legacy_key = format!("key_raw:{key_id}");
    let legacy = store.get_app_state(&legacy_key).await.unwrap_or_default();
    if legacy.trim().is_empty() {
        return Ok(None);
    }
    let _ = set_api_key_raw_secret(workspace_id, key_id, &legacy);
    let _ = store.delete_app_state(&legacy_key).await;
    Ok(Some(legacy))
}

#[cfg(unix)]
fn process_kill_unix(pid: i32) -> Result<()> {
    let status = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()?;
    if !status.success() {
        return Err(anyhow!("failed to stop process {pid}"));
    }
    Ok(())
}

async fn session_command(store: &SqliteStore, cmd: SessionCommands) -> Result<Value> {
    match cmd {
        SessionCommands::Start => {
            store.set_app_state("session_active", "1").await?;
            Ok(
                json!({"ok":true,"code":"session_started","message":"session started","active":true}),
            )
        }
        SessionCommands::Stop => {
            store.set_app_state("session_active", "0").await?;
            Ok(
                json!({"ok":true,"code":"session_stopped","message":"session stopped","active":false}),
            )
        }
        SessionCommands::Status => {
            let active = store.get_app_state("session_active").await? == "1";
            Ok(json!({"active":active}))
        }
    }
}

async fn generate_cli_docs() -> Result<Value> {
    let markdown = clap_markdown::help_markdown::<Cli>();
    let path = PathBuf::from("docs/CLI_REFERENCE.md");
    fs::write(&path, markdown)?;
    Ok(json!({"generated": path.to_string_lossy()}))
}

fn parse_optional_rfc3339(raw: Option<&str>) -> Result<Option<u64>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let dt = DateTime::parse_from_rfc3339(raw)
        .with_context(|| format!("invalid RFC3339 datetime: {raw}"))?;
    Ok(Some(dt.with_timezone(&Utc).timestamp() as u64))
}

async fn ensure_local_base_url(store: &SqliteStore) -> Result<String> {
    let local = store.get_app_state("local_base_url").await?;
    if local.is_empty() {
        Ok(format!("http://{DEFAULT_LISTEN}"))
    } else {
        Ok(local)
    }
}

async fn ensure_default_api_key_exists(store: &SqliteStore) -> Result<()> {
    let keys = store.list_api_keys().await?;
    if !keys.is_empty() {
        return Ok(());
    }
    let now = now_unix();
    let raw_key = format!("hs_{}", uuid::Uuid::new_v4().simple());
    let key = ApiKeyRecord {
        key_id: uuid::Uuid::new_v4().to_string(),
        label: "default".to_string(),
        key_hash: hash_key(&raw_key),
        tool_choice_policy: "auto".to_string(),
        created_unix: now,
        expires_unix: None,
        revoked_unix: None,
        last_used_unix: None,
    };
    store.create_api_key(&key).await?;
    store
        .set_app_state(&format!("key_raw:{}", key.key_id), &raw_key)
        .await?;
    Ok(())
}

async fn exchange_with_state_record(
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> Result<llm_auth::TokenBundle> {
    let config = llm_auth::OpenAiOAuthConfig::default();
    let response = reqwest::Client::new()
        .post(&config.token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", config.client_id.as_str()),
            ("code", code),
            ("code_verifier", code_verifier),
            ("redirect_uri", redirect_uri),
        ])
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "token exchange failed: status={} body={}",
            response.status().as_u16(),
            response.text().await.unwrap_or_default()
        ));
    }
    let token: serde_json::Value = response.json().await?;
    let access_token = token
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing access_token"))?
        .to_string();
    let refresh_token = token
        .get("refresh_token")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing refresh_token"))?
        .to_string();
    let expires_in = token
        .get("expires_in")
        .and_then(Value::as_u64)
        .unwrap_or(3600);
    Ok(llm_auth::TokenBundle {
        account_id: llm_auth::openai_chatgpt_account_id(&access_token, &config.jwt_claim_path),
        email: llm_auth::openai_email_from_access_token(&access_token),
        expires_at_unix: now_unix().saturating_add(expires_in.saturating_sub(60)),
        expires_in,
        access_token,
        refresh_token,
    })
}

async fn resolve_remote_assignment(store: &SqliteStore) -> RemoteAssignment {
    let mut fallback_base = std::env::var("LLMHOTSPOT_REMOTE_BASE_URL").ok();
    let mut fallback_console = std::env::var("LLMHOTSPOT_REMOTE_CONSOLE_URL").ok();
    if fallback_base.is_none() || fallback_console.is_none() {
        if let Ok(Some(file_assignment)) = read_main_app_remote_control_assignment() {
            if fallback_base.is_none() {
                fallback_base = file_assignment.remote_base_url;
            }
            if fallback_console.is_none() {
                fallback_console = file_assignment.remote_console_url;
            }
        }
    }

    // In tunnel-runtime mode, TunnelManager owns the remote assignment.
    // Return whatever is stored — including trycloudflare URLs — so the
    // refresh loop never overwrites a live tunnel URL with null.
    if env_truthy("LLMHOTSPOT_TUNNEL_RUNTIME") {
        if let Ok(current) = store.get_remote_assignment().await {
            if current.remote_base_url.is_some() || current.remote_console_url.is_some() {
                return current;
            }
        }
    }
    // In tunnel-runtime mode we want to avoid depending on an external website.
    // TunnelManager will provision via the broker and then populate remote assignment locally.
    if !env_truthy("LLMHOTSPOT_DISABLE_WEBSITE_ASSIGNMENT")
        && !env_truthy("LLMHOTSPOT_TUNNEL_RUNTIME")
    {
        let website_base = std::env::var("LLMHOTSPOT_WEBSITE_BASE_URL")
            .ok()
            .or_else(|| std::env::var("APP_BASE_URL").ok());
        let website_license_key = std::env::var("LLMHOTSPOT_LICENSE_KEY").ok();
        if let (Some(base_url), Some(license_key)) = (website_base, website_license_key) {
            let device_id = resolve_device_id(store).await;
            let app_session_token = store.get_app_state("app_auth_session_token").await.ok();
            match fetch_website_remote_assignment(
                &base_url,
                &license_key,
                &device_id,
                app_session_token.as_deref(),
            )
            .await
            {
                Ok(Some(assignment)) => {
                    let _ =
                        persist_remote_assignment_metadata(store, &assignment, None, None).await;
                    return assignment;
                }
                Ok(None) => {}
                Err(err) => {
                    let _ = persist_remote_assignment_metadata(
                        store,
                        &RemoteAssignment {
                            remote_base_url: fallback_base.clone(),
                            remote_console_url: fallback_console.clone(),
                        },
                        Some(format!("website assignment failed: {err}")),
                        None,
                    )
                    .await;
                }
            }
        }
    }

    let Some(control_plane_url) = std::env::var("LLMHOTSPOT_CONTROL_PLANE_URL").ok() else {
        let assignment = RemoteAssignment {
            remote_base_url: fallback_base,
            remote_console_url: fallback_console,
        };
        let _ = persist_remote_assignment_metadata(store, &assignment, None, None).await;
        return assignment;
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("reqwest client");
    let mut attempt = 0_u32;
    let max_attempts = 3_u32;
    let mut last_error = String::new();
    while attempt < max_attempts {
        attempt += 1;
        let mut request = client
            .get(format!(
                "{}/assignment",
                control_plane_url.trim_end_matches('/')
            ))
            .header("accept", "application/json");
        if let Ok(node_id) = std::env::var("LLMHOTSPOT_NODE_ID") {
            request = request.query(&[("node_id", node_id)]);
        }
        if let Ok(token) = std::env::var("LLMHOTSPOT_CONTROL_PLANE_TOKEN") {
            request = request.bearer_auth(token);
        }

        match request.send().await {
            Ok(response) if response.status().is_success() => {
                if let Ok(payload) = response.json::<Value>().await {
                    match validate_assignment_payload(payload) {
                        Ok(valid) => {
                            let assignment = RemoteAssignment {
                                remote_base_url: valid.0.or_else(|| fallback_base.clone()),
                                remote_console_url: valid.1.or_else(|| fallback_console.clone()),
                            };
                            let _ = persist_remote_assignment_metadata(
                                store,
                                &assignment,
                                None,
                                valid.2,
                            )
                            .await;
                            return assignment;
                        }
                        Err(err) => last_error = err.to_string(),
                    }
                } else {
                    last_error = "control-plane returned invalid json".to_string();
                }
            }
            Ok(response) => {
                last_error = format!("control-plane status={}", response.status().as_u16());
            }
            Err(err) => {
                last_error = format!("control-plane request failed: {err}");
            }
        }
        sleep(Duration::from_millis(150 * (1_u64 << attempt.min(3)))).await;
    }

    let assignment = RemoteAssignment {
        remote_base_url: fallback_base,
        remote_console_url: fallback_console,
    };
    let _ = persist_remote_assignment_metadata(
        store,
        &assignment,
        Some(if last_error.is_empty() {
            "control-plane assignment unavailable".to_string()
        } else {
            last_error
        }),
        None,
    )
    .await;
    assignment
}

fn read_main_app_remote_control_assignment() -> Result<Option<RemoteAssignment>> {
    for path in main_app_remote_control_candidates() {
        if !path.exists() {
            continue;
        }
        match read_remote_control_file(&path) {
            Ok(Some(assignment)) => return Ok(Some(assignment)),
            Ok(None) => continue,
            Err(_) => continue,
        }
    }
    Ok(None)
}

fn main_app_remote_control_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") else {
        return out;
    };
    let base = PathBuf::from(local_app_data);
    for dir in [
        "com.llm.hotspot",
        "com.oberon.hotspot",
        "com.oberon.daemon",
        "com.oberon.desktop",
    ] {
        out.push(base.join(dir).join("remote-control.json"));
    }
    out
}

fn read_remote_control_file(path: &PathBuf) -> Result<Option<RemoteAssignment>> {
    let raw = fs::read_to_string(path)?;
    let payload: Value = serde_json::from_str(&raw)?;
    let enabled = payload
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !enabled {
        return Ok(None);
    }
    let Some(external_base) = payload
        .get("external_base_url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return Ok(None);
    };
    // Use `/remote` instead of `/console` so the remote console always serves the device runtime UI
    // (and avoids any legacy UI that might exist at `/console` on the same origin).
    let remote_console = format!("{}/remote", external_base.trim_end_matches('/'));
    let (base, console, _) = validate_assignment_payload(json!({
        "remote_base_url": external_base,
        "remote_console_url": remote_console,
    }))?;
    Ok(Some(RemoteAssignment {
        remote_base_url: base,
        remote_console_url: console,
    }))
}

async fn fetch_website_remote_assignment(
    website_base_url: &str,
    license_key: &str,
    device_id: &str,
    app_session_token: Option<&str>,
) -> Result<Option<RemoteAssignment>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("reqwest client");
    let details_payload = website_post_json(
        &client,
        website_base_url,
        "/api/endpoints/details",
        license_key,
        device_id,
        None,
        app_session_token,
    )
    .await?;
    if let Some(assignment) = website_assignment_from_details_payload(details_payload.clone())? {
        return Ok(Some(assignment));
    }

    let base_slug = std::env::var("LLMHOTSPOT_ENDPOINT_SLUG")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default_endpoint_slug(license_key, device_id));
    let mut attempted = Vec::<String>::new();
    for suffix in ["", "-1", "-2"] {
        let mut slug = format!("{base_slug}{suffix}");
        if slug.len() > 32 {
            slug.truncate(32);
            while slug.ends_with('-') {
                slug.pop();
            }
        }
        if slug.len() < 3 {
            continue;
        }
        if attempted.iter().any(|s| s == &slug) {
            continue;
        }
        attempted.push(slug.clone());

        let register_payload = website_post_json(
            &client,
            website_base_url,
            "/api/endpoints/register",
            license_key,
            device_id,
            Some(&slug),
            app_session_token,
        )
        .await?;

        let ok = register_payload
            .get("ok")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if ok {
            return website_assignment_from_details_payload(register_payload);
        }

        let reason = register_payload
            .get("reason")
            .or_else(|| register_payload.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("register_failed");
        if reason == "endpoint_exists" {
            let retry = website_post_json(
                &client,
                website_base_url,
                "/api/endpoints/details",
                license_key,
                device_id,
                None,
                app_session_token,
            )
            .await?;
            return website_assignment_from_details_payload(retry);
        }
        if reason != "slug_taken" {
            return Err(anyhow!("website endpoint-register failed: {reason}"));
        }
    }

    Err(anyhow!(
        "website endpoint-register failed: slug_taken after {} attempts",
        attempted.len()
    ))
}

fn website_assignment_from_details_payload(payload: Value) -> Result<Option<RemoteAssignment>> {
    let ok = payload.get("ok").and_then(Value::as_bool).unwrap_or(false);
    if !ok {
        let err = payload
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("endpoint details request failed");
        return Err(anyhow!("{err}"));
    }
    let Some(endpoint) = payload.get("endpoint") else {
        return Err(anyhow!("endpoint details missing endpoint field"));
    };
    if endpoint.is_null() {
        return Ok(None);
    }

    let base = endpoint
        .get("publicBaseUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("endpoint details missing endpoint.publicBaseUrl"))?
        .trim_end_matches('/')
        .to_string();
    // Always prefer the device-served console at `/remote`.
    // Some deployments also serve a legacy web UI at `/console`, which would bypass the
    // device runtime's admin-cookie exchange and show the wrong interface.
    let console = Some(format!("{base}/remote"));
    // Reuse existing validation rules for remote endpoint + optional console URL.
    validate_assignment_payload(json!({
        "remote_base_url": base,
        "remote_console_url": console,
    }))?;

    Ok(Some(RemoteAssignment {
        remote_base_url: Some(base),
        remote_console_url: console,
    }))
}

async fn website_post_json(
    client: &reqwest::Client,
    website_base_url: &str,
    path: &str,
    license_key: &str,
    device_id: &str,
    slug: Option<&str>,
    app_session_token: Option<&str>,
) -> Result<Value> {
    let url = format!(
        "{}/{}",
        website_base_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    );
    let mut body = json!({
        "licenseKey": license_key,
        "deviceId": device_id,
    });
    if let Some(device_name) = resolve_device_name() {
        body["device_name"] = Value::String(device_name.clone());
        body["deviceName"] = Value::String(device_name);
    }
    if let Some(os_name) = resolve_os_name() {
        body["os"] = Value::String(os_name);
    }
    if let Some(slug) = slug {
        body["slug"] = Value::String(slug.to_string());
    }
    if let Some(token) = app_session_token.map(str::trim).filter(|v| !v.is_empty()) {
        body["session_token"] = Value::String(token.to_string());
        body["sessionToken"] = Value::String(token.to_string());
    }
    let response = client
        .post(url)
        .header("accept", "application/json")
        .json(&body)
        .send()
        .await?;
    let status = response.status().as_u16();
    let mut payload = response
        .json::<Value>()
        .await
        .unwrap_or_else(|_| json!({"ok": false, "error": "invalid_json"}));
    payload["_http_status"] = Value::from(status);
    Ok(payload)
}

fn resolve_device_name() -> Option<String> {
    for key in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(v) = std::env::var(key) {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn resolve_os_name() -> Option<String> {
    let raw = std::env::consts::OS.trim();
    if raw.is_empty() {
        None
    } else {
        Some(raw.to_string())
    }
}

async fn resolve_device_id(store: &SqliteStore) -> String {
    if let Some(v) = env_nonempty("LLMHOTSPOT_DEVICE_ID") {
        return v;
    }
    if let Ok(existing) = store.get_app_state("device_id").await {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    #[cfg(unix)]
    let machine_id = fs::read_to_string("/etc/machine-id")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_default();
    #[cfg(not(unix))]
    let machine_id = String::new();

    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let base = if !machine_id.is_empty() {
        let mut short = machine_id.replace('-', "");
        short.truncate(12);
        short
    } else {
        suffix.chars().take(12).collect::<String>()
    };
    let out = format!("dev-{base}-{}", suffix.chars().take(6).collect::<String>());
    let _ = store.set_app_state("device_id", &out).await;
    out
}

fn resolve_cloudflared_path() -> String {
    if let Some(v) = env_nonempty("LLMHOTSPOT_CLOUDFLARED_PATH") {
        return v;
    }

    let exe_suffix = if cfg!(windows) { ".exe" } else { "" };
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    if let Some(dir) = exe_dir {
        let sibling = dir.join(format!("cloudflared{exe_suffix}"));
        if sibling.is_file() {
            return sibling.to_string_lossy().to_string();
        }
    }

    // Dev fallback: scripts/_bin/cloudflared
    if let Ok(cwd) = std::env::current_dir() {
        let dev = cwd
            .join("scripts")
            .join("_bin")
            .join(format!("cloudflared{exe_suffix}"));
        if dev.is_file() {
            return dev.to_string_lossy().to_string();
        }
    }

    format!("cloudflared{exe_suffix}")
}

async fn app_is_pro_tier(store: &SqliteStore) -> bool {
    let tier = store
        .get_app_state("app_entitlement_tier")
        .await
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if tier == "pro" {
        let session_token = store
            .get_app_state("app_auth_session_token")
            .await
            .unwrap_or_default();
        if !session_token.trim().is_empty() {
            let expires = store
                .get_app_state("app_auth_session_expires_unix")
                .await
                .unwrap_or_default()
                .parse::<u64>()
                .unwrap_or(0);
            if expires <= now_unix() {
                return false;
            }
        }
        return true;
    }
    env_nonempty("LLMHOTSPOT_LICENSE_KEY")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
}

fn app_broker_base_url() -> Option<String> {
    env_nonempty("LLMHOTSPOT_TUNNEL_BROKER_URL")
        .or_else(|| env_nonempty("LLMHOTSPOT_WEBSITE_BASE_URL"))
        .or_else(|| env_nonempty("APP_BASE_URL"))
}

async fn downgrade_app_entitlement_to_free(store: &SqliteStore) {
    clear_managed_tunnel_cache(store).await;
    let _ = store.set_app_state("app_entitlement_tier", "free").await;
    let _ = store.set_app_state("app_auth_session_token", "").await;
    let _ = store
        .set_app_state("app_auth_session_expires_unix", "0")
        .await;
    let _ = store.set_app_state("app_entitlement_license_id", "").await;
    let _ = store.set_app_state("app_entitlement_seat_index", "0").await;
    let _ = store.set_app_state("app_provisioning_mode", "quick").await;
    let _ = store
        .set_remote_assignment(&RemoteAssignment {
            remote_base_url: None,
            remote_console_url: None,
        })
        .await;
}

async fn downgrade_app_entitlement_to_free_with_warning(store: &SqliteStore, warning: &str) {
    downgrade_app_entitlement_to_free(store).await;
    let _ = store.set_app_state(APP_BROKER_WARNING_KEY, warning).await;
}

async fn clear_managed_tunnel_cache(store: &SqliteStore) {
    let _ = store.set_app_state(APP_MANAGED_TUNNEL_ID_KEY, "").await;
    let _ = store
        .set_app_state(APP_MANAGED_TUNNEL_HOSTNAME_KEY, "")
        .await;
    let _ = store.set_app_state(APP_MANAGED_TUNNEL_TOKEN_KEY, "").await;
    let _ = store
        .set_app_state(APP_MANAGED_TUNNEL_CACHED_UNIX_KEY, "0")
        .await;
}

async fn set_broker_connectivity_healthy(store: &SqliteStore) {
    let _ = store
        .set_app_state(APP_BROKER_LAST_OK_UNIX_KEY, &now_unix().to_string())
        .await;
    let _ = store
        .set_app_state(APP_BROKER_OFFLINE_SINCE_UNIX_KEY, "0")
        .await;
    let _ = store.set_app_state(APP_BROKER_LAST_ERROR_KEY, "").await;
    let _ = store.set_app_state(APP_BROKER_WARNING_KEY, "").await;
}

async fn broker_offline_since_unix(store: &SqliteStore) -> u64 {
    store
        .get_app_state(APP_BROKER_OFFLINE_SINCE_UNIX_KEY)
        .await
        .unwrap_or_default()
        .trim()
        .parse::<u64>()
        .unwrap_or(0)
}

async fn broker_offline_elapsed_secs(store: &SqliteStore) -> u64 {
    let offline_since = broker_offline_since_unix(store).await;
    if offline_since == 0 {
        return 0;
    }
    now_unix().saturating_sub(offline_since)
}

async fn broker_outage_grace_expired(store: &SqliteStore) -> bool {
    broker_offline_elapsed_secs(store).await >= BROKER_OFFLINE_ENFORCEMENT_SECS
}

async fn set_broker_connectivity_unavailable(store: &SqliteStore, err: &str) -> u64 {
    let now = now_unix();
    let mut offline_since = broker_offline_since_unix(store).await;
    if offline_since == 0 {
        offline_since = now;
        let _ = store
            .set_app_state(
                APP_BROKER_OFFLINE_SINCE_UNIX_KEY,
                &offline_since.to_string(),
            )
            .await;
    }
    let elapsed = now.saturating_sub(offline_since);
    let remaining = BROKER_OFFLINE_ENFORCEMENT_SECS.saturating_sub(elapsed);
    let warning = format!(
        "Broker offline for {}s; auto-free in {}s if connectivity is not restored.",
        elapsed, remaining
    );
    let _ = store.set_app_state(APP_BROKER_LAST_ERROR_KEY, err).await;
    let _ = store.set_app_state(APP_BROKER_WARNING_KEY, &warning).await;
    elapsed
}

async fn managed_tunnel_cache(store: &SqliteStore) -> Option<(String, String, String)> {
    let tunnel_id = store
        .get_app_state(APP_MANAGED_TUNNEL_ID_KEY)
        .await
        .unwrap_or_default()
        .trim()
        .to_string();
    let hostname = store
        .get_app_state(APP_MANAGED_TUNNEL_HOSTNAME_KEY)
        .await
        .unwrap_or_default()
        .trim()
        .to_string();
    let token = store
        .get_app_state(APP_MANAGED_TUNNEL_TOKEN_KEY)
        .await
        .unwrap_or_default()
        .trim()
        .to_string();
    if tunnel_id.is_empty() || hostname.is_empty() || token.is_empty() {
        return None;
    }
    Some((tunnel_id, hostname, token))
}

#[derive(Debug)]
enum SessionSyncOutcome {
    ValidatedPro(Value),
    ValidatedFree,
    ValidationUnavailable(String),
    ValidationUnauthorized(String),
}

async fn sync_app_session_entitlement(store: &SqliteStore) -> bool {
    let token = store
        .get_app_state("app_auth_session_token")
        .await
        .unwrap_or_default();
    if token.trim().is_empty() {
        return false;
    }
    let expires = store
        .get_app_state("app_auth_session_expires_unix")
        .await
        .unwrap_or_default()
        .parse::<u64>()
        .unwrap_or(0);
    if expires <= now_unix() {
        downgrade_app_entitlement_to_free_with_warning(
            store,
            "Pro session expired. Sign in again to restore Pro features.",
        )
        .await;
        return true;
    }
    let Some(base) = app_broker_base_url() else {
        let elapsed =
            set_broker_connectivity_unavailable(store, "broker base url not configured").await;
        if elapsed >= BROKER_OFFLINE_ENFORCEMENT_SECS {
            downgrade_app_entitlement_to_free_with_warning(
                store,
                "Pro expired due 24h broker connectivity loss.",
            )
            .await;
            return true;
        }
        return false;
    };
    let device_id = resolve_device_id(store).await;
    let url = format!("{}/api/app/session/status", base.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .post(url)
        .header("content-type", "application/json")
        .json(&json!({
            "session_token": token,
            "device_id": device_id
        }))
        .send()
        .await;
    let outcome = match resp {
        Ok(resp) => {
            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                if status == 401 || status == 403 {
                    SessionSyncOutcome::ValidationUnauthorized(format!(
                        "Broker session rejected (HTTP {}).",
                        status
                    ))
                } else {
                    SessionSyncOutcome::ValidationUnavailable(format!(
                        "Broker session status failed (HTTP {}).",
                        status
                    ))
                }
            } else {
                let payload: Value = resp.json().await.unwrap_or_else(|_| json!({}));
                let tier = payload
                    .get("tier")
                    .and_then(Value::as_str)
                    .unwrap_or("free")
                    .to_ascii_lowercase();
                if tier == "pro" {
                    SessionSyncOutcome::ValidatedPro(payload)
                } else {
                    SessionSyncOutcome::ValidatedFree
                }
            }
        }
        Err(err) => SessionSyncOutcome::ValidationUnavailable(format!(
            "Broker session validation transport error: {}",
            err
        )),
    };

    match outcome {
        SessionSyncOutcome::ValidatedPro(payload) => {
            set_broker_connectivity_healthy(store).await;
            let mut changed = false;
            let current_tier = store
                .get_app_state("app_entitlement_tier")
                .await
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if current_tier != "pro" {
                let _ = store.set_app_state("app_entitlement_tier", "pro").await;
                changed = true;
            }
            if let Some(v) = payload.get("licenseId").and_then(Value::as_str) {
                let _ = store.set_app_state("app_entitlement_license_id", v).await;
            }
            if let Some(v) = payload.get("seatIndex").and_then(Value::as_i64) {
                let _ = store
                    .set_app_state("app_entitlement_seat_index", &v.to_string())
                    .await;
            }
            changed
        }
        SessionSyncOutcome::ValidatedFree => {
            downgrade_app_entitlement_to_free_with_warning(
                store,
                "Pro access is no longer active on broker. Switched to Free mode.",
            )
            .await;
            true
        }
        SessionSyncOutcome::ValidationUnauthorized(reason) => {
            downgrade_app_entitlement_to_free_with_warning(store, &reason).await;
            true
        }
        SessionSyncOutcome::ValidationUnavailable(reason) => {
            let elapsed = set_broker_connectivity_unavailable(store, &reason).await;
            if elapsed >= BROKER_OFFLINE_ENFORCEMENT_SECS {
                downgrade_app_entitlement_to_free_with_warning(
                    store,
                    "Pro expired due 24h broker connectivity loss.",
                )
                .await;
                return true;
            }
            false
        }
    }
}

async fn has_valid_app_session_token(store: &SqliteStore) -> bool {
    let session_token = store
        .get_app_state("app_auth_session_token")
        .await
        .unwrap_or_default();
    if session_token.trim().is_empty() {
        return false;
    }
    let expires = store
        .get_app_state("app_auth_session_expires_unix")
        .await
        .unwrap_or_default()
        .parse::<u64>()
        .unwrap_or(0);
    expires > now_unix()
}

async fn tunnel_manager_loop(store: SqliteStore, mode_notify: std::sync::Arc<tokio::sync::Notify>) {
    let enabled = env_nonempty("LLMHOTSPOT_TUNNEL_RUNTIME")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !enabled {
        return;
    }

    // Clear any stale remote assignment from a previous run.
    let _ = store
        .set_remote_assignment(&RemoteAssignment {
            remote_base_url: None,
            remote_console_url: None,
        })
        .await;

    let cloudflared_bin = resolve_cloudflared_path();
    eprintln!("TunnelManager: cloudflared={cloudflared_bin}");
    let origin_url = store
        .get_app_state("local_base_url")
        .await
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:11434".to_string());

    // Optional broker URL — broker mode requires this to be set.
    let broker_base_url = env_nonempty("LLMHOTSPOT_TUNNEL_BROKER_URL")
        .or_else(|| env_nonempty("LLMHOTSPOT_WEBSITE_BASE_URL"))
        .or_else(|| env_nonempty("APP_BASE_URL"));
    // Broker mode can ignore license keys; keep field for backwards compatibility.
    let license_key =
        env_nonempty("LLMHOTSPOT_LICENSE_KEY").unwrap_or_else(|| "unused".to_string());
    let device_id = resolve_device_id(&store).await;
    let endpoint_slug = env_nonempty("LLMHOTSPOT_ENDPOINT_SLUG");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("reqwest client");

    let mut backoff_secs = 2_u64;
    loop {
        let pro_tier = app_is_pro_tier(&store).await;
        let has_broker_session = has_valid_app_session_token(&store).await;
        let has_license_key = {
            let lk = license_key.trim();
            !lk.is_empty() && lk != "unused"
        };
        let can_auth_broker = has_broker_session || has_license_key;
        if broker_base_url.is_some() && pro_tier && !can_auth_broker {
            let _ = store
                .set_app_state(
                    APP_BROKER_WARNING_KEY,
                    "Pro is active, but broker auth is missing. Reconnect Manage Subscription (or set LLMHOTSPOT_LICENSE_KEY) to restore managed tunnel.",
                )
                .await;
        }
        let use_broker = broker_base_url.is_some() && pro_tier && can_auth_broker;

        if !use_broker {
            let _ = store.set_app_state("app_provisioning_mode", "quick").await;
            let ok =
                run_trycloudflare_once(&store, &cloudflared_bin, &origin_url, &mode_notify).await;
            if ok {
                backoff_secs = 2;
            } else {
                // Interruptible backoff — wake immediately if mode changes.
                tokio::select! {
                    _ = sleep(Duration::from_secs(backoff_secs)) => {}
                    _ = mode_notify.notified() => {}
                }
                backoff_secs = (backoff_secs.saturating_mul(2)).min(60);
            }
            continue;
        }

        // Wrap the entire provision + session block in a labeled block so that any
        // failure can break out to the single fallback handler below.
        let broker_url = broker_base_url.as_deref().unwrap_or_default();
        let mut provision_failed = false;
        let _ = store
            .set_app_state("app_provisioning_mode", "managed")
            .await;

        // Fast-path: if we already have a managed tunnel assignment cached, reuse it.
        // This avoids unnecessary reprovision calls on every app start.
        if let Some((cached_tunnel_id, cached_hostname, cached_token)) =
            managed_tunnel_cache(&store).await
        {
            let assignment = RemoteAssignment {
                remote_base_url: Some(format!("https://{}", cached_hostname.trim())),
                remote_console_url: Some(format!("https://{}/remote", cached_hostname.trim())),
            };
            let _ = store.set_remote_assignment(&assignment).await;
            let _ = persist_remote_assignment_metadata(&store, &assignment, None, None).await;
            eprintln!("TunnelManager: reusing cached managed tunnel: {cached_hostname}");
            let app_session_token = store.get_app_state("app_auth_session_token").await.ok();
            let reprovision = run_cloudflared_session(
                &client,
                broker_url,
                &license_key,
                &device_id,
                app_session_token.as_deref(),
                &cloudflared_bin,
                &cached_tunnel_id,
                &cached_hostname,
                &origin_url,
                &cached_token,
                &store,
                mode_notify.as_ref(),
            )
            .await;
            if !reprovision {
                continue;
            }
            eprintln!(
                "TunnelManager: cached managed tunnel requires reprovision; requesting new lease"
            );
            clear_managed_tunnel_cache(&store).await;
        }

        'provision: {
            let app_session_token = store.get_app_state("app_auth_session_token").await.ok();
            let payload = match website_post_json(
                &client,
                broker_url,
                "/api/tunnels/provision",
                &license_key,
                &device_id,
                endpoint_slug.as_deref(),
                app_session_token.as_deref(),
            )
            .await
            {
                Ok(p) => p,
                Err(err) => {
                    eprintln!("TunnelManager: provision request error: {err:#}");
                    backoff_secs = (backoff_secs.saturating_mul(2)).min(60);
                    provision_failed = true;
                    break 'provision;
                }
            };

            let ok = payload.get("ok").and_then(Value::as_bool).unwrap_or(false);
            if !ok {
                let status = payload
                    .get("_http_status")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let reason = payload
                    .get("reason")
                    .or_else(|| payload.get("error"))
                    .and_then(Value::as_str)
                    .unwrap_or("provision_failed");
                eprintln!("TunnelManager: provision failed status={status} reason={reason}");
                backoff_secs = (backoff_secs.saturating_mul(2)).min(60);
                provision_failed = true;
                break 'provision;
            }

            let token = payload
                .pointer("/tunnel/tunnelToken")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| v.to_string());
            let hostname = payload
                .pointer("/tunnel/hostname")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let tunnel_id = payload
                .pointer("/tunnel/tunnelId")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| v.to_string());

            let Some(token) = token else {
                eprintln!("TunnelManager: provision missing tunnelToken");
                backoff_secs = (backoff_secs.saturating_mul(2)).min(60);
                provision_failed = true;
                break 'provision;
            };
            let Some(tunnel_id) = tunnel_id else {
                eprintln!("TunnelManager: provision missing tunnelId");
                backoff_secs = (backoff_secs.saturating_mul(2)).min(60);
                provision_failed = true;
                break 'provision;
            };

            // Safety: a broker should return a stable hostname under our domain, not a
            // temporary `trycloudflare.com` quick-tunnel URL.
            if hostname.to_ascii_lowercase().contains("trycloudflare.com") {
                eprintln!("TunnelManager: provision returned trycloudflare hostname={hostname}");
                let _ = persist_remote_assignment_metadata(
                    &store,
                    &RemoteAssignment { remote_base_url: None, remote_console_url: None },
                    Some("tunnel provision returned trycloudflare hostname; check broker routing/config".to_string()),
                    None,
                )
                .await;
                backoff_secs = (backoff_secs.saturating_mul(2)).min(60);
                provision_failed = true;
                break 'provision;
            }

            if !hostname.trim().is_empty() {
                let assignment = RemoteAssignment {
                    remote_base_url: Some(format!("https://{}", hostname.trim())),
                    // Keep admin bootstrap keys out of stored remote URLs. The local console UI
                    // can append the key from sessionStorage when presenting a copyable link.
                    remote_console_url: Some(format!("https://{}/remote", hostname.trim())),
                };
                let _ = store.set_remote_assignment(&assignment).await;
                let _ = persist_remote_assignment_metadata(&store, &assignment, None, None).await;
            }
            let _ = store
                .set_app_state(APP_MANAGED_TUNNEL_ID_KEY, &tunnel_id)
                .await;
            let _ = store
                .set_app_state(APP_MANAGED_TUNNEL_HOSTNAME_KEY, &hostname)
                .await;
            let _ = store
                .set_app_state(APP_MANAGED_TUNNEL_TOKEN_KEY, &token)
                .await;
            let _ = store
                .set_app_state(APP_MANAGED_TUNNEL_CACHED_UNIX_KEY, &now_unix().to_string())
                .await;

            eprintln!("TunnelManager: starting cloudflared for {hostname}");
            backoff_secs = 2;

            let reprovision = run_cloudflared_session(
                &client,
                broker_url,
                &license_key,
                &device_id,
                app_session_token.as_deref(),
                &cloudflared_bin,
                &tunnel_id,
                &hostname,
                &origin_url,
                &token,
                &store,
                mode_notify.as_ref(),
            )
            .await;

            if !reprovision {
                // Normal exit; short delay to avoid thrash.
                sleep(Duration::from_secs(2)).await;
            }
        } // end 'provision

        if provision_failed {
            let still_pro_grace_valid =
                app_is_pro_tier(&store).await && !broker_outage_grace_expired(&store).await;
            if still_pro_grace_valid {
                if let Some((cached_tunnel_id, cached_hostname, cached_token)) =
                    managed_tunnel_cache(&store).await
                {
                    eprintln!(
                        "TunnelManager: using cached managed tunnel while broker is unavailable: {cached_hostname}"
                    );
                    let app_session_token =
                        store.get_app_state("app_auth_session_token").await.ok();
                    let reprovision = run_cloudflared_session(
                        &client,
                        broker_url,
                        &license_key,
                        &device_id,
                        app_session_token.as_deref(),
                        &cloudflared_bin,
                        &cached_tunnel_id,
                        &cached_hostname,
                        &origin_url,
                        &cached_token,
                        &store,
                        mode_notify.as_ref(),
                    )
                    .await;
                    if !reprovision {
                        sleep(Duration::from_secs(2)).await;
                    }
                    continue;
                }
            }
            // Broker is unavailable or returned an error — provide the free tunnel as
            // a fallback so the user always has a working remote endpoint.  The outer
            // loop will retry the broker once the free session ends.
            eprintln!("TunnelManager: broker unavailable; running free tunnel as fallback (will retry broker when session ends)");
            let _ = store.set_app_state("app_provisioning_mode", "quick").await;
            let tc_ok =
                run_trycloudflare_once(&store, &cloudflared_bin, &origin_url, &mode_notify).await;
            if !tc_ok {
                // trycloudflare also failed — wait before hammering either service.
                tokio::select! {
                    _ = sleep(Duration::from_secs(backoff_secs)) => {}
                    _ = mode_notify.notified() => {}
                }
            } else {
                backoff_secs = 2;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)] // Session launch needs explicit tunnel/runtime inputs to avoid hidden global state.
async fn run_cloudflared_session(
    client: &reqwest::Client,
    website_base_url: &str,
    license_key: &str,
    device_id: &str,
    app_session_token: Option<&str>,
    cloudflared_bin: &str,
    tunnel_id: &str,
    hostname: &str,
    origin_url: &str,
    tunnel_token: &str,
    store: &SqliteStore,
    mode_notify: &tokio::sync::Notify,
) -> bool {
    // Write config + token to a temp dir to avoid leaking token via process args.
    let mut state_dir = std::env::temp_dir();
    state_dir.push("llmhotspot");
    state_dir.push("tunnels");
    let _ = fs::create_dir_all(&state_dir);

    let safe_name = hostname.replace('.', "_");
    let config_path = state_dir.join(format!("cf-{safe_name}.yml"));
    let token_path = state_dir.join(format!("cf-{safe_name}.token"));

    // Best-effort file permissions; non-fatal on platforms that don't support unix perms.
    let _ = fs::write(&token_path, tunnel_token.as_bytes());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600));
    }

    let config = format!(
        "tunnel: {tunnel_id}\n\ningress:\n  - hostname: {hostname}\n    service: {origin_url}\n  - service: http_status:404\n"
    );
    let _ = fs::write(&config_path, config.as_bytes());

    let mut child = match TokioCommand::new(cloudflared_bin)
        .arg("tunnel")
        .arg("--no-autoupdate")
        .arg("--config")
        .arg(config_path)
        .arg("run")
        .arg("--token-file")
        .arg(token_path)
        .arg(tunnel_id)
        .spawn()
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("TunnelManager: failed to spawn cloudflared ({cloudflared_bin}): {err}");
            sleep(Duration::from_secs(10)).await;
            return true;
        }
    };

    let mut heartbeat_tick = tokio::time::interval(Duration::from_secs(15));
    loop {
        tokio::select! {
            _ = mode_notify.notified() => {
                // Entitlement/mode changed; if no longer pro, stop managed tunnel immediately.
                if !app_is_pro_tier(store).await {
                    eprintln!("TunnelManager: entitlement switched to free; stopping managed tunnel");
                    let _ = child.kill().await;
                    return false;
                }
            }
            _ = heartbeat_tick.tick() => {
                match website_post_json(
                    client,
                    website_base_url,
                    "/api/tunnels/heartbeat",
                    license_key,
                    device_id,
                    None,
                    app_session_token,
                ).await {
                    Ok(payload) => {
                        let ok = payload.get("ok").and_then(Value::as_bool).unwrap_or(false);
                        if ok {
                            continue;
                        }
                        let status = payload.get("_http_status").and_then(Value::as_u64).unwrap_or(0);
                        let err = payload.get("error").or_else(|| payload.get("reason")).and_then(Value::as_str).unwrap_or("heartbeat_failed");
                        eprintln!("TunnelManager: heartbeat failed status={status} error={err}");
                        if status == 404 && err == "lease_not_found" {
                            let _ = child.kill().await;
                            return true;
                        }
                        if status == 403 && err == "device_mismatch" {
                            let _ = child.kill().await;
                            return true;
                        }
                        if status == 401
                            || err.eq_ignore_ascii_case("license_key_required")
                            || err.eq_ignore_ascii_case("license_revoked")
                            || err.eq_ignore_ascii_case("session_invalid")
                            || err.eq_ignore_ascii_case("unauthorized")
                        {
                            let _ = child.kill().await;
                            return false;
                        }
                    }
                    Err(err) => {
                        eprintln!("TunnelManager: heartbeat request error: {err:#}");
                    }
                }
            }
            status = child.wait() => {
                match status {
                    Ok(code) => {
                        eprintln!("TunnelManager: cloudflared exited: {code}");
                    }
                    Err(err) => {
                        eprintln!("TunnelManager: cloudflared wait error: {err}");
                    }
                }
                return true;
            }
        }
    }
}

/// Spawn cloudflared in quick-tunnel mode (`--url`) for a single session.
/// Returns `true` on a clean run (URL was obtained), `false` on spawn/parse failure.
/// If entitlement switches to Pro while the tunnel is live, cloudflared is
/// killed immediately via `mode_notify` so the outer loop can hand off without delay.
async fn run_trycloudflare_once(
    store: &SqliteStore,
    cloudflared_bin: &str,
    origin_url: &str,
    mode_notify: &tokio::sync::Notify,
) -> bool {
    use tokio::io::{AsyncBufReadExt, BufReader};

    eprintln!("TunnelManager[trycloudflare]: spawning cloudflared --url {origin_url}");

    let mut child = match TokioCommand::new(cloudflared_bin)
        .arg("tunnel")
        .arg("--no-autoupdate")
        .arg("--url")
        .arg(origin_url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!(
                "TunnelManager[trycloudflare]: failed to spawn cloudflared ({cloudflared_bin}): {err}"
            );
            return false;
        }
    };

    let stderr = child.stderr.take().expect("stderr piped");
    let mut lines = BufReader::new(stderr).lines();

    // Read stderr until we spot the trycloudflare URL. Fail fast on known
    // quick-tunnel throttle/parse errors instead of waiting the full timeout.
    let found_url = tokio::time::timeout(Duration::from_secs(20), async {
        while let Ok(Some(l)) = lines.next_line().await {
            eprintln!("cloudflared: {l}");
            if let Some(url) = extract_trycloudflare_url(&l) {
                return Ok(url);
            }
            let low = l.to_ascii_lowercase();
            if low.contains("status_code=\"429")
                || low.contains("too many requests")
                || low.contains("error code: 1015")
            {
                return Err("quick tunnel rate-limited (cloudflare 429/1015)".to_string());
            }
            if low.contains("failed to parse quick tunnel id")
                || low.contains("failed to unmarshal quick tunnel")
            {
                return Err("quick tunnel handshake failed".to_string());
            }
        }
        Err("quick tunnel ended before url was assigned".to_string())
    })
    .await;

    let url = match found_url {
        Ok(Ok(url)) => url,
        Ok(Err(reason)) => {
            eprintln!("TunnelManager[trycloudflare]: {reason}; killing cloudflared");
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = persist_remote_assignment_metadata(
                store,
                &RemoteAssignment {
                    remote_base_url: None,
                    remote_console_url: None,
                },
                Some(reason),
                None,
            )
            .await;
            return false;
        }
        Err(_) => {
            // Never got a URL — kill cloudflared and signal failure to caller.
            eprintln!(
                "TunnelManager[trycloudflare]: did not find tunnel URL within 20 s; killing cloudflared"
            );
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = persist_remote_assignment_metadata(
                store,
                &RemoteAssignment {
                    remote_base_url: None,
                    remote_console_url: None,
                },
                Some("quick tunnel url assignment timed out".to_string()),
                None,
            )
            .await;
            return false;
        }
    };

    let assignment = RemoteAssignment {
        remote_base_url: Some(url.clone()),
        remote_console_url: Some(format!("{url}/remote")),
    };
    let _ = store.set_remote_assignment(&assignment).await;
    let _ = persist_remote_assignment_metadata(store, &assignment, None, None).await;
    eprintln!("TunnelManager[trycloudflare]: tunnel URL assigned → {url}");

    // Tunnel is live. Wait for cloudflared to exit naturally, but also react
    // immediately when mode_notify fires (triggered by the /v1/tunnel-mode API).
    loop {
        tokio::select! {
            _ = mode_notify.notified() => {
                if app_is_pro_tier(store).await {
                    eprintln!("TunnelManager[trycloudflare]: entitlement switched to pro; killing cloudflared");
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    break;
                }
                // Notify fired but entitlement is still free — keep running.
            }
            status = child.wait() => {
                match status {
                    Ok(code) => eprintln!("TunnelManager[trycloudflare]: cloudflared exited: {code}"),
                    Err(e) => eprintln!("TunnelManager[trycloudflare]: cloudflared wait error: {e}"),
                }
                break;
            }
        }
    }

    // Keep the last known endpoint visible while reconnecting to avoid endpoint flicker.
    let current = store
        .get_remote_assignment()
        .await
        .unwrap_or(RemoteAssignment {
            remote_base_url: None,
            remote_console_url: None,
        });
    let _ = persist_remote_assignment_metadata(
        store,
        &current,
        Some("trycloudflare session ended; reconnecting".to_string()),
        None,
    )
    .await;

    true
}

/// Extract a `https://....trycloudflare.com` URL from a cloudflared log line.
/// cloudflared emits lines like:
///   INF |  https://abc-def-ghi.trycloudflare.com  |
///   {"level":"info","url":"https://abc.trycloudflare.com",...}
fn extract_trycloudflare_url(line: &str) -> Option<String> {
    let marker = "trycloudflare.com";
    let pos = line.find(marker)?;
    let slice = &line[..pos + marker.len()];
    let start = slice.rfind("https://")?;
    let url = &slice[start..];
    let end = url
        .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '|' | '>' | ','))
        .unwrap_or(url.len());
    let url = &url[..end];
    if url.len() > "https://x.trycloudflare.com".len() {
        Some(url.to_string())
    } else {
        None
    }
}

fn default_endpoint_slug(_license_key: &str, _device_id: &str) -> String {
    const ADJECTIVES: &[&str] = &[
        "amber", "brisk", "calm", "daring", "ember", "frost", "gold", "hollow", "ivory", "jolly",
        "keen", "lucky", "mellow", "noble", "opal", "proud", "quiet", "rapid", "solar", "tidal",
        "urban", "vivid", "wild", "young", "zesty",
    ];
    const NOUNS: &[&str] = &[
        "anchor", "beacon", "comet", "delta", "ember", "forest", "glade", "harbor", "island",
        "jungle", "kernel", "lagoon", "meadow", "nebula", "oasis", "prairie", "quartz", "river",
        "summit", "thunder", "uplink", "valley", "willow", "yonder", "zenith",
    ];

    let mut rng = rand::thread_rng();
    let first = ADJECTIVES.choose(&mut rng).copied().unwrap_or("amber");
    let second = NOUNS.choose(&mut rng).copied().unwrap_or("river");
    let third = NOUNS.choose(&mut rng).copied().unwrap_or("summit");
    format!("{first}-{second}-{third}")
}

fn validate_assignment_payload(
    payload: Value,
) -> Result<(Option<String>, Option<String>, Option<u64>)> {
    let data = payload.get("data").cloned().unwrap_or(payload);
    let base = data
        .get("remote_base_url")
        .and_then(Value::as_str)
        .map(str::to_string);
    let console = data
        .get("remote_console_url")
        .and_then(Value::as_str)
        .map(str::to_string);

    if base.is_none() && console.is_none() {
        return Err(anyhow!("assignment payload missing urls"));
    }

    let allow_http = std::env::var("LLMHOTSPOT_ALLOW_HTTP_REMOTE")
        .ok()
        .map(|v| v == "1")
        .unwrap_or(false);

    if let Some(ref base_url) = base {
        validate_remote_url(base_url, allow_http)?;
    }
    if let Some(ref console_url) = console {
        validate_remote_url(console_url, allow_http)?;
    }
    if let (Some(base_url), Some(console_url)) = (&base, &console) {
        let base_parsed = Url::parse(base_url)?;
        let console_parsed = Url::parse(console_url)?;
        if base_parsed.origin() != console_parsed.origin() {
            return Err(anyhow!(
                "remote_console_url must share origin with remote_base_url"
            ));
        }
    }

    let lease_ttl_secs = data.get("lease_ttl_secs").and_then(Value::as_u64);
    let issued_at_unix = data.get("issued_at_unix").and_then(Value::as_u64);
    let lease_expires = match (lease_ttl_secs, issued_at_unix) {
        (Some(ttl), Some(issued)) => Some(issued.saturating_add(ttl)),
        _ => None,
    };

    Ok((base, console, lease_expires))
}

fn validate_remote_url(value: &str, allow_http: bool) -> Result<()> {
    let parsed = Url::parse(value)?;
    let scheme_ok = parsed.scheme() == "https" || (allow_http && parsed.scheme() == "http");
    if !scheme_ok {
        return Err(anyhow!(
            "remote url must be https unless LLMHOTSPOT_ALLOW_HTTP_REMOTE=1"
        ));
    }
    Ok(())
}

async fn persist_remote_assignment_metadata(
    store: &SqliteStore,
    assignment: &RemoteAssignment,
    error: Option<String>,
    lease_expires_unix: Option<u64>,
) -> Result<()> {
    let assignment_json = serde_json::to_string(assignment)?;
    store
        .set_app_state("remote_assignment_json", &assignment_json)
        .await?;
    store
        .set_app_state(
            "remote_assignment_last_refresh_unix",
            &now_unix().to_string(),
        )
        .await?;
    store
        .set_app_state(
            "remote_assignment_last_error",
            error.as_deref().unwrap_or(""),
        )
        .await?;
    store
        .set_app_state(
            "remote_assignment_lease_expires_unix",
            &lease_expires_unix
                .map(|v| v.to_string())
                .unwrap_or_default(),
        )
        .await?;
    Ok(())
}

async fn clear_runtime_lock(store: &SqliteStore) -> Result<()> {
    store.set_app_state(RUNTIME_LOCK_KEY, "").await?;
    store.set_app_state(RUNTIME_STARTED_KEY, "").await?;
    store.set_app_state("runtime_ipc_secret_hash", "").await?;
    Ok(())
}

async fn validate_console_admin_key_access(
    store: &SqliteStore,
    local_auth: Option<&str>,
) -> Result<()> {
    let expected_hash = store
        .get_app_state("runtime_ipc_secret_hash")
        .await
        .unwrap_or_default();
    if expected_hash.trim().is_empty() {
        return Ok(());
    }
    let provided = local_auth
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| anyhow!("local auth required for console admin key retrieval"))?;
    let provided_hash = hash_key(provided);
    if provided_hash != expected_hash {
        return Err(anyhow!("local auth failed for console admin key retrieval"));
    }
    Ok(())
}

async fn doctor_command(store: &SqliteStore) -> Result<Value> {
    let env_check = RuntimeEnvConfig::from_env()
        .map(|_| json!({"ok": true}))
        .unwrap_or_else(|e| json!({"ok": false, "error": e.to_string()}));

    let db_path = store
        .get_app_state("runtime_db_path")
        .await
        .unwrap_or_default();
    let ipc_pipe = runtime_ipc::runtime_pipe_name(&db_path);
    #[cfg(target_os = "windows")]
    let ipc_ready = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&ipc_pipe)
        .is_ok();
    #[cfg(not(target_os = "windows"))]
    let ipc_ready = false;

    let script_path = std::path::Path::new("scripts")
        .join("release")
        .join("release-windows.ps1");
    let nsis_path = std::path::Path::new(r"C:\Program Files (x86)\NSIS\makensis.exe");
    let release_ready = script_path.exists() && nsis_path.exists();

    let doctor_snapshot = json!({
        "run_unix": now_unix(),
        "source": "cli_doctor",
        "ok": env_check.get("ok").and_then(Value::as_bool).unwrap_or(false),
        "env": env_check,
        "ipc": {
            "ok": ipc_ready,
            "pipe": ipc_pipe,
            "runtime_db_path": if db_path.is_empty() { Value::Null } else { Value::String(db_path) },
            "protocol_version": runtime_ipc::RUNTIME_IPC_PROTOCOL_VERSION,
            "capabilities": runtime_ipc::RUNTIME_IPC_CAPABILITIES,
        },
        "release": {
            "ok": release_ready,
            "release_script_present": script_path.exists(),
            "makensis_present": nsis_path.exists(),
        }
    });
    if let Err(e) = store
        .set_app_state("doctor_last_run_json", &doctor_snapshot.to_string())
        .await
    {
        let _ = store
            .set_app_state("doctor_last_error", &e.to_string())
            .await;
    } else {
        let _ = store.set_app_state("doctor_last_error", "").await;
    }
    Ok(doctor_snapshot)
}

async fn add_app_state_counter(store: &SqliteStore, key: &str, delta: u64) -> Result<()> {
    if delta == 0 {
        return Ok(());
    }
    let current = store
        .get_app_state(key)
        .await
        .unwrap_or_default()
        .parse::<u64>()
        .unwrap_or(0);
    let next = current.saturating_add(delta);
    store.set_app_state(key, &next.to_string()).await?;
    Ok(())
}

async fn release_runtime_lock(store: &SqliteStore) -> Result<()> {
    clear_runtime_lock(store).await
}

async fn acquire_runtime_lock(store: &SqliteStore) -> Result<()> {
    let existing = store
        .get_app_state(RUNTIME_LOCK_KEY)
        .await
        .unwrap_or_default();
    if !existing.is_empty() {
        if let Ok(pid) = existing.parse::<i32>() {
            if process_is_alive(pid) {
                return Err(anyhow!("app already running with pid={pid}"));
            }
            clear_runtime_lock(store).await?;
        };
    }
    store
        .set_app_state(RUNTIME_LOCK_KEY, &process::id().to_string())
        .await?;
    Ok(())
}

fn process_is_alive(pid: i32) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

async fn compute_assignment_refresh_interval_secs(store: &SqliteStore) -> Result<u64> {
    let clock = SystemClock;
    compute_assignment_refresh_interval_secs_with_clock(store, &clock).await
}

async fn compute_assignment_refresh_interval_secs_with_clock(
    store: &SqliteStore,
    clock: &dyn Clock,
) -> Result<u64> {
    let force = store
        .get_app_state("remote_assignment_force_refresh")
        .await?
        == "1";
    if force {
        return Ok(1);
    }
    let lease_expires = store
        .get_app_state("remote_assignment_lease_expires_unix")
        .await?
        .parse::<u64>()
        .unwrap_or(0);
    let now = clock.now_unix();
    if lease_expires == 0 || lease_expires <= now {
        return Ok(30);
    }
    let ttl_left = lease_expires.saturating_sub(now);
    let half = ttl_left / 2;
    let base = half.clamp(10, 60);
    let jitter = (now % 5).min(4);
    Ok(base.saturating_add(jitter))
}

fn is_lease_stale(lease_expires_unix: u64, clock: &dyn Clock) -> bool {
    lease_expires_unix <= clock.now_unix()
}

fn hash_key(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn print_human(payload: &Value) {
    if let Some(obj) = payload.as_object() {
        for (k, v) in obj {
            if v.is_object() || v.is_array() {
                println!(
                    "{k}: {}",
                    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
                );
            } else {
                println!("{k}: {v}");
            }
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string())
        );
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::sync::{Mutex, OnceLock};
    struct FixedClock(u64);
    impl Clock for FixedClock {
        fn now_unix(&self) -> u64 {
            self.0
        }
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn assignment_payload_validation_accepts_data_envelope() {
        let payload = json!({
            "data": {
                "remote_base_url": "https://remote.example.com",
                "remote_console_url": "https://remote.example.com/console",
                "lease_ttl_secs": 300,
                "issued_at_unix": 1_000
            }
        });
        let out = validate_assignment_payload(payload).expect("valid");
        assert_eq!(out.0.as_deref(), Some("https://remote.example.com"));
        assert_eq!(out.2, Some(1_300));
    }

    #[test]
    fn assignment_payload_rejects_origin_mismatch() {
        let payload = json!({
            "remote_base_url": "https://a.example.com",
            "remote_console_url": "https://b.example.com/console"
        });
        let err = validate_assignment_payload(payload).expect_err("must fail");
        assert!(format!("{err}").contains("share origin"));
    }

    #[test]
    fn website_assignment_parses_public_base_url() {
        let payload = json!({
            "ok": true,
            "endpoint": {
                "publicBaseUrl": "https://acme.llmhotspot.com/"
            }
        });
        let out = website_assignment_from_details_payload(payload).expect("parse");
        let out = out.expect("some assignment");
        assert_eq!(
            out.remote_base_url.as_deref(),
            Some("https://acme.llmhotspot.com")
        );
        assert_eq!(
            out.remote_console_url.as_deref(),
            Some("https://acme.llmhotspot.com/remote")
        );
    }

    #[test]
    fn website_assignment_parses_explicit_remote_console_url() {
        let payload = json!({
            "ok": true,
            "endpoint": {
                "publicBaseUrl": "https://acme.llmhotspot.com/",
                "remoteConsoleUrl": "https://acme.llmhotspot.com/console/"
            }
        });
        let out = website_assignment_from_details_payload(payload).expect("parse");
        let out = out.expect("some assignment");
        assert_eq!(
            out.remote_base_url.as_deref(),
            Some("https://acme.llmhotspot.com")
        );
        assert_eq!(
            out.remote_console_url.as_deref(),
            Some("https://acme.llmhotspot.com/remote")
        );
    }

    #[test]
    fn website_assignment_allows_unassigned_endpoint() {
        let payload = json!({
            "ok": true,
            "endpoint": null
        });
        let out = website_assignment_from_details_payload(payload).expect("parse");
        assert!(out.is_none());
    }

    #[test]
    fn website_assignment_rejects_non_https_url() {
        let payload = json!({
            "ok": true,
            "endpoint": {
                "publicBaseUrl": "ftp://acme.llmhotspot.com"
            }
        });
        let err = website_assignment_from_details_payload(payload).expect_err("must fail");
        assert!(format!("{err}").contains("remote url must be https"));
    }

    #[test]
    fn default_endpoint_slug_is_wordy_and_valid_shape() {
        let slug = default_endpoint_slug("LLMH-DEMO-0001-TEST-0001", "demo-device-001");
        let parts: Vec<&str> = slug.split('-').collect();
        assert_eq!(parts.len(), 3);
        assert!(slug.len() >= 8);
        assert!(slug.len() <= 63);
        assert!(parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase())));
    }

    #[test]
    fn lease_stale_uses_clock() {
        let clock = FixedClock(100);
        assert!(is_lease_stale(100, &clock));
        assert!(!is_lease_stale(101, &clock));
    }

    #[tokio::test]
    async fn refresh_interval_uses_lease_clock_window() {
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        store
            .set_app_state("remote_assignment_lease_expires_unix", "200")
            .await
            .expect("set");

        let clock = FixedClock(100);
        let secs = compute_assignment_refresh_interval_secs_with_clock(&store, &clock)
            .await
            .expect("interval");
        assert!((10..=64).contains(&secs));
    }

    #[tokio::test]
    async fn ensure_default_api_key_exists_creates_once_when_empty() {
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");

        ensure_default_api_key_exists(&store)
            .await
            .expect("first ensure");
        ensure_default_api_key_exists(&store)
            .await
            .expect("second ensure");

        let keys = store.list_api_keys().await.expect("list keys");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].label, "default");
    }

    #[tokio::test]
    async fn ensure_default_api_key_exists_does_not_create_when_any_key_exists() {
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let existing_raw = "hs_existing";
        let existing = ApiKeyRecord {
            key_id: "k1".to_string(),
            label: "custom".to_string(),
            key_hash: hash_key(existing_raw),
            tool_choice_policy: "auto".to_string(),
            created_unix: now_unix(),
            expires_unix: None,
            revoked_unix: None,
            last_used_unix: None,
        };
        store
            .create_api_key(&existing)
            .await
            .expect("create existing");

        ensure_default_api_key_exists(&store).await.expect("ensure");

        let keys = store.list_api_keys().await.expect("list keys");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].label, "custom");
    }

    #[test]
    fn with_admin_key_query_sets_query_only_to_admin_key() {
        let out = with_admin_key_query("http://127.0.0.1:11434/console?x=1", Some("hs_abc"));
        assert_eq!(out, "http://127.0.0.1:11434/console?admin_key=hs_abc");
    }

    #[test]
    fn local_client_base_url_uses_loopback_for_unspecified_bind() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let prev_a = std::env::var("LLMHOTSPOT_LOCAL_ADVERTISE_HOST").ok();
        let prev_b = std::env::var("LLMHOTSPOT_ADVERTISE_HOST").ok();

        let v4 = "0.0.0.0:11434".parse::<SocketAddr>().expect("parse v4");
        std::env::remove_var("LLMHOTSPOT_LOCAL_ADVERTISE_HOST");
        std::env::remove_var("LLMHOTSPOT_ADVERTISE_HOST");
        let cfg = RuntimeEnvConfig::from_env().expect("runtime env");
        let out = local_client_base_url(v4, &cfg);
        assert!(out.ends_with(":11434"));
        assert!(out.starts_with("http://"));
        let v6 = "[::]:11434".parse::<SocketAddr>().expect("parse v6");
        let out6 = local_client_base_url(v6, &cfg);
        assert!(out6.ends_with(":11434"));
        assert!(out6.starts_with("http://"));

        match prev_a {
            Some(v) => std::env::set_var("LLMHOTSPOT_LOCAL_ADVERTISE_HOST", v),
            None => std::env::remove_var("LLMHOTSPOT_LOCAL_ADVERTISE_HOST"),
        }
        match prev_b {
            Some(v) => std::env::set_var("LLMHOTSPOT_ADVERTISE_HOST", v),
            None => std::env::remove_var("LLMHOTSPOT_ADVERTISE_HOST"),
        }
    }

    #[test]
    fn local_client_base_url_uses_advertise_host_for_unspecified_bind() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let prev_a = std::env::var("LLMHOTSPOT_LOCAL_ADVERTISE_HOST").ok();
        let prev_b = std::env::var("LLMHOTSPOT_ADVERTISE_HOST").ok();

        std::env::set_var("LLMHOTSPOT_LOCAL_ADVERTISE_HOST", "192.168.1.50");
        let v4 = "0.0.0.0:11434".parse::<SocketAddr>().expect("parse v4");
        let cfg = RuntimeEnvConfig::from_env().expect("runtime env");
        assert_eq!(local_client_base_url(v4, &cfg), "http://192.168.1.50:11434");

        match prev_a {
            Some(v) => std::env::set_var("LLMHOTSPOT_LOCAL_ADVERTISE_HOST", v),
            None => std::env::remove_var("LLMHOTSPOT_LOCAL_ADVERTISE_HOST"),
        }
        match prev_b {
            Some(v) => std::env::set_var("LLMHOTSPOT_ADVERTISE_HOST", v),
            None => std::env::remove_var("LLMHOTSPOT_ADVERTISE_HOST"),
        }
    }

    #[tokio::test]
    async fn ensure_console_admin_bootstrap_key_creates_and_reuses_value() {
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let first = ensure_console_admin_bootstrap_key(&store).await;
        let second = ensure_console_admin_bootstrap_key(&store).await;
        assert!(first.is_some());
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn ensure_console_admin_bootstrap_key_fresh_preserves_recent_key() {
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let now = now_unix();
        store
            .set_app_state("console_admin_bootstrap_key", "adm_recent")
            .await
            .expect("set key");
        store
            .set_app_state("console_admin_bootstrap_key_created_unix", &now.to_string())
            .await
            .expect("set created");
        store
            .set_app_state("console_admin_session_token_hash", "hash_1")
            .await
            .expect("set hash");
        store
            .set_app_state("console_admin_session_expires_unix", "123")
            .await
            .expect("set exp");

        let key = ensure_console_admin_bootstrap_key_fresh(&store)
            .await
            .expect("fresh");
        assert_eq!(key, "adm_recent");
        let hash = store
            .get_app_state("console_admin_session_token_hash")
            .await
            .expect("read hash");
        assert_eq!(hash, "hash_1");
        let exp = store
            .get_app_state("console_admin_session_expires_unix")
            .await
            .expect("read exp");
        assert_eq!(exp, "123");
    }

    #[tokio::test]
    async fn ensure_console_admin_bootstrap_key_fresh_rotates_expired_key() {
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let stale = now_unix().saturating_sub(CONSOLE_ADMIN_BOOTSTRAP_TTL_SECS + 1);
        store
            .set_app_state("console_admin_bootstrap_key", "adm_old")
            .await
            .expect("set key");
        store
            .set_app_state(
                "console_admin_bootstrap_key_created_unix",
                &stale.to_string(),
            )
            .await
            .expect("set created");
        store
            .set_app_state("console_admin_session_token_hash", "hash_1")
            .await
            .expect("set hash");

        let key = ensure_console_admin_bootstrap_key_fresh(&store)
            .await
            .expect("fresh");
        assert!(key.starts_with("adm_"));
        assert_ne!(key, "adm_old");
        let prev = store
            .get_app_state("console_admin_previous_bootstrap_key")
            .await
            .expect("read previous");
        assert_eq!(prev, "adm_old");
        let hash = store
            .get_app_state("console_admin_session_token_hash")
            .await
            .expect("read hash");
        assert!(hash.is_empty());
    }

    #[tokio::test]
    async fn rotate_console_admin_bootstrap_key_resets_session() {
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        store
            .set_app_state("console_admin_bootstrap_key", "adm_prev")
            .await
            .expect("set bootstrap key");
        store
            .set_app_state("console_admin_session_token_hash", "abc")
            .await
            .expect("set hash");
        store
            .set_app_state("console_admin_session_expires_unix", "9")
            .await
            .expect("set exp");
        rotate_console_admin_bootstrap_key(&store)
            .await
            .expect("rotate");
        let key = store
            .get_app_state("console_admin_bootstrap_key")
            .await
            .expect("read key");
        assert!(key.starts_with("adm_"));
        assert_ne!(key, "adm_prev");
        let prev = store
            .get_app_state("console_admin_previous_bootstrap_key")
            .await
            .expect("read previous");
        assert_eq!(prev, "adm_prev");
        let hash = store
            .get_app_state("console_admin_session_token_hash")
            .await
            .expect("read hash");
        assert!(hash.is_empty());
        let exp = store
            .get_app_state("console_admin_session_expires_unix")
            .await
            .expect("read exp");
        assert_eq!(exp, "0");
    }

    #[tokio::test]
    async fn broker_unavailable_sets_offline_state_and_warning() {
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let elapsed = set_broker_connectivity_unavailable(&store, "transport error").await;
        assert_eq!(elapsed, 0);
        let offline_since = store
            .get_app_state(APP_BROKER_OFFLINE_SINCE_UNIX_KEY)
            .await
            .expect("offline since")
            .parse::<u64>()
            .expect("offline parse");
        assert!(offline_since > 0);
        let warning = store
            .get_app_state(APP_BROKER_WARNING_KEY)
            .await
            .expect("warning");
        assert!(warning.contains("Broker offline"));
    }

    #[tokio::test]
    async fn downgrade_clears_cached_managed_tunnel_credentials() {
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        store
            .set_app_state(APP_MANAGED_TUNNEL_ID_KEY, "tnl_123")
            .await
            .expect("set id");
        store
            .set_app_state(APP_MANAGED_TUNNEL_HOSTNAME_KEY, "pro.example.com")
            .await
            .expect("set hostname");
        store
            .set_app_state(APP_MANAGED_TUNNEL_TOKEN_KEY, "token_123")
            .await
            .expect("set token");

        downgrade_app_entitlement_to_free(&store).await;

        let tunnel_id = store
            .get_app_state(APP_MANAGED_TUNNEL_ID_KEY)
            .await
            .expect("read id");
        let hostname = store
            .get_app_state(APP_MANAGED_TUNNEL_HOSTNAME_KEY)
            .await
            .expect("read hostname");
        let token = store
            .get_app_state(APP_MANAGED_TUNNEL_TOKEN_KEY)
            .await
            .expect("read token");
        assert!(tunnel_id.is_empty());
        assert!(hostname.is_empty());
        assert!(token.is_empty());
    }

    #[tokio::test]
    async fn doctor_command_persists_last_run_snapshot() {
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        let out = doctor_command(&store).await.expect("doctor");
        assert!(
            out.get("run_unix").and_then(Value::as_u64).is_some(),
            "doctor output should contain run_unix"
        );
        let raw = store
            .get_app_state("doctor_last_run_json")
            .await
            .expect("read doctor snapshot");
        assert!(
            !raw.trim().is_empty(),
            "doctor snapshot should be persisted to app state"
        );
    }
}
