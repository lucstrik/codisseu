use codisseu_core::{AppService, Command, Selection};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn fixture(text: &str) -> (tempfile::TempDir, Harness<'static, CodisseuApp>) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("main.ts");
    fs::write(&p, text).unwrap();
    let mut s = AppService::new(dir.path().join("session.json"));
    s.dispatch(Command::OpenFile(p)).unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    while s.is_busy() {
        s.poll();
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
    }
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .with_max_steps(60)
        .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), CodisseuApp::new(s));
    h.step();
    h.get_by_role(Role::MultilineTextInput).click();
    h.step();
    (dir, h)
}
fn key(h: &mut Harness<'_, CodisseuApp>, mut m: Modifiers, k: Key) {
    if m.ctrl {
        m.command = true;
    }
    h.key_press_modifiers(m, k);
    h.step();
    h.step();
}
fn text(h: &mut Harness<'_, CodisseuApp>, text: &str) {
    h.event(egui::Event::Text(text.into()));
    h.step();
    h.step();
}
#[test]
fn native_widget_groups_click_focus_keyboard_scroll_shared_undo_and_collapse() {
    let source = "const n = 1;\n".repeat(180);
    let (_dir, mut h) = fixture(&source);
    let left = h.state().service.active_view().unwrap();
    if h.query_by_label("Exibir").is_none() {
        h.get_by_label("Menu principal").click();
        h.run();
    }
    h.get_by_label("Exibir").click();
    h.run();
    h.get_by_label("Dividir à direita").click();
    h.step();
    h.step();
    let right = h.state().service.active_view().unwrap();
    assert_ne!(left, right);
    assert_eq!(h.get_all_by_role(Role::MultilineTextInput).count(), 2);
    let divider = h
        .get_by_label("Redimensionar grupos do editor")
        .rect()
        .center();
    let width = h
        .get_all_by_role(Role::MultilineTextInput)
        .next()
        .unwrap()
        .rect()
        .width();
    h.hover_at(divider);
    h.step();
    h.drag_at(divider);
    h.step();
    h.hover_at(divider + egui::vec2(48., 0.));
    h.step();
    h.drop_at(divider + egui::vec2(48., 0.));
    h.step();
    h.step();
    let expanded = h
        .get_all_by_role(Role::MultilineTextInput)
        .next()
        .unwrap()
        .rect()
        .width();
    assert!(
        (expanded - width - 48.).abs() < 2.,
        "divider must resize real view geometry"
    );
    key(&mut h, Modifiers::CTRL, Key::End);
    assert!(h.state().editor_view_metrics()[&right].visible_start > 130);
    assert_eq!(h.state().editor_view_metrics()[&left].visible_start, 0);
    h.get_all_by_role(Role::MultilineTextInput)
        .next()
        .unwrap()
        .click();
    h.step();
    key(&mut h, Modifiers::CTRL, Key::Home);
    assert_eq!(h.state().service.active_view(), Some(left));
    text(&mut h, "(");
    assert!(
        h.state()
            .service
            .active_document()
            .unwrap()
            .text()
            .starts_with("(")
    );
    key(&mut h, Modifiers::CTRL | Modifiers::ALT, Key::Tab);
    assert_eq!(h.state().service.active_view(), Some(right));
    assert!(h.state().editor_view_metrics()[&right].visible_start > 130);
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(h.state().service.active_document().unwrap().text(), source);
    key(&mut h, Modifiers::CTRL, Key::End);
    if h.query_by_label("Exibir").is_none() {
        h.get_by_label("Menu principal").click();
        h.run();
    }
    h.get_by_label("Exibir").click();
    h.run();
    h.get_by_label("Dividir abaixo").click();
    h.step();
    h.step();
    assert_eq!(h.get_all_by_role(Role::MultilineTextInput).count(), 2);
    if h.query_by_label("Exibir").is_none() {
        h.get_by_label("Menu principal").click();
        h.run();
    }
    h.get_by_label("Exibir").click();
    h.run();
    assert!(
        h.state().editor_view_metrics()[&right].visible_end >= 181,
        "EOF remains visible after splitting below: {:?}; active {:?}; selection {:?}",
        h.state().editor_view_metrics(),
        h.state().service.active_view(),
        h.state().service.active_document().unwrap().selection()
    );
    h.get_by_label("Fechar divisão").click();
    h.step();
    h.step();
    assert_eq!(h.get_all_by_role(Role::MultilineTextInput).count(), 1);
    assert_eq!(h.state().service.documents.len(), 1);
}
#[test]
fn delimiters_backspace_wrapping_paste_ime_and_undo_use_real_widget_events() {
    let (_dir, mut h) = fixture("");
    key(&mut h, Modifiers::CTRL, Key::Home);
    text(&mut h, "(");
    assert_eq!(h.state().service.active_document().unwrap().text(), "()");
    key(&mut h, Modifiers::NONE, Key::Backspace);
    assert_eq!(h.state().service.active_document().unwrap().text(), "");
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(h.state().service.active_document().unwrap().text(), "()");
    key(&mut h, Modifiers::CTRL, Key::A);
    h.event(egui::Event::Paste("😀e\u{301}".into()));
    h.step();
    key(&mut h, Modifiers::CTRL, Key::A);
    text(&mut h, "[");
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        "[😀e\u{301}]"
    );
    assert_eq!(
        h.state().service.active_document().unwrap().selection(),
        Selection { anchor: 1, head: 4 }
    );
    key(&mut h, Modifiers::CTRL, Key::A);
    h.event(egui::Event::Paste("(".into()));
    h.step();
    assert_eq!(h.state().service.active_document().unwrap().text(), "(");
    key(&mut h, Modifiers::CTRL, Key::A);
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("{".into())));
    h.step();
    assert_eq!(h.state().service.active_document().unwrap().text(), "{");
}

