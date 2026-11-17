use codisseu_core::{
    AppService, Command, DocumentId, EditKind, Selection, TextDelta,
    remote::{ConnectionConfig, Engine, RemoteCommand},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn settle(service: &mut AppService) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline, "{:?}", service.last_error);
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn run(service: &mut AppService, command: Command) {
    service.dispatch(command).unwrap();
    settle(service);
    assert!(service.last_error.is_none(), "{:?}", service.last_error);
    assert!(
        service.remote_db.error.is_none(),
        "{:?}",
        service.remote_db.error
    );
}
fn source(service: &mut AppService, id: &str, name: &str, engine: Engine) {
    run(
        service,
        Command::RemoteDatabase(RemoteCommand::Save(ConnectionConfig {
            id: id.into(),
            name: name.into(),
            engine,
            save_credentials: false,
            ..Default::default()
        })),
    );
}
fn query(service: &mut AppService, owner: &str) -> (DocumentId, PathBuf) {
    run(
        service,
        Command::RemoteDatabase(RemoteCommand::CreateConsoleFor(owner.into())),
    );
    let doc = service.active_document().unwrap();
    (doc.id, doc.path.clone())
}
fn replace(service: &mut AppService, id: DocumentId, text: &str) {
    let doc = service.documents.iter().find(|doc| doc.id == id).unwrap();
    service
        .dispatch(Command::EditTransaction {
            id,
            expected_version: doc.version,
            delta: TextDelta {
                range: 0..doc.len_chars(),
                insert: text.into(),
            },
            kind: EditKind::Atomic,
            before: doc.selection(),
            after: Selection::caret(0),
        })
        .unwrap();
}
fn document_text(service: &AppService, id: DocumentId) -> &str {
    service
        .documents
        .iter()
        .find(|doc| doc.id == id)
        .unwrap()
        .text()
}
fn owns(service: &AppService, path: &Path, owner: &str) {
    assert_eq!(service.remote_db.console_target(path).unwrap(), Some(owner));
}

