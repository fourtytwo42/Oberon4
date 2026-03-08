fn main() {
    let options = match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [] => oberon4_tui::TuiLaunchOptions::default(),
        [flag] if flag == "--no-alt-screen" => oberon4_tui::TuiLaunchOptions::default(),
        [flag] if flag == "--fullscreen" => oberon4_tui::TuiLaunchOptions { fullscreen: true },
        _ => {
            eprintln!("usage: oberon4-tui [--fullscreen|--no-alt-screen]");
            std::process::exit(2);
        }
    };

    if let Err(err) = oberon4_tui::run_terminal_ui_with_options(options) {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
