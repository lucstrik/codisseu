use codisseu_core::{
    AppService, Command, EditKind, Selection, TextDelta,
    remote::{ConnectionConfig, RemoteCommand},
};
use std::time::{Duration, Instant};
fn settle(s: &mut AppService) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while s.is_busy() {
        s.poll();
        assert!(Instant::now() < deadline, "{:?}", s.last_error);
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn renamed_query_preserves_dirty_buffer_undo_save_and_reopens_after_project_close_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("profile/session.json");
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut s = AppService::new(profile.clone());
    settle(&mut s);
    for command in [
        Command::OpenWorkspace(project),
        Command::RemoteDatabase(RemoteCommand::Save(ConnectionConfig {
            id: "fixture".into(),
            name: "Fixture".into(),
            save_credentials: false,
            ..Default::default()
        })),
        Command::RemoteDatabase(RemoteCommand::CreateConsoleFor("fixture".into())),
    ] {
        s.dispatch(command).unwrap();
        settle(&mut s);
    }
    let doc = s.active_document().unwrap();
    let id = doc.id;
    let old = doc.path.clone();
    assert_eq!(old.file_name().unwrap(), "query-1.sql");
    s.dispatch(Command::EditTransaction {
        id,
        expected_version: doc.version,
        delta: TextDelta {
            range: 0..doc.len_chars(),
            insert: "SELECT 'saved';\n".into(),
        },
        kind: EditKind::Atomic,
        before: doc.selection(),
        after: Selection::caret(0),
    })
    .unwrap();
    s.dispatch(Command::Save { id }).unwrap();
    settle(&mut s);
    let doc = s.active_document().unwrap();
    s.dispatch(Command::EditTransaction {
        id,
        expected_version: doc.version,
        delta: TextDelta {
            range: 0..doc.len_chars(),
            insert: "SELECT 'unsaved';\n".into(),
        },
        kind: EditKind::Atomic,
        before: doc.selection(),
        after: Selection::caret(0),
    })
    .unwrap();
    s.dispatch(Command::RemoteDatabase(RemoteCommand::RenameConsole {
        path: old.clone(),
        name: "Relatório".into(),
    }))
    .unwrap();
    assert!(s.dispatch(Command::Save { id }).is_err());
    assert!(s.dispatch(Command::PersistSession).is_err());
    settle(&mut s);
    let renamed = s
        .documents
        .iter()
        .find(|doc| doc.id == id)
        .unwrap()
        .path
        .clone();
    assert_eq!(renamed.file_name().unwrap(), "Relatório.sql");
    assert!(!old.exists());
    assert_eq!(
        s.documents.iter().find(|doc| doc.id == id).unwrap().text(),
        "SELECT 'unsaved';\n"
    );
    assert!(
        s.documents
            .iter()
            .find(|doc| doc.id == id)
            .unwrap()
            .is_dirty()
    );
    assert_eq!(
        std::fs::read_to_string(&renamed).unwrap(),
        "SELECT 'saved';\n"
    );
    s.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(
        s.documents.iter().find(|doc| doc.id == id).unwrap().text(),
        "SELECT 'saved';\n"
    );
    s.dispatch(Command::Redo { id }).unwrap();
    s.dispatch(Command::Save { id }).unwrap();
    settle(&mut s);
    s.dispatch(Command::CloseWorkspace).unwrap();
    assert!(s.documents.is_empty());
    assert!(s.remote_db.console_bindings.contains_key(&renamed));
    drop(s);
    let mut s = AppService::new(profile);
    settle(&mut s);
    assert!(s.documents.is_empty());
    assert!(s.remote_db.active.is_none());
    assert!(s.remote_db.console_bindings.contains_key(&renamed));
    s.dispatch(Command::OpenFile(renamed)).unwrap();
    settle(&mut s);
    assert_eq!(s.active_document().unwrap().text(), "SELECT 'unsaved';\n");
    assert_eq!(
        s.remote_db
            .console_target(&s.active_document().unwrap().path)
            .unwrap(),
        Some("fixture")
    );
}

#[test]
fn deleting_query_rejects_dirty_buffer_then_closes_clean_document_and_prevents_resave() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("session.json");
    let mut s = AppService::new(profile.clone());
    settle(&mut s);
    for command in [
        RemoteCommand::Save(ConnectionConfig {
            id: "fixture".into(),
            save_credentials: false,
            ..Default::default()
        }),
        RemoteCommand::CreateConsoleFor("fixture".into()),
    ] {
        s.dispatch(Command::RemoteDatabase(command)).unwrap();
        settle(&mut s);
    }
    let doc = s.active_document().unwrap();
    let path = doc.path.clone();
    let id = doc.id;
    s.dispatch(Command::EditTransaction {
        id,
        expected_version: doc.version,
        delta: TextDelta {
            range: 0..doc.len_chars(),
            insert: "SELECT 2;".into(),
        },
        kind: EditKind::Atomic,
        before: doc.selection(),
        after: Selection::caret(0),
    })
    .unwrap();
    assert!(
        s.dispatch(Command::RemoteDatabase(RemoteCommand::DeleteConsole(
            path.clone()
        )))
        .is_err()
    );
    assert!(path.exists());
    assert!(s.active_document().unwrap().is_dirty());
    s.dispatch(Command::Save { id }).unwrap();
    settle(&mut s);
    s.dispatch(Command::RemoteDatabase(RemoteCommand::DeleteConsole(
        path.clone(),
    )))
    .unwrap();
    assert!(s.dispatch(Command::Undo { id }).is_err());
    assert!(s.dispatch(Command::Save { id }).is_err());
    settle(&mut s);
    assert!(s.documents.is_empty());
    assert!(s.document_views().is_empty());
    assert!(!path.exists());
    drop(s);
    let mut s = AppService::new(profile);
    settle(&mut s);
    assert!(s.remote_db.console_bindings.is_empty());
}
