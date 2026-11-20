//! Drawing files travel through the same asynchronous document commands as code.
use codisseu_core::{AppService, Command};
use codisseu_draw::{Element, ElementKind, Scene};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn settle(service: &mut AppService) {
    let end = Instant::now() + Duration::from_secs(15);
    let mut idle = 0;
    while idle < 3 {
        service.poll();
        idle = if service.is_busy() { 0 } else { idle + 1 };
        assert!(
            Instant::now() < end,
            "drawing worker timed out: {} {:?}",
            service.status,
            service.last_error
        );
        thread::sleep(Duration::from_millis(2));
    }
}
fn project(service: &mut AppService, path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while service.workspace_change_blocker(path).is_some() {
        service.poll();
        assert!(
            Instant::now() < deadline,
            "workspace readiness: {:?}",
            service.workspace_change_blocker(path)
        );
        thread::sleep(Duration::from_millis(2));
    }
    service
        .dispatch(Command::OpenWorkspace(path.into()))
        .unwrap();
    settle(service);
    assert!(service.last_error.is_none(), "{:?}", service.last_error);
    assert!(
        service.drawings.error.is_none(),
        "{:?}",
        service.drawings.error
    );
}
fn create(service: &mut AppService, name: &str) -> u64 {
    service.drawings.create(name.into()).unwrap();
    settle(service);
    assert!(
        service.drawings.error.is_none(),
        "{:?}",
        service.drawings.error
    );
    assert!(service.last_error.is_none(), "{:?}", service.last_error);
    let document = service.active_document().expect("created drawing opened");
    assert_eq!(document.path.extension().unwrap(), "excalidraw");
    document.id
}
fn edit(service: &mut AppService, id: u64, scene: &Scene) {
    let version = service
        .documents
        .iter()
        .find(|d| d.id == id)
        .unwrap()
        .version;
    service.dispatch(Command::EndEditGroup { id }).unwrap();
    service
        .dispatch(Command::Edit {
            id,
            expected_version: version,
            text: scene.to_json().unwrap(),
        })
        .unwrap();
    service.dispatch(Command::EndEditGroup { id }).unwrap();
}

#[test]
fn drawing_create_edit_history_save_and_reopen_use_shared_documents() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    let profile = temp.path().join("profile/session.json");
    let mut service = AppService::new(profile.clone());
    project(&mut service, &root);
    let id = create(&mut service, "Arquitetura ação");
    let path = service.active_document().unwrap().path.clone();
    let mut scene = Scene::from_json(service.active_document().unwrap().text()).unwrap();
    scene.elements.push(Element::new(
        ElementKind::Rectangle,
        30.0,
        50.0,
        200.0,
        100.0,
    ));
    edit(&mut service, id, &scene);
    assert!(service.active_document().unwrap().is_dirty());
    service.dispatch(Command::Undo { id }).unwrap();
    assert!(
        Scene::from_json(service.active_document().unwrap().text())
            .unwrap()
            .elements
            .is_empty()
    );
    service.dispatch(Command::Redo { id }).unwrap();
    assert_eq!(
        Scene::from_json(service.active_document().unwrap().text()).unwrap(),
        scene
    );
    service.dispatch(Command::Save { id }).unwrap();
    settle(&mut service);
    assert!(service.last_error.is_none(), "{:?}", service.last_error);
    assert_eq!(
        Scene::from_json(&fs::read_to_string(&path).unwrap()).unwrap(),
        scene
    );
    assert!(!service.active_document().unwrap().is_dirty());
    service
        .dispatch(Command::Close { id, discard: false })
        .unwrap();
    assert!(service.documents.is_empty());
    drop(service);
    let mut restarted = AppService::new(profile);
    project(&mut restarted, &root);
    assert_eq!(restarted.documents.len(), 1);
    assert_eq!(restarted.active_document().unwrap().path, path);
    assert_eq!(
        Scene::from_json(restarted.active_document().unwrap().text()).unwrap(),
        scene
    );
}

#[test]
fn drawing_catalog_is_isolated_between_projects_and_restores_closed_tabs() {
    let temp = tempfile::tempdir().unwrap();
    let a = temp.path().join("A");
    let b = temp.path().join("B");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    let mut service = AppService::new(temp.path().join("profile/session.json"));
    project(&mut service, &a);
    let id = create(&mut service, "flow");
    let drawing = service.active_document().unwrap().path.clone();
    service
        .dispatch(Command::Close { id, discard: false })
        .unwrap();
    settle(&mut service);
    assert!(
        service.documents.is_empty(),
        "closing a clean drawing must stay closed in the current project"
    );
    project(&mut service, &b);
    assert!(service.drawings.paths.is_empty());
    assert!(service.documents.is_empty());
    project(&mut service, &a);
    assert_eq!(service.drawings.paths, vec![drawing.clone()]);
    assert_eq!(service.documents.len(), 1);
    assert_eq!(service.documents[0].path, drawing);
}

