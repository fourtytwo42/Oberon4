fn main() {
    let socket_path = oberon4_daemon::default_socket_path();
    let app = oberon4_daemon::DaemonApp::default();

    if let Err(err) = oberon4_daemon::serve_unix_socket(&socket_path, app) {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
