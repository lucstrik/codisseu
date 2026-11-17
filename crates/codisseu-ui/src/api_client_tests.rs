//! Real request widgets on the common native editor surface; no Postman account.
use super::*;
use codisseu_core::api::*;
use egui_kittest::{Harness, kittest::Queryable};
use std::{fs, thread, time::Instant};

type ApiHarness = Harness<'static, CodisseuApp>;
fn settle(h: &mut ApiHarness) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        h.step();
        if !h.state().service.is_busy() {
            break;
        }
        assert!(Instant::now() < deadline, "API worker did not settle");
        thread::sleep(Duration::from_millis(3));
    }
    h.run();
}
fn fixture(size: [f32; 2]) -> (tempfile::TempDir, ApiHarness, String) {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir(&project).unwrap();
    let source = project.join("backend.php");
    fs::write(&source, "<?php\n// source preserved\n").unwrap();
    let mut service = AppService::new(dir.path().join("profile/session.json"));
    service.dispatch(Command::OpenWorkspace(project)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    service.dispatch(Command::OpenFile(source)).unwrap();
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    let c = service.api.workspace.new_collection("Backend Services");
    let id = service.api.workspace.new_request(&c, "Health").unwrap();
    service.api.workspace.find_request_mut(&id).unwrap().url = "http://127.0.0.1:1/health".into();
    let mut app = CodisseuApp::new(service);
    app.show_database = false;
    let mut h = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(
            std::env::var("CODISSEU_API_POLISH_SCALE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1.),
        )
        .with_max_steps(40)
        .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
    settle(&mut h);
    (dir, h, id)
}
#[test]
fn api_tabs_share_editor_row_and_preserve_code_buffer_and_close_targets() {
    let (_dir, mut h, id) = fixture([1280., 800.]);
    let original = h
        .state()
        .service
        .active_document()
        .unwrap()
        .text()
        .to_owned();
    h.get_by_label("APIs · Coleções e ambientes").click();
    h.run();
    assert!(h.state().api_ui.open);
    h.state_mut()
        .api_open_tab(api_client::ApiTab::Request(id.clone()));
    h.run();
    let code = h.get_by_label("backend.php").rect();
    let api = h.get_by_label("GET Health").rect();
    assert!((code.top() - api.top()).abs() < 2.);
    assert!((code.height() - api.height()).abs() < 2.);
    assert!(h.query_by_label("Send").is_some());
    // Review actual widget geometry at full width and between two open sidebars.
    for (width, height, database) in [(1280., 800., false), (900., 650., true)] {
        h.state_mut().show_database = database;
        h.set_size(egui::vec2(width, height));
        h.run();
        for label in ["Send", "+ Adicionar linha", "cURL"] {
            let rect = h.get_by_label(label).rect();
            let text = h
                .ctx
                .fonts_mut(|fonts| {
                    fonts.layout_no_wrap(
                        label.into(),
                        FontId::proportional(h.state().tokens.ui_font_size),
                        Color32::WHITE,
                    )
                })
                .size();
            assert!(
                rect.width() >= text.x + 16.,
                "horizontal padding for {label}: {rect:?}"
            );
            assert!(
                rect.height() >= text.y + 8.,
                "vertical padding for {label}: {rect:?}"
            );
            assert!(rect.right() < width - 40., "button outside editor: {label}");
        }
        let url = h.get_by_label("URL da requisição").rect();
        assert!(url.width() >= 180., "URL was squeezed: {url:?}");
        assert!(!url.intersects(h.get_by_label("Send").rect()));
        if !database {
            let method = h.get_by_label("Método HTTP").rect();
            assert!((method.center().y - url.center().y).abs() < 2.);
            assert!((method.center().y - h.get_by_label("Send").rect().center().y).abs() < 2.);
        }
        if let Some(root) = std::env::var_os("CODISSEU_API_POLISH_CAPTURE") {
            let root = PathBuf::from(root);
            fs::create_dir_all(&root).unwrap();
            h.event(egui::Event::PointerGone);
            h.run();
            h.render()
                .unwrap()
                .save(root.join(format!("buttons-{width:.0}.png")))
                .unwrap();
        }
    }
    h.state_mut().show_database = false;
    h.set_size(egui::vec2(1280., 800.));
    h.run();
    h.key_press_modifiers(Modifiers::CTRL, Key::S);
    settle(&mut h);
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        original
    );
    assert!(!h.state().service.active_document().unwrap().is_dirty());
    h.get_by_label("backend.php").click();
    h.run();
    assert!(h.state().api_ui.active.is_none());
    h.get_by_label("GET Health").click();
    h.run();
    assert!(h.state().api_ui.active.is_some());
    h.get_by_label("Fechar API GET Health").click();
    h.run();
    assert!(h.state().api_ui.tabs.is_empty());
    assert!(h.state().api_ui.active.is_none());
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        original
    );
}
#[test]
fn curl_paste_in_native_url_field_converts_request_without_touching_source() {
    let (_dir, mut h, id) = fixture([1280., 800.]);
    h.state_mut()
        .api_open_tab(api_client::ApiTab::Request(id.clone()));
    h.run();
    h.get_by_label("URL da requisição").click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run();
    h.event(egui::Event::Paste("curl 'http://127.0.0.1:9/items?name=caf%C3%A9' \\\n -H 'Content-Type: application/json' --data-raw '{\"name\":\"ação\"}'".into()));
    h.run();
    let r = h.state().service.api.workspace.find_request(&id).unwrap();
    assert_eq!(r.method, "POST", "URL after paste: {:?}", r.url);
    assert!(matches!(&r.body,RequestBody::Raw{text,..}if text.contains("ação")));
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        "<?php\n// source preserved\n"
    );
    assert!(h.state().service.api.responses.is_empty());
}
#[test]
fn api_send_runs_http_and_script_and_empty_document_surface_can_open_requests() {
    let (_dir, mut h, id) = fixture([1280., 800.]);
    let mock = MockDefinition {
        id: "mock".into(),
        name: "Own API".into(),
        routes: vec![MockRoute {
            method: "GET".into(),
            path: "/login".into(),
            status: 200,
            headers: vec![KeyValue::new("Content-Type", "application/json")],
            body: r#"{"data":{"token":"fixture-token"}}"#.into(),
        }],
    };
    h.state_mut().service.api.workspace.mocks.push(mock);
    h.state_mut().service.api.start_mock("mock").unwrap();
    settle(&mut h);
    let url = h.state().service.api.mock_url.clone().unwrap();
    let env = h.state_mut().service.api.workspace.new_environment("Local");
    h.state_mut().service.api.workspace.active_environment = Some(env.clone());
    let r = h
        .state_mut()
        .service
        .api
        .workspace
        .find_request_mut(&id)
        .unwrap();
    r.url = format!("{url}/login");
    r.post_response_script="pm.environment.set('token',pm.response.json().data.token); pm.test('status',()=>pm.response.to.have.status(200));".into();
    h.state_mut()
        .api_open_tab(api_client::ApiTab::Request(id.clone()));
    h.run();
    h.get_by_label("Executar scripts desta request e da collection")
        .click();
    h.run();
    h.get_by_label("Send").click();
    settle(&mut h);
    assert_eq!(h.state().service.api.responses[&id].response.status, 200);
    assert!(h.state().service.api.responses[&id].script_error.is_none());
    assert!(h.state().service.api.responses[&id].script_result.tests[0].passed);
    let env = h
        .state()
        .service
        .api
        .workspace
        .active_environment()
        .unwrap();
    assert_eq!(
        env.variables
            .iter()
            .find(|r| r.key == "token")
            .unwrap()
            .value,
        "fixture-token"
    );
    if let Some(root) = std::env::var_os("CODISSEU_API_CAPTURE") {
        let root = PathBuf::from(root);
        fs::create_dir_all(&root).unwrap();
        h.get_by_label("APIs · Coleções e ambientes").click();
        h.run();
        h.event(egui::Event::PointerGone);
        h.run();
        h.render()
            .unwrap()
            .save(root.join("request-response.png"))
            .unwrap();
    }
    let doc = h.state().service.active.unwrap();
    h.state_mut()
        .service
        .dispatch(Command::Close {
            id: doc,
            discard: false,
        })
        .unwrap();
    settle(&mut h);
    assert!(h.state().service.documents.is_empty());
    assert!(h.query_by_label("Send").is_some());
    h.state_mut().service.api.stop_mock();
    settle(&mut h);
    assert!(h.state().service.api.mock_url.is_none());
}
#[test]
fn returning_after_session_save_failure_keeps_api_service_usable() {
    let (dir, mut h, id) = fixture([1280., 800.]);
    fs::create_dir_all(dir.path().join("profile/session.json")).unwrap();
    h.state_mut().request_exit();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        h.step();
        if matches!(h.state().confirm, Some(Confirm::ExitWithoutSession)) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "{:?}",
            h.state().service.last_error
        );
        thread::sleep(Duration::from_millis(3));
    }
    h.run();
    h.get_by_label("Voltar ao editor").click();
    h.run();
    assert!(!h.state().exit_when_saved);
    assert!(h.state().confirm.is_none());
    assert!(h.state().service.api.can_save());
    assert!(h.state_mut().service.api.send(&id, false).is_ok());
    h.state_mut().service.api.cancel();
    settle(&mut h);
}