#[test]
fn ligature_preference_checkbox_keyboard_modal_preserves_directed_selection_at_two_scales() {
    for scale in [1.0, 1.25] {
        let (dir, mut h) = fixture("// ffi -> e\u{301}\n");
        h.ctx.set_pixels_per_point(scale);
        h.step();
        let before = h
            .state()
            .service
            .active_document()
            .unwrap()
            .text()
            .to_owned();
        let selection = h.state().service.active_document().unwrap().selection();
        key(&mut h, Modifiers::CTRL.plus(Modifiers::ALT), Key::S);
        for label in [
            "Usar ligaduras de fonte",
            "Autoria da linha atual",
            "Anotações de autoria do arquivo",
            "Detalhes ao passar o mouse",
        ] {
            assert!(h.query_by_role_and_label(Role::CheckBox, label).is_some());
        }
        let git_before = h.state().service.editor_preferences();
        h.get_by_label("Usar ligaduras de fonte").click();
        h.step();
        h.step();
        assert!(!h.state().service.editor_preferences().ligatures);
        // Space operates the focused checkbox; editor shortcuts must not consume it.
        key(&mut h, Modifiers::NONE, Key::Space);
        assert!(h.state().service.editor_preferences().ligatures);
        key(&mut h, Modifiers::NONE, Key::Space);
        assert!(!h.state().service.editor_preferences().ligatures);
        let after = h.state().service.editor_preferences();
        assert_eq!(
            (
                after.git_current_line,
                after.git_hover,
                after.git_file_blame
            ),
            (
                git_before.git_current_line,
                git_before.git_hover,
                git_before.git_file_blame
            )
        );
        h.get_by_label("Concluído").click();
        h.step();
        h.step();
        let d = h.state().service.active_document().unwrap();
        assert_eq!(d.text(), before);
        assert_eq!(d.selection(), selection);
        assert!(!d.is_dirty());
        assert!(!d.can_undo());
        assert_eq!(
            fs::read_to_string(dir.path().join("main.ts")).unwrap(),
            before
        );
        // The menu opens the same preferences, including when the editor has focus.
        if h.query_by_label("Editar").is_none() {
            h.get_by_label("Menu principal").click();
            h.run();
        }
        h.get_by_label("Editar").click();
        h.step();
        h.step();
        h.get_by_label("Configurações…    Ctrl+Alt+S").click();
        h.step();
        h.step();
        assert!(h.query_by_label("Usar ligaduras de fonte").is_some());
        key(&mut h, Modifiers::NONE, Key::Escape);
        assert!(h.query_by_label("Usar ligaduras de fonte").is_none());
    }
}

