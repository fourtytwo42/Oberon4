use std::{thread, time::Duration};

use oberon4_auth::{AccountRegistry, InMemoryAccountMetadataStore};
use oberon4_cli::{CliApp, CliCommand};
use oberon4_core::{
    DaemonCommand, DaemonEvent, FakeChatProvider, FakeOAuthProvider, InMemorySecretStore,
    OAuthProviderKind,
};
use oberon4_daemon::{DaemonApp, DaemonIpcClient, DaemonRuntime, serve_unix_socket};
use oberon4_tui::TuiApp;

#[test]
fn end_to_end_endpoint_configuration_and_connectivity() {
    unsafe {
        std::env::set_var("OPENAI_API_KEY", "test-key");
    }
    let mut app = CliApp::default();
    let set_endpoint = app.run(CliCommand::ConfigSet {
        key: "endpoint.base_url".to_string(),
        value: "https://gateway.example.test".to_string(),
    });
    let set_model = app.run(CliCommand::ConfigSet {
        key: "endpoint.model".to_string(),
        value: "o4-mini".to_string(),
    });
    let endpoint_test = app.run(CliCommand::EndpointTest);

    assert_eq!(set_endpoint.exit_code, 0);
    assert_eq!(set_model.exit_code, 0);
    assert_eq!(endpoint_test.exit_code, 0);
    assert_eq!(endpoint_test.stdout, "endpoint test passed");
    unsafe {
        std::env::remove_var("OPENAI_API_KEY");
    }
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
        account: "acct-local-cli".to_string(),
    });
    let activate_openai = app.run(CliCommand::AccountActivate {
        provider: oberon4_core::OAuthProviderKind::OpenAi,
        account: "acct-local-cli".to_string(),
    });
    let connect_anthropic = app.run(CliCommand::AccountConnect {
        provider: oberon4_core::OAuthProviderKind::Anthropic,
    });

    assert_eq!(connect_openai.exit_code, 0);
    assert!(connect_openai.stdout.contains("auth.openai.com"));
    assert!(list_after_openai.stdout.contains("acct-local-cli"));
    assert_eq!(disable_openai.exit_code, 0);
    assert_eq!(activate_openai.exit_code, 0);
    assert_eq!(connect_anthropic.exit_code, 0);
    assert!(connect_anthropic.stdout.contains("console.anthropic.com"));
}

#[test]
fn end_to_end_cli_chat_and_tui_chat_path() {
    let socket_path =
        std::env::temp_dir().join(format!("oberon4-cli-e2e-chat-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket_path);

    unsafe {
        std::env::set_var("OBERON4_SOCKET_PATH", &socket_path);
    }

    let thread_socket = socket_path.clone();
    let handle = thread::spawn(move || {
        let app = DaemonApp::new(
            DaemonRuntime::default(),
            FakeChatProvider::default(),
            FakeOAuthProvider::new(OAuthProviderKind::OpenAi),
            FakeOAuthProvider::new(OAuthProviderKind::Anthropic),
            AccountRegistry::default(),
            InMemorySecretStore::default(),
            InMemoryAccountMetadataStore::default(),
        );
        serve_unix_socket(&thread_socket, app)
    });

    for _ in 0..20 {
        if socket_path.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }

    let mut cli = CliApp::default();
    let cli_chat = cli.run(CliCommand::Chat {
        message: "hello".to_string(),
    });

    let mut tui = TuiApp::default();
    let tui_chat = tui.handle_input("hello from tui").unwrap();

    assert_eq!(cli_chat.exit_code, 0);
    assert!(!cli_chat.stdout.trim().is_empty());
    assert_eq!(tui_chat.output, "assistant reply");
    assert_eq!(tui.transcript().len(), 2);

    let stop = DaemonIpcClient::new(&socket_path)
        .send(&DaemonCommand::Shutdown)
        .unwrap();
    match stop {
        DaemonEvent::Ack { message } => assert_eq!(message, "daemon stopped"),
        other => panic!("unexpected stop event: {other:?}"),
    }
    handle.join().unwrap().unwrap();

    unsafe {
        std::env::remove_var("OBERON4_SOCKET_PATH");
    }
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