#[test]
fn same_named_queries_keep_distinct_buffers_rename_delete_and_restored_session_paths() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let profile = root.path().join("profile/session.json");
    let mut app = AppService::new(profile.clone());
    settle(&mut app);
    run(&mut app, Command::OpenWorkspace(project));
    source(&mut app, "dev", "ExampleGroup DEV", Engine::Mysql);
    source(&mut app, "qa", "ExampleGroup QA", Engine::Mysql);
    source(&mut app, "prod", "ExampleGroup PROD", Engine::Postgres);
    source(&mut app, "cache", "Redis DEV", Engine::Redis);
    let (dev_id, dev) = query(&mut app, "dev");
    let (qa_id, qa) = query(&mut app, "qa");
    let (prod_id, prod) = query(&mut app, "prod");
    let (cache_id, cache) = query(&mut app, "cache");
    for file in [&dev, &qa, &prod] {
        assert_eq!(file.file_name().unwrap(), "query-1.sql");
    }
    assert_eq!(cache.file_name().unwrap(), "query-1.txt");
    assert_ne!(dev.parent(), qa.parent());
    assert_ne!(qa.parent(), prod.parent());
    assert_eq!(app.documents.len(), 4);
    assert_eq!(
        app.document_views()
            .iter()
            .map(|view| view.tabs().len())
            .sum::<usize>(),
        4
    );
    for (id, content) in [
        (dev_id, "SELECT 'DEV';"),
        (qa_id, "SELECT 'QA';"),
        (prod_id, "SELECT 'PROD';"),
        (cache_id, "PING\n"),
    ] {
        replace(&mut app, id, content);
        run(&mut app, Command::Save { id });
    }
    replace(&mut app, dev_id, "SELECT 'DEV dirty';");
    run(
        &mut app,
        Command::RemoteDatabase(RemoteCommand::RenameConsole {
            path: dev.clone(),
            name: "Pedidos".into(),
        }),
    );
    let renamed = app
        .documents
        .iter()
        .find(|doc| doc.id == dev_id)
        .unwrap()
        .path
        .clone();
    assert_eq!(renamed, dev.with_file_name("Pedidos.sql"));
    assert_eq!(document_text(&app, dev_id), "SELECT 'DEV dirty';");
    assert_eq!(document_text(&app, qa_id), "SELECT 'QA';");
    assert_eq!(document_text(&app, prod_id), "SELECT 'PROD';");
    assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "SELECT 'DEV';");
    run(&mut app, Command::Undo { id: dev_id });
    assert_eq!(document_text(&app, dev_id), "SELECT 'DEV';");
    run(&mut app, Command::Redo { id: dev_id });
    run(&mut app, Command::Save { id: dev_id });
    run(
        &mut app,
        Command::RemoteDatabase(RemoteCommand::DeleteConsole(qa.clone())),
    );
    assert!(!app.documents.iter().any(|doc| doc.id == qa_id));
    assert_eq!(app.documents.len(), 3);
    assert_eq!(
        app.document_views()
            .iter()
            .map(|view| view.tabs().len())
            .sum::<usize>(),
        3
    );
    assert!(!qa.exists());
    assert_eq!(std::fs::read_to_string(&prod).unwrap(), "SELECT 'PROD';");
    assert_eq!(std::fs::read_to_string(&cache).unwrap(), "PING\n");
    for (file, owner) in [(&renamed, "dev"), (&prod, "prod"), (&cache, "cache")] {
        owns(&app, file, owner);
    }
    run(&mut app, Command::Activate { id: prod_id });
    run(&mut app, Command::PersistSession);
    drop(app);
    let mut restored = AppService::new(profile);
    settle(&mut restored);
    run(&mut restored, Command::RestoreSession);
    assert_eq!(restored.documents.len(), 3);
    assert_eq!(restored.active_document().unwrap().path, prod);
    let actual: BTreeMap<_, _> = restored
        .documents
        .iter()
        .map(|doc| (doc.path.clone(), doc.text()))
        .collect();
    assert_eq!(
        actual,
        BTreeMap::from([
            (renamed.clone(), "SELECT 'DEV dirty';"),
            (prod.clone(), "SELECT 'PROD';"),
            (cache.clone(), "PING\n")
        ])
    );
    assert!(!restored.remote_db.console_bindings.contains_key(&qa));
    assert!(restored.remote_db.active.is_none());
    for (file, owner) in [(&renamed, "dev"), (&prod, "prod"), (&cache, "cache")] {
        owns(&restored, file, owner);
    }
}

#[test]
fn existing_flat_query_and_new_namespaced_query_restore_without_relocation() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile/session.json");
    let consoles = profile.parent().unwrap().join("consoles");
    std::fs::create_dir_all(&consoles).unwrap();
    let legacy = consoles.join("query-1.sql");
    std::fs::write(&legacy, "SELECT 'legacy';").unwrap();
    std::fs::write(
        profile.with_extension("datasources.console-bindings.json"),
        serde_json::to_vec(&BTreeMap::from([(legacy.clone(), "dev")])).unwrap(),
    )
    .unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut app = AppService::new(profile.clone());
    settle(&mut app);
    run(&mut app, Command::OpenWorkspace(project));
    run(&mut app, Command::ClaimLegacyDatabase);
    source(&mut app, "dev", "ExampleGroup DEV", Engine::Mysql);
    source(&mut app, "qa", "ExampleGroup QA", Engine::Postgres);
    run(&mut app, Command::OpenFile(legacy.clone()));
    let legacy_id = app.active_document().unwrap().id;
    let (_, dev) = query(&mut app, "dev");
    let (_, qa) = query(&mut app, "qa");
    assert_eq!(dev.file_name().unwrap(), "query-2.sql");
    assert_eq!(qa.file_name().unwrap(), "query-1.sql");
    source(&mut app, "dev", "DEV novo nome", Engine::Mysql);
    assert_eq!(
        app.documents
            .iter()
            .find(|doc| doc.id == legacy_id)
            .unwrap()
            .path,
        legacy
    );
    run(&mut app, Command::PersistSession);
    drop(app);
    let mut restored = AppService::new(profile);
    settle(&mut restored);
    run(&mut restored, Command::RestoreSession);
    assert_eq!(restored.documents.len(), 3);
    assert_eq!(
        std::fs::read_to_string(&legacy).unwrap(),
        "SELECT 'legacy';"
    );
    assert!(
        restored
            .documents
            .iter()
            .any(|doc| doc.path == legacy && doc.text() == "SELECT 'legacy';")
    );
    owns(&restored, &legacy, "dev");
    owns(&restored, &dev, "dev");
    owns(&restored, &qa, "qa");
    assert_eq!(
        restored
            .remote_db
            .profiles
            .iter()
            .find(|source| source.id == "dev")
            .unwrap()
            .name,
        "DEV novo nome"
    );
}

