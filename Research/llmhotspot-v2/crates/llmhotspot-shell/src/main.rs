#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("llmhotspot-shell is only supported on Windows.");
}

#[cfg(target_os = "windows")]
fn main() {
    if let Err(err) = windows_shell::run() {
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "windows")]
mod windows_shell {
    use anyhow::{anyhow, Context, Result};
    use image::imageops::FilterType;
    use rand::{Rng, RngCore};
    use serde::{Deserialize, Serialize};
    use serde_json::{json, Value};
    use std::collections::HashSet;
    use std::fs::{self, OpenOptions};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    use windows_sys::Win32::System::Threading::OpenProcess;
    use windows_sys::Win32::System::Threading::{PROCESS_SET_QUOTA, PROCESS_TERMINATE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, EnumWindows, GetClassNameW, GetWindowThreadProcessId, PeekMessageW,
        TranslateMessage, MSG, PM_REMOVE,
    };

    #[cfg(target_os = "windows")]
    use std::os::windows::process::CommandExt;
    use url::Url;

    const CONTROL_PORT: u16 = 45297;
    // Default listen addr for the daemon. Override with `LLMHOTSPOT_SHELL_LISTEN` to use any
    // explicit bind address/port.
    const APP_LISTEN_DEFAULT: &str = "0.0.0.0:51824";
    const STARTUP_FILE: &str = ".llmhotspot.console.url";

    #[derive(Clone)]
    struct AppPaths {
        root: PathBuf,
        log_file: PathBuf,
        daemon_log_file: PathBuf,
        runtime_file: PathBuf,
        startup_file: PathBuf,
        db_path: PathBuf,
        llmhotspot_exe: PathBuf,
    }

    #[derive(Default, Serialize)]
    struct RuntimeSnapshot {
        shell_pid: u32,
        daemon_pid: Option<u32>,
        local_url: Option<String>,
        updated_unix: u64,
    }

    #[derive(Debug, Deserialize)]
    struct StatusResponse {
        local_url: Option<String>,
    }

    struct DaemonHandle {
        child: Child,
        /// Win32 Job Object handle. When closed, all processes in the job are terminated.
        job_handle: *mut std::ffi::c_void,
    }

    // SAFETY: The job_handle is only accessed under the mutex and is a kernel handle (thread-safe).
    unsafe impl Send for DaemonHandle {}

    #[derive(Clone)]
    struct ShellState {
        paths: AppPaths,
        env: ShellEnvConfig,
        ipc_secret: String,
        ipc_metrics: Arc<Mutex<IpcClientMetrics>>,
        ipc_hello: Arc<Mutex<Option<IpcNegotiation>>>,
        flush_backoff_failures: Arc<Mutex<u32>>,
        daemon: Arc<Mutex<Option<DaemonHandle>>>,
        shutdown: Arc<AtomicBool>,
    }

    #[derive(Default)]
    struct IpcClientMetrics {
        pending_hit_delta: u64,
        pending_fallback_delta: u64,
        pending_fail_delta: u64,
    }

    #[derive(Debug, Clone, Copy)]
    enum IpcOperation {
        Status,
        ConsoleUrl,
        ConsoleAdminKey,
    }

    #[derive(Debug, Clone, Copy)]
    enum IpcCallFailureReason {
        TransportDown,
        ProtocolMismatch,
        CapabilityMissing,
        ServerReject,
    }

