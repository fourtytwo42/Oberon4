use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct MockHotspotServer {
    base_url: String,
    shutdown: Arc<AtomicBool>,
    shutdown_handle: Option<thread::JoinHandle<()>>,
}

impl MockHotspotServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let address = listener.local_addr().expect("local addr");
        listener
            .set_nonblocking(true)
            .expect("listener should be nonblocking");
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_signal = Arc::clone(&shutdown);

        let shutdown_handle = thread::spawn(move || {
            loop {
                if shutdown_signal.load(Ordering::Relaxed) {
                    break;
                }

                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut buffer = [0_u8; 4096];
                        let bytes_read = stream.read(&mut buffer).expect("request should read");
                        let request = String::from_utf8_lossy(&buffer[..bytes_read]).to_string();
                        if request.is_empty() {
                            continue;
                        }
                        stream
                            .flush()
                            .expect("stream should stay healthy during request parsing");

                        let (status_line, body) = if request.starts_with("GET /v1/version HTTP/1.1")
                        {
                            ("HTTP/1.1 200 OK", r#"{"version":"test","git_hash":"mock"}"#)
                        } else if request.starts_with("POST /v1/chat/completions HTTP/1.1") {
                            (
                                "HTTP/1.1 200 OK",
                                r#"{"id":"chatcmpl-test","object":"chat.completion","created":1,"model":"gpt-5.1-codex-mini","choices":[{"index":0,"message":{"role":"assistant","content":"pty smoke ok","tool_calls":[]},"finish_reason":"stop"}]}"#,
                            )
                        } else {
                            ("HTTP/1.1 404 Not Found", r#"{"error":"not found"}"#)
                        };

                        let response = format!(
                            "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        stream
                            .write_all(response.as_bytes())
                            .expect("response should write");
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            base_url: format!("http://127.0.0.1:{}", address.port()),
            shutdown,
            shutdown_handle: Some(shutdown_handle),
        }
    }
}

impl Drop for MockHotspotServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        let _ = std::net::TcpStream::connect(self.base_url.trim_start_matches("http://"));
        if let Some(handle) = self.shutdown_handle.take() {
            let _ = handle.join();
        }
    }
}

struct PtyHarness {
    child: Box<dyn portable_pty::Child + Send>,
    writer: Box<dyn Write + Send>,
    transcript: Arc<Mutex<Vec<u8>>>,
    reader_thread: Option<thread::JoinHandle<()>>,
}

impl PtyHarness {
    fn spawn(cli: &Path, data_dir: &Path, socket_path: &Path, extra_args: &[&str]) -> Self {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 30,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("pty should open");

        let mut command = CommandBuilder::new(
            cli.to_str()
                .expect("cli binary path should be valid utf-8 for PTY launch"),
        );
        command.arg("tui");
        for arg in extra_args {
            command.arg(arg);
        }
        command.cwd(repo_root());
        command.env("OBERON4_DATA_DIR", data_dir);
        command.env("OBERON4_SOCKET_PATH", socket_path);
        command.env("OBERON4_DAEMON_BIN", debug_binary("oberon4-daemon"));
        command.env("OBERON4_TUI_BIN", debug_binary("oberon4-tui"));
        command.env("OPENAI_API_KEY", "test-key");

        let child = pair.slave.spawn_command(command).expect("tui should spawn");
        drop(pair.slave);

        let writer = pair
            .master
            .take_writer()
            .expect("writer should be available");
        let mut reader = pair.master.try_clone_reader().expect("reader should clone");
        let transcript = Arc::new(Mutex::new(Vec::new()));
        let reader_transcript = Arc::clone(&transcript);
        let reader_thread = thread::spawn(move || {
            let mut chunk = [0_u8; 4096];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(bytes_read) => {
                        reader_transcript
                            .lock()
                            .expect("transcript lock")
                            .extend_from_slice(&chunk[..bytes_read]);
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });

        Self {
            child,
            writer,
            transcript,
            reader_thread: Some(reader_thread),
        }
    }

    fn send_line(&mut self, input: &str) {
        self.writer
            .write_all(input.as_bytes())
            .expect("input should write");
        self.writer.write_all(b"\r").expect("enter should write");
        self.writer.flush().expect("flush should succeed");
    }

    fn send_bytes(&mut self, input: &[u8]) {
        self.writer.write_all(input).expect("input should write");
        self.writer.flush().expect("flush should succeed");
    }

    fn wait_for_text(&self, needle: &str, timeout: Duration) {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            let snapshot = self.screen_contents();
            if snapshot.contains(needle) {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }

        panic!(
            "timed out waiting for text {:?} in screen:\n{}",
            needle,
            self.screen_contents()
        );
    }

    fn screen_contents(&self) -> String {
        let bytes = self.transcript.lock().expect("transcript lock").clone();
        let mut parser = vt100::Parser::new(30, 120, 0);
        parser.process(&bytes);
        parser.screen().contents()
    }

    fn raw_output(&self) -> String {
        String::from_utf8_lossy(&self.transcript.lock().expect("transcript lock")).into_owned()
    }

    fn wait_for_exit(&mut self) {
        let status = self.child.wait().expect("child should exit");
        assert!(status.success(), "tui exited unsuccessfully: {status:?}");
        if let Some(reader_thread) = self.reader_thread.take() {
            let _ = reader_thread.join();
        }
    }
}

impl Drop for PtyHarness {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader_thread) = self.reader_thread.take() {
            let _ = reader_thread.join();
        }
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("oberon4-tui-pty-{name}-{nonce}"))
}

