fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let output = oberon4_cli::run_cli(&args);
    if !output.stdout.is_empty() {
        println!("{}", output.stdout);
    }
    if !output.stderr.is_empty() {
        eprintln!("{}", output.stderr);
    }
    std::process::exit(output.exit_code);
}
