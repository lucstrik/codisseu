//! Real editor hit testing and native menus; the LSP peer only records origins.
use codisseu_core::{AppService, Command, Language, ProviderConfig, ProviderState, Selection};
use codisseu_ui::CodisseuApp;
use egui::{Modifiers, PointerButton, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

type H = Harness<'static, CodisseuApp>;
fn wait(h: &mut H, condition: impl Fn(&CodisseuApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        h.step();
        if condition(h.state()) {
            h.step();
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{} {:?}",
            h.state().service.status,
            h.state().service.last_error
        );
        thread::sleep(Duration::from_millis(3));
    }
}
fn fixture(ext: &str, scale: f32) -> (tempfile::TempDir, PathBuf, String, H) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    fs::create_dir(&root).unwrap();
    let text = if ext == "php" {
        "<?php\nclass Weather { public function rainfall() { return 1; } public function untouched() {} }\n$weather = new Weather();\necho '🌧 é'; $weather->rainfall();\n$weather->untouched();\n"
    } else {
        "class Weather { rainfall() { return 1; } untouched() {} }\nconst weather = new Weather();\nconsole.log('🌧 é'); weather.rainfall();\nweather.untouched();\n"
    }.to_owned();
    let path = root.join(format!("main.{ext}"));
    fs::write(&path, &text).unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service
        .dispatch(Command::SetAutoSave {
            enabled: false,
            delay_ms: 1000,
        })
        .unwrap();
    service.dispatch(Command::OpenWorkspace(root)).unwrap();
    service.dispatch(Command::OpenFile(path.clone())).unwrap();
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .with_pixels_per_point(scale)
        .build_ui_state(
            |ui, a: &mut CodisseuApp| a.show(ui),
            CodisseuApp::new(service),
        );
    wait(&mut h, |a| {
        !a.service.is_busy()
            && !a.editor_metrics().layout_pending
            && !a.editor_metrics().wrap_pending
    });
    let language = if ext == "php" {
        Language::Php
    } else {
        Language::TypeScript
    };
    h.state_mut()
        .service
        .dispatch(Command::ConfigureProviders(vec![ProviderConfig {
            language,
            program: "python3".into(),
            args: vec![
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/rename_context_server.py")
                    .display()
                    .to_string(),
                dir.path().join("wire.jsonl").display().to_string(),
            ],
        }]))
        .unwrap();
    h.state_mut()
        .service
        .dispatch(Command::StartProviders)
        .unwrap();
    wait(&mut h, |a| {
        a.service
            .language
            .statuses
            .iter()
            .any(|s| s.language == language && s.state == ProviderState::Ready)
    });
    (dir, path, text, h)
}
fn selection(h: &mut H, range: Selection) {
    let doc = h.state().service.active_document().unwrap();
    let (id, expected_version) = (doc.id, doc.version);
    h.state_mut()
        .service
        .dispatch(Command::SetSelection {
            id,
            expected_version,
            selection: range,
        })
        .unwrap();
    h.step();
    h.step();
}
fn method_point(h: &H) -> egui::Pos2 {
    let run = h
        .root()
        .children_recursive()
        .find(|node| {
            let a = node.accesskit_node();
            a.role() == Role::TextRun && a.value().is_some_and(|s| s.contains("rainfall();"))
        })
        .expect("actual source TextRun");
    let node = run.accesskit_node();
    let value = node.value().unwrap();
    let byte = value.find("rainfall();").unwrap();
    let i = value[..byte].chars().count() + 2;
    let x = node.data().character_positions().unwrap()[i]
        + node.data().character_widths().unwrap()[i] * 0.25;
    egui::pos2(run.rect().min.x + x, run.rect().center().y)
}
fn context(h: &mut H) {
    let point = method_point(h);
    h.hover_at(point);
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: point,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    h.run();
    h.get_by_label("Refatorar").click();
    h.run();
}
#[test]
fn context_method_origin_survives_broad_selection_and_unicode_at_both_scales() {
    for ext in ["php", "js", "ts"] {
        for scale in [1., 1.25] {
            let (dir, path, text, mut h) = fixture(ext, scale);
            let selected = Selection {
                anchor: 0,
                head: text.chars().count(),
            };
            selection(&mut h, selected);
            context(&mut h);
            assert_eq!(
                h.state().service.active_document().unwrap().selection(),
                selected,
                "right click preserves copy selection"
            );
            h.get_by_label("Renomear símbolo…    Shift+F6").click();
            wait(&mut h, |a| a.service.rename.is_prepared());
            assert_eq!(
                h.state().service.rename.placeholder.as_deref(),
                Some("rainfall"),
                "{ext} {scale}"
            );
            let wire = fs::read_to_string(dir.path().join("wire.jsonl")).unwrap();
            let prepare: serde_json::Value = wire
                .lines()
                .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
                .find(|v| v["method"] == "textDocument/prepareRename")
                .unwrap();
            let byte = text.rfind("rainfall();").unwrap() + 2;
            let before = &text[..byte];
            assert_eq!(
                prepare["params"]["position"],
                serde_json::json!({"line":before.matches('\n').count(), "character":before.rsplit('\n').next().unwrap().encode_utf16().count()})
            );
            assert_eq!(fs::read_to_string(&path).unwrap(), text);
            assert_eq!(h.state().service.active_document().unwrap().text(), text);
            h.state_mut()
                .service
                .dispatch(Command::StopProviders)
                .unwrap();
        }
    }
}
#[test]
fn context_action_disables_after_source_snapshot_changes() {
    let (_dir, _path, text, mut h) = fixture("ts", 1.);
    context(&mut h);
    let doc = h.state().service.active_document().unwrap();
    let (id, expected_version) = (doc.id, doc.version);
    h.state_mut()
        .service
        .dispatch(Command::Edit {
            id,
            expected_version,
            text: format!("{text}// changed\n"),
        })
        .unwrap();
    h.step();
    assert!(
        h.get_by_label("Renomear símbolo…    Shift+F6")
            .accesskit_node()
            .is_disabled()
    );
    assert!(!h.state().service.rename.is_open());
    h.state_mut()
        .service
        .dispatch(Command::StopProviders)
        .unwrap();
}

