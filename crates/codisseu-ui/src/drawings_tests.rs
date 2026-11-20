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
fn menu_action(h: &mut H, label: &str) {
    h.get_by_label("Menu do desenho").click();
    h.run();
    h.get_by_label(label).click();
    h.run();
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
        let start = rect.min + egui::vec2(320., 180.);
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
        menu_action(&mut h, "Salvar desenho");
        settle(&mut h);
        let path = h.state().service.active_document().unwrap().path.clone();
        assert_eq!(
            Scene::from_json(&fs::read_to_string(&path).unwrap()).unwrap(),
            expected
        );
        assert!(!h.state().service.active_document().unwrap().is_dirty());
        capture(&mut h, &format!("drawing-{scale}"));
        menu_action(&mut h, "Código JSON");
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
    let start = rect.min + egui::vec2(320., 180.);
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

#[test]
fn drawing_canvas_defaults_dark_and_themes_preserve_authored_colors_and_insets() {
    for scale in [1.0, 1.25] {
        let (_temp, mut h) = fixture(scale);
        create(&mut h);
        let before = scene(&h);
        let canvas = h.get_by_label("Tela de desenho Excalidraw").rect();
        for label in [
            "Menu do desenho",
            "Retângulo · R ou 3",
            "Desfazer",
            "Refazer",
        ] {
            let rect = h.get_by_label(label).rect();
            assert!(
                rect.left() >= canvas.left() + 8. && rect.top() >= canvas.top() + 8.,
                "{label}: {rect:?} outside {canvas:?}"
            );
            assert!(
                rect.right() <= canvas.right() - 8. && rect.bottom() <= canvas.bottom() - 8.,
                "{label}: {rect:?} outside {canvas:?}"
            );
        }
        let sample = canvas.min + egui::vec2(canvas.width() - 24., canvas.height() * 0.75);
        let pixel = |h: &mut H| {
            let image = h.render().unwrap();
            image
                .get_pixel(
                    (sample.x * scale).round() as u32,
                    (sample.y * scale).round() as u32,
                )
                .0
        };
        assert_eq!(pixel(&mut h), [18, 18, 18, 255]);
        capture(&mut h, &format!("empty-system-{scale}"));
        for (mode, expected) in [
            ("Claro", [255, 255, 255, 255]),
            ("Escuro", [18, 18, 18, 255]),
            ("Sistema", [18, 18, 18, 255]),
        ] {
            menu_action(&mut h, mode);
            h.get_by_label("Menu do desenho").click();
            h.run();
            let after = scene(&h);
            assert_eq!(after.elements, before.elements);
            assert_eq!(
                after.app_state.get("viewBackgroundColor"),
                before.app_state.get("viewBackgroundColor")
            );
            assert_eq!(pixel(&mut h), expected);
            capture(&mut h, &format!("theme-{mode}-{scale}"));
        }
        menu_action(&mut h, "Claro");
        h.get_by_label("Menu do desenho").click();
        h.run();
        menu_action(&mut h, "Salvar desenho");
        settle(&mut h);
        let document = h.state().service.active_document().unwrap();
        let (id, path) = (document.id, document.path.clone());
        assert_eq!(
            Scene::from_json(&fs::read_to_string(&path).unwrap())
                .unwrap()
                .app_state["codisseuTheme"],
            "light"
        );
        h.state_mut()
            .service
            .dispatch(Command::Close { id, discard: false })
            .unwrap();
        h.run();
        h.state_mut()
            .service
            .dispatch(Command::OpenFile(path))
            .unwrap();
        settle(&mut h);
        assert_eq!(scene(&h).app_state["codisseuTheme"], "light");
        assert_eq!(pixel(&mut h), [255, 255, 255, 255]);
        if scale == 1.0 {
            h.set_size(egui::vec2(760., 600.));
            h.run();
            let canvas = h.get_by_label("Tela de desenho Excalidraw").rect();
            let menu = h.get_by_label("Menu do desenho").rect();
            let lock = h.get_by_label("Manter ferramenta ativa").rect();
            assert!(canvas.contains_rect(menu) && canvas.contains_rect(lock));
            assert!(
                menu.right() + 4. <= lock.left(),
                "floating toolbar must keep a gap from the menu"
            );
            capture(&mut h, "narrow-light");
        }
    }
}

#[test]
fn drawing_undo_then_save_in_one_frame_saves_the_post_history_scene() {
    let (_temp, mut h) = fixture(1.0);
    create(&mut h);
    h.get_by_label("Retângulo · R ou 3").click();
    h.run();
    let rect = h.get_by_label("Tela de desenho Excalidraw").rect();
    let start = rect.min + egui::vec2(320., 180.);
    drag(&mut h, start, start + egui::vec2(160., 90.));
    let path = h.state().service.active_document().unwrap().path.clone();
    h.hover_at(rect.right_bottom() - egui::vec2(120., 100.));
    h.step();
    for key in [Key::Z, Key::S] {
        h.event(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        });
    }
    settle(&mut h);
    assert!(scene(&h).elements.is_empty());
    assert!(
        Scene::from_json(&fs::read_to_string(path).unwrap())
            .unwrap()
            .elements
            .is_empty()
    );
    assert!(!h.state().service.active_document().unwrap().is_dirty());
}

