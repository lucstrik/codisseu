//! Native widgets, pointer gestures and common document history work together.
use super::*;
use codisseu_draw::Scene;
use egui_kittest::{Harness, kittest::Queryable};
use std::{fs, thread, time::Instant};

type H = Harness<'static, CodisseuApp>;
fn settle(h: &mut H) {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut idle = 0;
    while idle < 3 {
        h.step();
        idle = if h.state().service.is_busy() {
            0
        } else {
            idle + 1
        };
        assert!(
            Instant::now() < deadline,
            "drawing worker timeout: {:?}",
            h.state().service.last_error
        );
        thread::sleep(Duration::from_millis(2));
    }
    h.run();
}
fn fixture(scale: f32) -> (tempfile::TempDir, H) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("main.php"), "<?php // preserved\n").unwrap();
    let mut service = AppService::new(temp.path().join("profile/session.json"));
    service.dispatch(Command::OpenWorkspace(root)).unwrap();
    let mut app = CodisseuApp::new(service);
    app.show_database = false;
    let mut h = Harness::builder()
        .with_size([1440., 900.])
        .with_pixels_per_point(scale)
        .with_max_steps(50)
        .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), app);
    settle(&mut h);
    (temp, h)
}
fn create(h: &mut H) {
    h.get_by_label("Desenhos · Excalidraw").click();
    h.run();
    h.get_by_label("Caminho do novo desenho").click();
    h.event(egui::Event::Text("Arquitetura".into()));
    h.run();
    h.get_by_label("Criar desenho").click();
    settle(h);
    assert!(
        h.state().service.drawings.error.is_none(),
        "{:?}",
        h.state().service.drawings.error
    );
    assert!(
        h.state().service.last_error.is_none(),
        "{:?}",
        h.state().service.last_error
    );
    assert!(h.query_by_label("Tela de desenho Excalidraw").is_some());
}
fn drag(h: &mut H, start: egui::Pos2, end: egui::Pos2) {
    h.hover_at(start);
    h.step();
    h.event(egui::Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.event(egui::Event::PointerMoved(start + (end - start) * 0.5));
    h.step();
    h.event(egui::Event::PointerMoved(end));
    h.step();
    h.event(egui::Event::PointerButton {
        pos: end,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run();
}
fn scene(h: &H) -> Scene {
    Scene::from_json(h.state().service.active_document().unwrap().text()).unwrap()
}
fn capture(h: &mut H, name: &str) {
    if let Some(folder) = std::env::var_os("CODISSEU_DRAWINGS_CAPTURES") {
        fs::create_dir_all(&folder).unwrap();
        h.render()
            .unwrap()
            .save(std::path::Path::new(&folder).join(format!("{name}.png")))
            .unwrap();
    }
}

#[test]
fn drawings_native_create_draw_save_history_source_and_project_navigation() {
    for scale in [1.0, 1.25] {
        let (_temp, mut h) = fixture(scale);
        create(&mut h);
        h.get_by_label("Retângulo · R ou 3").click();
        h.run();
        let rect = h.get_by_label("Tela de desenho Excalidraw").rect();
        let start = rect.min + egui::vec2(140., 100.);
        let end = start + egui::vec2(200., 130.);
        drag(&mut h, start, end);
        assert_eq!(
            scene(&h).elements.len(),
            1,
            "canvas gesture must edit the shared document"
        );
        assert!(h.state().service.active_document().unwrap().is_dirty());
        let expected = scene(&h);
        h.get_by_label("Desfazer").click();
        h.run();
        assert!(scene(&h).elements.is_empty());
        h.get_by_label("Refazer").click();
        h.run();
        assert_eq!(scene(&h), expected);
        h.get_by_label("Salvar desenho").click();
        settle(&mut h);
        let path = h.state().service.active_document().unwrap().path.clone();
        assert_eq!(
            Scene::from_json(&fs::read_to_string(&path).unwrap()).unwrap(),
            expected
        );
        assert!(!h.state().service.active_document().unwrap().is_dirty());
        capture(&mut h, &format!("drawing-{scale}"));
        h.get_by_label("Código JSON").click();
        h.run();
        assert!(h.query_by_label("Tela de desenho Excalidraw").is_none());
        assert_eq!(scene(&h), expected);
        capture(&mut h, &format!("json-{scale}"));
        h.get_by_label("Desenho").click();
        h.run();
        assert!(h.query_by_label("Tela de desenho Excalidraw").is_some());
        h.get_by_label("Alternar painel Projeto").click();
        h.run();
        assert!(!h.state().drawings_ui.open);
        assert!(h.query_by_label("main.php").is_some());
        assert_eq!(scene(&h), expected);
    }
}

#[test]
fn drawings_share_editor_with_api_tabs_without_overwriting_the_scene() {
    let (_temp, mut h) = fixture(1.0);
    create(&mut h);
    let original = scene(&h);
    let collection = h
        .state_mut()
        .service
        .api
        .workspace
        .new_collection("Fixture");
    let request = h
        .state_mut()
        .service
        .api
        .workspace
        .new_request(&collection, "Health")
        .unwrap();
    h.state_mut()
        .api_open_tab(api_client::ApiTab::Request(request));
    h.run();
    assert!(h.query_by_label("Send").is_some());
    assert!(h.query_by_label("Tela de desenho Excalidraw").is_none());
    h.get_by_label("Arquitetura.excalidraw").click();
    h.run();
    assert!(h.query_by_label("Tela de desenho Excalidraw").is_some());
    assert_eq!(scene(&h), original);
    assert_eq!(h.state().service.drawings.paths.len(), 1);
}

#[test]
fn drawing_revision_change_cancels_uncommitted_pointer_preview() {
    use codisseu_draw::{Element, ElementKind};
    let (_temp, mut h) = fixture(1.0);
    create(&mut h);
    h.get_by_label("Retângulo · R ou 3").click();
    h.run();
    let rect = h.get_by_label("Tela de desenho Excalidraw").rect();
    let start = rect.min + egui::vec2(150., 100.);
    let end = start + egui::vec2(120., 80.);
    h.hover_at(start);
    h.step();
    h.event(egui::Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.event(egui::Event::PointerMoved(end));
    h.step();
    assert!(
        scene(&h).elements.is_empty(),
        "preview must not commit before pointer release"
    );
    let mut replacement = Scene::default();
    replacement
        .elements
        .push(Element::new(ElementKind::Ellipse, 80.0, 90.0, 40.0, 60.0));
    let doc = h.state().service.active_document().unwrap();
    let id = doc.id;
    let version = doc.version;
    h.state_mut()
        .service
        .dispatch(Command::Edit {
            id,
            expected_version: version,
            text: replacement.to_json().unwrap(),
        })
        .unwrap();
    h.step();
    h.event(egui::Event::PointerButton {
        pos: end,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run();
    assert_eq!(
        scene(&h),
        replacement,
        "release must not replay stale preview over a new document revision"
    );
}
