use llm_tray::{RecordingHandler, TrayAction, TrayRuntime};
use std::sync::Arc;

#[test]
fn dispatches_expected_actions() {
    let handler = Arc::new(RecordingHandler::default());
    let runtime = TrayRuntime::new("http://127.0.0.1:8080/console", handler.clone());

    runtime.left_click();
    runtime.menu_restart();
    runtime.menu_exit();

    let actions = handler.actions();
    assert_eq!(actions[0], TrayAction::OpenConsole);
    assert_eq!(actions[1], TrayAction::RestartApp);
    assert_eq!(actions[2], TrayAction::Exit);
}
