//! FULL-01 M2, one real application service with PHP, TS and SQLite.
use codisseu_core::{AppService, Command, DbCommand, ProviderConfig, ProviderState};
use rusqlite::{Connection, types::Value};
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

fn wait(app: &mut AppService, predicate: impl Fn(&AppService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(app.last_error.is_none(), "{:?}", app.last_error);
        assert!(
            app.language.last_error.is_none(),
            "{:?}",
            app.language.last_error
        );
        assert!(app.db.last_error.is_none(), "{:?}", app.db.last_error);
        assert!(
            Instant::now() < deadline,
            "Timed out: {} {:?}",
            app.status,
            app.language.statuses
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn idle(app: &mut AppService) {
    wait(app, |s| !s.is_busy());
}

#[test]
#[ignore = "explicit FULL-01 with installed PHP/TS providers; requires CODISSEU_PROVIDER_CONFIG"]
fn php_typescript_sqlite_and_session_in_one_application_service() {
    let config = PathBuf::from(
        std::env::var_os("CODISSEU_PROVIDER_CONFIG")
            .expect("CODISSEU_PROVIDER_CONFIG required; no silent skip"),
    );
    let configs: Vec<ProviderConfig> = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let php = workspace.join("index.php");
    let ts = workspace.join("app.ts");
    let sql = workspace.join("query.sql");
    let db = workspace.join("fixture.sqlite");
    fs::write(&php, "<?php echo '🌧'; $broken = ;\n").unwrap();
    fs::write(
        &ts,
        "export const emoji = '🌧'; export const count: number = 'wrong';\n",
    )
    .unwrap();
    fs::write(&sql, "SELECT id, name, note FROM people ORDER BY id;\n").unwrap();
    fs::write(
        workspace.join("tsconfig.json"),
        r#"{"compilerOptions":{"strict":true,"target":"ES2022","noEmit":true},"include":["*.ts"]}"#,
    )
    .unwrap();
    {
        let connection = Connection::open(&db).unwrap();
        connection.execute_batch("CREATE TABLE people(id INTEGER PRIMARY KEY, name TEXT NOT NULL, note TEXT); INSERT INTO people VALUES(1,'Olá',NULL),(2,'Codisseu','');").unwrap();
    }
    let session = dir.path().join("session.json");
    let mut app = AppService::new(session.clone());
    app.dispatch(Command::OpenWorkspace(workspace.clone()))
        .unwrap();
    idle(&mut app);
    for path in [&php, &ts, &sql] {
        app.dispatch(Command::OpenFile(path.clone())).unwrap();
    }
    idle(&mut app);
    assert!(app.language.statuses.is_empty());
    assert!(!app.db.connected, "opening workspace must not auto-connect");
    app.dispatch(Command::Database(DbCommand::Connect(db.clone())))
        .unwrap();
    idle(&mut app);
    let table = app.db.schema.iter().find(|t| t.name == "people").unwrap();
    assert_eq!(table.columns.len(), 3);
    assert!(table.columns[0].primary_key);
    let sql_doc = app.documents.iter().find(|d| d.path == sql).unwrap();
    let sql_id = sql_doc.id;
    app.dispatch(Command::ExecuteSql {
        id: sql_id,
        expected_version: sql_doc.version,
        selection: None,
    })
    .unwrap();
    idle(&mut app);
    let result = app.db.result.as_ref().unwrap();
    assert_eq!(result.columns, vec!["id", "name", "note"]);
    assert_eq!(
        result.rows,
        vec![
            vec![Value::Integer(1), Value::Text("Olá".into()), Value::Null],
            vec![
                Value::Integer(2),
                Value::Text("Codisseu".into()),
                Value::Text("".into())
            ]
        ]
    );
    app.dispatch(Command::ConfigureProviders(configs.clone()))
        .unwrap();
    app.dispatch(Command::StartProviders).unwrap();
    wait(&mut app, |s| {
        s.language.statuses.len() == 2
            && s.language
                .statuses
                .iter()
                .all(|p| p.state == ProviderState::Ready)
            && s.language
                .diagnostics
                .iter()
                .any(|d| d.path == php && d.severity == 1)
            && s.language
                .diagnostics
                .iter()
                .any(|d| d.path == ts && d.severity == 1)
    });
    let php_id = app.documents.iter().find(|d| d.path == php).unwrap().id;
    let ts_id = app.documents.iter().find(|d| d.path == ts).unwrap().id;
    let diagnostic = app
        .language
        .diagnostics
        .iter()
        .find(|d| d.path == ts && d.severity == 1)
        .unwrap();
    let text = app.documents.iter().find(|d| d.id == ts_id).unwrap().text();
    let offset = codisseu_language::utf16_byte_offset(
        text,
        diagnostic.start_line,
        diagnostic.start_character,
    )
    .unwrap();
    assert_eq!(&text[offset..offset + 5], "count");
    let php_fixed = "<?php echo '🌧';\n";
    let ts_fixed = "export const emoji = '🌧'; export const count: number = 42;\n";
    app.dispatch(Command::Edit {
        id: php_id,
        expected_version: 1,
        text: php_fixed.into(),
    })
    .unwrap();
    app.dispatch(Command::Edit {
        id: ts_id,
        expected_version: 1,
        text: ts_fixed.into(),
    })
    .unwrap();
    // A dirty local copy closes without sending didClose for the canonical
    // document. Both real providers must still analyze the surviving buffers.
    for id in [php_id, ts_id] {
        app.dispatch(Command::Activate { id }).unwrap();
        let original = app.active_view().unwrap();
        app.dispatch(Command::CreateView).unwrap();
        let copy = app.active_view().unwrap();
        app.dispatch(Command::CloseTab {
            view: copy,
            id,
            discard: false,
        })
        .unwrap();
        assert_eq!(app.active_view(), Some(original));
        let d = app.active_document().unwrap();
        assert_eq!(d.id, id);
        assert_eq!(d.version, 2);
        assert!(d.is_dirty() && d.can_undo());
        assert_eq!(app.documents.len(), 3);
    }
    wait(&mut app, |s| {
        s.language.analyzed_versions.get(&php) == Some(&2)
            && s.language.analyzed_versions.get(&ts) == Some(&2)
            && s.language.diagnostics.iter().all(|d| d.severity > 2)
    });
    assert!(
        app.language.unversioned_paths.contains(&ts),
        "TS publication is inferred, not server-version-confirmed"
    );
    assert!(!app.language.unversioned_paths.contains(&php));
    for id in [php_id, ts_id] {
        app.dispatch(Command::Save { id }).unwrap();
    }
    idle(&mut app);
    assert_eq!(fs::read_to_string(&php).unwrap(), php_fixed);
    assert_eq!(fs::read_to_string(&ts).unwrap(), ts_fixed);
    app.dispatch(Command::StopProviders).unwrap();
    idle(&mut app);
    app.dispatch(Command::Activate { id: sql_id }).unwrap();
    app.dispatch(Command::PersistSession).unwrap();
    idle(&mut app);
    drop(app);
    let mut restored = AppService::new(session);
    restored.dispatch(Command::RestoreSession).unwrap();
    idle(&mut restored);
    assert_eq!(restored.workspace, Some(workspace));
    assert_eq!(restored.documents.len(), 3);
    assert_eq!(restored.active_document().unwrap().path, sql);
    assert_eq!(restored.database_path, Some(db));
    assert_eq!(restored.provider_configs, configs);
    assert!(
        !restored.db.connected,
        "restore stores config but must not reconnect"
    );
    assert!(
        restored.language.statuses.is_empty(),
        "restore must not execute providers"
    );
}
