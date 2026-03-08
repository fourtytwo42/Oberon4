use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug)]
struct CliRun {
    stdout: String,
    stderr: String,
}

fn live_enabled() -> bool {
    std::env::var("OBERON4_LIVE_HOTSPOT").as_deref() == Ok("1")
}

fn normalized_base_url() -> String {
    let raw = std::env::var("OBERON4_LIVE_BASE_URL")
        .expect("OBERON4_LIVE_BASE_URL must be set for live daemon tests");
    raw.trim_end_matches('/')
        .trim_end_matches("/v1")
        .to_string()
}

fn openai_model() -> String {
    std::env::var("OBERON4_LIVE_OPENAI_MODEL").unwrap_or_else(|_| "gpt-5.1-codex-mini".to_string())
}

fn api_key_env() -> Option<String> {
    std::env::var("OBERON4_LIVE_API_KEY_ENV").ok()
}

fn temp_dir(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("oberon4-live-daemon-{name}-{nonce}"))
}

fn debug_binary(name: &str) -> PathBuf {
    let current = std::env::current_exe().expect("current exe");
    let debug_dir = current
        .parent()
        .and_then(Path::parent)
        .expect("integration test should run from target/debug/deps");
    debug_dir.join(name)
}

fn run_cli(
    cli: &Path,
    daemon: &Path,
    data_dir: &Path,
    socket_path: &Path,
    args: &[&str],
) -> CliRun {
    let output = Command::new(cli)
        .args(args)
        .env("OBERON4_DAEMON_BIN", daemon)
        .env("OBERON4_DATA_DIR", data_dir)
        .env("OBERON4_SOCKET_PATH", socket_path)
        .envs(
            api_key_env()
                .into_iter()
                .filter_map(|name| std::env::var(&name).ok().map(|value| (name, value))),
        )
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
        stdout: String::from_utf8(output.stdout).expect("stdout must be utf8"),
        stderr: String::from_utf8(output.stderr).expect("stderr must be utf8"),
    }
}

fn stage(name: &str) {
    println!("LIVE_STAGE {name}");
}

#[test]
#[ignore = "requires OBERON4_LIVE_HOTSPOT=1 and a reachable live endpoint"]
fn live_daemon_chat_round_trip() {
    if !live_enabled() {
        return;
    }

    let cli = debug_binary("oberon4-cli");
    let daemon = debug_binary("oberon4-daemon");
    assert!(cli.exists(), "cli binary missing at {}", cli.display());
    assert!(
        daemon.exists(),
        "daemon binary missing at {}",
        daemon.display()
    );

    let data_dir = temp_dir("data");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    let socket_path = data_dir.join("daemon.sock");

    stage("daemon_start");
    let status = run_cli(&cli, &daemon, &data_dir, &socket_path, &["daemon", "start"]);
    assert!(status.stdout.contains("running=true"));

    stage("config_base_url");
    let set_base = run_cli(
        &cli,
        &daemon,
        &data_dir,
        &socket_path,
        &["config", "set", "endpoint.base_url", &normalized_base_url()],
    );
    assert!(set_base.stdout.contains("set endpoint.base_url="));

    stage("config_model");
    let set_model = run_cli(
        &cli,
        &daemon,
        &data_dir,
        &socket_path,
        &["config", "set", "endpoint.model", &openai_model()],
    );
    assert!(set_model.stdout.contains("set endpoint.model="));

    if let Some(api_key_env) = api_key_env() {
        stage("config_api_key_env");
        let set_key_env = run_cli(
            &cli,
            &daemon,
            &data_dir,
            &socket_path,
            &["config", "set", "endpoint.api_key_env", &api_key_env],
        );
        assert!(set_key_env.stdout.contains("set endpoint.api_key_env="));
    }

    stage("config_timeout");
    let set_timeout = run_cli(
        &cli,
        &daemon,
        &data_dir,
        &socket_path,
        &["config", "set", "runtime.request_timeout_ms", "8000"],
    );
    assert!(
        set_timeout
            .stdout
            .contains("set runtime.request_timeout_ms=8000")
    );

    stage("config_retries");
    let set_retries = run_cli(
        &cli,
        &daemon,
        &data_dir,
        &socket_path,
        &["config", "set", "runtime.max_retries", "0"],
    );
    assert!(set_retries.stdout.contains("set runtime.max_retries=0"));

    stage("endpoint_test");
    let endpoint = run_cli(
        &cli,
        &daemon,
        &data_dir,
        &socket_path,
        &["endpoint", "test"],
    );
    assert!(
        endpoint.stdout.contains("endpoint test passed"),
        "endpoint probe stdout={} stderr={}",
        endpoint.stdout,
        endpoint.stderr
    );

    stage("chat_request");
    let chat = run_cli(
        &cli,
        &daemon,
        &data_dir,
        &socket_path,
        &["chat", "--message", "Say live daemon ok"],
    );
    assert!(
        !chat.stdout.trim().is_empty(),
        "live chat reply should not be empty"
    );

    stage("daemon_stop");
    let stop = run_cli(&cli, &daemon, &data_dir, &socket_path, &["daemon", "stop"]);
    assert!(stop.stdout.contains("running=false"));

    stage("complete");

    let _ = std::fs::remove_dir_all(&data_dir);
}
