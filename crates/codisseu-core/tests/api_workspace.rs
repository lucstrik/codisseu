//! Project lifecycle integration uses the same commands as menus/CLI, real files.
use codisseu_core::{AppService, Command};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn settle(service: &mut AppService) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline, "{:?}", service.last_error);
        thread::sleep(Duration::from_millis(2));
    }
    service.poll();
    assert!(service.last_error.is_none(), "{:?}", service.last_error);
}
#[test]
fn project_open_switch_close_and_restore_keep_api_data_scoped() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a");
    let b = directory.path().join("b");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    let session = directory.path().join("profile/session.json");
    let mut service = AppService::new(session.clone());
    service.dispatch(Command::OpenWorkspace(a.clone())).unwrap();
    settle(&mut service);
    assert_eq!(service.api.project.as_ref(), Some(&a));
    assert!(!service.api.loading);
    service.api.new_collection("A endpoints");
    service.dispatch(Command::OpenWorkspace(b.clone())).unwrap();
    settle(&mut service);
    assert_eq!(service.api.project.as_ref(), Some(&b));
    assert!(service.api.workspace.collections.is_empty());
    service.api.new_collection("B endpoints");
    service.dispatch(Command::CloseWorkspace).unwrap();
    settle(&mut service);
    assert!(service.workspace.is_none());
    assert!(service.api.project.is_none());
    assert!(service.api.workspace.collections.is_empty());
    service.dispatch(Command::OpenWorkspace(a.clone())).unwrap();
    settle(&mut service);
    assert_eq!(service.api.workspace.collections[0].name, "A endpoints");
    service.api.save().unwrap();
    settle(&mut service);
    service.dispatch(Command::PersistSession).unwrap();
    settle(&mut service);
    service.api.shutdown();
    settle(&mut service);
    drop(service);
    let mut restored = AppService::new(session);
    restored.dispatch(Command::RestoreSession).unwrap();
    settle(&mut restored);
    assert_eq!(restored.workspace.as_ref(), Some(&a));
    assert_eq!(restored.api.project.as_ref(), Some(&a));
    assert_eq!(restored.api.workspace.collections[0].name, "A endpoints");
    restored.dispatch(Command::OpenWorkspace(b)).unwrap();
    settle(&mut restored);
    assert_eq!(restored.api.workspace.collections[0].name, "B endpoints");
    restored.api.shutdown();
    settle(&mut restored);
}
#[test]
fn edits_while_folder_is_loading_do_not_switch_api_scope_when_switch_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a");
    let b = directory.path().join("b");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    let file = a.join("a.php");
    fs::write(&file, "<?php\n").unwrap();
    let mut service = AppService::new(directory.path().join("profile/session.json"));
    service.dispatch(Command::OpenWorkspace(a.clone())).unwrap();
    settle(&mut service);
    service.dispatch(Command::OpenFile(file.clone())).unwrap();
    settle(&mut service);
    service.api.new_collection("Keep A");
    service.dispatch(Command::OpenWorkspace(b)).unwrap();
    let doc = service.active_document().unwrap();
    let id = doc.id;
    let expected_version = doc.version;
    service
        .dispatch(Command::Edit {
            id,
            expected_version,
            text: "<?php\n// unsaved while loading\n".into(),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    assert!(
        service
            .last_error
            .as_ref()
            .is_some_and(|e| e.contains("alterações não salvas"))
    );
    assert_eq!(service.workspace.as_ref(), Some(&a));
    assert_eq!(service.api.project.as_ref(), Some(&a));
    assert_eq!(service.api.workspace.collections[0].name, "Keep A");
    assert!(service.active_document().unwrap().is_dirty());
    assert_eq!(fs::read_to_string(file).unwrap(), "<?php\n");
}

#[test]
fn neighboring_workspace_files_keep_requests_environments_and_legacy_carrier_separate() {
    let directory = tempfile::tempdir().unwrap();
    let carrier = directory.path().join("carrier");
    let roots = [
        directory.path().join("shipping"),
        directory.path().join("game"),
    ];
    for root in [&carrier, &roots[0], &roots[1]] {
        fs::create_dir(root).unwrap();
    }
    let projects = [
        carrier.join("shipping.code-workspace"),
        carrier.join("game.code-workspace"),
    ];
    for (project, root) in projects.iter().zip(&roots) {
        fs::write(
            project,
            serde_json::json!({"folders":[{"path":root}]}).to_string(),
        )
        .unwrap();
    }
    let profile = directory.path().join("profile/session.json");
    let storage = codisseu_core::api::ApiStorage::new(profile.with_extension("api"));
    let mut legacy = codisseu_core::api::ApiWorkspace::default();
    legacy.new_collection("Legacy carrier, unassigned");
    storage.save(&carrier, &legacy).unwrap();
    let legacy_path = storage.path_for(&carrier).unwrap();
    let legacy_bytes = fs::read(&legacy_path).unwrap();
    let mut service = AppService::new(profile.clone());
    for (project, name) in projects.iter().zip(["Shipping", "Game"]) {
        service
            .dispatch(Command::OpenWorkspace(project.clone()))
            .unwrap();
        settle(&mut service);
        assert_eq!(service.api.project.as_ref(), Some(project));
        assert!(service.api.workspace.collections.is_empty());
        assert!(service.api.workspace.environments.is_empty());
        let collection = service.api.new_collection(name);
        let request = service
            .api
            .workspace
            .new_request(&collection, format!("{name} request"))
            .unwrap();
        service
            .api
            .workspace
            .find_request_mut(&request)
            .unwrap()
            .url = format!("https://{}.invalid/api", name.to_ascii_lowercase());
        service
            .api
            .workspace
            .new_environment(format!("{name} environment"));
    }
    service
        .dispatch(Command::OpenWorkspace(projects[0].clone()))
        .unwrap();
    settle(&mut service);
    assert_eq!(service.api.workspace.collections[0].name, "Shipping");
    assert_eq!(
        service.api.workspace.environments[0].name,
        "Shipping environment"
    );
    assert_eq!(service.api.workspace.collections.len(), 1);
    service.dispatch(Command::PersistSession).unwrap();
    settle(&mut service);
    service.api.shutdown();
    settle(&mut service);
    drop(service);

    let mut restored = AppService::new(profile);
    restored.dispatch(Command::RestoreSession).unwrap();
    settle(&mut restored);
    assert_eq!(restored.api.project.as_ref(), Some(&projects[0]));
    assert_eq!(restored.api.workspace.collections[0].name, "Shipping");
    restored
        .dispatch(Command::OpenWorkspace(projects[1].clone()))
        .unwrap();
    settle(&mut restored);
    assert_eq!(restored.api.workspace.collections[0].name, "Game");
    assert_eq!(
        restored.api.workspace.environments[0].name,
        "Game environment"
    );
    assert_eq!(fs::read(legacy_path).unwrap(), legacy_bytes);
    restored.api.shutdown();
    settle(&mut restored);
}

#[test]
fn distinct_window_sessions_use_the_same_project_profile_without_exposing_another_project() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("shipping");
    let b = directory.path().join("game");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    let profile = directory.path().join("profile/session.json");
    let mut first =
        AppService::new_with_profile(directory.path().join("first.json"), profile.clone());
    first.dispatch(Command::OpenWorkspace(a.clone())).unwrap();
    settle(&mut first);
    first.api.new_collection("Shipping endpoints");
    first.api.save().unwrap();
    settle(&mut first);
    first.api.shutdown();
    settle(&mut first);
    drop(first);

    let mut second = AppService::new_with_profile(directory.path().join("second.json"), profile);
    second.dispatch(Command::OpenWorkspace(b)).unwrap();
    settle(&mut second);
    assert!(second.api.workspace.collections.is_empty());
    second.dispatch(Command::OpenWorkspace(a)).unwrap();
    settle(&mut second);
    assert_eq!(
        second.api.workspace.collections[0].name,
        "Shipping endpoints"
    );
    second.dispatch(Command::CloseWorkspace).unwrap();
    settle(&mut second);
    assert!(second.api.project.is_none());
    assert!(second.api.workspace.collections.is_empty());
    assert!(second.api.responses.is_empty());
    assert!(second.api.history.is_empty());
    second.api.shutdown();
    settle(&mut second);
}
