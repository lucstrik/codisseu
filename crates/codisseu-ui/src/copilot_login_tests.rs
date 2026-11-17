#![cfg(unix)]
use super::*;
use egui_kittest::{Harness, kittest::Queryable};
use std::{fs, os::unix::fs::PermissionsExt, time::Instant};

struct LoginApp {
    app: CodisseuApp,
    opened: Vec<String>,
}
#[test]
fn copilot_continue_button_copies_code_requests_browser_and_updates_account_after_reply() {
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("peer.py");
    fs::write(
        &program,
        include_str!("../../codisseu-ai/tests/fixtures/copilot_server.py"),
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(
        program.with_extension("json"),
        serde_json::json!({"login":true,"finish_after_show_document_reply":true}).to_string(),
    )
    .unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service
        .dispatch(Command::SetCopilotProgram(program.display().to_string()))
        .unwrap();
    service.dispatch(Command::SetCopilotEnabled(true)).unwrap();
    let mut app = CodisseuApp::new(service);
    app.copilot_ui.settings = true;
    let mut h = Harness::builder()
        .with_size([1100., 850.])
        .with_max_steps(40)
        .build_ui_state(
            |ui, state: &mut LoginApp| {
                state.app.show(ui);
                state.opened.extend(ui.ctx().output(|o| {
                    o.commands
                        .iter()
                        .filter_map(|c| {
                            if let egui::OutputCommand::OpenUrl(url) = c {
                                Some(url.url.clone())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                }));
            },
            LoginApp {
                app,
                opened: vec![],
            },
        );
    let until = Instant::now() + Duration::from_secs(10);
    while !h
        .state()
        .app
        .copilot_ui
        .client
        .as_ref()
        .is_some_and(|c| c.login.is_some())
    {
        h.step();
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(5));
    }
    h.run_steps(5);
    assert!(h.state().opened.is_empty());
    h.get_by_label("Continuar no GitHub").click();
    h.step();
    while !h
        .state()
        .app
        .copilot_ui
        .client
        .as_ref()
        .is_some_and(|c| c.authenticated)
    {
        h.step();
        assert!(
            Instant::now() < until,
            "{:?}",
            h.state().app.copilot_ui.error
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    h.step();
    assert_eq!(h.state().opened, vec!["https://github.com/login/device"]);
    assert!(h.query_by_label("Conta Copilot autenticada").is_some());
    assert!(
        !h.state()
            .app
            .copilot_ui
            .client
            .as_ref()
            .unwrap()
            .login_in_progress
    );
    let records = fs::read_to_string(program.with_extension("log")).unwrap();
    assert_eq!(
        records
            .lines()
            .filter_map(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .filter(|r| r["method"] == "workspace/executeCommand")
            .count(),
        1
    );
}
