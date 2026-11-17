#![cfg(unix)]
use super::*;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{fs, os::unix::fs::PermissionsExt, time::Instant};
pub(crate) fn setup(
    scale: f32,
    extension: &str,
    suggestion: &str,
) -> (tempfile::TempDir, PathBuf, Harness<'static, CodisseuApp>) {
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("server.py");
    fs::write(
        &program,
        include_str!("../../codisseu-ai/tests/fixtures/copilot_server.py"),
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(
        program.with_extension("json"),
        serde_json::json!({"suggestion":suggestion,"old_delay":0.08,"delay":0.04}).to_string(),
    )
    .unwrap();
    let file = dir.path().join(format!("sample.{extension}"));
    fs::write(&file, "// owned fixture\n").unwrap();
    let mut s = AppService::new(dir.path().join("profile/session.json"));
    s.dispatch(Command::OpenFile(file.clone())).unwrap();
    s.dispatch(Command::SetAutoSave {
        enabled: false,
        delay_ms: 1000,
    })
    .unwrap();
    s.dispatch(Command::SetCopilotProgram(program.display().to_string()))
        .unwrap();
    s.dispatch(Command::SetCopilotEnabled(true)).unwrap();
    let mut app = CodisseuApp::new(s);
    app.show_project = false;
    app.show_database = false;
    app.show_activity = false;
    let mut h = Harness::builder()
        .with_size([1100., 780.])
        .with_pixels_per_point(scale)
        .with_max_steps(40)
        .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), app);
    wait(&mut h, |a| {
        a.service.active_document().is_some()
            && a.copilot_ui
                .client
                .as_ref()
                .is_some_and(|c| c.authenticated)
    });
    h.get_by_role(Role::MultilineTextInput).focus();
    h.step();
    h.key_press_modifiers(Modifiers::CTRL, Key::End);
    h.step();
    (dir, file, h)
}
pub(crate) fn wait(h: &mut Harness<'_, CodisseuApp>, condition: impl Fn(&CodisseuApp) -> bool) {
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        h.step();
        if condition(h.state()) {
            h.step();
            return;
        }
        assert!(
            Instant::now() < end,
            "Copilot UI timeout: {:?} {:?}",
            h.state().copilot_ui.error,
            h.state()
                .copilot_ui
                .client
                .as_ref()
                .and_then(|c| c.error.clone())
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
pub(crate) fn request(h: &mut Harness<'_, CodisseuApp>) {
    h.key_press_modifiers(Modifiers::ALT, Key::Backslash);
    h.step();
    wait(h, |a| {
        a.copilot_ui
            .client
            .as_ref()
            .is_some_and(|c| c.suggestion.is_some())
    });
}
#[test]
fn copilot_ghost_tab_undo_escape_and_any_text_file_use_real_editor_transactions() {
    for scale in [1., 1.25] {
        for extension in ["php", "sql", "md", "js", "ts", "unknown"] {
            let (_dir, file, mut h) = setup(scale, extension, "return 42;");
            let before = h
                .state()
                .service
                .active_document()
                .unwrap()
                .text()
                .to_owned();
            request(&mut h);
            assert!(h.query_by_label("Sugestão Copilot: return 42;").is_some());
            assert_eq!(h.state().service.active_document().unwrap().text(), before);
            assert_eq!(fs::read_to_string(&file).unwrap(), before);
            if extension == "php"
                && let Some(out) = std::env::var_os("CODISSEU_COPILOT_CAPTURES")
            {
                fs::create_dir_all(&out).unwrap();
                h.render()
                    .unwrap()
                    .save(PathBuf::from(out).join(format!("ghost-{scale}.png")))
                    .unwrap();
            }
            h.key_press(Key::Tab);
            h.step();
            assert_eq!(
                h.state().service.active_document().unwrap().text(),
                format!("{before}return 42;")
            );
            assert_eq!(fs::read_to_string(&file).unwrap(), before);
            h.key_press_modifiers(Modifiers::CTRL, Key::Z);
            h.step();
            assert_eq!(h.state().service.active_document().unwrap().text(), before);
            request(&mut h);
            h.key_press(Key::Escape);
            h.step();
            assert!(
                h.state()
                    .copilot_ui
                    .client
                    .as_ref()
                    .unwrap()
                    .suggestion
                    .is_none()
            );
            assert_eq!(h.state().service.active_document().unwrap().text(), before);
        }
    }
}
#[test]
fn copilot_multiline_is_inline_and_insertion_preserves_existing_suffix() {
    let (_dir, file, mut h) = setup(1., "php", "return 42;\n// next line\n");
    request(&mut h);
    assert!(h.state().editor_bridge.metrics.copilot_ghost_visible);
    assert!(
        h.query_by_label("Copilot · Tab aceita · Esc descarta")
            .is_none()
    );
    if let Some(out) = std::env::var_os("CODISSEU_COPILOT_CAPTURES") {
        fs::create_dir_all(&out).unwrap();
        h.render()
            .unwrap()
            .save(PathBuf::from(out).join("multiline-preview.png"))
            .unwrap();
    }
    h.key_press(Key::Tab);
    h.step();
    let expected = "// owned fixture\nreturn 42;\n// next line\n";
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        expected
    );
    assert_eq!(fs::read_to_string(file).unwrap(), "// owned fixture\n");
}
#[test]
fn copilot_typing_debounces_and_caret_or_focus_changes_discard_pending_suggestions() {
    let (_dir, _file, mut h) = setup(1., "ts", "return 42;");
    h.event(egui::Event::Text("const ".into()));
    h.step();
    wait(&mut h, |a| {
        a.copilot_ui
            .client
            .as_ref()
            .is_some_and(|c| c.suggestion.is_some())
    });
    h.key_press(Key::ArrowLeft);
    h.step();
    assert!(
        h.state()
            .copilot_ui
            .client
            .as_ref()
            .unwrap()
            .suggestion
            .is_none()
    );
    request(&mut h);
    h.get_by_label("Copilot").click();
    wait(&mut h, |a| a.copilot_ui.settings);
    h.run_steps(5);
    assert!(h.query_by_label("Copilot · Sugestões de código").is_some());
    assert!(
        h.state()
            .copilot_ui
            .client
            .as_ref()
            .unwrap()
            .suggestion
            .is_none()
    );
    h.get_by_label("Ativar sugestões do GitHub Copilot").click();
    h.step();
    assert!(!h.state().service.editor_preferences().copilot_enabled);
    h.step();
    assert!(h.state().copilot_ui.client.is_none());
}
#[test]
fn copilot_stale_version_and_semantic_completion_never_accept_wrong_code() {
    let (_dir, _file, mut h) = setup(1., "php", "return 42;");
    request(&mut h);
    let d = h.state().service.active_document().unwrap();
    let (id, version, text) = (d.id, d.version, d.text().to_owned());
    h.state_mut()
        .service
        .dispatch(Command::Edit {
            id,
            expected_version: version,
            text: format!("{text}new"),
        })
        .unwrap();
    h.key_press(Key::Tab);
    h.step();
    assert!(
        !h.state()
            .service
            .active_document()
            .unwrap()
            .text()
            .contains("return 42;")
    );
    request(&mut h);
    let a = h.state_mut();
    a.completion_popup = Some(crate::completion::Popup::fixture(
        id,
        a.service.active_document().unwrap().version,
        a.service
            .view(a.service.active_view().unwrap())
            .unwrap()
            .selection()
            .head,
    ));
    a.service.completion.items = vec![codisseu_core::CompletionItem {
        label: "provider item".into(),
        detail: None,
        kind: None,
        filter_text: None,
        sort_text: None,
        new_text: "provider item".into(),
        text_edit: None,
        insert_edit: None,
        additional_edits: vec![],
        unsupported_reason: None,
    }];
    // The semantic popup owns Tab; Copilot must leave it untouched.
    let ctx = egui::Context::default();
    let mut input = egui::RawInput::default();
    input.events.push(egui::Event::Key {
        key: Key::Tab,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    let mut output = ctx.run_ui(input, |ui| {
        ui.ctx().memory_mut(|m| {
            m.request_focus(egui::Id::unique((
                "codisseu.editor",
                a.service.active_view().unwrap(),
            )))
        });
        assert!(!a.copilot_blocked(ui.ctx()));
        assert!(!a.copilot_shortcuts(ui.ctx()));
    });
    output.textures_delta.clear();
}

#[test]
fn copilot_midline_is_inline_and_inserts_before_existing_suffix_without_deleting_it() {
    let (_dir, _file, mut h) = setup(1., "js", "argument");
    let d = h.state().service.active_document().unwrap();
    let (id, v) = (d.id, d.version);
    h.state_mut()
        .service
        .dispatch(Command::Edit {
            id,
            expected_version: v,
            text: "call()".into(),
        })
        .unwrap();
    let v = h.state().service.active_document().unwrap().version;
    h.state_mut()
        .service
        .dispatch(Command::SetSelection {
            id,
            expected_version: v,
            selection: Selection::caret(5),
        })
        .unwrap();
    h.step();
    request(&mut h);
    assert!(h.state().editor_bridge.metrics.copilot_ghost_visible);
    assert!(
        h.query_by_label("Copilot · Tab aceita · Esc descarta")
            .is_none()
    );
    h.key_press(Key::Tab);
    h.step();
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        "call(argument)"
    );
}

#[test]
fn copilot_text_and_tab_in_one_input_batch_cannot_accept_the_previous_suggestion() {
    let (_dir, _file, mut h) = setup(1., "php", "return 42;");
    request(&mut h);
    // Harness::event processes each queued event in a separate frame. Use RawInput
    // here to exercise the native same-frame text + key batch.
    h.input_mut().events.extend([
        egui::Event::Text("x".into()),
        egui::Event::Key {
            key: Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        },
    ]);
    h.step();
    let text = h.state().service.active_document().unwrap().text();
    assert!(text.contains('x'));
    assert!(!text.contains("return 42;"));
}

#[test]
fn copilot_manual_shortcut_owns_its_native_backslash_text_event() {
    let (_dir, _file, mut h) = setup(1., "php", "return 42;");
    let before = h
        .state()
        .service
        .active_document()
        .unwrap()
        .text()
        .to_owned();
    // Native backslash text and its Alt shortcut belong to one input batch.
    h.input_mut().events.extend([
        egui::Event::ModifiersChanged(Modifiers::ALT),
        egui::Event::Key {
            key: Key::Backslash,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::ALT,
        },
        egui::Event::Text("\\".into()),
        egui::Event::Key {
            key: Key::Backslash,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::ALT,
        },
    ]);
    h.step();
    h.input_mut()
        .events
        .push(egui::Event::ModifiersChanged(Modifiers::NONE));
    wait(&mut h, |a| {
        a.copilot_ui
            .client
            .as_ref()
            .is_some_and(|c| c.suggestion.is_some())
    });
    assert_eq!(h.state().service.active_document().unwrap().text(), before);
}

#[test]
fn copilot_enter_schedules_automatic_completion_for_the_new_indented_line() {
    let (_dir, _file, mut h) = setup(1., "php", "return 42;");
    h.event(egui::Event::Text("// comment".into()));
    h.step();
    h.key_press(Key::Enter);
    h.step();
    wait(&mut h, |a| {
        a.copilot_ui
            .client
            .as_ref()
            .is_some_and(|c| c.suggestion.is_some())
    });
    let log = h.state().service.editor_preferences().copilot_program;
    let rows = fs::read_to_string(PathBuf::from(log).with_extension("log")).unwrap();
    assert!(
        rows.lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .any(|v| v["method"] == "textDocument/inlineCompletion"
                && v["params"]["context"]["triggerKind"] == 2)
    );
}
