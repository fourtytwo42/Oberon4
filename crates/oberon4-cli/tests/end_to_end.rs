use oberon4_cli::{CliApp, CliCommand, run_cli};
use oberon4_tui::TuiApp;

fn run(args: &[&str]) -> oberon4_cli::CliOutput {
    let owned: Vec<String> = args.iter().map(|value| value.to_string()).collect();
    run_cli(&owned)
}

#[test]
fn end_to_end_endpoint_configuration_and_connectivity() {
    let set_endpoint = run(&[
        "config",
        "set",
        "endpoint.base_url",
        "https://gateway.example.test",
    ]);
    let set_model = run(&["config", "set", "endpoint.model", "o4-mini"]);
    let endpoint_test = run(&["endpoint", "test"]);

    assert_eq!(set_endpoint.exit_code, 0);
    assert_eq!(set_model.exit_code, 0);
    assert_eq!(endpoint_test.exit_code, 0);
    assert_eq!(endpoint_test.stdout, "endpoint test passed");
}

#[test]
fn end_to_end_oauth_account_connect_and_switch() {
    let mut app = CliApp::default();
    let connect_openai = app.run(CliCommand::AccountConnect {
        provider: oberon4_core::OAuthProviderKind::OpenAi,
    });
    let list_after_openai = app.run(CliCommand::AccountList);
    let disable_openai = app.run(CliCommand::AccountDisable {
        provider: oberon4_core::OAuthProviderKind::OpenAi,
        account: "openai-default".to_string(),
    });
    let activate_openai = app.run(CliCommand::AccountActivate {
        provider: oberon4_core::OAuthProviderKind::OpenAi,
        account: "openai-default".to_string(),
    });
    let connect_anthropic = app.run(CliCommand::AccountConnect {
        provider: oberon4_core::OAuthProviderKind::Anthropic,
    });

    assert_eq!(connect_openai.exit_code, 0);
    assert!(connect_openai.stdout.contains("auth.openai.com"));
    assert!(list_after_openai.stdout.contains("openai-default"));
    assert_eq!(disable_openai.exit_code, 0);
    assert_eq!(activate_openai.exit_code, 0);
    assert_eq!(connect_anthropic.exit_code, 0);
    assert!(connect_anthropic.stdout.contains("console.anthropic.com"));
}

#[test]
fn end_to_end_cli_chat_and_tui_chat_path() {
    let cli_chat = run(&["chat", "--message", "hello"]);

    let mut tui = TuiApp::default();
    let tui_chat = tui.handle_input("hello from tui").unwrap();

    assert_eq!(cli_chat.exit_code, 0);
    assert_eq!(cli_chat.stdout, "pong");
    assert_eq!(tui_chat.output, "assistant reply");
    assert_eq!(tui.transcript().len(), 2);
}

#[test]
fn end_to_end_session_save_and_load() {
    let mut tui = TuiApp::default();
    let _ = tui
        .handle_command(oberon4_tui::TuiCommand::System("be concise".to_string()))
        .unwrap();
    let _ = tui.handle_input("save this conversation").unwrap();
    let save = tui
        .handle_command(oberon4_tui::TuiCommand::Save("release-smoke".to_string()))
        .unwrap();
    let _ = tui.handle_command(oberon4_tui::TuiCommand::Clear).unwrap();
    let load = tui
        .handle_command(oberon4_tui::TuiCommand::Load("release-smoke".to_string()))
        .unwrap();

    assert_eq!(save.output, "saved session release-smoke");
    assert_eq!(load.output, "loaded session release-smoke");
    assert_eq!(tui.transcript().len(), 3);
    assert_eq!(tui.transcript()[0].content, "be concise");
}
