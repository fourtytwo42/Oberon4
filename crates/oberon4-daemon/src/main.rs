fn main() {
    let status = oberon4_daemon::daemon_status(&oberon4_core::AppConfig::default());
    println!(
        "{}",
        status
            .data
            .unwrap_or_else(|| "daemon unavailable".to_string())
    );
}