fn debug_binary(name: &str) -> PathBuf {
    ensure_workspace_binaries_built();
    repo_root().join("target").join("debug").join(name)
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate manifest should live under workspace root")
        .to_path_buf()
}

fn ensure_workspace_binaries_built() {
    static BUILT: OnceLock<()> = OnceLock::new();
    BUILT.get_or_init(|| {
        let cargo_bin = std::env::var_os("CARGO")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("cargo"));
        let status = Command::new(cargo_bin)
            .args([
                "build",
                "-p",
                "oberon4-cli",
                "-p",
                "oberon4-daemon",
                "-p",
                "oberon4-tui",
            ])
            .current_dir(repo_root())
            .status()
            .expect("workspace binaries should build for PTY tests");
        assert!(
            status.success(),
            "workspace binary build failed: {status:?}"
        );
    });
}

#[derive(Debug)]
struct CliRun {
    stdout: String,
    stderr: String,
}

fn run_cli(
    cli: &Path,
    _daemon: &Path,
    data_dir: &Path,
    socket_path: &Path,
    args: &[&str],
) -> CliRun {
    let output = Command::new(cli)
        .args(args)
        .env("OBERON4_DAEMON_BIN", debug_binary("oberon4-daemon"))
        .env("OBERON4_DATA_DIR", data_dir)
        .env("OBERON4_SOCKET_PATH", socket_path)
        .env("OPENAI_API_KEY", "test-key")
        .output()
        .expect("cli should execute");

    assert!(
        output.status.success(),
        "cli failed for {:?}: stdout={} stderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    CliRun {
        stdout: String::from_utf8(output.stdout).expect("stdout must be utf-8"),
        stderr: String::from_utf8(output.stderr).expect("stderr must be utf-8"),
    }
}

fn configure_daemon(
    cli: &Path,
    daemon: &Path,
    data_dir: &Path,
    socket_path: &Path,
    base_url: &str,
) {
    let start = run_cli(cli, daemon, data_dir, socket_path, &["daemon", "start"]);
    assert!(start.stdout.contains("running=true"));

    let set_base = run_cli(
        cli,
        daemon,
        data_dir,
        socket_path,
        &["config", "set", "endpoint.base_url", base_url],
    );
    assert!(set_base.stdout.contains("set endpoint.base_url="));

    let set_model = run_cli(
        cli,
        daemon,
        data_dir,
        socket_path,
        &["config", "set", "endpoint.model", "gpt-5.1-codex-mini"],
    );
    assert!(set_model.stdout.contains("set endpoint.model="));

    let set_timeout = run_cli(
        cli,
        daemon,
        data_dir,
        socket_path,
        &["config", "set", "runtime.request_timeout_ms", "3000"],
    );
    assert!(
        set_timeout
            .stdout
            .contains("set runtime.request_timeout_ms=3000")
    );

    let set_retries = run_cli(
        cli,
        daemon,
        data_dir,
        socket_path,
        &["config", "set", "runtime.max_retries", "0"],
    );
    assert!(set_retries.stdout.contains("set runtime.max_retries=0"));

    let disable_streaming = run_cli(
        cli,
        daemon,
        data_dir,
        socket_path,
        &["config", "set", "runtime.stream_responses", "false"],
    );
    assert!(
        disable_streaming
            .stdout
            .contains("set runtime.stream_responses=false")
    );
}

