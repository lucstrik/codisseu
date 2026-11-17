use codisseu_core::{AppService, Command, Language, ProviderConfig, ProviderState};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};
fn wait(ui: &mut Harness<'_, CodisseuApp>, f: impl Fn(&CodisseuApp) -> bool) {
    let end = Instant::now() + Duration::from_secs(45);
    loop {
        ui.step();
        if f(ui.state()) {
            ui.step();
            return;
        }
        assert!(
            Instant::now() < end,
            "{} {:?}",
            ui.state().service.status,
            ui.state().service.rename.error
        );
        thread::sleep(Duration::from_millis(3));
    }
}
fn idle(ui: &mut Harness<'_, CodisseuApp>) {
    wait(ui, |a| !a.service.is_busy());
}
fn focus(ui: &mut Harness<'_, CodisseuApp>) {
    let p = ui.get_by_role(Role::MultilineTextInput).rect().left_top() + egui::vec2(90., 10.);
    ui.hover_at(p);
    for pressed in [true, false] {
        ui.event(egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    ui.step();
}
fn request(ui: &mut Harness<'_, CodisseuApp>) {
    focus(ui);
    ui.key_press_modifiers(Modifiers::CTRL, Key::Home);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::F);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    ui.event(egui::Event::Text("rainfall".into()));
    wait(ui, |a| {
        a.service
            .search
            .as_ref()
            .is_some_and(|r| r.query == "rainfall" && !r.ranges.is_empty())
    });
    ui.key_press(Key::Escape);
    ui.key_press(Key::ArrowLeft);
    ui.step();
    ui.key_press_modifiers(Modifiers::SHIFT, Key::F6);
    wait(ui, |a| a.service.rename.is_prepared());
    // Inplace geometry is produced by the editor after preparation. Wait for
    // that real widget, then use the second Shift+F6 to request a preview.
    for _ in 0..8 {
        ui.step();
        if ui
            .query_by_label("Renomear símbolo e seus usos para:")
            .is_some_and(|field| field.is_focused())
        {
            break;
        }
    }
    assert!(
        ui.get_by_label("Renomear símbolo e seus usos para:")
            .is_focused()
    );
    ui.key_press_modifiers(Modifiers::SHIFT, Key::F6);
    ui.step();
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    ui.event(egui::Event::Text("forecast".into()));
    ui.step();
    ui.get_by_label("Preview").click();
    wait(ui, |a| {
        !a.service.rename.files.is_empty() && !a.service.rename.is_pending()
    });
}
#[test]
fn rename_preview_cancel_apply_grouped_history_and_save_use_widgets() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    let source = root.join("main.ts");
    let target = root.join("target.ts");
    let outside = root.join("unused.ts");
    let original_source = "export class Weather { rainfall() { return 'rainfall'; } }\nconst weather = new Weather();\nconsole.log('🌧 é'); weather.rainfall();";
    let original_target =
        "import { Weather } from './main';\nconst weather = new Weather();\nweather.rainfall();";
    let dirty_source = format!("{original_source} // dirty");
    let dirty_target = format!("{original_target} // dirty");
    fs::write(&source, original_source).unwrap();
    fs::write(&target, original_target).unwrap();
    fs::write(&outside, "unused").unwrap();
    let mut s = AppService::new(root.join("session.json"));
    s.dispatch(Command::SetAutoSave {
        enabled: false,
        delay_ms: 1000,
    })
    .unwrap();
    s.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    let mut ui = Harness::builder()
        .with_size([1280., 800.])
        .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), CodisseuApp::new(s));
    idle(&mut ui);
    let mut ids = Vec::new();
    for (path, text) in [
        (&target, dirty_target.as_str()),
        (&source, dirty_source.as_str()),
    ] {
        ui.state_mut()
            .service
            .dispatch(Command::OpenFile(path.clone()))
            .unwrap();
        idle(&mut ui);
        focus(&mut ui);
        ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
        ui.event(egui::Event::Text(text.into()));
        idle(&mut ui);
        ids.push(ui.state().service.active.unwrap());
    }
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rename_methods_server.py");
    ui.state_mut()
        .service
        .dispatch(Command::ConfigureProviders(vec![ProviderConfig {
            language: Language::TypeScript,
            program: "python3".into(),
            args: vec![
                script.display().to_string(),
                "safe".into(),
                root.join("wire.jsonl").display().to_string(),
                source.display().to_string(),
                target.display().to_string(),
                outside.display().to_string(),
            ],
        }]))
        .unwrap();
    ui.state_mut()
        .service
        .dispatch(Command::StartProviders)
        .unwrap();
    wait(&mut ui, |a| {
        a.service
            .language
            .statuses
            .iter()
            .any(|s| s.state == ProviderState::Ready)
            && !a.service.is_busy()
    });
    request(&mut ui);
    assert_eq!(ui.query_all_by_role(Role::ListBoxOption).count(), 3);
    assert!(
        ui.state().service.rename.can_apply(),
        "{} {:?}",
        ui.state().service.status,
        ui.state().service.rename.validation
    );
    assert!(
        ui.get_by_label("Verificação local: chamadas e receptores conhecidos.")
            .rect()
            .width()
            > 100.
    );
    let row = ui
        .query_all_by_role(Role::ListBoxOption)
        .next()
        .unwrap()
        .rect();
    let point = egui::pos2(160., row.center().y);
    ui.hover_at(point);
    // The native backend may coalesce the click and following arrow in one frame.
    for pressed in [true, false] {
        ui.input_mut().events.push(egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    for pressed in [true, false] {
        ui.input_mut().events.push(egui::Event::Key {
            key: Key::ArrowDown,
            physical_key: Some(Key::ArrowDown),
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
    }
    ui.step();
    ui.step();
    assert!(
        ui.query_all_by_role(Role::ListBoxOption)
            .nth(1)
            .unwrap()
            .is_focused()
    );
    assert_eq!(
        ui.query_all_by_role(Role::ListBoxOption)
            .filter(|n| n.is_focused())
            .count(),
        1
    );
    ui.get_by_label("Após renomear").click();
    ui.step();
    ui.get_by_label("Cancelar").click();
    idle(&mut ui);
    assert_eq!(
        ui.state()
            .service
            .documents
            .iter()
            .find(|d| d.id == ids[0])
            .unwrap()
            .text(),
        dirty_target
    );
    assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
    // Explicitly reopen the activity panel after cancellation: an unrelated
    // error no longer opens panels as a side effect. The idle preview must
    // not claim a running semantic verification.
    ui.state_mut().service.last_error = Some("unrelated error".into());
    if ui.query_by_label("Exibir").is_none() {
        ui.get_by_label("Menu principal").click();
        ui.run();
    }
    ui.get_by_label("Exibir").click();
    ui.run();
    ui.get_by_label("Painel de atividade").click();
    ui.run();
    assert!(ui.get_by_label("Nenhum rename preparado.").rect().width() > 20.);
    ui.state_mut().service.last_error = None;
    request(&mut ui);
    ui.get_by_label("Refatorar").click();
    idle(&mut ui);
    assert_eq!(
        ui.state()
            .service
            .documents
            .iter()
            .find(|d| d.id == ids[0])
            .unwrap()
            .text(),
        dirty_target.replace("rainfall()", "forecast()")
    );
    assert_eq!(
        ui.state()
            .service
            .documents
            .iter()
            .find(|d| d.id == ids[1])
            .unwrap()
            .text(),
        dirty_source.replace("rainfall()", "forecast()")
    );
    assert_eq!(fs::read_to_string(&source).unwrap(), original_source);
    assert_eq!(fs::read_to_string(&target).unwrap(), original_target);
    focus(&mut ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    idle(&mut ui);
    assert_eq!(
        ui.state()
            .service
            .documents
            .iter()
            .find(|d| d.id == ids[0])
            .unwrap()
            .text(),
        dirty_target
    );
    ui.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
    idle(&mut ui);
    assert_eq!(
        ui.state()
            .service
            .documents
            .iter()
            .find(|d| d.id == ids[0])
            .unwrap()
            .text(),
        dirty_target.replace("rainfall()", "forecast()")
    );
    ui.key_press_modifiers(Modifiers::COMMAND, Key::S);
    idle(&mut ui);
    assert_eq!(
        fs::read_to_string(&source).unwrap(),
        dirty_source.replace("rainfall()", "forecast()")
    );
    ui.state_mut()
        .service
        .dispatch(Command::StopProviders)
        .unwrap();
    idle(&mut ui);
}

#[test]
fn semantic_conflict_in_dirty_buffer_blocks_apply_and_allows_correcting_name() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    let source = root.join("main.ts");
    let target = root.join("target.ts");
    let outside = root.join("unused.ts");
    let original_source = "export class Weather { rainfall() { return 'rainfall'; } }\nconst weather = new Weather();\nconsole.log('🌧 é'); weather.rainfall();";
    let original_target =
        "import { Weather } from './main';\nconst weather = new Weather();\nweather.rainfall();";
    let dirty_source =
        original_source.replace(" } }", " } forecast() { return 'sun'; } } // dirty");
    let dirty_target = format!("{original_target} // dirty");
    fs::write(&source, original_source).unwrap();
    fs::write(&target, original_target).unwrap();
    fs::write(&outside, "unused").unwrap();
    let mut s = AppService::new(root.join("session.json"));
    s.dispatch(Command::SetAutoSave {
        enabled: false,
        delay_ms: 1000,
    })
    .unwrap();
    s.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    let mut ui = Harness::builder()
        .with_size([1280., 800.])
        .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), CodisseuApp::new(s));
    idle(&mut ui);
    let mut ids = Vec::new();
    for (path, text) in [
        (&target, dirty_target.as_str()),
        (&source, dirty_source.as_str()),
    ] {
        ui.state_mut()
            .service
            .dispatch(Command::OpenFile(path.clone()))
            .unwrap();
        idle(&mut ui);
        focus(&mut ui);
        ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
        ui.event(egui::Event::Text(text.into()));
        idle(&mut ui);
        ids.push(ui.state().service.active.unwrap());
    }
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rename_methods_server.py");
    ui.state_mut()
        .service
        .dispatch(Command::ConfigureProviders(vec![ProviderConfig {
            language: Language::TypeScript,
            program: "python3".into(),
            args: vec![
                script.display().to_string(),
                "safe".into(),
                root.join("wire.jsonl").display().to_string(),
                source.display().to_string(),
                target.display().to_string(),
                outside.display().to_string(),
            ],
        }]))
        .unwrap();
    ui.state_mut()
        .service
        .dispatch(Command::StartProviders)
        .unwrap();
    wait(&mut ui, |a| {
        a.service
            .language
            .statuses
            .iter()
            .any(|s| s.state == ProviderState::Ready)
            && !a.service.is_busy()
    });

    request(&mut ui);
    assert!(ui.state().service.rename.semantic_blocked());
    assert!(!ui.state().service.rename.can_apply());
    assert_eq!(
        ui.state()
            .service
            .rename
            .validation
            .as_ref()
            .unwrap()
            .conflicts
            .len(),
        1
    );
    assert!(ui.get_by_label("Aplicação bloqueada").rect().width() > 20.);
    assert!(
        ui.state_mut()
            .service
            .dispatch(Command::ApplyRename)
            .is_err()
    );
    let before: Vec<_> = ui
        .state()
        .service
        .documents
        .iter()
        .map(|d| (d.id, d.version, d.text().to_owned()))
        .collect();
    // A read-only code preview cannot accept typing or Enter as an apply action.
    ui.hover_at(egui::pos2(600., 400.));
    ui.event(egui::Event::PointerButton {
        pos: egui::pos2(600., 400.),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    ui.event(egui::Event::PointerButton {
        pos: egui::pos2(600., 400.),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    ui.event(egui::Event::Text("cannot edit".into()));
    ui.key_press(Key::Enter);
    ui.step();
    assert_eq!(
        before,
        ui.state()
            .service
            .documents
            .iter()
            .map(|d| (d.id, d.version, d.text().to_owned()))
            .collect::<Vec<_>>()
    );
    assert_eq!(fs::read_to_string(&source).unwrap(), original_source);
    assert_eq!(fs::read_to_string(&target).unwrap(), original_target);
    ui.query_all_by_label("Cancelar")
        .find(|n| (650. ..700.).contains(&n.rect().center().y))
        .unwrap()
        .click();
    idle(&mut ui);
    for _ in 0..3 {
        ui.step();
    }
    assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
    assert_eq!(
        before,
        ui.state()
            .service
            .documents
            .iter()
            .map(|d| (d.id, d.version, d.text().to_owned()))
            .collect::<Vec<_>>()
    );
    request(&mut ui);
    let point = ui.get_by_label("Alterar nome").rect().center();
    ui.hover_at(point);
    for pressed in [true, false] {
        ui.event(egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    for _ in 0..3 {
        ui.step();
    }
    assert!(
        ui.get_by_label("Renomear símbolo e seus usos para:")
            .is_focused()
    );
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    ui.event(egui::Event::Text("precipitation".into()));
    ui.step();
    ui.get_by_label("Preview").click();
    wait(&mut ui, |a| a.service.rename.can_apply());
    ui.get_by_label("Refatorar").click();
    idle(&mut ui);
    for (id, _, text) in &before {
        assert_eq!(
            ui.state()
                .service
                .documents
                .iter()
                .find(|d| d.id == *id)
                .unwrap()
                .text(),
            text.replace("rainfall()", "precipitation()")
        );
    }
    assert_eq!(fs::read_to_string(&source).unwrap(), original_source);
    assert_eq!(fs::read_to_string(&target).unwrap(), original_target);
    focus(&mut ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    idle(&mut ui);
    for (id, _, text) in &before {
        assert_eq!(
            ui.state()
                .service
                .documents
                .iter()
                .find(|d| d.id == *id)
                .unwrap()
                .text(),
            text
        );
    }
    ui.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
    idle(&mut ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::S);
    idle(&mut ui);
    assert_eq!(
        fs::read_to_string(&source).unwrap(),
        dirty_source.replace("rainfall()", "precipitation()")
    );
    ui.state_mut()
        .service
        .dispatch(Command::StopProviders)
        .unwrap();
    idle(&mut ui);
}

#[test]
fn dirty_ancestor_conflict_is_shown_then_corrected_without_saving_context() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    let source = root.join("main.ts");
    let target = root.join("target.ts");
    let outside = root.join("unused.ts");
    let base = root.join("base.ts");
    let base_disk = "export class Base { sunshine() {} }";
    let base_dirty = "export class Base { sunshine() {} forecast() {} } // dirty base";
    fs::write(&base, base_disk).unwrap();
    let original_source = "import { Base } from './base';\nexport class Weather extends Base { rainfall() { return 'rainfall'; } }\nconst weather = new Weather();\nconsole.log('🌧 é'); weather.rainfall();";
    let original_target =
        "import { Weather } from './main';\nconst weather = new Weather();\nweather.rainfall();";
    let dirty_source = format!("{original_source} // dirty");
    let dirty_target = format!("{original_target} // dirty");
    fs::write(&source, original_source).unwrap();
    fs::write(&target, original_target).unwrap();
    fs::write(&outside, "unused").unwrap();
    let mut s = AppService::new(root.join("session.json"));
    s.dispatch(Command::SetAutoSave {
        enabled: false,
        delay_ms: 1000,
    })
    .unwrap();
    s.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    let mut ui = Harness::builder()
        .with_size([1280., 800.])
        .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), CodisseuApp::new(s));
    idle(&mut ui);
    let mut ids = Vec::new();
    for (path, text) in [
        (&base, base_dirty),
        (&target, dirty_target.as_str()),
        (&source, dirty_source.as_str()),
    ] {
        ui.state_mut()
            .service
            .dispatch(Command::OpenFile(path.clone()))
            .unwrap();
        idle(&mut ui);
        focus(&mut ui);
        ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
        ui.event(egui::Event::Text(text.into()));
        idle(&mut ui);
        ids.push(ui.state().service.active.unwrap());
    }
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rename_methods_server.py");
    ui.state_mut()
        .service
        .dispatch(Command::ConfigureProviders(vec![ProviderConfig {
            language: Language::TypeScript,
            program: "python3".into(),
            args: vec![
                script.display().to_string(),
                "safe".into(),
                root.join("wire.jsonl").display().to_string(),
                source.display().to_string(),
                target.display().to_string(),
                outside.display().to_string(),
            ],
        }]))
        .unwrap();
    ui.state_mut()
        .service
        .dispatch(Command::StartProviders)
        .unwrap();
    wait(&mut ui, |a| {
        a.service
            .language
            .statuses
            .iter()
            .any(|s| s.state == ProviderState::Ready)
            && !a.service.is_busy()
    });

    request(&mut ui);
    let validation = ui.state().service.rename.validation.as_ref().unwrap();
    assert_eq!(
        validation.scope,
        codisseu_core::ValidationScope::KnownReceiverBindings
    );
    let conflict = validation
        .conflicts
        .iter()
        .find(|c| c.kind == codisseu_core::ConflictKind::InheritedMember)
        .unwrap();
    assert_eq!(conflict.path, base);
    assert_eq!(conflict.related.as_ref().unwrap().path, source);
    assert!(
        ui.state()
            .service
            .rename
            .conflict_previews
            .iter()
            .any(|c| c.path == base && c.before.as_ref() == base_dirty)
    );
    assert!(!ui.state().service.rename.can_apply());
    assert_eq!(fs::read_to_string(&base).unwrap(), base_disk);
    let point = ui.get_by_label("Alterar nome").rect().center();
    ui.hover_at(point);
    for pressed in [true, false] {
        ui.event(egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    for _ in 0..3 {
        ui.step();
    }
    assert!(
        ui.get_by_label("Renomear símbolo e seus usos para:")
            .is_focused()
    );
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    ui.event(egui::Event::Text("precipitation".into()));
    ui.step();
    ui.get_by_label("Preview").click();
    wait(&mut ui, |a| a.service.rename.can_apply());
    assert!(
        ui.get_by_label("Verificação local: chamadas e receptores conhecidos.")
            .rect()
            .width()
            > 20.
    );
    ui.get_by_label("Refatorar").click();
    idle(&mut ui);
    assert_eq!(
        ui.state()
            .service
            .documents
            .iter()
            .find(|d| d.path == base)
            .unwrap()
            .text(),
        base_dirty
    );
    for (path, before) in [
        (&source, dirty_source.as_str()),
        (&target, dirty_target.as_str()),
    ] {
        assert_eq!(
            ui.state()
                .service
                .documents
                .iter()
                .find(|d| d.path == *path)
                .unwrap()
                .text(),
            before.replace("rainfall()", "precipitation()")
        );
    }
    assert_eq!(fs::read_to_string(&base).unwrap(), base_disk);
    assert_eq!(fs::read_to_string(&source).unwrap(), original_source);
    assert_eq!(fs::read_to_string(&target).unwrap(), original_target);
    focus(&mut ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    idle(&mut ui);
    assert_eq!(
        ui.state()
            .service
            .documents
            .iter()
            .find(|d| d.path == source)
            .unwrap()
            .text(),
        dirty_source
    );
    assert_eq!(
        ui.state()
            .service
            .documents
            .iter()
            .find(|d| d.path == base)
            .unwrap()
            .text(),
        base_dirty
    );
    ui.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
    idle(&mut ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::S);
    idle(&mut ui);
    assert_eq!(
        fs::read_to_string(&source).unwrap(),
        dirty_source.replace("rainfall()", "precipitation()")
    );
    assert_eq!(fs::read_to_string(&base).unwrap(), base_disk);
    ui.state_mut()
        .service
        .dispatch(Command::StopProviders)
        .unwrap();
    idle(&mut ui);
}

#[test]
fn complete_override_preview_applies_both_methods_and_preserves_dirty_history() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let source = root.join("main.ts");
    let target = root.join("target.ts");
    let outside = root.join("unused.txt");
    let a = "export class Weather { rainfall(): string { return 'rainfall'; } }\nnew Weather().rainfall();\n";
    let b = "import { Weather } from './main';\nclass WetWeather extends Weather { override rainfall(): string { return 'wet'; } }\nnew WetWeather().rainfall();\n";
    fs::write(&source, a).unwrap();
    fs::write(&target, b).unwrap();
    fs::write(&outside, "untouched").unwrap();
    let mut service = AppService::new(root.join("session.json"));
    service
        .dispatch(Command::SetAutoSave {
            enabled: false,
            delay_ms: 1000,
        })
        .unwrap();
    service
        .dispatch(Command::OpenWorkspace(root.clone()))
        .unwrap();
    let mut ui = Harness::builder().with_size([1280., 800.]).build_ui_state(
        |ui, a: &mut CodisseuApp| a.show(ui),
        CodisseuApp::new(service),
    );
    idle(&mut ui);
    for (path, text) in [(&target, b), (&source, a)] {
        ui.state_mut()
            .service
            .dispatch(Command::OpenFile(path.clone()))
            .unwrap();
        idle(&mut ui);
        focus(&mut ui);
        ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
        ui.event(egui::Event::Text(format!("{text}// dirty\n")));
        idle(&mut ui);
    }
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rename_methods_server.py");
    ui.state_mut()
        .service
        .dispatch(Command::ConfigureProviders(vec![ProviderConfig {
            language: Language::TypeScript,
            program: "python3".into(),
            args: vec![
                script.display().to_string(),
                "safe".into(),
                root.join("wire.jsonl").display().to_string(),
                source.display().to_string(),
                target.display().to_string(),
                outside.display().to_string(),
            ],
        }]))
        .unwrap();
    ui.state_mut()
        .service
        .dispatch(Command::StartProviders)
        .unwrap();
    wait(&mut ui, |a| {
        !a.service.is_busy()
            && a.service
                .language
                .statuses
                .iter()
                .any(|s| s.state == ProviderState::Ready)
    });
    request(&mut ui);
    assert!(ui.state().service.rename.choosing_methods());
    assert_eq!(ui.query_all_by_role(Role::CheckBox).count(), 2);
    assert!(!ui.state().service.rename.can_apply());
    ui.get_by_label("Incluir WetWeather.rainfall()").click();
    ui.step();
    assert!(!ui.state().service.rename.can_confirm_methods());
    assert!(
        ui.get_by_label("Incluir WetWeather.rainfall()")
            .is_focused()
    );
    ui.key_press(Key::Space);
    ui.step();
    assert!(ui.state().service.rename.can_confirm_methods());
    ui.key_press(Key::Space);
    ui.step();
    assert!(!ui.state().service.rename.can_confirm_methods());
    assert!(
        ui.state_mut()
            .service
            .dispatch(Command::ApplyRename)
            .is_err()
    );
    ui.get_by_label("Desmarcar todos").click();
    ui.step();
    assert!((0..2).all(|n| !ui.state().service.rename.method_selected(n)));
    ui.get_by_label("Selecionar todos").click();
    ui.step();
    assert!(ui.state().service.rename.can_confirm_methods());
    ui.key_press(Key::ArrowDown);
    ui.step();
    ui.key_press(Key::Space);
    ui.step();
    assert!(!ui.state().service.rename.can_confirm_methods());
    ui.key_press(Key::Space);
    ui.step();
    assert!(ui.state().service.rename.can_confirm_methods());
    ui.get_by_label("OK").click();
    idle(&mut ui);
    assert_eq!(ui.query_all_by_role(Role::ListBoxOption).count(), 4);
    assert!(
        ui.state().service.rename.can_apply(),
        "{:?}",
        ui.state().service.rename.validation
    );
    assert!(
        ui.get_by_label("Hierarquia conhecida: 1 par(es) de contratos verificados.")
            .rect()
            .width()
            > 100.
    );
    ui.get_by_label("Cancelar").click();
    idle(&mut ui);
    for d in &ui.state().service.documents {
        assert!(d.text().contains("rainfall()"));
        assert!(d.is_dirty());
    }
    request(&mut ui);
    ui.get_by_label("Cancelar").click();
    idle(&mut ui);
    assert!(!ui.state().service.rename.is_open());
    request(&mut ui);
    ui.key_press(Key::Enter);
    idle(&mut ui);
    ui.get_by_label("Refatorar").click();
    idle(&mut ui);
    for d in &ui.state().service.documents {
        assert!(d.text().contains("forecast()"));
        assert!(d.text().ends_with("// dirty\n"));
    }
    assert_eq!(fs::read_to_string(&source).unwrap(), a);
    assert_eq!(fs::read_to_string(&target).unwrap(), b);
    focus(&mut ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    idle(&mut ui);
    for d in &ui.state().service.documents {
        assert!(d.text().contains("rainfall()"));
        assert!(d.text().ends_with("// dirty\n"));
    }
    ui.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
    idle(&mut ui);
    for d in &ui.state().service.documents {
        assert!(d.text().contains("forecast()"));
    }
    ui.state_mut()
        .service
        .dispatch(Command::StopProviders)
        .unwrap();
    idle(&mut ui);
}
