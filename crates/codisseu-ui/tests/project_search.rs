//! Real workspace files through the compact search modal and keyboard input.
use codisseu_core::{AppService, Command};
use codisseu_ui::CodisseuApp;
use egui::{Event, Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn settle(h: &mut Harness<'_, CodisseuApp>) {
    h.step();
    let end = Instant::now() + Duration::from_secs(10);
    while h.state().service.is_busy() || h.state().project_search_pending() {
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
        h.step();
    }
    h.run_steps(4);
}
fn setup() -> (tempfile::TempDir, Harness<'static, CodisseuApp>) {
    setup_at_scale(1.)
}
fn setup_at_scale(scale: f32) -> (tempfile::TempDir, Harness<'static, CodisseuApp>) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("src/ação.ts"),
        "const ação = 1;\nconst b = 2;\n",
    )
    .unwrap();
    fs::write(dir.path().join("src/other.php"), "<?php echo 1;\n").unwrap();
    let mut service = AppService::new(dir.path().join("state/session.json"));
    service
        .dispatch(Command::OpenWorkspace(dir.path().into()))
        .unwrap();
    service
        .dispatch(Command::OpenFile(dir.path().join("src/other.php")))
        .unwrap();
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .with_pixels_per_point(scale)
        .with_max_steps(30)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|_| CodisseuApp::new(service));
    settle(&mut h);
    h.ctx
        .all_styles_mut(|s| s.visuals.text_cursor.blink = false);
    h.step();
    (dir, h)
}

#[test]
fn query_hint_text_and_caret_stay_centered_at_both_scales() {
    fn text_bounds(shapes: &[egui::epaint::ClippedShape], text: &str) -> egui::Rect {
        fn find(shape: &egui::Shape, text: &str) -> Option<egui::Rect> {
            match shape {
                egui::Shape::Text(s) if s.galley.text() == text => {
                    Some(s.galley.rect.translate(s.pos.to_vec2()))
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, text)),
                _ => None,
            }
        }
        shapes.iter().find_map(|s| find(&s.shape, text)).unwrap()
    }
    for scale in [1., 1.25] {
        let (_dir, mut h) = setup_at_scale(scale);
        shift(&mut h);
        shift(&mut h);
        settle(&mut h);
        h.run_steps(20); // Finish the modal fade before inspecting/rendering it.
        let field = h.get_by_label("Pesquisar no projeto").rect();
        let hint = text_bounds(&h.output().shapes, "Search Everywhere");
        capture_if_requested(&mut h, &format!("query-empty-{scale}"));
        assert!(
            (hint.center().y - field.center().y).abs() <= 1.,
            "hint={hint:?}, field={field:?}, scale={scale}"
        );
        h.event(Event::Text("ação Ágjpq".into()));
        settle(&mut h);
        let typed = text_bounds(&h.output().shapes, "ação Ágjpq");
        capture_if_requested(&mut h, &format!("query-typed-{scale}"));
        assert!(
            (typed.center().y - field.center().y).abs() <= 1.,
            "typed={typed:?}, field={field:?}, scale={scale}"
        );
        assert!((typed.center().y - hint.center().y).abs() <= 1.);
        let caret = h
            .output()
            .shapes
            .iter()
            .find_map(|s| {
                if let egui::Shape::LineSegment { points, .. } = &s.shape {
                    let rect = egui::Rect::from_two_pos(points[0], points[1]);
                    (rect.width() < 0.1 && rect.height() > 10. && field.contains_rect(rect))
                        .then_some(rect)
                } else {
                    None
                }
            })
            .unwrap_or_else(||panic!("focused input must paint a caret inside the field: focus={}, field={field:?}, scale={scale}, shapes={:?}",h.get_by_label("Pesquisar no projeto").is_focused(),h.output().shapes.iter().filter_map(|s|if let egui::Shape::LineSegment{points,..}=&s.shape{Some(points)}else{None}).collect::<Vec<_>>()));
        assert!((caret.center().y - field.center().y).abs() <= 1.);
        // The cursor uses this same galley; selection/backspace must keep focus
        // and preserve the centered layout when the hint comes back.
        assert!(h.get_by_label("Pesquisar no projeto").is_focused());
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.key_press(Key::Backspace);
        settle(&mut h);
        let cleared = text_bounds(&h.output().shapes, "Search Everywhere");
        assert!((cleared.center().y - field.center().y).abs() <= 1.);
    }
}
fn shift(h: &mut Harness<'_, CodisseuApp>) {
    for pressed in [true, false] {
        h.event(Event::Key {
            key: Key::ShiftLeft,
            physical_key: Some(Key::ShiftLeft),
            pressed,
            repeat: false,
            modifiers: if pressed {
                Modifiers::SHIFT
            } else {
                Modifiers::NONE
            },
        });
        h.step();
    }
}
fn capture_if_requested(h: &mut Harness<'_, CodisseuApp>, name: &str) {
    if let Some(path) = std::env::var_os("CODISSEU_SEARCH_CAPTURE_DIR") {
        let path = std::path::PathBuf::from(path);
        fs::create_dir_all(&path).unwrap();
        h.render()
            .expect("WGPU test render")
            .save(path.join(format!("{name}.png")))
            .unwrap();
    }
}