    impl IpcCallFailureReason {
        fn as_str(self) -> &'static str {
            match self {
                Self::TransportDown => "transport_down",
                Self::ProtocolMismatch => "protocol_mismatch",
                Self::CapabilityMissing => "cap_missing",
                Self::ServerReject => "server_reject",
            }
        }
    }

    struct IpcCallError {
        reason: IpcCallFailureReason,
        request_id: String,
    }

    struct IpcNegotiation {
        protocol_version: u32,
        capabilities: HashSet<String>,
    }

    #[derive(Clone)]
    struct ShellEnvConfig {
        daemon_listen: String,
        tunnel_broker_url: String,
    }

    impl ShellEnvConfig {
        fn from_env() -> Result<Self> {
            let daemon_listen = std::env::var("LLMHOTSPOT_SHELL_LISTEN")
                .unwrap_or_else(|_| APP_LISTEN_DEFAULT.to_string());
            daemon_listen
                .parse::<std::net::SocketAddr>()
                .map_err(|e| anyhow!("invalid LLMHOTSPOT_SHELL_LISTEN `{daemon_listen}`: {e}"))?;
            let tunnel_broker_url = std::env::var("LLMHOTSPOT_TUNNEL_BROKER_URL")
                .unwrap_or_else(|_| "https://keys.llmhotspot.com".to_string());
            let parsed = Url::parse(&tunnel_broker_url)
                .map_err(|e| anyhow!("invalid LLMHOTSPOT_TUNNEL_BROKER_URL: {e}"))?;
            if !matches!(parsed.scheme(), "https" | "http") {
                return Err(anyhow!("LLMHOTSPOT_TUNNEL_BROKER_URL must be http/https"));
            }
            Ok(Self {
                daemon_listen,
                tunnel_broker_url,
            })
        }
    }

    fn ipc_metrics_flush_interval_ms() -> u64 {
        std::env::var("LLMHOTSPOT_IPC_METRICS_FLUSH_MS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|v| *v >= 1_000)
            .unwrap_or(15_000)
    }

    pub fn run() -> Result<()> {
        let deep_link = std::env::args()
            .skip(1)
            .find(|arg| arg.starts_with("llmhotspot://"));
        let paths = resolve_paths()?;
        let env = ShellEnvConfig::from_env()?;
        let ipc_secret = generate_runtime_ipc_secret();
        let state = ShellState {
            paths: paths.clone(),
            env,
            ipc_secret,
            ipc_metrics: Arc::new(Mutex::new(IpcClientMetrics::default())),
            ipc_hello: Arc::new(Mutex::new(None)),
            flush_backoff_failures: Arc::new(Mutex::new(0)),
            daemon: Arc::new(Mutex::new(None)),
            shutdown: Arc::new(AtomicBool::new(false)),
        };
        log_line(&paths.log_file, "shell starting");
        log_windows_theme_diagnostics(&paths.log_file);

        let listener = match TcpListener::bind(("127.0.0.1", CONTROL_PORT)) {
            Ok(v) => Some(v),
            Err(_) => {
                if signal_existing_open(deep_link.as_deref()) {
                    return Ok(());
                }
                Some(TcpListener::bind(("127.0.0.1", CONTROL_PORT)).context("bind control port")?)
            }
        };

        start_or_restart_daemon(&state).context("start daemon")?;
        let opened = open_console_retry(&state, "startup");
        if !opened {
            let open_retry_state = state.clone();
            thread::spawn(move || {
                // Installer startup can race browser/default-handler readiness.
                thread::sleep(Duration::from_secs(2));
                let _ = open_console_retry(&open_retry_state, "startup delayed");
            });
        }

        if let Some(control) = listener {
            let control_state = state.clone();
            thread::spawn(move || control_loop(control_state, control));
        }

        if let Some(url) = deep_link.as_deref() {
            handle_protocol_url(&state, url, "startup deep link");
        }

        let monitor_state = state.clone();
        thread::spawn(move || monitor_loop(monitor_state));
        let metrics_state = state.clone();
        thread::spawn(move || flush_metrics_loop(metrics_state));

        tray_loop(state)
    }

    fn resolve_paths() -> Result<AppPaths> {
        let local =
            std::env::var_os("LOCALAPPDATA").ok_or_else(|| anyhow!("LOCALAPPDATA not set"))?;
        let root = PathBuf::from(local).join("LLMHotspot");
        fs::create_dir_all(&root).context("create runtime dir")?;
        let exe_dir = std::env::current_exe()
            .context("resolve current exe")?
            .parent()
            .ok_or_else(|| anyhow!("failed to resolve executable directory"))?
            .to_path_buf();
        Ok(AppPaths {
            root: root.clone(),
            log_file: root.join("shell.log"),
            daemon_log_file: root.join("daemon.log"),
            runtime_file: root.join("runtime.json"),
            startup_file: root.join(STARTUP_FILE),
            db_path: root.join("llmhotspot.db"),
            llmhotspot_exe: exe_dir.join("llmhotspot.exe"),
        })
    }

    fn tray_loop(state: ShellState) -> Result<()> {
        enable_windows_auto_dark_menus(&state.paths.log_file);
        let mut dark_mode = windows_prefers_dark_mode();
        log_line(
            &state.paths.log_file,
            &format!(
                "theme: tray icon asset selected={}",
                if dark_mode {
                    "light-logo.png (dark mode)"
                } else {
                    "dark-logo.png (light mode)"
                }
            ),
        );
        let icon = load_tray_icon_for_mode(dark_mode)?;
        let menu = Menu::new();
        let open_item = MenuItem::new("Open Console", true, None);
        let restart_item = MenuItem::new("Restart App", true, None);
        let exit_item = MenuItem::new("Exit", true, None);
        menu.append(&open_item)
            .map_err(|e| anyhow!(e.to_string()))?;
        menu.append(&restart_item)
            .map_err(|e| anyhow!(e.to_string()))?;
        menu.append(&exit_item)
            .map_err(|e| anyhow!(e.to_string()))?;

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .with_tooltip(concat!(
                "LLM Hotspot v",
                env!("CARGO_PKG_VERSION"),
                " (",
                env!("GIT_HASH"),
                ")"
            ))
            .with_menu_on_left_click(false)
            .build()
            .map_err(|e| anyhow!(e.to_string()))?;
        apply_dark_mode_to_tray_window(&state.paths.log_file);

        loop {
            if state.shutdown.load(Ordering::SeqCst) {
                break;
            }
            let current_dark = windows_prefers_dark_mode();
            if current_dark != dark_mode {
                dark_mode = current_dark;
                if let Ok(icon) = load_tray_icon_for_mode(dark_mode) {
                    let _ = tray.set_icon(Some(icon));
                    log_line(
                        &state.paths.log_file,
                        &format!(
                            "theme: tray icon updated={}",
                            if dark_mode {
                                "light-logo.png (dark mode)"
                            } else {
                                "dark-logo.png (light mode)"
                            }
                        ),
                    );
                }
            }
            pump_win32_messages();
            if let Ok(event) = MenuEvent::receiver().try_recv() {
                let id = event.id;
                if id == open_item.id() {
                    log_line(&state.paths.log_file, "menu open_console");
                    open_console_retry(&state, "menu open");
                } else if id == restart_item.id() {
                    log_line(&state.paths.log_file, "menu restart_app");
                    let _ = start_or_restart_daemon(&state);
                    open_console_retry(&state, "menu restart");
                } else if id == exit_item.id() {
                    log_line(&state.paths.log_file, "menu exit");
                    state.shutdown.store(true, Ordering::SeqCst);
                    stop_daemon(&state);
                    thread::spawn(|| {
                        thread::sleep(Duration::from_millis(2200));
                        std::process::exit(0);
                    });
                    break;
                }
            }
            if let Ok(TrayIconEvent::Click {
                button,
                button_state,
                ..
            }) = TrayIconEvent::receiver().try_recv()
            {
                if button == MouseButton::Left && button_state == MouseButtonState::Up {
                    log_line(&state.paths.log_file, "tray left_click");
                    open_console_retry(&state, "tray left click");
                }
            }
            thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    }

    fn enable_windows_auto_dark_menus(log_file: &Path) {
        // Ask Windows to apply native menu dark/light appearance based on system settings.
        unsafe {
            let module = LoadLibraryA(c"uxtheme.dll".as_ptr() as _);
            if module.is_null() {
                log_line(log_file, "theme: uxtheme.dll not loaded");
                return;
            }
            log_line(log_file, "theme: uxtheme.dll loaded");
            let set_preferred =
                get_proc_by_name_or_ordinal(module, c"SetPreferredAppMode".as_ptr() as _, 135);
            if let Some(set_preferred) = set_preferred {
                let set_preferred: extern "system" fn(i32) -> i32 =
                    std::mem::transmute(set_preferred);
                const ALLOW_DARK: i32 = 1;
                let result = set_preferred(ALLOW_DARK);
                log_line(
                    log_file,
                    &format!("theme: SetPreferredAppMode found result={result} mode=ALLOW_DARK"),
                );
            } else {
                log_line(log_file, "theme: SetPreferredAppMode not found");
            }
            let flush_menu_themes =
                get_proc_by_name_or_ordinal(module, c"FlushMenuThemes".as_ptr() as _, 136);
            if let Some(flush_menu_themes) = flush_menu_themes {
                let flush_menu_themes: extern "system" fn() =
                    std::mem::transmute(flush_menu_themes);
                flush_menu_themes();
                log_line(log_file, "theme: FlushMenuThemes invoked");
            } else {
                log_line(log_file, "theme: FlushMenuThemes not found");
            }
        }
    }

    fn apply_dark_mode_to_tray_window(log_file: &Path) {
        use std::ffi::c_void;
        let Some(hwnd) = find_tray_host_hwnd() else {
            log_line(log_file, "theme: tray host hwnd not found");
            return;
        };
        log_line(log_file, &format!("theme: tray host hwnd found={hwnd:#x}"));
        unsafe {
            let enabled: i32 = 1;
            let dwm_status = DwmSetWindowAttribute(
                hwnd as *mut c_void,
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                &enabled as *const _ as _,
                std::mem::size_of::<i32>() as u32,
            );
            log_line(
                log_file,
                &format!("theme: DwmSetWindowAttribute(DARK_MODE) status={dwm_status}"),
            );
            let module = LoadLibraryA(c"uxtheme.dll".as_ptr() as _);
            if module.is_null() {
                log_line(log_file, "theme: uxtheme.dll not loaded for hwnd dark mode");
                return;
            }
            let allow_for_window =
                get_proc_by_name_or_ordinal(module, c"AllowDarkModeForWindow".as_ptr() as _, 133);
            if let Some(allow_for_window) = allow_for_window {
                let allow_for_window: extern "system" fn(isize, i32) -> i32 =
                    std::mem::transmute(allow_for_window);
                let result = allow_for_window(hwnd, 1);
                log_line(
                    log_file,
                    &format!("theme: AllowDarkModeForWindow found result={result}"),
                );
            } else {
                log_line(log_file, "theme: AllowDarkModeForWindow not found");
            }
            let flush_menu_themes =
                get_proc_by_name_or_ordinal(module, c"FlushMenuThemes".as_ptr() as _, 136);
            if let Some(flush_menu_themes) = flush_menu_themes {
                let flush_menu_themes: extern "system" fn() =
                    std::mem::transmute(flush_menu_themes);
                flush_menu_themes();
                log_line(
                    log_file,
                    "theme: FlushMenuThemes invoked after hwnd dark mode",
                );
            } else {
                log_line(
                    log_file,
                    "theme: FlushMenuThemes not found after hwnd dark mode",
                );
            }
        }
    }

    fn get_proc_by_name_or_ordinal(
        module: *mut std::ffi::c_void,
        name: *const u8,
        ordinal: u16,
    ) -> Option<unsafe extern "system" fn() -> isize> {
        unsafe {
            let by_name = GetProcAddress(module, name as _);
            if by_name.is_some() {
                return by_name;
            }
            GetProcAddress(module, ordinal as usize as *const u8)
        }
    }

    fn log_windows_theme_diagnostics(log_file: &Path) {
        let apps = query_windows_personalize_value("AppsUseLightTheme");
        let system = query_windows_personalize_value("SystemUsesLightTheme");
        let apps_mode = match apps {
            Some(0) => "dark",
            Some(1) => "light",
            Some(_) => "unknown",
            None => "unavailable",
        };
        let system_mode = match system {
            Some(0) => "dark",
            Some(1) => "light",
            Some(_) => "unknown",
            None => "unavailable",
        };
        log_line(
            log_file,
            &format!(
                "theme: personalize apps_use_light={:?} ({apps_mode}) system_use_light={:?} ({system_mode})",
                apps, system
            ),
        );
    }

    fn query_windows_personalize_value(name: &str) -> Option<u32> {
        let subkey: Vec<u16> =
            "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0"
                .encode_utf16()
                .collect();
        let value_name: Vec<u16> = format!("{name}\0").encode_utf16().collect();
        let mut data: u32 = 0;
        let mut data_len: u32 = std::mem::size_of::<u32>() as u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                value_name.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                &mut data as *mut u32 as *mut _,
                &mut data_len,
            )
        };
        (status == 0).then_some(data)
    }

    fn find_tray_host_hwnd() -> Option<isize> {
        use std::ffi::c_void;
        struct Ctx {
            pid: u32,
            hwnd: isize,
        }
        unsafe extern "system" fn enum_windows_cb(hwnd: *mut c_void, lparam: isize) -> i32 {
            let ctx = &mut *(lparam as *mut Ctx);
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid != ctx.pid {
                return 1;
            }
            let mut class_name = [0u16; 128];
            let len = GetClassNameW(hwnd, class_name.as_mut_ptr(), class_name.len() as i32);
            if len <= 0 {
                return 1;
            }
            let class_name = String::from_utf16_lossy(&class_name[..len as usize]);
            if class_name == "tray_icon_app" {
                ctx.hwnd = hwnd as isize;
                return 0;
            }
            1
        }

        let mut ctx = Ctx {
            pid: std::process::id(),
            hwnd: 0,
        };
        unsafe {
            EnumWindows(Some(enum_windows_cb), &mut ctx as *mut Ctx as isize);
        }
        (ctx.hwnd != 0).then_some(ctx.hwnd)
    }

    fn load_tray_icon_for_mode(dark_mode: bool) -> Result<Icon> {
        let icon_bytes = if dark_mode {
            include_bytes!("../../../light-logo.png").as_slice()
        } else {
            include_bytes!("../../../dark-logo.png").as_slice()
        };
        let icon_img = image::load_from_memory(icon_bytes)
            .context("decode tray icon png")?
            .to_rgba8();
        let icon_resized = image::imageops::resize(&icon_img, 32, 32, FilterType::Lanczos3);
        Icon::from_rgba(icon_resized.into_raw(), 32, 32).map_err(|e| anyhow!(e.to_string()))
    }

    fn windows_prefers_dark_mode() -> bool {
        matches!(
            query_windows_personalize_value("AppsUseLightTheme"),
            Some(0)
        )
    }

    fn pump_win32_messages() {
        // tray-icon on Windows requires the thread's Win32 message queue to be pumped.
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    fn monitor_loop(state: ShellState) {
        while !state.shutdown.load(Ordering::SeqCst) {
            let _ = ensure_daemon_running(&state);
            let _ = write_runtime_snapshot(&state);
            thread::sleep(Duration::from_secs(2));
        }
    }

    fn control_loop(state: ShellState, listener: TcpListener) {
        let _ = listener.set_nonblocking(true);
        while !state.shutdown.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut buf = [0_u8; 8192];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let msg = String::from_utf8_lossy(&buf[..n]);
                    let trimmed = msg.trim();
                    if trimmed.eq_ignore_ascii_case("OPEN") {
                        open_console_retry(&state, "control open");
                    } else if let Some(url) = trimmed.strip_prefix("DEEPLINK ") {
                        handle_protocol_url(&state, url.trim(), "control deep link");
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(80));
                }
                Err(_) => {
                    thread::sleep(Duration::from_millis(120));
                }
            }
        }
    }

    fn signal_existing_open(deep_link: Option<&str>) -> bool {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], CONTROL_PORT));
        let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(300)) else {
            return false;
        };
        let payload = if let Some(url) = deep_link {
            format!("DEEPLINK {url}\n")
        } else {
            "OPEN\n".to_string()
        };
        let _ = s.write_all(payload.as_bytes());
        true
    }

    fn handle_protocol_url(state: &ShellState, protocol_url: &str, reason: &str) {
        if let Some((code, state_token)) = parse_auth_callback_url(protocol_url) {
            if complete_pro_callback_exchange(state, &code, &state_token, reason) {
                let _ = open_console_retry(state, "pro callback complete");
                return;
            }
            log_line(
                &state.paths.log_file,
                &format!("protocol callback failed reason={reason}"),
            );
            let _ = open_console_retry(state, "pro callback failed");
            return;
        }
        log_line(
            &state.paths.log_file,
            &format!(
                "protocol url ignored reason={reason} url={}",
                redact_admin_key_in_url(protocol_url)
            ),
        );
    }

    fn parse_auth_callback_url(raw: &str) -> Option<(String, String)> {
        let parsed = Url::parse(raw.trim()).ok()?;
        if parsed.scheme() != "llmhotspot" {
            return None;
        }
        if parsed.host_str() != Some("auth") || parsed.path() != "/callback" {
            return None;
        }
        let mut code: Option<String> = None;
        let mut state_token: Option<String> = None;
        for (k, v) in parsed.query_pairs() {
            if k == "code" {
                code = Some(v.into_owned());
            } else if k == "state" {
                state_token = Some(v.into_owned());
            }
        }
        let code = code.unwrap_or_default();
        let state_token = state_token.unwrap_or_default();
        if code.is_empty() || state_token.is_empty() {
            return None;
        }
        Some((code, state_token))
    }

    fn complete_pro_callback_exchange(
        state: &ShellState,
        code: &str,
        state_token: &str,
        reason: &str,
    ) -> bool {
        for attempt in 0..16 {
            let _ = ensure_daemon_running(state);
            let Some(local) =
                command_local_url(state).and_then(|v| force_loopback_http_url(&v).or(Some(v)))
            else {
                thread::sleep(Duration::from_millis(350));
                continue;
            };
            let Some(admin_key) = command_console_admin_key(state) else {
                thread::sleep(Duration::from_millis(350));
                continue;
            };
            let payload = json!({"code": code, "state": state_token});
            match post_local_json(
                &local,
                "/v1/pro/callback/complete",
                &admin_key,
                payload,
                Duration::from_secs(4),
            ) {
                Ok(resp) => {
                    if resp.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                        log_line(
                            &state.paths.log_file,
                            &format!(
                                "protocol callback complete reason={reason} attempt={attempt}"
                            ),
                        );
                        return true;
                    }
                    let err = resp
                        .get("error")
                        .and_then(|e| {
                            // error_response wraps as {"error":{"code":...,"message":"..."}}
                            e.get("message")
                                .and_then(Value::as_str)
                                .or_else(|| e.as_str())
                        })
                        .unwrap_or("unknown");
                    log_line(
                        &state.paths.log_file,
                        &format!(
                            "protocol callback rejected reason={reason} attempt={attempt} error={err}"
                        ),
                    );
                }
                Err(err) => {
                    log_line(
                        &state.paths.log_file,
                        &format!(
                            "protocol callback request failed reason={reason} attempt={attempt} err={err}"
                        ),
                    );
                }
            }
            thread::sleep(Duration::from_millis(500));
        }
        false
    }

    fn post_local_json(
        local_base_url: &str,
        path: &str,
        bearer: &str,
        payload: Value,
        timeout: Duration,
    ) -> Result<Value> {
        let base = Url::parse(local_base_url.trim())
            .with_context(|| format!("invalid local base url: {local_base_url}"))?;
        let url = base
            .join(path.trim_start_matches('/'))
            .with_context(|| format!("invalid local request path: {path}"))?;
        let auth = format!("Bearer {bearer}");
        let request = ureq::post(url.as_str())
            .timeout(timeout)
            .set("Authorization", &auth)
            .set("Content-Type", "application/json");
        match request.send_json(payload) {
            Ok(resp) => resp
                .into_json::<Value>()
                .context("parse local json response"),
            Err(ureq::Error::Status(_, resp)) => resp
                .into_json::<Value>()
                .context("parse local json error response"),
            Err(err) => Err(anyhow!(err.to_string())),
        }
    }

    fn open_console_retry(state: &ShellState, reason: &str) -> bool {
        for attempt in 0..8 {
            if let Some(url) = resolve_console_url(state) {
                if open_url(&url) {
                    log_line(
                        &state.paths.log_file,
                        &format!(
                            "open_console ok reason={reason} attempt={attempt} url={}",
                            redact_admin_key_in_url(&url)
                        ),
                    );
                    return true;
                }
            }
            thread::sleep(Duration::from_millis(450));
            let _ = ensure_daemon_running(state);
        }
        log_line(
            &state.paths.log_file,
            &format!("open_console failed reason={reason}"),
        );
        false
    }

    fn open_url(url: &str) -> bool {
        if webbrowser::open(url).is_ok() {
            return true;
        }
        // Fallback: delegate to explorer, which often succeeds even when elevated.
        let mut cmd = Command::new("explorer.exe");
        cmd.arg(url);
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.spawn().is_ok()
    }

    fn resolve_console_url(state: &ShellState) -> Option<String> {
        if let Some(v) = command_console_url(state) {
            // Always open loopback for local console so OAuth "Connect" works even if the daemon
            // is bound to 0.0.0.0 for LAN access.
            return Some(force_loopback_http_url(&v).unwrap_or(v));
        }
        if let Some(local) = command_local_url(state) {
            let url = format!("{}/console", local.trim_end_matches('/'));
            return Some(force_loopback_http_url(&url).unwrap_or(url));
        }
        let raw = fs::read_to_string(&state.paths.startup_file).ok()?;
        let line = raw.lines().next()?.trim();
        if line.is_empty() {
            None
        } else {
            Some(line.to_string())
        }
    }

    // Convert any local http URL into an explicit loopback URL. We intentionally do not touch
    // https remote URLs.
    fn force_loopback_http_url(raw: &str) -> Option<String> {
        let s = raw.trim();
        if !s.starts_with("http://") {
            return None;
        }

        let rest = &s["http://".len()..];
        let (authority, path) = match rest.split_once('/') {
            Some((a, p)) => (a, format!("/{}", p)),
            None => (rest, String::new()),
        };

        // Ignore any userinfo; not expected here but safe.
        let hostport = authority.rsplit('@').next().unwrap_or(authority);
        let port = if hostport.starts_with('[') {
            hostport.rsplit_once("]:").map(|(_, p)| p)
        } else {
            hostport.rsplit_once(':').map(|(_, p)| p)
        }?;

        Some(format!("http://127.0.0.1:{}{}", port, path))
    }

    fn run_llmhotspot_json(paths: &AppPaths, args: &[&str]) -> Option<serde_json::Value> {
        let mut cmd = Command::new(&paths.llmhotspot_exe);
        cmd.arg("--json").arg("--db-path").arg(&paths.db_path);
        for arg in args {
            cmd.arg(arg);
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
        let out = cmd.output().ok()?;
        if !out.status.success() {
            return None;
        }
        serde_json::from_slice(&out.stdout).ok()
    }

    fn fnv1a64(s: &str) -> u64 {
        let mut hash: u64 = 0xcbf29ce484222325;
        for b in s.as_bytes() {
            hash ^= *b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash
    }

    fn runtime_pipe_name(paths: &AppPaths) -> String {
        let db_norm = paths
            .db_path
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase();
        format!(r"\\.\pipe\llmhotspot-{:016x}-runtime", fnv1a64(&db_norm))
    }

    fn generate_runtime_ipc_secret() -> String {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    }

    fn generate_ipc_request_id() -> String {
        let mut bytes = [0u8; 6];
        rand::thread_rng().fill_bytes(&mut bytes);
        format!(
            "ipc_{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]
        )
    }

    fn current_pending_metrics(state: &ShellState) -> IpcClientMetrics {
        if let Ok(guard) = state.ipc_metrics.lock() {
            IpcClientMetrics {
                pending_hit_delta: guard.pending_hit_delta,
                pending_fallback_delta: guard.pending_fallback_delta,
                pending_fail_delta: guard.pending_fail_delta,
            }
        } else {
            IpcClientMetrics::default()
        }
    }

    fn record_backoff_failure(state: &ShellState, retry_in_ms: u64) {
        if let Ok(mut guard) = state.flush_backoff_failures.lock() {
            *guard = guard.saturating_add(1);
        }
        log_line(
            &state.paths.log_file,
            &format!("ipc metrics flush failed; retry_in_ms={retry_in_ms}"),
        );
    }

    fn reset_backoff_on_success(state: &ShellState) {
        if let Ok(mut guard) = state.flush_backoff_failures.lock() {
            if *guard > 0 {
                log_line(
                    &state.paths.log_file,
                    "ipc metrics flush recovered; backoff reset",
                );
            }
            *guard = 0;
        }
    }

    fn jittered_backoff_ms(base_ms: u64, failures: u32) -> u64 {
        let exp = failures.min(8);
        let mut delay = base_ms.saturating_mul(1u64 << exp);
        delay = delay.min(300_000);
        let jitter_span = (delay / 5).max(1);
        let jitter = rand::thread_rng().gen_range(0..=jitter_span.saturating_mul(2));
        delay
            .saturating_sub(jitter_span)
            .saturating_add(jitter)
            .max(1_000)
    }

    fn negotiate_runtime_ipc(state: &ShellState) -> Result<IpcNegotiation, IpcCallError> {
        if let Ok(guard) = state.ipc_hello.lock() {
            if let Some(existing) = &*guard {
                return Ok(IpcNegotiation {
                    protocol_version: existing.protocol_version,
                    capabilities: existing.capabilities.clone(),
                });
            }
        }
        let request_id = generate_ipc_request_id();
        log_line(
            &state.paths.log_file,
            &format!("ipc req id={request_id} op=hello"),
        );
        let hello =
            match runtime_ipc_raw_call(&state.paths, json!({"op":"hello","request_id":request_id}))
            {
                Ok(v) => {
                    log_line(
                        &state.paths.log_file,
                        &format!("ipc ok id={} op=hello", request_id),
                    );
                    v
                }
                Err(reason) => {
                    log_line(
                        &state.paths.log_file,
                        &format!(
                            "ipc fail id={} op=hello reason={}",
                            request_id,
                            reason.as_str()
                        ),
                    );
                    return Err(IpcCallError { reason, request_id });
                }
            };
        let protocol_version = hello
            .get("protocol_version")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .ok_or(IpcCallError {
                reason: IpcCallFailureReason::ServerReject,
                request_id: request_id.clone(),
            })?;
        let capabilities = hello
            .get("capabilities")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect::<HashSet<String>>()
            })
            .unwrap_or_default();
        let negotiated = IpcNegotiation {
            protocol_version,
            capabilities,
        };
        if let Ok(mut guard) = state.ipc_hello.lock() {
            *guard = Some(IpcNegotiation {
                protocol_version: negotiated.protocol_version,
                capabilities: negotiated.capabilities.clone(),
            });
        }
        Ok(negotiated)
    }

    fn runtime_ipc_raw_call(paths: &AppPaths, req: Value) -> Result<Value, IpcCallFailureReason> {
        let pipe = runtime_pipe_name(paths);
        let mut f = OpenOptions::new()
            .read(true)
            .write(true)
            .open(pipe)
            .map_err(|_| IpcCallFailureReason::TransportDown)?;
        let payload =
            serde_json::to_string(&req).map_err(|_| IpcCallFailureReason::TransportDown)?;
        if f.write_all(payload.as_bytes()).is_err() {
            return Err(IpcCallFailureReason::TransportDown);
        }
        if f.write_all(b"\n").is_err() {
            return Err(IpcCallFailureReason::TransportDown);
        }
        let mut out = String::new();
        if f.read_to_string(&mut out).is_err() {
            return Err(IpcCallFailureReason::TransportDown);
        }
        let line = out.lines().next().unwrap_or("").trim();
        let resp: Value =
            serde_json::from_str(line).map_err(|_| IpcCallFailureReason::TransportDown)?;
        if !resp.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            return Err(IpcCallFailureReason::ServerReject);
        }
        resp.get("data")
            .cloned()
            .ok_or(IpcCallFailureReason::ServerReject)
    }

    fn record_ipc_hit(state: &ShellState) {
        if let Ok(mut guard) = state.ipc_metrics.lock() {
            guard.pending_hit_delta = guard.pending_hit_delta.saturating_add(1);
        }
    }

    fn record_ipc_failure(state: &ShellState) {
        if let Ok(mut guard) = state.ipc_metrics.lock() {
            guard.pending_fail_delta = guard.pending_fail_delta.saturating_add(1);
        }
    }

    fn record_cli_fallback(state: &ShellState) {
        if let Ok(mut guard) = state.ipc_metrics.lock() {
            guard.pending_fallback_delta = guard.pending_fallback_delta.saturating_add(1);
        }
    }

    fn clear_ipc_negotiation(state: &ShellState) {
        if let Ok(mut guard) = state.ipc_hello.lock() {
            *guard = None;
        }
    }

    fn should_fallback(op: IpcOperation, _reason: IpcCallFailureReason) -> bool {
        !matches!(op, IpcOperation::ConsoleAdminKey)
    }

    fn take_pending_metrics(state: &ShellState) -> IpcClientMetrics {
        if let Ok(mut guard) = state.ipc_metrics.lock() {
            let out = IpcClientMetrics {
                pending_hit_delta: guard.pending_hit_delta,
                pending_fallback_delta: guard.pending_fallback_delta,
                pending_fail_delta: guard.pending_fail_delta,
            };
            guard.pending_hit_delta = 0;
            guard.pending_fallback_delta = 0;
            guard.pending_fail_delta = 0;
            out
        } else {
            IpcClientMetrics::default()
        }
    }

    fn restore_pending_metrics(state: &ShellState, delta: &IpcClientMetrics) {
        if delta.pending_hit_delta == 0
            && delta.pending_fallback_delta == 0
            && delta.pending_fail_delta == 0
        {
            return;
        }
        if let Ok(mut guard) = state.ipc_metrics.lock() {
            guard.pending_hit_delta = guard
                .pending_hit_delta
                .saturating_add(delta.pending_hit_delta);
            guard.pending_fallback_delta = guard
                .pending_fallback_delta
                .saturating_add(delta.pending_fallback_delta);
            guard.pending_fail_delta = guard
                .pending_fail_delta
                .saturating_add(delta.pending_fail_delta);
        }
    }

    fn publish_pending_metrics_snapshot(state: &ShellState) {
        let pending = current_pending_metrics(state);
        let args = [
            "app".to_string(),
            "ipc-metrics-pending".to_string(),
            "--hits".to_string(),
            pending.pending_hit_delta.to_string(),
            "--fallbacks".to_string(),
            pending.pending_fallback_delta.to_string(),
            "--fails".to_string(),
            pending.pending_fail_delta.to_string(),
            "--updated-unix".to_string(),
            now_unix().to_string(),
        ];
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let _ = run_llmhotspot_json(&state.paths, &arg_refs);
    }

    fn flush_metrics_once_via_cli(state: &ShellState) -> bool {
        let delta = take_pending_metrics(state);
        if delta.pending_hit_delta == 0
            && delta.pending_fallback_delta == 0
            && delta.pending_fail_delta == 0
        {
            return true;
        }
        let args = [
            "app".to_string(),
            "ipc-metrics".to_string(),
            "--hits".to_string(),
            delta.pending_hit_delta.to_string(),
            "--fallbacks".to_string(),
            delta.pending_fallback_delta.to_string(),
            "--fails".to_string(),
            delta.pending_fail_delta.to_string(),
        ];
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        if run_llmhotspot_json(&state.paths, &arg_refs).is_some() {
            true
        } else {
            restore_pending_metrics(state, &delta);
            false
        }
    }

    fn flush_metrics_loop(state: ShellState) {
        let base_ms = ipc_metrics_flush_interval_ms();
        while !state.shutdown.load(Ordering::SeqCst) {
            publish_pending_metrics_snapshot(&state);
            let ok = flush_metrics_once_via_cli(&state);
            if ok {
                reset_backoff_on_success(&state);
            } else {
                let failures = state.flush_backoff_failures.lock().map(|v| *v).unwrap_or(0);
                let retry_in = jittered_backoff_ms(base_ms, failures);
                record_backoff_failure(&state, retry_in);
                thread::sleep(Duration::from_millis(retry_in));
                continue;
            }
            thread::sleep(Duration::from_millis(base_ms));
        }
    }

    fn runtime_ipc_call(
        state: &ShellState,
        op: IpcOperation,
        mut req: Value,
        required_cap: &str,
    ) -> Result<Value, IpcCallError> {
        let request_id = generate_ipc_request_id();
        let hello = negotiate_runtime_ipc(state)?;
        if hello.protocol_version != 1 {
            return Err(IpcCallError {
                reason: IpcCallFailureReason::ProtocolMismatch,
                request_id,
            });
        }
        if !hello.capabilities.contains(required_cap) {
            return Err(IpcCallError {
                reason: IpcCallFailureReason::CapabilityMissing,
                request_id,
            });
        }
        let pending = take_pending_metrics(state);
        req["request_id"] = json!(request_id);
        req["protocol_version"] = json!(hello.protocol_version);
        if hello.capabilities.contains("client_metrics_v1") {
            req["client_metrics"] = json!({
                "ipc_hit_delta": pending.pending_hit_delta,
                "fallback_delta": pending.pending_fallback_delta,
                "ipc_fail_delta": pending.pending_fail_delta
            });
        }
        if matches!(op, IpcOperation::ConsoleAdminKey) {
            req["auth"] = json!(state.ipc_secret.clone());
        }
        log_line(
            &state.paths.log_file,
            &format!("ipc req id={} op={:?}", req["request_id"], op),
        );
        let data = match runtime_ipc_raw_call(&state.paths, req) {
            Ok(v) => {
                log_line(
                    &state.paths.log_file,
                    &format!("ipc ok id={} op={:?}", request_id, op),
                );
                v
            }
            Err(err) => {
                restore_pending_metrics(state, &pending);
                if matches!(err, IpcCallFailureReason::TransportDown) {
                    clear_ipc_negotiation(state);
                }
                record_ipc_failure(state);
                log_line(
                    &state.paths.log_file,
                    &format!(
                        "ipc fail id={} op={:?} reason={}",
                        request_id,
                        op,
                        err.as_str()
                    ),
                );
                return Err(IpcCallError {
                    reason: err,
                    request_id,
                });
            }
        };
        record_ipc_hit(state);
        reset_backoff_on_success(state);
        Ok(data)
    }

    fn runtime_or_fallback_call(
        state: &ShellState,
        op: IpcOperation,
        req: Value,
        required_cap: &str,
        fallback_args: &[&str],
    ) -> Option<Value> {
        match runtime_ipc_call(state, op, req, required_cap) {
            Ok(v) => Some(v),
            Err(err) => {
                if should_fallback(op, err.reason) {
                    record_cli_fallback(state);
                    log_line(
                        &state.paths.log_file,
                        &format!(
                            "ipc fallback id={} op={:?} reason={}",
                            err.request_id,
                            op,
                            err.reason.as_str()
                        ),
                    );
                    run_llmhotspot_json(&state.paths, fallback_args)
                } else {
                    None
                }
            }
        }
    }

    fn command_console_url(state: &ShellState) -> Option<String> {
        let v = runtime_or_fallback_call(
            state,
            IpcOperation::ConsoleUrl,
            json!({"op":"console_url","remote":false}),
            "console_url",
            &["app", "console-url"],
        )?;
        v.get("url")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    }

    fn command_console_admin_key(state: &ShellState) -> Option<String> {
        let v = runtime_or_fallback_call(
            state,
            IpcOperation::ConsoleAdminKey,
            json!({"op":"console_admin_key"}),
            "console_admin_key_auth_v1",
            &[],
        )?;
        v.get("admin_key")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty())
    }

    fn command_local_url(state: &ShellState) -> Option<String> {
        let raw = runtime_or_fallback_call(
            state,
            IpcOperation::Status,
            json!({"op":"status"}),
            "status",
            &["status"],
        )?;
        let v: StatusResponse = serde_json::from_value(raw).ok()?;
        v.local_url
    }

    fn redact_admin_key_in_url(raw: &str) -> String {
        let Ok(mut parsed) = Url::parse(raw) else {
            return raw.to_string();
        };
        let mut changed = false;
        let pairs: Vec<(String, String)> = parsed
            .query_pairs()
            .map(|(k, v)| {
                if k == "admin_key" {
                    changed = true;
                    (k.to_string(), "[redacted]".to_string())
                } else {
                    (k.to_string(), v.to_string())
                }
            })
            .collect();
        if !changed {
            return raw.to_string();
        }
        parsed
            .query_pairs_mut()
            .clear()
            .extend_pairs(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        parsed.to_string()
    }

    fn ensure_daemon_running(state: &ShellState) -> Result<()> {
        let mut guard = state
            .daemon
            .lock()
            .map_err(|_| anyhow!("daemon mutex poisoned"))?;
        let need_start = match guard.as_mut() {
            Some(handle) => match handle.child.try_wait() {
                Ok(Some(_)) => true,
                Ok(None) => false,
                Err(_) => true,
            },
            None => true,
        };
        if need_start {
            *guard = Some(spawn_daemon(state)?);
            log_line(&state.paths.log_file, "daemon spawned");
        }
        Ok(())
    }

    fn start_or_restart_daemon(state: &ShellState) -> Result<()> {
        stop_daemon(state);
        clear_ipc_negotiation(state);
        ensure_daemon_running(state)
    }

    fn stop_daemon(state: &ShellState) {
        if let Ok(mut guard) = state.daemon.lock() {
            if let Some(mut handle) = guard.take() {
                // Closing the job handle kills all processes in the job (daemon + cloudflared).
                if !handle.job_handle.is_null() {
                    unsafe {
                        windows_sys::Win32::Foundation::CloseHandle(handle.job_handle);
                    }
                    handle.job_handle = std::ptr::null_mut();
                }
                let _ = handle.child.kill();
                let _ = handle.child.wait();
            }
        }
    }

    fn spawn_daemon(state: &ShellState) -> Result<DaemonHandle> {
        let paths = &state.paths;
        let daemon_log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&paths.daemon_log_file)
            .context("open daemon log")?;
        let daemon_log_err = daemon_log.try_clone().context("clone daemon log handle")?;
        let mut cmd = Command::new(&paths.llmhotspot_exe);
        cmd.current_dir(&paths.root)
            .arg("--db-path")
            .arg(&paths.db_path)
            .arg("serve")
            .arg("--listen")
            .arg(&state.env.daemon_listen)
            .arg("--no-tray")
            .env("LLMHOTSPOT_OPEN_CONSOLE_ON_START", "0")
            .env("LLMHOTSPOT_TRAY_HOST_MODE", "shell_host")
            .env("LLMHOTSPOT_TRAY_HOST_PID", std::process::id().to_string())
            .env("LLMHOTSPOT_TUNNEL_RUNTIME", "1")
            .env("LLMHOTSPOT_RUNTIME_IPC_SECRET", &state.ipc_secret)
            .env("LLMHOTSPOT_TUNNEL_BROKER_URL", &state.env.tunnel_broker_url)
            .stdout(Stdio::from(daemon_log))
            .stderr(Stdio::from(daemon_log_err));
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
        let child = cmd.spawn().context("spawn llmhotspot daemon")?;

        // Create a Win32 Job Object so that when the shell exits (or we close the handle),
        // all processes in the job — including grandchildren like cloudflared — are killed.
        let job_handle = unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if !job.is_null() {
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const std::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                // Open a process handle for the child and assign it to the job.
                let proc_handle = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, child.id());
                if !proc_handle.is_null() {
                    AssignProcessToJobObject(job, proc_handle);
                    windows_sys::Win32::Foundation::CloseHandle(proc_handle);
                }
            }
            job
        };

        Ok(DaemonHandle { child, job_handle })
    }

    fn write_runtime_snapshot(state: &ShellState) -> Result<()> {
        let daemon_pid = state
            .daemon
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|h| h.child.id()));
        let local_url = command_local_url(state);
        let snapshot = RuntimeSnapshot {
            shell_pid: std::process::id(),
            daemon_pid,
            local_url,
            updated_unix: now_unix(),
        };
        let raw = serde_json::to_string_pretty(&snapshot)?;
        fs::write(&state.paths.runtime_file, raw)?;
        Ok(())
    }

    fn log_line(path: &Path, msg: &str) {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "[{}] {}", now_unix(), msg);
        }
    }

    fn now_unix() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    #[cfg(test)]
    mod tests {
        use super::{jittered_backoff_ms, should_fallback, IpcCallFailureReason, IpcOperation};

        #[test]
        fn sensitive_ipc_operation_never_fallbacks() {
            assert!(!should_fallback(
                IpcOperation::ConsoleAdminKey,
                IpcCallFailureReason::CapabilityMissing
            ));
            assert!(!should_fallback(
                IpcOperation::ConsoleAdminKey,
                IpcCallFailureReason::TransportDown
            ));
        }

        #[test]
        fn non_sensitive_ipc_operations_allow_fallbacks() {
            assert!(should_fallback(
                IpcOperation::Status,
                IpcCallFailureReason::ProtocolMismatch
            ));
            assert!(should_fallback(
                IpcOperation::ConsoleUrl,
                IpcCallFailureReason::ServerReject
            ));
        }

        #[test]
        fn backoff_jitter_respects_bounds() {
            let base = 15_000u64;
            let v = jittered_backoff_ms(base, 3);
            assert!(v >= 1_000);
            assert!(v <= 300_000);
        }
    }
}