#[test]
fn drawing_commands_do_not_wait_for_a_tab_that_changes_in_the_same_frame() {
    let (_temp, mut h) = fixture(1.0);
    create(&mut h);
    let id = h.state().service.active_document().unwrap().id;
    let drawing_path = h.state().service.active_document().unwrap().path.clone();
    let other = h
        .state()
        .service
        .workspace
        .as_ref()
        .unwrap()
        .join("other.php");
    fs::write(&other, "<?php // independent tab\n").unwrap();
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(other.clone()))
        .unwrap();
    settle(&mut h);
    h.state_mut()
        .service
        .dispatch(Command::Activate { id })
        .unwrap();
    h.run();
    h.get_by_label("Retângulo · R ou 3").click();
    h.run();
    let canvas = h.get_by_label("Tela de desenho Excalidraw").rect();
    let start = canvas.min + egui::vec2(320., 180.);
    drag(&mut h, start, start + egui::vec2(160., 90.));
    assert!(h.state().service.active_document().unwrap().is_dirty());
    let tab = h
        .get_by_role_and_label(egui::accesskit::Role::Button, "other.php")
        .rect()
        .center();
    let combined = |h: &mut H, key| {
        h.event(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        });
        h.event(egui::Event::PointerMoved(tab));
        h.event(egui::Event::PointerButton {
            pos: tab,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        h.event(egui::Event::PointerButton {
            pos: tab,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        settle(h);
    };
    combined(&mut h, Key::S);
    assert_eq!(h.state().service.active_document().unwrap().path, other);
    assert_eq!(
        Scene::from_json(&fs::read_to_string(&drawing_path).unwrap())
            .unwrap()
            .elements
            .len(),
        1,
        "Save must not wait until returning to the drawing tab"
    );
    // The accepted intent also has to finish if this frame paints only the other tab.
    let view = h.state().service.active_view().unwrap();
    let document = h
        .state()
        .service
        .documents
        .iter()
        .find(|d| d.id == id)
        .unwrap();
    let version = document.version;
    let mut updated = Scene::from_json(document.text()).unwrap();
    updated.elements.push(codisseu_draw::Element::new(
        codisseu_draw::ElementKind::Ellipse,
        200.,
        200.,
        60.,
        40.,
    ));
    h.state_mut()
        .service
        .dispatch(Command::Edit {
            id,
            expected_version: version,
            text: updated.to_json().unwrap(),
        })
        .unwrap();
    h.state_mut().drawings_queue(view, id, vec!["save"]);
    settle(&mut h);
    assert_eq!(
        Scene::from_json(&fs::read_to_string(&drawing_path).unwrap())
            .unwrap()
            .elements
            .len(),
        2,
        "accepted Save must finish even when its tab is not painted"
    );
    h.state_mut().drawings_queue(view, id, vec!["close"]);
    settle(&mut h);
    assert!(
        !h.state().service.documents.iter().any(|d| d.id == id),
        "Close must finish in its original frame"
    );
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(drawing_path))
        .unwrap();
    settle(&mut h);
    assert!(
        h.state()
            .service
            .active_document()
            .is_some_and(|d| codisseu_core::drawings::is_drawing(&d.path)),
        "returning to the file must not replay a stale Close"
    );
}

#[test]
fn drawing_exit_waits_for_the_current_inline_text_frame_before_checking_dirty() {
    let (_temp, mut h) = fixture(1.0);
    create(&mut h);
    h.get_by_label("Texto · T ou 9").click();
    h.run();
    let canvas = h.get_by_label("Tela de desenho Excalidraw").rect();
    let point = canvas.min + egui::vec2(420., 220.);
    drag(&mut h, point, point);
    assert!(h.query_by_label("Conteúdo do texto do desenho").is_some());
    assert!(!h.state().service.active_document().unwrap().is_dirty());
    h.event(egui::Event::Text("texto do último frame".into()));
    h.state_mut().request_exit();
    h.run();
    assert!(
        scene(&h)
            .elements
            .iter()
            .any(|e| e.text == "texto do último frame")
    );
    assert!(matches!(h.state().confirm, Some(Confirm::Exit)));
    assert!(!h.state().exit_when_saved && !h.state().allow_close);
}
