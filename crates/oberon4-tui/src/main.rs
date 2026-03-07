fn main() {
    if let Err(err) = oberon4_tui::run_terminal_ui() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
