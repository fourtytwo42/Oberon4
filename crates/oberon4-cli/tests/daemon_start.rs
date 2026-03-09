use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn temp_dir(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("oberon4-daemon-start-{name}-{nonce}"))
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
) -> String {
    let output = Command::new(cli)
        .args(args)
        .env("OBERON4_DAEMON_BIN", daemon)
        .env("OBERON4_DATA_DIR", data_dir)
        .env("OBERON4_SOCKET_PATH", socket_path)
        .output()
        .expect("cli should execute");

    assert!(
        output.status.success(),
        "cli failed for {:?}: stdout={} stderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).expect("stdout must be utf8")
}

#[test]
fn daemon_start_restarts_existing_daemon() {
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

    let first = run_cli(&cli, &daemon, &data_dir, &socket_path, &["daemon", "start"]);
    let second = run_cli(&cli, &daemon, &data_dir, &socket_path, &["daemon", "start"]);
    let status = run_cli(
        &cli,
        &daemon,
        &data_dir,
        &socket_path,
        &["daemon", "status"],
    );
    let stop = run_cli(&cli, &daemon, &data_dir, &socket_path, &["daemon", "stop"]);

    assert!(first.contains("running=true"));
    assert!(second.contains("running=true"));
    assert!(status.contains("running=true"));
    assert!(stop.contains("running=false"));

    let _ = std::fs::remove_dir_all(&data_dir);
}