#[test]
fn double_shift_opens_search_unicode_and_line_navigation_without_editing_old_document() {
    let (dir, mut h) = setup();
    h.get_by_role(Role::MultilineTextInput).focus();
    h.step();
    shift(&mut h);
    shift(&mut h);
    settle(&mut h);
    assert!(h.get_by_label("Pesquisar no projeto").is_focused());
    h.event(Event::Text("ação:2:3".into()));
    settle(&mut h);
    assert!(h.query_by_label("Abrir src/ação.ts").is_some());
    h.key_press(Key::Enter);
    settle(&mut h);
    assert!(h.query_by_label("Pesquisar no projeto").is_none());
    assert_eq!(
        h.state().service.active_document().unwrap().path,
        dir.path().join("src/ação.ts")
    );
    assert_eq!(h.state().cursor, (2, 3));
    assert_eq!(
        fs::read_to_string(dir.path().join("src/other.php")).unwrap(),
        "<?php echo 1;\n"
    );
    assert!(h.get_by_role(Role::MultilineTextInput).is_focused());
}
#[test]
fn shortcut_and_mouse_search_cancel_restore_focus_and_empty_results_are_honest() {
    let (_, mut h) = setup();
    h.get_by_label("Pesquisar arquivos").click();
    settle(&mut h);
    h.event(Event::Text("does-not-exist".into()));
    settle(&mut h);
    assert!(h.query_by_label("Nenhum resultado").is_some());
    h.key_press(Key::Enter);
    settle(&mut h);
    assert!(h.query_by_label("Pesquisar no projeto").is_some());
    h.key_press(Key::Escape);
    settle(&mut h);
    assert!(h.get_by_role(Role::MultilineTextInput).is_focused());
    h.get_by_label("Pesquisar arquivos").click();
    settle(&mut h);
    h.get_by_label("Fechar pesquisa").click();
    settle(&mut h);
    assert!(h.query_by_label("Pesquisar no projeto").is_none());
}
#[test]
fn shift_chord_and_shift_in_path_input_do_not_open_search() {
    let (_, mut h) = setup();
    shift(&mut h);
    h.key_press_modifiers(Modifiers::SHIFT, Key::ArrowRight);
    h.step();
    shift(&mut h);
    settle(&mut h);
    assert!(h.query_by_label("Pesquisar no projeto").is_none());
    h.key_press_modifiers(Modifiers::COMMAND, Key::O);
    settle(&mut h);
    shift(&mut h);
    shift(&mut h);
    settle(&mut h);
    assert!(h.query_by_label("Pesquisar no projeto").is_none());
    assert!(h.get_by_label("Caminho").is_focused());
}
#[test]
fn actions_and_open_categories_only_offer_real_items_and_menu_settings_work() {
    let (_, mut h) = setup();
    h.get_by_label("Pesquisar arquivos").click();
    settle(&mut h);
    h.get_by_label("Search filters").click();
    h.run();
    h.get_by_label("Open files only").click();
    settle(&mut h);
    assert!(h.query_by_label("Abrir src/other.php").is_some());
    assert!(h.query_by_label("Abrir src/ação.ts").is_none());
    h.get_by_label("Search filters").click();
    h.run();
    h.get_by_label("Open files only").click();
    settle(&mut h);
    h.get_by_label("Ações").click();
    settle(&mut h);
    h.get_by_label("Abrir Configurações…").click();
    settle(&mut h);
    assert!(h.query_by_label("Usar ligaduras de fonte").is_some());
}

