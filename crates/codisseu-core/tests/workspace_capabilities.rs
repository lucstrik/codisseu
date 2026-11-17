//! Workspace transitions must not move capabilities between unrelated roots.
use codisseu_core::{AppService, Command, DbCommand, Language, ProviderConfig, ProviderState};
use rusqlite::{Connection, types::Value};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

struct Fixture {
    _directory: tempfile::TempDir,
    first: PathBuf,
    second: PathBuf,
    database: PathBuf,
    document: PathBuf,
    session: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first");
        let second = directory.path().join("second");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        let document = first.join("index.php");
        fs::write(&document, "<?php echo 'first';\n").unwrap();
        fs::write(second.join("app.ts"), "export const second = true;\n").unwrap();
        let database = first.join("fixture.sqlite");
        Connection::open(&database)
            .unwrap()
            .execute_batch(
                "CREATE TABLE owner(name TEXT NOT NULL); INSERT INTO owner VALUES ('first');",
            )
            .unwrap();
        let session = directory.path().join("session.json");
        Self {
            _directory: directory,
            first: first.canonicalize().unwrap(),
            second: second.canonicalize().unwrap(),
            database: database.canonicalize().unwrap(),
            document: document.canonicalize().unwrap(),
            session,
        }
    }

    fn app(&self) -> AppService {
        let mut app = AppService::new(self.session.clone());
        app.dispatch(Command::OpenWorkspace(self.first.clone()))
            .unwrap();
        idle(&mut app);
        app.dispatch(Command::OpenFile(self.document.clone()))
            .unwrap();
        idle(&mut app);
        app
    }
}

fn idle(app: &mut AppService) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.poll();
        if !app.is_busy() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "operations did not finish: {} / {} / {:?}",
            app.status,
            app.db.status,
            app.language.statuses
        );
        thread::sleep(Duration::from_millis(1));
    }
}

fn connect(app: &mut AppService, path: &Path) {
    app.dispatch(Command::Database(DbCommand::Connect(path.into())))
        .unwrap();
    idle(app);
    assert!(app.db.connected, "{:?}", app.db.last_error);
    assert!(app.db.last_error.is_none());
    assert_eq!(app.db.config.as_deref(), Some(path));
}

fn select_owner(app: &mut AppService) {
    app.dispatch(Command::Database(DbCommand::Execute {
        sql: "SELECT name FROM owner".into(),
    }))
    .unwrap();
    idle(app);
    assert!(app.db.last_error.is_none(), "{:?}", app.db.last_error);
    assert_eq!(
        app.db.result.as_ref().unwrap().rows,
        vec![vec![Value::Text("first".into())]]
    );
}

#[test]
fn pending_connection_rejects_workspace_change_without_invalidating_connection() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    let document_id = app.active_document().unwrap().id;
    app.dispatch(Command::Database(DbCommand::Connect(
        fixture.database.clone(),
    )))
    .unwrap();
    // No poll: the accepted connection is pending even if its worker has replied.
    assert!(app.db.is_busy());
    let error = app
        .dispatch(Command::OpenWorkspace(fixture.second.clone()))
        .unwrap_err();
    assert_eq!(error, "Aguardando a operação de banco de dados terminar");
    assert_eq!(app.workspace.as_ref(), Some(&fixture.first));
    assert_eq!(app.active, Some(document_id));
    assert_eq!(app.active_document().unwrap().path, fixture.document);
    assert!(app.db.is_busy());
    assert!(app.language.statuses.is_empty());

    idle(&mut app);
    assert!(app.db.connected, "{:?}", app.db.last_error);
    assert_eq!(app.db.config.as_ref(), Some(&fixture.database));
    assert_eq!(app.workspace.as_ref(), Some(&fixture.first));
    assert_eq!(app.active, Some(document_id));
    select_owner(&mut app);
}