#[test]
fn explicit_legacy_rename_moves_dirty_query_to_its_source_without_changing_other_source() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile/session.json");
    let consoles = profile.parent().unwrap().join("consoles");
    std::fs::create_dir_all(&consoles).unwrap();
    let dev = consoles.join("query-1.sql");
    let qa = consoles.join("query-2.sql");
    std::fs::write(&dev, "SELECT 'DEV';").unwrap();
    std::fs::write(&qa, "SELECT 'QA saved';").unwrap();
    std::fs::write(
        profile.with_extension("datasources.console-bindings.json"),
        serde_json::to_vec(&BTreeMap::from([(dev.clone(), "dev"), (qa.clone(), "qa")])).unwrap(),
    )
    .unwrap();
    let project = root.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut app = AppService::new(profile.clone());
    settle(&mut app);
    run(&mut app, Command::OpenWorkspace(project));
    run(&mut app, Command::ClaimLegacyDatabase);
    run(&mut app, Command::OpenFile(dev.clone()));
    let dev_id = app.active_document().unwrap().id;
    run(&mut app, Command::OpenFile(qa.clone()));
    let qa_id = app.active_document().unwrap().id;
    replace(&mut app, qa_id, "SELECT 'QA dirty';");
    let target = app.remote_db.console_rename_target(&qa, "query-1").unwrap();
    assert_ne!(target, dev);
    run(
        &mut app,
        Command::RemoteDatabase(RemoteCommand::RenameConsole {
            path: qa.clone(),
            name: "query-1".into(),
        }),
    );
    let moved = app.documents.iter().find(|doc| doc.id == qa_id).unwrap();
    assert_eq!(moved.path, target);
    assert!(moved.is_dirty());
    assert_eq!(moved.text(), "SELECT 'QA dirty';");
    assert_eq!(target.file_name().unwrap(), "query-1.sql");
    assert_eq!(target.parent().unwrap().parent(), Some(consoles.as_path()));
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "SELECT 'QA saved';"
    );
    assert!(!qa.exists());
    assert_eq!(document_text(&app, dev_id), "SELECT 'DEV';");
    assert_eq!(std::fs::read_to_string(&dev).unwrap(), "SELECT 'DEV';");
    run(&mut app, Command::Undo { id: qa_id });
    assert_eq!(document_text(&app, qa_id), "SELECT 'QA saved';");
    run(&mut app, Command::Redo { id: qa_id });
    run(&mut app, Command::Save { id: qa_id });
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "SELECT 'QA dirty';"
    );
    owns(&app, &target, "qa");
    owns(&app, &dev, "dev");
    run(&mut app, Command::PersistSession);
    drop(app);
    let mut restored = AppService::new(profile);
    settle(&mut restored);
    run(&mut restored, Command::RestoreSession);
    assert_eq!(restored.documents.len(), 2);
    assert!(
        restored
            .documents
            .iter()
            .any(|doc| doc.path == target && doc.text() == "SELECT 'QA dirty';")
    );
    assert!(
        restored
            .documents
            .iter()
            .any(|doc| doc.path == dev && doc.text() == "SELECT 'DEV';")
    );
    assert!(!restored.remote_db.console_bindings.contains_key(&qa));
    assert_eq!(std::fs::read_to_string(&dev).unwrap(), "SELECT 'DEV';");
    owns(&restored, &target, "qa");
    owns(&restored, &dev, "dev");
}