#[test]
fn workspace_file_identity_keeps_drawing_links_separate_with_shared_roots() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("src");
    fs::create_dir(&root).unwrap();
    let a = temp.path().join("A.code-workspace");
    let b = temp.path().join("B.code-workspace");
    let workspace = r#"{"folders":[{"path":"src"}]}"#;
    fs::write(&a, workspace).unwrap();
    fs::write(&b, workspace).unwrap();
    let profile = temp.path().join("profile/session.json");
    let mut service = AppService::new(profile.clone());
    project(&mut service, &a);
    let id = create(&mut service, "flow");
    let drawing = service.active_document().unwrap().path.clone();
    service
        .dispatch(Command::Close { id, discard: false })
        .unwrap();
    project(&mut service, &b);
    assert!(service.drawings.paths.is_empty());
    assert!(service.documents.is_empty());
    project(&mut service, &a);
    assert_eq!(service.active_document().unwrap().path, drawing);
    service.dispatch(Command::PersistSession).unwrap();
    settle(&mut service);
    assert!(service.last_error.is_none(), "{:?}", service.last_error);
    drop(service);
    let mut restored = AppService::new(profile);
    restored.dispatch(Command::RestoreSession).unwrap();
    settle(&mut restored);
    assert!(restored.last_error.is_none(), "{:?}", restored.last_error);
    assert_eq!(restored.drawings.paths, vec![drawing.clone()]);
    assert_eq!(
        restored
            .documents
            .iter()
            .filter(|d| d.path == drawing)
            .count(),
        1,
        "session restore and catalog must deduplicate the same drawing"
    );
}

#[test]
fn drawing_save_conflict_preserves_external_file_and_unsaved_json() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    let mut service = AppService::new(temp.path().join("profile/session.json"));
    project(&mut service, &root);
    let id = create(&mut service, "flow");
    let path = service.active_document().unwrap().path.clone();
    let mut local = Scene::default();
    local
        .elements
        .push(Element::new(ElementKind::Text, 5.0, 10.0, 200.0, 30.0));
    local.elements[0].text = "ação local 🦀".into();
    edit(&mut service, id, &local);
    let mut external = Scene::default();
    external
        .elements
        .push(Element::new(ElementKind::Ellipse, 1.0, 2.0, 50.0, 70.0));
    let external_json = external.to_json().unwrap();
    fs::write(&path, &external_json).unwrap();
    service.dispatch(Command::Save { id }).unwrap();
    settle(&mut service);
    assert!(
        service
            .last_error
            .as_ref()
            .is_some_and(|e| e.contains("Conflito")),
        "{:?}",
        service.last_error
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), external_json);
    assert_eq!(
        Scene::from_json(service.active_document().unwrap().text()).unwrap(),
        local
    );
    assert!(service.active_document().unwrap().is_dirty());
}

#[test]
fn invalid_catalog_does_not_retry_registration_forever_or_overwrite_metadata() {
    use sha2::{Digest, Sha256};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let drawing = root.join("external.excalidraw");
    fs::write(&drawing, Scene::default().to_json().unwrap()).unwrap();
    let profile = temp.path().join("profile/session.json");
    let directory = profile.with_extension("drawings");
    fs::create_dir_all(&directory).unwrap();
    let name = format!(
        "{:x}.json",
        Sha256::digest(root.as_os_str().as_encoded_bytes())
    );
    let metadata = directory.join(name);
    fs::write(&metadata, "invalid catalog kept for recovery").unwrap();
    let mut service = AppService::new(profile);
    service.dispatch(Command::OpenWorkspace(root)).unwrap();
    settle(&mut service);
    assert!(service.drawings.error.is_some());
    service
        .dispatch(Command::OpenFile(drawing.clone()))
        .unwrap();
    settle(&mut service);
    assert!(!service.is_busy());
    for _ in 0..10 {
        service.poll();
        assert!(
            !service.drawings.is_busy(),
            "failed registration must require explicit retry"
        );
    }
    assert_eq!(service.active_document().unwrap().path, drawing);
    assert_eq!(
        fs::read_to_string(metadata).unwrap(),
        "invalid catalog kept for recovery"
    );
}