#[test]
fn pending_workspace_rejects_database_and_provider_start_without_changing_engines() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    connect(&mut app, &fixture.database);
    select_owner(&mut app);
    let configs = vec![ProviderConfig {
        language: Language::Php,
        program: fixture.first.join("provider-must-not-start"),
        args: vec![],
    }];
    app.dispatch(Command::ConfigureProviders(configs.clone()))
        .unwrap();
    app.dispatch(Command::OpenWorkspace(fixture.second.clone()))
        .unwrap();
    // Do not pump completions until all conflicting commands were rejected.
    for command in [
        DbCommand::Connect(fixture.database.clone()),
        DbCommand::Disconnect,
        DbCommand::RefreshSchema,
        DbCommand::Execute {
            sql: "SELECT 99".into(),
        },
    ] {
        let error = app.dispatch(Command::Database(command)).unwrap_err();
        assert!(error.contains("abertura da pasta"), "{error}");
        assert!(app.db.connected);
        assert!(!app.db.is_busy());
        assert_eq!(app.db.config.as_ref(), Some(&fixture.database));
        assert_eq!(
            app.db.result.as_ref().unwrap().rows,
            vec![vec![Value::Text("first".into())]]
        );
        assert!(app.db.last_error.is_none());
    }
    let error = app.dispatch(Command::StartProviders).unwrap_err();
    assert!(error.contains("abertura da pasta"), "{error}");
    assert_eq!(app.provider_configs, configs);
    assert!(app.language.statuses.is_empty());
    assert!(!app.language.is_busy());
    assert!(app.language.last_error.is_none());
    assert_eq!(app.workspace.as_ref(), Some(&fixture.first));
    assert_eq!(app.active_document().unwrap().path, fixture.document);

    // Cancellation is intentionally still available during a workspace read.
    app.dispatch(Command::Database(DbCommand::Cancel)).unwrap();
    idle(&mut app);
    assert_eq!(app.workspace.as_ref(), Some(&fixture.second));
    assert!(!app.db.connected);
    assert!(app.language.statuses.is_empty());
}

#[test]
fn completed_workspace_change_disconnects_database_and_clears_its_configuration() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    connect(&mut app, &fixture.database);
    select_owner(&mut app);
    assert_eq!(app.database_path.as_ref(), Some(&fixture.database));

    app.dispatch(Command::OpenWorkspace(fixture.second.clone()))
        .unwrap();
    idle(&mut app);

    assert_eq!(app.workspace.as_ref(), Some(&fixture.second));
    assert!(app.documents.is_empty());
    assert!(app.active.is_none());
    assert!(!app.db.connected);
    assert!(!app.db.is_busy());
    assert!(app.db.config.is_none());
    assert!(app.db.schema.is_empty());
    assert!(app.db.result.is_none());
    assert!(
        app.database_path.is_none(),
        "the old database path must not be restored while disconnect is pending"
    );
    app.dispatch(Command::PersistSession).unwrap();
    idle(&mut app);
    let session: serde_json::Value =
        serde_json::from_slice(&fs::read(&fixture.session).unwrap()).unwrap();
    assert!(session["database_path"].is_null());
}

#[test]
fn pending_provider_start_rejects_workspace_change_without_consuming_start() {
    let fixture = Fixture::new();
    let mut app = fixture.app();
    let configs = vec![ProviderConfig {
        language: Language::Php,
        program: fixture.first.join("missing-provider"),
        args: vec![],
    }];
    app.dispatch(Command::ConfigureProviders(configs.clone()))
        .unwrap();
    app.dispatch(Command::StartProviders).unwrap();
    // Starting state is set synchronously, so rejection does not depend on timing.
    assert!(app.language.is_busy());
    assert_eq!(app.language.statuses[0].state, ProviderState::Starting);
    let error = app
        .dispatch(Command::OpenWorkspace(fixture.second.clone()))
        .unwrap_err();
    assert_eq!(error, "Aguardando a análise do editor terminar");
    assert_eq!(app.workspace.as_ref(), Some(&fixture.first));
    assert_eq!(app.provider_configs, configs);
    assert_eq!(app.language.statuses[0].state, ProviderState::Starting);
    assert_eq!(app.active_document().unwrap().path, fixture.document);

    idle(&mut app);
    assert_eq!(app.workspace.as_ref(), Some(&fixture.first));
    assert_eq!(app.language.statuses[0].state, ProviderState::Failed);
    assert!(app.language.last_error.is_some());
    assert!(!app.db.connected);
}