#[test]
fn api_folder_creation_uses_standard_plus_and_contextual_children_and_url_params_are_bidirectional()
{
    let (_dir, mut h, id) = fixture([1280., 800.]);
    h.get_by_label("APIs · Coleções e ambientes").click();
    h.run();
    let collection = h.state().service.api.workspace.collections[0].id.clone();
    h.state_mut()
        .api_open_tab(api_client::ApiTab::Collection(collection.clone()));
    h.run();
    let plus = h.get_by_label("Adicionar API").rect();
    assert_eq!(plus.size(), egui::vec2(26., 26.));
    h.get_by_label("Adicionar API").click();
    h.run();
    h.get_by_label("Pasta").click();
    h.run();
    let folder = match h.state().api_ui.active.clone() {
        Some(api_client::ApiTab::Folder(id)) => id,
        _ => panic!("Folder editor must open"),
    };
    h.get_by_label("Nome da pasta").click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run();
    h.event(egui::Event::Text("8.4".into()));
    h.run();
    h.get_by_label("Nova subpasta").click();
    h.run();
    let dev = match h.state().api_ui.active.clone() {
        Some(api_client::ApiTab::Folder(id)) => id,
        _ => panic!(),
    };
    h.get_by_label("Nome da pasta").click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run();
    h.event(egui::Event::Text("DEV".into()));
    h.run();
    h.get_by_label("Nova request").click();
    h.run();
    let child = match h.state().api_ui.active.clone() {
        Some(api_client::ApiTab::Request(id)) => id,
        _ => panic!(),
    };
    assert_eq!(
        h.state().service.api.workspace.request_path(&child),
        ["Backend Services", "8.4", "DEV", "New Request"]
    );
    assert_eq!(
        h.state().service.api.workspace.parent_id(&child),
        Some(dev.clone())
    );
    assert_eq!(
        h.state().service.api.workspace.parent_id(&dev),
        Some(folder)
    );
    h.get_by_label("URL da requisição").click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run();
    h.event(egui::Event::Paste(
        "http://example.test/users/:id?a=1&a=2".into(),
    ));
    h.run();
    let r = h
        .state()
        .service
        .api
        .workspace
        .find_request(&child)
        .unwrap();
    assert_eq!(r.params.len(), 2);
    assert_eq!(r.path_variables[0].key, "id");
    h.get_by_label("Valor request.params linha 1").click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run();
    h.event(egui::Event::Text("3".into()));
    h.run();
    assert!(
        h.state()
            .service
            .api
            .workspace
            .find_request(&child)
            .unwrap()
            .display_url()
            .contains("a=3&a=2")
    );
    h.get_by_label("URL da requisição").click();
    h.run();
    h.key_press(Key::End);
    h.run();
    h.event(egui::Event::Text("&b=4".into()));
    h.run();
    let r = h
        .state()
        .service
        .api
        .workspace
        .find_request(&child)
        .unwrap();
    assert_eq!(r.params[0].value, "3");
    assert_eq!(r.params.last().unwrap().key, "b");
    assert_eq!(r.params.last().unwrap().value, "4");
    if let Some(root) = std::env::var_os("CODISSEU_API_WORKFLOW_CAPTURE") {
        let root = PathBuf::from(root);
        fs::create_dir_all(&root).unwrap();
        h.event(egui::Event::PointerGone);
        h.run();
        h.render()
            .unwrap()
            .save(root.join("folders-request.png"))
            .unwrap();
    }
    h.state_mut().api_open_tab(api_client::ApiTab::Folder(dev));
    h.run();
    assert!(h.query_by_label("Nome da pasta").is_some());
    assert!(h.query_by_label("Executar pasta").is_some());
    h.state_mut().api_open_tab(api_client::ApiTab::Request(id));
    h.run();
    assert!(
        h.state()
            .service
            .active_document()
            .unwrap()
            .text()
            .contains("source preserved")
    );
    settle(&mut h);
}