#[test]
fn real_class_symbol_tabs_preview_and_right_split_preserve_other_buffer() {
    let (dir, mut h) = setup();
    // This scenario explicitly compares an unsaved buffer with unchanged disk.
    h.state_mut()
        .service
        .dispatch(Command::SetAutoSave {
            enabled: false,
            delay_ms: 1000,
        })
        .unwrap();
    fs::write(
        dir.path().join("src/Greeter.ts"),
        "export class Greeter {\n  greet() { return 'hello'; }\n}\n",
    )
    .unwrap();
    let old = h.state().service.active_document().unwrap().id;
    let version = h.state().service.active_document().unwrap().version;
    h.state_mut()
        .service
        .dispatch(Command::Edit {
            id: old,
            expected_version: version,
            text: "<?php // unsaved original 😀\n".into(),
        })
        .unwrap();
    h.get_by_label("Pesquisar arquivos").click();
    settle(&mut h);
    h.get_by_label("Classes").click();
    settle(&mut h);
    h.event(Event::Text("Greeter".into()));
    settle(&mut h);
    assert!(
        h.query_by_label("Abrir classe Greeter em src/Greeter.ts")
            .is_some()
    );
    capture_if_requested(&mut h, "classes");
    h.key_press(Key::Tab);
    settle(&mut h);
    assert!(h.query_by_label("Abrir src/Greeter.ts").is_some());
    assert!(
        h.query_by_label("Abrir classe Greeter em src/Greeter.ts")
            .is_none()
    );
    h.key_press_modifiers(Modifiers::SHIFT, Key::Tab);
    settle(&mut h);
    h.get_by_label("Show Preview").click();
    settle(&mut h);
    assert!(
        h.query_by_label("export class Greeter {").is_some(),
        "actual file text is rendered by the read-only preview"
    );
    capture_if_requested(&mut h, "preview");
    h.key_press_modifiers(Modifiers::SHIFT, Key::Enter);
    settle(&mut h);
    assert_eq!(h.state().service.document_views().len(), 2);
    assert_eq!(
        h.state().service.active_document().unwrap().path,
        dir.path().join("src/Greeter.ts")
    );
    assert_eq!(
        h.state()
            .service
            .documents
            .iter()
            .find(|d| d.id == old)
            .unwrap()
            .text(),
        "<?php // unsaved original 😀\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("src/other.php")).unwrap(),
        "<?php echo 1;\n"
    );
    h.get_by_label("Pesquisar arquivos").click();
    settle(&mut h);
    h.get_by_label("Symbols").click();
    settle(&mut h);
    h.get_by_label("Pesquisar no projeto").click();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text("greet".into()));
    settle(&mut h);
    assert!(
        h.query_by_label("Abrir símbolo greet em Greeter — src/Greeter.ts")
            .is_some()
    );
    h.key_press(Key::Enter);
    settle(&mut h);
    assert_eq!(h.state().cursor, (2, 3));
}

