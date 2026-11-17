//! Search Everywhere interaction and selected-source preview regressions.
use codisseu_core::{AppService, Command};
use codisseu_ui::CodisseuApp;
use egui::{Event, Key, Modifiers};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::{
    collections::BTreeSet,
    fs,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

type H = Harness<'static, CodisseuApp>;

fn settle(h: &mut H) {
    h.step();
    let until = Instant::now() + Duration::from_secs(10);
    while h.state().service.is_busy() || h.state().project_search_pending() {
        assert!(
            Instant::now() < until,
            "Search/selected preview did not settle"
        );
        thread::sleep(Duration::from_millis(1));
        h.step();
    }
    h.run_steps(8);
}

fn fixture(scale: f32) -> (tempfile::TempDir, H) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("src/Base.txt"),
        "original active document\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("src/PreviewA.php"),
        "<?php\n$label = \"from disk php\";\n\t$sum = 40 + 2;\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("src/PreviewB_.py"),
        "def greeting(name):\n\treturn \"saved python\" + name\n",
    )
    .unwrap();
    let mut service = AppService::new(dir.path().join("state/session.json"));
    service
        .dispatch(Command::SetAutoSave {
            enabled: false,
            delay_ms: 1000,
        })
        .unwrap();
    service
        .dispatch(Command::OpenWorkspace(dir.path().into()))
        .unwrap();
    service
        .dispatch(Command::OpenFile(dir.path().join("src/Base.txt")))
        .unwrap();
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .with_pixels_per_point(scale)
        .with_max_steps(60)
        .with_step_dt(1. / 60.)
        .build_eframe(|_| CodisseuApp::new(service));
    settle(&mut h);
    (dir, h)
}

fn open_files(h: &mut H, query: &str) {
    h.get_by_label("Pesquisar arquivos").click();
    settle(h);
    h.get_by_label("Files").click();
    settle(h);
    h.get_by_label("Pesquisar no projeto").click();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text(query.into()));
    settle(h);
}

fn shift(h: &mut H) {
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

fn close_active(h: &mut H) {
    let id = h.state().service.active_document().unwrap().id;
    h.state_mut()
        .service
        .dispatch(Command::Close { id, discard: false })
        .unwrap();
    settle(h);
}

fn painted(h: &H) -> Vec<(egui::Pos2, Arc<egui::Galley>)> {
    fn visit(shape: &egui::Shape, result: &mut Vec<(egui::Pos2, Arc<egui::Galley>)>) {
        match shape {
            egui::Shape::Text(text) => result.push((text.pos, text.galley.clone())),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, result);
                }
            }
            _ => {}
        }
    }
    let mut result = Vec::new();
    for shape in &h.output().shapes {
        visit(&shape.shape, &mut result);
    }
    result
}

fn assert_colored_line(h: &H, line: &str) {
    let shapes = painted(h);
    let (_, galley) = shapes
        .iter()
        .find(|(_, galley)| galley.text() == line)
        .unwrap_or_else(|| panic!("Full source line missing from preview paint: {line:?}"));
    let colors = galley
        .job
        .sections
        .iter()
        .map(|section| section.format.color.to_array())
        .collect::<BTreeSet<_>>();
    assert!(
        colors.len() >= 2,
        "Expected real syntax colors for {line:?}"
    );
}

