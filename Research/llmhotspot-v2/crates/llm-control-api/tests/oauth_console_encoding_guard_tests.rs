use std::fs;
use std::path::PathBuf;

#[test]
fn console_html_has_no_known_mojibake_markers() {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("assets");
    path.push("console.html");
    let html = fs::read_to_string(path).expect("read assets/console.html");

    for marker in [
        "\u{00C3}",
        "\u{00E2}\u{201A}\u{00AC}",
        "\u{00C3}\u{0192}",
        "\u{00C2}\u{00A7}\u{00C3}",
    ] {
        assert!(
            !html.contains(marker),
            "found mojibake marker '{marker}' in CONSOLE_HTML"
        );
    }
}