#[test]
fn inline_rename_keeps_the_field_on_its_actual_wrapped_row() {
    let (_dir, path, disk, mut h) = fixture("ts", 1.25);
    let text = format!(
        "class Weather {{ rainfall() {{ return 1; }} untouched() {{}} }} // {}\n",
        "long context ".repeat(40)
    );
    let doc = h.state().service.active_document().unwrap();
    let (id, expected_version) = (doc.id, doc.version);
    h.state_mut()
        .service
        .dispatch(Command::Edit {
            id,
            expected_version,
            text: text.clone(),
        })
        .unwrap();
    h.set_size(egui::vec2(980., 760.));
    wait(&mut h, |a| {
        !a.editor_metrics().layout_pending && !a.editor_metrics().wrap_pending
    });
    assert!(h.state().editor_metrics().wrapped_lines > 0);
    h.get_by_role(Role::MultilineTextInput).focus();
    selection(&mut h, Selection::caret(text.find("rainfall").unwrap() + 2));
    h.key_press_modifiers(Modifiers::SHIFT, egui::Key::F6);
    wait(&mut h, |a| a.service.rename.is_prepared());
    h.run_steps(8);
    assert!(
        h.get_by_label("Renomear símbolo e seus usos para:")
            .is_focused()
    );
    assert!(
        h.state().service.rename.is_open(),
        "offscreen continuation must not cancel the first row field"
    );
    assert_eq!(h.state().service.active_document().unwrap().text(), text);
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(!h.state().service.rename.is_open());
    assert_eq!(h.state().service.active_document().unwrap().text(), text);
    assert_eq!(fs::read_to_string(path).unwrap(), disk);
    h.state_mut()
        .service
        .dispatch(Command::StopProviders)
        .unwrap();
}