#[test]
fn selected_preview_keeps_dirty_source_syntax_tabs_long_lines_and_independent_scroll() {
    for scale in [1., 1.25] {
        let (dir, mut h) = fixture(scale);
        let base = dir.path().join("src/Base.txt");
        let python = dir.path().join("src/PreviewB_.py");
        let php = dir.path().join("src/PreviewA.php");
        let python_disk = fs::read_to_string(&python).unwrap();
        let php_disk = fs::read_to_string(&php).unwrap();
        let long_line = format!(
            "# {}end-of-complete-long-line",
            "preview Unicode ação 😀 ".repeat(12)
        );
        assert!(long_line.chars().count() > 160);
        let mut dirty =
            format!("def greeting(name):\n\treturn \"dirty 😀 preview\" + name\n{long_line}\n");
        for index in 4..180 {
            dirty.push_str(&format!("value_{index:03} = {index}\n"));
        }
        h.state_mut()
            .service
            .dispatch(Command::OpenFile(python.clone()))
            .unwrap();
        settle(&mut h);
        let document = h.state().service.active_document().unwrap();
        let id = document.id;
        let version = document.version;
        h.state_mut()
            .service
            .dispatch(Command::Edit {
                id,
                expected_version: version,
                text: dirty.clone(),
            })
            .unwrap();
        h.state_mut()
            .service
            .dispatch(Command::OpenFile(base.clone()))
            .unwrap();
        settle(&mut h);
        let active = h.state().service.active_document().unwrap().id;
        let dirty_version = h
            .state()
            .service
            .documents
            .iter()
            .find(|document| document.id == id)
            .unwrap()
            .version;

        open_files(&mut h, "Preview");
        assert_eq!(h.state().project_search_result().unwrap().hits.len(), 2);
        // The equal-length fixture names place PHP first and Python second.
        // Enabling the eye must retain the user's second-row selection.
        h.key_press(Key::ArrowDown);
        settle(&mut h);
        h.get_by_label("Show Preview").click();
        settle(&mut h);
        assert!(h.query_by_label("def greeting(name):").is_some());
        assert!(
            h.query_by_label("\treturn \"dirty 😀 preview\" + name")
                .is_some()
        );
        assert!(h.query_by_label(&long_line).is_some());
        assert_colored_line(&h, "    return \"dirty 😀 preview\" + name");
        assert!(
            painted(&h)
                .iter()
                .any(|(_, galley)| galley.text() == long_line)
        );

        // The gutter is a separate painted value, never a prefix in source text.
        let first_line = h.get_by_label("def greeting(name):").rect();
        assert!(painted(&h).iter().any(|(position, galley)| {
            let bounds = galley.rect.translate(position.to_vec2());
            galley.text() == "1"
                && bounds.right() <= first_line.left()
                && bounds.y_range().intersects(first_line.y_range())
        }));
        let point = h
            .get_by_label("\treturn \"dirty 😀 preview\" + name")
            .rect()
            .center();
        h.event(Event::PointerMoved(point));
        h.event(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0., -100_000.),
            modifiers: Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
        });
        h.run_steps(30);
        if let Some(out) = std::env::var_os("CODISSEU_SEARCH_CAPTURE_DIR") {
            let out = std::path::PathBuf::from(out);
            fs::create_dir_all(&out).unwrap();
            h.render()
                .unwrap()
                .save(out.join(format!("audit-scroll-{scale}.png")))
                .unwrap();
        }
        assert!(
            h.query_by_label("value_179 = 179").is_some(),
            "visible code after wheel: {:?}",
            painted(&h)
                .iter()
                .map(|(_, g)| g.text())
                .filter(|s| s.starts_with("value_"))
                .collect::<Vec<_>>()
        );
        assert!(h.query_by_label("Abrir src/PreviewA.php").is_some());
        assert!(h.query_by_label("Abrir src/PreviewB_.py").is_some());
        assert_eq!(h.state().service.active_document().unwrap().id, active);

        // Preview-on clicks select another file without activating its document.
        h.get_by_label("Abrir src/PreviewA.php").click();
        settle(&mut h);
        assert!(h.query_by_label("Pesquisar no projeto").is_some());
        assert_eq!(h.state().service.active_document().unwrap().id, active);
        assert!(h.query_by_label("$label = \"from disk php\";").is_some());
        assert!(h.query_by_label("\t$sum = 40 + 2;").is_some());
        assert_colored_line(&h, "$label = \"from disk php\";");
        let document = h
            .state()
            .service
            .documents
            .iter()
            .find(|document| document.id == id)
            .unwrap();
        assert_eq!(document.text(), dirty);
        assert_eq!(document.version, dirty_version);
        assert_eq!(fs::read_to_string(&python).unwrap(), python_disk);
        assert_eq!(fs::read_to_string(&php).unwrap(), php_disk);
        assert_eq!(
            fs::read_to_string(&base).unwrap(),
            "original active document\n"
        );

        // After the first click's interval has elapsed, a real double click opens.
        h.run_steps(40);
        h.get_by_label("Abrir src/PreviewA.php").click();
        h.step();
        h.get_by_label("Abrir src/PreviewA.php").click();
        settle(&mut h);
        assert!(h.query_by_label("Pesquisar no projeto").is_none());
        assert_eq!(h.state().service.active_document().unwrap().path, php);
        assert_eq!(fs::read_to_string(&python).unwrap(), python_disk);
    }
}

