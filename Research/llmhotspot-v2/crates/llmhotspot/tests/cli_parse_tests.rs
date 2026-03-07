use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "llmhotspot")]
struct MirrorCli {
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true, default_value = "./llmhotspot.db")]
    db_path: String,
    #[command(subcommand)]
    command: MirrorCommands,
}

#[derive(Debug, clap::Subcommand)]
enum MirrorCommands {
    Serve,
    Status,
    Oauth,
    Keys,
    Endpoint,
    App,
    Session,
    GenerateCliDocs,
}

#[test]
fn parse_top_level_commands() {
    let cli = MirrorCli::parse_from(["llmhotspot", "--json", "status"]);
    assert!(cli.json);
    assert!(matches!(cli.command, MirrorCommands::Status));
}
