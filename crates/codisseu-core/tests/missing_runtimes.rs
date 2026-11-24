//! Release safety: unavailable or stalled analysis must not disable files/SQLite.
use codisseu_core::{AppService, Command, DbCommand, Language, ProviderConfig, ProviderState};
use rusqlite::{Connection, types::Value};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

fn wait(app: &mut AppService, seconds: u64, ready: impl Fn(&AppService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        app.poll();
        assert!(app.last_error.is_none(), "{:?}", app.last_error);
        assert!(app.db.last_error.is_none(), "{:?}", app.db.last_error);
        if ready(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{} {:?}",
            app.status,
            app.language.statuses
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn open(root: &Path) -> AppService {
    let workspace = root.join("project");
    fs::create_dir(&workspace).unwrap();
    let mut app = AppService::new(root.join("profile/session.json"));
    app.dispatch(Command::OpenWorkspace(workspace)).unwrap();
    wait(&mut app, 5, |app| !app.is_busy());
    app
}

fn query(app: &mut AppService, root: &Path) {
    let database = root.join("project/fixture.sqlite");
    Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE TABLE sample(value TEXT); INSERT INTO sample VALUES('Olá 🌧');")
        .unwrap();
    app.dispatch(Command::Database(DbCommand::Connect(database)))
        .unwrap();
    wait(app, 5, |app| app.db.connected);
    let sql = root.join("project/query.sql");
    fs::write(&sql, "SELECT value FROM sample;").unwrap();
    app.dispatch(Command::OpenFile(sql.clone())).unwrap();
    wait(app, 5, |app| app.documents.iter().any(|d| d.path == sql));
    let document = app.documents.iter().find(|d| d.path == sql).unwrap();
    app.dispatch(Command::ExecuteSql {
        id: document.id,
        expected_version: document.version,
        selection: None,
    })
    .unwrap();
    wait(app, 5, |app| app.db.result.is_some());
    assert_eq!(
        app.db.result.as_ref().unwrap().rows,
        vec![vec![Value::Text("Olá 🌧".into())]]
    );
}

fn edit_and_save(app: &mut AppService, path: PathBuf, original: &str, updated: &str) {
    fs::write(&path, original).unwrap();
    app.dispatch(Command::OpenFile(path.clone())).unwrap();
    wait(app, 5, |app| app.documents.iter().any(|d| d.path == path));
    let document = app.documents.iter().find(|d| d.path == path).unwrap();
    let id = document.id;
    app.dispatch(Command::Edit {
        id,
        expected_version: document.version,
        text: updated.into(),
    })
    .unwrap();
    app.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(
        app.documents.iter().find(|d| d.id == id).unwrap().text(),
        original
    );
    app.dispatch(Command::Redo { id }).unwrap();
    app.dispatch(Command::Save { id }).unwrap();
    wait(app, 5, |app| {
        app.documents.iter().any(|d| d.id == id && !d.is_dirty())
    });
    assert_eq!(fs::read_to_string(path).unwrap(), updated);
}

#[test]
fn missing_php_node_python_and_rust_preserve_editing_saving_and_sqlite() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = open(temp.path());
    let configs: Vec<ProviderConfig> = [
        Language::Php,
        Language::TypeScript,
        Language::Python,
        Language::Rust,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, language)| ProviderConfig {
        language,
        program: temp.path().join(format!("not-installed-{index}")),
        args: vec![],
    })
    .collect();
    let configuration = temp.path().join("providers.json");
    fs::write(&configuration, serde_json::to_vec(&configs).unwrap()).unwrap();
    app.dispatch(Command::LoadProviderConfig(configuration))
        .unwrap();
    wait(&mut app, 5, |app| !app.is_busy());
    assert_eq!(app.provider_configs, configs);
    app.dispatch(Command::StartProviders).unwrap();
    wait(&mut app, 5, |app| !app.is_busy());
    assert_eq!(app.language.statuses.len(), 4);
    assert!(
        app.language
            .statuses
            .iter()
            .all(|p| p.state == ProviderState::Failed)
    );
    assert!(app.language.statuses.iter().all(|p| !p.detail.is_empty()));
    let failed_generation = app.language.lifecycle_generation();
    for (file, original, updated) in [
        ("index.php", "<?php // original\n", "<?php echo 'Olá 🌧';\n"),
        (
            "app.ts",
            "export {};\n",
            "export const value: number = 42;\n",
        ),
        ("main.py", "# original\n", "print('Olá 🌧')\n"),
        (
            "main.rs",
            "// original\n",
            "fn main() { println!(\"Olá 🌧\"); }\n",
        ),
    ] {
        edit_and_save(
            &mut app,
            temp.path().join("project").join(file),
            original,
            updated,
        );
    }
    query(&mut app, temp.path());
    for _ in 0..100 {
        app.poll();
        // File/history/Git acknowledgements may still be busy after the SQLite
        // reply. Inspect actual accepted Start/Stop transitions, including a
        // retry that failed completely between two polls.
        assert_eq!(
            app.language.lifecycle_generation(),
            failed_generation,
            "failed tools must not restart in a loop"
        );
        assert_eq!(app.language.statuses.len(), 4);
        assert!(
            app.language
                .statuses
                .iter()
                .all(|p| p.state == ProviderState::Failed)
        );
    }
    app.dispatch(Command::StopProviders).unwrap();
    wait(&mut app, 5, |app| !app.is_busy());
}

#[cfg(unix)]
#[test]
fn stalled_provider_does_not_block_saving_or_sqlite_and_reaches_real_deadline() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = open(temp.path());
    let started = temp.path().join("provider-started");
    app.dispatch(Command::ConfigureProviders(vec![ProviderConfig {
        language: Language::Php,
        program: "/bin/sh".into(),
        args: vec![
            "-c".into(),
            "printf started > \"$1\"; exec /bin/sleep 60".into(),
            "codisseu-controlled-fixture".into(),
            started.to_string_lossy().into_owned(),
        ],
    }]))
    .unwrap();
    app.dispatch(Command::StartProviders).unwrap();
    wait(&mut app, 5, |_| started.is_file());
    edit_and_save(
        &mut app,
        temp.path().join("project/index.php"),
        "<?php // original\n",
        "<?php echo 'Saved while analysis waits';\n",
    );
    query(&mut app, temp.path());
    assert_eq!(app.language.statuses[0].state, ProviderState::Starting);
    wait(&mut app, 20, |app| {
        app.language.statuses[0].state == ProviderState::Failed && !app.is_busy()
    });
    assert!(
        app.language.statuses[0]
            .detail
            .contains("Timeout de initialize após 15 segundos")
    );
    app.dispatch(Command::StopProviders).unwrap();
    wait(&mut app, 5, |app| !app.is_busy());
    assert_eq!(app.language.statuses[0].state, ProviderState::Stopped);
}