#[test]
fn restart_restores_two_native_widgets_divider_selections_and_scroll_before_input() {
    let (_dir, mut h) = fixture(&"const n = 1;\n".repeat(240));
    if h.query_by_label("Exibir").is_none() {
        h.get_by_label("Menu principal").click();
        h.run();
    }
    h.get_by_label("Exibir").click();
    h.run();
    h.get_by_label("Dividir à direita").click();
    h.step();
    h.step();
    let right = h.state().service.active_view().unwrap();
    key(&mut h, Modifiers::CTRL, Key::End);
    key(&mut h, Modifiers::SHIFT, Key::Home);
    let expected = h.state().service.view(right).unwrap().selection();
    let scroll = h.state().editor_view_metrics()[&right].scroll_y;
    assert!(scroll > 1000.);
    let point = h
        .get_by_label("Redimensionar grupos do editor")
        .rect()
        .center();
    h.hover_at(point);
    h.step();
    h.drag_at(point);
    h.step();
    h.hover_at(point + egui::vec2(40., 0.));
    h.step();
    h.drop_at(point + egui::vec2(40., 0.));
    h.step();
    h.step();
    let split = h.state().service.editor_split();
    h.state_mut()
        .service
        .dispatch(Command::PersistSession)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().service.is_busy() {
        h.step();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    let mut restored = AppService::new(_dir.path().join("session.json"));
    restored.dispatch(Command::RestoreSession).unwrap();
    while restored.is_busy() {
        restored.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    let mut r = Harness::builder()
        .with_size([1280., 800.])
        .with_max_steps(60)
        .build_ui_state(
            |ui, a: &mut CodisseuApp| a.show(ui),
            CodisseuApp::new(restored),
        );
    for _ in 0..8 {
        r.step();
    }
    assert_eq!(r.get_all_by_role(Role::MultilineTextInput).count(), 2);
    assert_eq!(r.state().service.editor_split(), split);
    let id = r.state().service.active_view().unwrap();
    assert_eq!(r.state().service.view(id).unwrap().selection(), expected);
    let actual = r.state().editor_view_metrics()[&id].scroll_y;
    assert!(
        (actual - scroll).abs() < 1.,
        "scroll before={scroll} after={actual}"
    );
    // The first key reaches the restored view and preserves shared history.
    r.get_all_by_role(Role::MultilineTextInput)
        .nth(1)
        .unwrap()
        .click();
    r.step();
    text(&mut r, "X");
    key(&mut r, Modifiers::CTRL, Key::Z);
    assert_eq!(
        r.state().service.active_document().unwrap().text(),
        "const n = 1;\n".repeat(240)
    );
}

#[test]
fn restored_zero_scroll_does_not_reveal_an_offscreen_caret() {
    let (dir, mut h) = fixture(&"const n = 1;\n".repeat(240));
    key(&mut h, Modifiers::CTRL, Key::End);
    let id = h.state().service.active_view().unwrap();
    let expected = h.state().service.view(id).unwrap().selection();
    h.state_mut()
        .service
        .dispatch(Command::SetViewScroll {
            view: id,
            scroll: [0.; 2],
        })
        .unwrap();
    for _ in 0..5 {
        h.step();
    }
    assert_eq!(h.state().editor_metrics().visible_start, 0);
    assert_eq!(h.state().service.view(id).unwrap().selection(), expected);
    h.state_mut()
        .service
        .dispatch(Command::PersistSession)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().service.is_busy() {
        h.step();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    let mut s = AppService::new(dir.path().join("session.json"));
    s.dispatch(Command::RestoreSession).unwrap();
    while s.is_busy() {
        s.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    let mut r = Harness::builder()
        .with_size([1280., 800.])
        .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), CodisseuApp::new(s));
    for _ in 0..5 {
        r.step();
    }
    assert_eq!(r.state().editor_metrics().visible_start, 0);
    assert_eq!(
        r.state().service.active_document().unwrap().selection(),
        expected
    );
}

#[test]
fn local_tabs_native_targets_passive_close_and_last_dirty_cancel_at_two_scales() {
    for scale in [1., 1.25] {
        let (dir, mut h) = fixture("const n = 1;\n");
        h.ctx.set_pixels_per_point(scale);
        h.step();
        let left = h.state().service.active_view().unwrap();
        let original = h.state().service.active_document().unwrap().id;
        if h.query_by_label("Exibir").is_none() {
            h.get_by_label("Menu principal").click();
            h.run();
        }
        h.get_by_label("Exibir").click();
        h.run();
        h.get_by_label("Dividir à direita").click();
        h.step();
        h.step();
        let right = h.state().service.active_view().unwrap();
        key(&mut h, Modifiers::CTRL, Key::Home);
        text(&mut h, "// edited\n");
        let shared = h
            .state()
            .service
            .active_document()
            .unwrap()
            .text()
            .to_owned();
        fs::write(dir.path().join("other.php"), "<?php\n").unwrap();
        h.state_mut()
            .service
            .dispatch(Command::OpenFile(dir.path().join("other.php")))
            .unwrap();
        let end = Instant::now() + Duration::from_secs(10);
        while h.state().service.is_busy() {
            h.step();
            assert!(Instant::now() < end);
            thread::sleep(Duration::from_millis(1));
        }
        h.step();
        assert_eq!(h.state().service.view(left).unwrap().tabs(), &[original]);
        assert_eq!(h.state().service.view(right).unwrap().tabs().len(), 2);
        assert_eq!(h.get_all_by_label("main.ts •").count(), 2);
        assert_eq!(h.get_all_by_label("other.php").count(), 1);
        // The top of the actual close hit target is outside the centered icon.
        let close = h.get_all_by_label("Fechar main.ts").next().unwrap().rect();
        assert!((close.height() - 41.).abs() < 1.);
        let point = egui::pos2(close.center().x, close.top() + 2.);
        h.event(egui::Event::PointerMoved(point));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            h.step();
        }
        h.step();
        h.step();
        assert_eq!(h.state().service.active_view(), Some(right));
        assert_eq!(h.state().service.document_views().len(), 1);
        assert_eq!(h.state().service.documents.len(), 2);
        assert!(h.query_by_label("Descartar e fechar").is_none());
        assert_eq!(
            h.state()
                .service
                .documents
                .iter()
                .find(|d| d.id == original)
                .unwrap()
                .text(),
            shared
        );
        // Ctrl+W closes the selected local PHP tab; shared dirty TS survives.
        h.get_by_role(Role::MultilineTextInput).click();
        h.step();
        key(&mut h, Modifiers::CTRL, Key::W);
        assert_eq!(h.state().service.documents.len(), 1);
        assert_eq!(h.state().service.active_document().unwrap().id, original);
        key(&mut h, Modifiers::CTRL, Key::Z);
        assert_eq!(
            h.state().service.active_document().unwrap().text(),
            "const n = 1;\n"
        );
        key(&mut h, Modifiers::CTRL | Modifiers::SHIFT, Key::Z);
        assert_eq!(h.state().service.active_document().unwrap().text(), shared);
        key(&mut h, Modifiers::CTRL, Key::W);
        assert!(h.query_by_label("Descartar e fechar").is_some());
        let selection = h.state().service.active_document().unwrap().selection();
        h.get_by_label("Voltar ao editor").click();
        h.step();
        h.step();
        assert_eq!(h.state().service.active_document().unwrap().text(), shared);
        assert_eq!(
            h.state().service.active_document().unwrap().selection(),
            selection
        );
        key(&mut h, Modifiers::CTRL, Key::W);
        h.get_by_label("Descartar e fechar").click();
        h.step();
        h.step();
        assert!(h.state().service.documents.is_empty());
        assert_eq!(
            fs::read_to_string(dir.path().join("main.ts")).unwrap(),
            "const n = 1;\n"
        );
    }
}

#[test]
fn local_tab_pointer_and_keyboard_activate_only_the_target_group() {
    let (dir, mut h) = fixture("const n = 1;\n");
    let left = h.state().service.active_view().unwrap();
    fs::write(dir.path().join("other.ts"), "const other = 2;\n").unwrap();
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(dir.path().join("other.ts")))
        .unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    while h.state().service.is_busy() {
        h.step();
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
    }
    if h.query_by_label("Exibir").is_none() {
        h.get_by_label("Menu principal").click();
        h.run();
    }
    h.get_by_label("Exibir").click();
    h.run();
    h.get_by_label("Dividir à direita").click();
    h.step();
    h.step();
    let right = h.state().service.active_view().unwrap();
    let other = h.state().service.active_document().unwrap().id;
    h.get_by_label("main.ts").click();
    h.step();
    h.step();
    assert_eq!(h.state().service.active_view(), Some(left));
    assert_eq!(h.state().service.view(right).unwrap().document, other);
    // Space on the focused tab is consumed by its native button semantics.
    h.get_all_by_label("other.ts").last().unwrap().focus();
    h.step();
    key(&mut h, Modifiers::NONE, Key::Space);
    assert_eq!(h.state().service.active_view(), Some(right));
    assert_eq!(h.state().service.active_document().unwrap().id, other);
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        "const other = 2;\n"
    );
    // Closing passive main.ts leaves the active editor and selection intact.
    key(&mut h, Modifiers::CTRL, Key::Home);
    key(&mut h, Modifiers::NONE, Key::ArrowRight);
    let selection = h.state().service.active_document().unwrap().selection();
    h.get_by_label("Fechar main.ts").click();
    h.step();
    h.step();
    assert_eq!(h.state().service.active_view(), Some(right));
    assert_eq!(
        h.state().service.active_document().unwrap().selection(),
        selection
    );
    assert_eq!(h.state().service.view(left).unwrap().tabs(), &[other]);
    assert_eq!(h.state().service.documents.len(), 1);
    text(&mut h, "X");
    assert_eq!(h.state().service.active_view(), Some(right));
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        "cXonst other = 2;\n"
    );
}