#[test]
fn text_and_enter_in_one_frame_never_activate_the_previous_query_hit() {
    let (dir, mut h) = fixture(1.);
    open_files(&mut h, "PreviewA");
    assert!(h.query_by_label("Abrir src/PreviewA.php").is_some());
    h.event(Event::Text("-no-such-result".into()));
    for pressed in [true, false] {
        h.event(Event::Key {
            key: Key::Enter,
            physical_key: Some(Key::Enter),
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
    }
    settle(&mut h);
    assert!(h.query_by_label("Pesquisar no projeto").is_some());
    assert!(h.query_by_label("Nenhum resultado").is_some());
    assert_eq!(
        h.state().service.active_document().unwrap().path,
        dir.path().join("src/Base.txt")
    );
    assert_eq!(h.state().service.documents.len(), 1);
}

#[test]
fn search_owns_ctrl_g_and_ctrl_n_without_acting_on_the_hidden_editor() {
    let (dir, mut h) = fixture(1.);
    open_files(&mut h, "Preview");
    h.key_press_modifiers(Modifiers::COMMAND, Key::G);
    settle(&mut h);
    assert!(h.get_by_label("Pesquisar no projeto").is_focused());
    assert!(h.query_by_label("Ir para linha").is_none());
    h.key_press_modifiers(Modifiers::COMMAND, Key::N);
    settle(&mut h);
    assert!(h.query_by_label("Pesquisar no projeto").is_some());
    assert_eq!(h.state().service.documents.len(), 1);
    assert_eq!(
        h.state().service.active_document().unwrap().path,
        dir.path().join("src/Base.txt")
    );
}

#[test]
fn enter_in_nested_filter_changes_the_filter_without_opening_a_result() {
    let (dir, mut h) = fixture(1.);
    open_files(&mut h, "Preview");
    h.get_by_label("Search filters").click();
    h.run();
    h.get_by_label("Open files only").focus();
    h.step();
    h.key_press(Key::Enter);
    settle(&mut h);
    assert!(h.query_by_label("Pesquisar no projeto").is_some());
    assert!(h.query_by_label("Abrir src/PreviewA.php").is_none());
    assert!(h.query_by_label("Abrir src/PreviewB_.py").is_none());
    assert_eq!(h.state().service.documents.len(), 1);
    assert_eq!(
        h.state().service.active_document().unwrap().path,
        dir.path().join("src/Base.txt")
    );
}

#[test]
fn cancelled_queries_reopen_from_search_history_without_opening_a_document() {
    let (dir, mut h) = fixture(1.);
    open_files(&mut h, "PreviewA");
    h.key_press(Key::Escape);
    settle(&mut h);
    assert!(h.query_by_label("Pesquisar no projeto").is_none());
    open_files(&mut h, "PreviewB_");
    h.key_press(Key::Escape);
    settle(&mut h);
    assert!(h.query_by_label("Pesquisar no projeto").is_none());
    h.get_by_label("Pesquisar arquivos").click();
    settle(&mut h);
    h.get_by_label("Search history").click();
    h.run();
    assert!(h.query_by_label("PreviewB_").is_some());
    h.get_by_label("PreviewA").click();
    settle(&mut h);
    assert_eq!(
        h.get_by_label("Pesquisar no projeto").value().as_deref(),
        Some("PreviewA")
    );
    assert!(h.query_by_label("Abrir src/PreviewA.php").is_some());
    assert!(h.query_by_label("Abrir src/PreviewB_.py").is_none());
    assert_eq!(h.state().service.documents.len(), 1);
    assert_eq!(
        h.state().service.active_document().unwrap().path,
        dir.path().join("src/Base.txt")
    );
}

#[test]
fn blank_all_reopens_a_recently_closed_file_without_listing_unvisited_files() {
    let (dir, mut h) = fixture(1.);
    let python = dir.path().join("src/PreviewB_.py");
    let before = fs::read_to_string(&python).unwrap();
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(python.clone()))
        .unwrap();
    settle(&mut h);
    close_active(&mut h);
    assert_eq!(h.state().service.documents.len(), 1);
    h.get_by_label("Pesquisar arquivos").click();
    settle(&mut h);
    assert_eq!(
        h.get_by_label("Pesquisar no projeto").value().as_deref(),
        Some("")
    );
    assert!(h.query_by_label("Abrir src/Base.txt").is_some());
    assert!(h.query_by_label("Abrir src/PreviewB_.py").is_some());
    assert!(h.query_by_label("Abrir src/PreviewA.php").is_none());
    h.get_by_label("Abrir src/PreviewB_.py").click();
    settle(&mut h);
    assert!(h.query_by_label("Pesquisar no projeto").is_none());
    assert_eq!(h.state().service.active_document().unwrap().path, python);
    assert_eq!(fs::read_to_string(&python).unwrap(), before);
}

#[test]
fn double_shift_inside_search_expands_known_scope_and_alt_n_reverses_it() {
    let (dir, mut h) = fixture(1.);
    let external = tempfile::tempdir().unwrap();
    let path = external.path().join("ExternalKnown.php");
    let source = "<?php // external known source\n";
    fs::write(&path, source).unwrap();
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(path.clone()))
        .unwrap();
    settle(&mut h);
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(dir.path().join("src/Base.txt")))
        .unwrap();
    settle(&mut h);
    h.get_by_label("Pesquisar arquivos").click();
    settle(&mut h);
    h.event(Event::Text("ExternalKnown".into()));
    settle(&mut h);
    let label = format!("Abrir {}", path.display());
    assert!(h.query_by_label(&label).is_none());
    shift(&mut h);
    shift(&mut h);
    settle(&mut h);
    assert!(h.query_by_label(&label).is_some());
    assert_eq!(
        h.get_by_label("Pesquisar no projeto").value().as_deref(),
        Some("ExternalKnown")
    );
    h.key_press_modifiers(Modifiers::ALT, Key::N);
    h.event(Event::Text("n".into()));
    settle(&mut h);
    assert!(h.query_by_label(&label).is_none());
    assert!(h.query_by_label("Pesquisar no projeto").is_some());
    assert_eq!(
        h.state().service.active_document().unwrap().path,
        dir.path().join("src/Base.txt")
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
}

