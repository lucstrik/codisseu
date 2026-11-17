use codisseu_core::{AppService, Command, Language, ProviderConfig};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn wait(h: &mut Harness<'_, CodisseuApp>, check: impl Fn(&CodisseuApp) -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        h.step();
        if check(h.state()) {
            return;
        }
        assert!(
            Instant::now() < end,
            "{:?} {:?}",
            h.state().service.language.last_error,
            h.state().service.language.diagnostics
        );
        thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn real_transport_decorations_suspend_for_ime_and_edit_without_affecting_history_or_selection() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    let path = root.join("unused.ts");
    let source = "🌧 name\n";
    fs::write(&path, source).unwrap();
    let mut service = AppService::new(root.join("session.json"));
    service
        .dispatch(Command::OpenWorkspace(root.clone()))
        .unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
    }
    service.dispatch(Command::OpenFile(path.clone())).unwrap();
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
    }
    service
        .dispatch(Command::ConfigureProviders(vec![ProviderConfig {
            language: Language::TypeScript,
            program: "python3".into(),
            args: vec![
                format!(
                    "{}/../codisseu-language/tests/fixtures/diagnostics_server.py",
                    env!("CARGO_MANIFEST_DIR")
                ),
                root.join("messages.jsonl").to_str().unwrap().into(),
            ],
        }]))
        .unwrap();
    service.dispatch(Command::StartProviders).unwrap();
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .with_max_steps(60)
        .build_ui_state(
            |ui, a: &mut CodisseuApp| a.show(ui),
            CodisseuApp::new(service),
        );
    wait(&mut h, |a| a.editor_metrics().unnecessary_spans == 1);
    h.get_by_role(Role::MultilineTextInput).click();
    h.step();
    h.key_press_modifiers(Modifiers::COMMAND, Key::End);
    h.step();
    let original_selection = h.state().service.active_document().unwrap().selection();
    let version = h.state().service.active_document().unwrap().version;
    h.event(egui::Event::Ime(egui::ImeEvent::Preedit {
        text: "你好".into(),
        active_range_chars: Some(0..2),
    }));
    h.step();
    assert!(h.state().editor_metrics().composing);
    assert_eq!(h.state().editor_metrics().unnecessary_spans, 0);
    assert_eq!(
        h.state().service.active_document().unwrap().version,
        version
    );
    h.key_press(Key::Escape);
    h.step();
    assert_eq!(
        h.state().service.active_document().unwrap().selection(),
        original_selection
    );
    assert_eq!(h.state().editor_metrics().unnecessary_spans, 1);
    h.event(egui::Event::Text("x".into()));
    wait(&mut h, |a| {
        a.editor_metrics().unnecessary_spans == 0
            && a.service.active_document().unwrap().version > version
    });
    assert!(
        h.state()
            .service
            .active_document()
            .unwrap()
            .text()
            .ends_with("x")
    );
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.step();
    assert_eq!(h.state().service.active_document().unwrap().text(), source);
    h.check_accessibility();
    assert_eq!(fs::read_to_string(path).unwrap(), source);
    h.state_mut()
        .service
        .dispatch(Command::StopProviders)
        .unwrap();
    wait(&mut h, |a| !a.service.is_busy());
}