#[test]
fn hovering_and_cancelling_close_preserves_passive_group_and_keyboard_input() {
    for scale in [1., 1.25] {
        let (dir, mut h) = fixture("const n = 1;\n");
        h.ctx.set_pixels_per_point(scale);
        h.step();
        let left = h.state().service.active_view().unwrap();
        if h.query_by_label("Exibir").is_none() {
            h.get_by_label("Menu principal").click();
            h.run();
        }
        h.get_by_label("Exibir").click();
        h.step();
        h.step();
        h.get_by_label("Dividir à direita").click();
        h.step();
        h.step();
        let right = h.state().service.active_view().unwrap();
        std::fs::write(dir.path().join("other.php"), "<?php\n").unwrap();
        h.state_mut()
            .service
            .dispatch(Command::OpenFile(dir.path().join("other.php")))
            .unwrap();
        let end = Instant::now() + Duration::from_secs(10);
        while h.state().service.is_busy() {
            h.step();
            assert!(Instant::now() < end);
            thread::sleep(Duration::from_millis(1));
        }
        h.step();
        h.get_all_by_label("main.ts")
            .min_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
            .unwrap()
            .click();
        h.step();
        h.step();
        key(&mut h, Modifiers::CTRL, Key::Home);
        let passive = h.state().service.view(right).unwrap().document;
        let main = h.state().service.active_document().unwrap().id;
        let version = h.state().service.active_document().unwrap().version;
        let body = h
            .get_all_by_label("main.ts")
            .max_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
            .unwrap()
            .rect()
            .center();
        h.hover_at(body);
        h.step();
        h.step();
        assert_eq!(h.state().service.active_view(), Some(left), "scale={scale}");
        assert_eq!(h.state().service.view(right).unwrap().document, passive);
        let close = h
            .get_all_by_label("Fechar main.ts")
            .max_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
            .unwrap()
            .rect()
            .center();
        h.hover_at(close);
        h.step();
        h.event(egui::Event::PointerButton {
            pos: close,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        h.step();
        let outside = egui::pos2(1240., 740.);
        h.hover_at(outside);
        h.step();
        h.event(egui::Event::PointerButton {
            pos: outside,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        h.step();
        h.step();
        assert_eq!(h.state().service.active_view(), Some(left), "scale={scale}");
        assert_eq!(h.state().service.view(right).unwrap().document, passive);
        assert_eq!(h.state().service.documents.len(), 2);
        assert_eq!(
            h.state().service.active_document().unwrap().version,
            version
        );
        assert!(!h.state().service.active_document().unwrap().can_undo());
        text(&mut h, "X");
        assert_eq!(
            h.state().service.active_document().unwrap().text(),
            "Xconst n = 1;\n"
        );
        assert_eq!(h.state().service.active_document().unwrap().id, main);
        key(&mut h, Modifiers::CTRL, Key::Z);
        assert_eq!(
            h.state().service.active_document().unwrap().text(),
            "const n = 1;\n"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("main.ts")).unwrap(),
            "const n = 1;\n"
        );
    }
}

#[test]
fn group_more_passive_cancel_orientation_and_unsplit_preserve_dirty_history() {
    for scale in [1., 1.25] {
        let source = "const n = 1;\n";
        let (_dir, mut h) = fixture(source);
        h.ctx.set_pixels_per_point(scale);
        h.step();
        h.step();
        let left = h.state().service.active_view().unwrap();
        h.get_by_label("Mais ações do grupo").focus();
        h.step();
        key(&mut h, Modifiers::NONE, Key::Enter);
        h.run();
        h.get_by_label("Dividir à direita").click();
        h.step();
        h.step();
        let right = h.state().service.active_view().unwrap();
        assert_ne!(left, right);
        text(&mut h, "dirty");
        let dirty = h
            .state()
            .service
            .active_document()
            .unwrap()
            .text()
            .to_owned();
        assert_ne!(dirty, source);
        h.get_all_by_label("Mais ações do grupo")
            .next()
            .unwrap()
            .click();
        h.run();
        assert_eq!(h.state().service.active_view(), Some(right));
        key(&mut h, Modifiers::NONE, Key::Escape);
        text(&mut h, "after");
        assert!(
            h.state()
                .service
                .active_document()
                .unwrap()
                .text()
                .contains("after")
        );
        key(&mut h, Modifiers::CTRL, Key::Z);
        assert_eq!(h.state().service.active_document().unwrap().text(), dirty);
        h.get_all_by_label("Mais ações do grupo")
            .next()
            .unwrap()
            .click();
        h.run();
        h.get_by_label("Alterar orientação da divisão").click();
        h.step();
        h.step();
        assert!(h.state().service.editor_split().horizontal);
        assert_eq!(h.state().service.active_view(), Some(right));
        assert_eq!(h.state().service.active_document().unwrap().text(), dirty);
        h.get_all_by_label("Mais ações do grupo")
            .next()
            .unwrap()
            .click();
        h.run();
        h.get_by_label("Remover divisão").click();
        h.step();
        h.step();
        assert_eq!(h.state().service.document_views().len(), 1);
        assert_eq!(h.state().service.active_view(), Some(right));
        assert_eq!(h.state().service.active_document().unwrap().text(), dirty);
        key(&mut h, Modifiers::CTRL, Key::Z);
        assert_eq!(h.state().service.active_document().unwrap().text(), source);
        key(&mut h, Modifiers::CTRL, Key::Y);
        assert_eq!(h.state().service.active_document().unwrap().text(), dirty);
    }
}