#[test]
fn include_non_project_finds_only_known_external_files_and_text_filter_is_effective() {
    let (dir, mut h) = setup();
    let external = tempfile::tempdir().unwrap();
    fs::write(
        external.path().join("external-known.txt"),
        "outside marker\n",
    )
    .unwrap();
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(
            external.path().join("external-known.txt"),
        ))
        .unwrap();
    settle(&mut h);
    fs::write(
        dir.path().join("src/Filter.ts"),
        "const marker = 'find-content-marker';\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("src/Filter.php"),
        "<?php echo 'find-content-marker';\n",
    )
    .unwrap();
    h.get_by_label("Pesquisar arquivos").click();
    settle(&mut h);
    h.event(Event::Text("external-known".into()));
    settle(&mut h);
    assert!(
        h.query_by_label(&format!(
            "Abrir {}",
            external.path().join("external-known.txt").display()
        ))
        .is_none()
    );
    h.get_by_label("Include non-project items").click();
    settle(&mut h);
    assert!(
        h.query_by_label(&format!(
            "Abrir {}",
            external.path().join("external-known.txt").display()
        ))
        .is_some()
    );
    h.get_by_label("Text").click();
    settle(&mut h);
    h.get_by_label("Pesquisar no projeto").click();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text("find-content-marker".into()));
    settle(&mut h);
    assert!(h.query_by_label("Abrir texto src/Filter.ts:1").is_some());
    assert!(h.query_by_label("Abrir texto src/Filter.php:1").is_some());
    h.get_by_label("Search filters").click();
    h.run();
    h.get_by_label("*.ts; *.tsx").click();
    settle(&mut h);
    assert!(h.query_by_label("Abrir texto src/Filter.ts:1").is_some());
    assert!(h.query_by_label("Abrir texto src/Filter.php:1").is_none());
    h.key_press(Key::Escape);
    settle(&mut h);
    assert_eq!(
        fs::read_to_string(external.path().join("external-known.txt")).unwrap(),
        "outside marker\n"
    );
}

#[test]
fn empty_all_reads_only_open_recent_files_and_measured_popup_bounds_are_exact() {
    let (dir, mut h) = setup();
    for index in 0..35 {
        fs::write(
            dir.path().join(format!("matchable-{index:02}.txt")),
            "closed file body\n",
        )
        .unwrap();
    }
    h.get_by_label("Pesquisar arquivos").click();
    settle(&mut h);
    let result = h.state().project_search_result().unwrap();
    assert_eq!(
        result.scanned_files, 1,
        "Blank All must only inspect the open document list, not traverse the project"
    );
    assert_eq!(
        result.searched_bytes, 0,
        "Blank All must not read or parse source text"
    );
    assert_eq!(result.hits.len(), 1);
    assert!(h.query_by_label("Abrir src/other.php").is_some());
    assert!(h.query_by_label("Abrir src/ação.ts").is_none());
    assert!(h.query_by_label("Abrir Configurações…").is_none());
    assert!(h.query_by_label("Abrir Terminal").is_none());
    h.event(Event::Text("matchable".into()));
    settle(&mut h);
    assert_eq!(h.state().project_search_result().unwrap().hits.len(), 35);
    assert!(
        h.state().project_search_result().unwrap().searched_bytes > 0,
        "A nonempty All query still performs real content/declaration search"
    );
    let rect = h.state().project_search_rect().unwrap();
    assert!(
        (rect.width() - 702.).abs() < 0.1,
        "Measured outer width: {rect:?}"
    );
    assert!(
        (rect.height() - 602.).abs() < 0.1,
        "Measured outer height: {rect:?}"
    );
    h.key_press(Key::Escape);
    settle(&mut h);
    let id = h.state().service.active_document().unwrap().id;
    h.state_mut()
        .service
        .dispatch(Command::Close { id, discard: false })
        .unwrap();
    settle(&mut h);
    h.get_by_label("Pesquisar arquivos").click();
    settle(&mut h);
    h.get_by_label("Pesquisar no projeto").click();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.key_press(Key::Backspace);
    settle(&mut h);
    let result = h.state().project_search_result().unwrap();
    assert!(result.hits.is_empty());
    assert_eq!(result.scanned_files, 0);
    assert_eq!(result.searched_bytes, 0);
    assert!(h.query_by_label("Abrir Configurações…").is_none());
    assert!(
        h.query_by_label("Abrir src/other.php").is_some(),
        "Recently closed files remain available without rescanning source"
    );
    let rect = h.state().project_search_rect().unwrap();
    assert!((rect.width() - 702.).abs() < 0.1);
    assert!(rect.height() > 125., "Recent file popup height: {rect:?}");
}