#[test]
fn optional_disabled_save_action_is_visible_but_enter_does_not_execute_it() {
    let (dir, mut h) = fixture(1.);
    close_active(&mut h);
    assert!(h.state().service.documents.is_empty());
    h.get_by_label("Pesquisar arquivos").click();
    settle(&mut h);
    h.get_by_label("Ações").click();
    settle(&mut h);
    h.event(Event::Text("Save".into()));
    settle(&mut h);
    assert!(h.query_by_label("Abrir Save").is_none());
    h.get_by_label("Include disabled actions").click();
    settle(&mut h);
    assert!(h.get_by_label("Abrir Save").accesskit_node().is_disabled());
    h.get_by_label("Pesquisar no projeto").click();
    h.key_press(Key::Enter);
    settle(&mut h);
    assert!(h.query_by_label("Pesquisar no projeto").is_some());
    assert!(h.state().service.documents.is_empty());
    assert!(h.state().service.last_error.is_none());
    assert!(h.query_by_label("Caminho").is_none());
    assert_eq!(
        fs::read_to_string(dir.path().join("src/Base.txt")).unwrap(),
        "original active document\n"
    );
}

#[test]
fn shift_enter_with_no_open_document_creates_two_views_after_loading_the_file() {
    let (dir, mut h) = fixture(1.);
    close_active(&mut h);
    assert!(h.state().service.documents.is_empty());
    assert!(h.state().service.document_views().is_empty());
    let path = dir.path().join("src/PreviewA.php");
    let before = fs::read_to_string(&path).unwrap();
    open_files(&mut h, "PreviewA");
    h.key_press_modifiers(Modifiers::SHIFT, Key::Enter);
    settle(&mut h);
    assert!(h.query_by_label("Pesquisar no projeto").is_none());
    assert_eq!(h.state().service.documents.len(), 1);
    assert_eq!(h.state().service.document_views().len(), 2);
    assert!(h.state().service.document_views().iter().all(|view| {
        h.state()
            .service
            .view_document(view.id)
            .is_some_and(|document| document.path == path)
    }));
    assert!(!h.state().service.editor_split().horizontal);
    assert!(h.state().service.last_error.is_none());
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn right_split_of_already_loaded_target_keeps_original_document_on_left() {
    let (dir, mut h) = fixture(1.);
    let base = dir.path().join("src/Base.txt");
    let target = dir.path().join("src/PreviewB_.py");
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(target.clone()))
        .unwrap();
    settle(&mut h);
    let target_id = h.state().service.active_document().unwrap().id;
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(base.clone()))
        .unwrap();
    settle(&mut h);
    open_files(&mut h, "PreviewB_");
    h.key_press_modifiers(Modifiers::SHIFT, Key::Enter);
    settle(&mut h);
    let views = h.state().service.document_views();
    assert_eq!(views.len(), 2);
    assert_eq!(
        h.state().service.view_document(views[0].id).unwrap().path,
        base
    );
    assert_eq!(
        h.state().service.view_document(views[1].id).unwrap().path,
        target
    );
    assert_eq!(
        h.state().service.view_document(views[1].id).unwrap().id,
        target_id
    );
    assert_eq!(h.state().service.active_view(), Some(views[1].id));
}