#[test]
fn tui_pty_initial_render_and_quit() {
    let mock_server = MockHotspotServer::start();
    let cli = debug_binary("oberon4-cli");
    let data_dir = temp_dir("data");
    std::fs::create_dir_all(&data_dir).expect("data dir should exist");
    let socket_path = data_dir.join("daemon.sock");

    let mut harness = PtyHarness::spawn(&cli, &data_dir, &socket_path, &[]);
    harness.wait_for_text("Oberon4 TUI", Duration::from_secs(5));
    harness.wait_for_text("Inline mode.", Duration::from_secs(5));
    harness.wait_for_text("you>", Duration::from_secs(5));

    let _ = mock_server.base_url;

    harness.send_bytes(&[3]);
    harness.wait_for_exit();

    let _ = std::fs::remove_dir_all(&data_dir);
}

#[test]
fn tui_pty_command_and_prompt_flow() {
    let mock_server = MockHotspotServer::start();
    let cli = debug_binary("oberon4-cli");
    let daemon = debug_binary("oberon4-daemon");
    let data_dir = temp_dir("flow-data");
    std::fs::create_dir_all(&data_dir).expect("data dir should exist");
    let socket_path = data_dir.join("daemon.sock");

    configure_daemon(
        &cli,
        &daemon,
        &data_dir,
        &socket_path,
        &mock_server.base_url,
    );

    let mut harness = PtyHarness::spawn(&cli, &data_dir, &socket_path, &[]);
    harness.wait_for_text("Oberon4 TUI", Duration::from_secs(5));
    harness.wait_for_text("Inline mode.", Duration::from_secs(5));

    harness.send_line("/settings");
    harness.wait_for_text("Settings", Duration::from_secs(5));
    harness.wait_for_text("OpenAI OAuth", Duration::from_secs(5));

    harness.send_bytes(&[27]);
    harness.wait_for_text("you>", Duration::from_secs(5));

    harness.send_line("/status");
    harness.wait_for_text("Status", Duration::from_secs(5));
    harness.wait_for_text("endpoint:", Duration::from_secs(5));

    harness.send_bytes(&[27]);
    harness.wait_for_text("you>", Duration::from_secs(5));

    harness.send_line("/help");
    harness.wait_for_text("Help", Duration::from_secs(5));
    harness.wait_for_text("/sessions", Duration::from_secs(5));

    harness.send_bytes(&[27]);
    harness.wait_for_text("you>", Duration::from_secs(5));

    harness.send_line("say hello from pty");
    harness.wait_for_text("pty smoke ok", Duration::from_secs(5));

    harness.send_line("/save smoke");
    harness.wait_for_text("saved session smoke", Duration::from_secs(5));

    harness.send_line("/fork smoke-copy");
    harness.wait_for_text("forked session smoke-copy", Duration::from_secs(5));

    harness.send_line("/sessions");
    harness.wait_for_text("Sessions", Duration::from_secs(5));
    harness.send_bytes(b"\x1b[B");
    harness.send_bytes(b"\r");
    harness.wait_for_text("you>", Duration::from_secs(5));
    let output = harness.raw_output();
    assert!(!output.contains("command: /load"));
    assert!(!output.contains("loaded session"));

    harness.send_bytes(&[3]);
    harness.wait_for_exit();

    let stop = run_cli(&cli, &daemon, &data_dir, &socket_path, &["daemon", "stop"]);
    assert!(
        stop.stdout.contains("running=false"),
        "stderr={}",
        stop.stderr
    );

    let _ = std::fs::remove_dir_all(&data_dir);
}

#[test]
fn tui_pty_fullscreen_mode_preserves_structured_render() {
    let mock_server = MockHotspotServer::start();
    let cli = debug_binary("oberon4-cli");
    let data_dir = temp_dir("inline-data");
    std::fs::create_dir_all(&data_dir).expect("data dir should exist");
    let socket_path = data_dir.join("daemon.sock");

    let mut harness = PtyHarness::spawn(&cli, &data_dir, &socket_path, &["--fullscreen"]);
    harness.wait_for_text("Oberon4 TUI", Duration::from_secs(5));
    harness.wait_for_text("Overview", Duration::from_secs(5));
    harness.wait_for_text("workspace=Chat", Duration::from_secs(5));

    let _ = mock_server.base_url;

    harness.send_bytes(&[3]);
    harness.wait_for_exit();

    let output = harness.raw_output();
    assert!(output.contains("Oberon4 TUI"));

    let _ = std::fs::remove_dir_all(&data_dir);
}
