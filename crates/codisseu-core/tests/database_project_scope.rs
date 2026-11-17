//! Real persisted metadata and query files; no network or credential access.
use codisseu_core::{
    AppService, Command,
    remote::{ConnectionConfig, RemoteCommand},
};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

fn settle(app: &mut AppService) {
    let until = Instant::now() + Duration::from_secs(15);
    while app.is_busy() {
        app.poll();
        assert!(
            Instant::now() < until,
            "{:?} / {:?}",
            app.last_error,
            app.remote_db.error
        );
        thread::sleep(Duration::from_millis(1));
    }
    app.poll();
}
fn run(app: &mut AppService, command: Command) {
    app.dispatch(command).unwrap();
    settle(app);
    assert!(app.last_error.is_none(), "{:?}", app.last_error);
    assert!(app.remote_db.error.is_none(), "{:?}", app.remote_db.error);
}
fn project(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    fs::create_dir_all(&path).unwrap();
    fs::canonicalize(path).unwrap()
}
fn config(id: &str) -> ConnectionConfig {
    ConnectionConfig {
        id: id.into(),
        name: format!("Fixture {id}"),
        save_credentials: false,
        ..Default::default()
    }
}
fn save(app: &mut AppService, id: &str) {
    run(
        app,
        Command::RemoteDatabase(RemoteCommand::Save(config(id))),
    );
}
fn query(app: &mut AppService, id: &str) -> PathBuf {
    run(
        app,
        Command::RemoteDatabase(RemoteCommand::CreateConsoleFor(id.into())),
    );
    app.active_document().unwrap().path.clone()
}

#[test]
fn databases_and_queries_follow_project_a_b_a_restart_and_shared_window_anchor() {
    let owned = tempfile::tempdir().unwrap();
    let a = project(owned.path(), "one/project");
    let b = project(owned.path(), "two/project");
    let anchor = owned.path().join("profile/session.json");
    let mut app = AppService::new_with_profile(anchor.clone(), anchor.clone());
    run(&mut app, Command::OpenWorkspace(a.clone()));
    save(&mut app, "a");
    let qa = query(&mut app, "a");
    run(&mut app, Command::OpenWorkspace(b.clone()));
    assert!(app.remote_db.profiles.is_empty());
    assert!(app.remote_db.console_bindings.is_empty());
    assert!(app.documents.is_empty());
    save(&mut app, "b");
    let qb = query(&mut app, "b");
    assert_ne!(qa.parent(), qb.parent());
    run(&mut app, Command::OpenWorkspace(a.clone()));
    assert_eq!(app.remote_db.profiles, vec![config("a")]);
    assert_eq!(app.remote_db.console_target(&qa).unwrap(), Some("a"));
    assert!(!app.remote_db.console_bindings.contains_key(&qb));
    run(&mut app, Command::PersistSession);
    drop(app);
    let mut window = AppService::new_with_profile(
        owned.path().join("profile/windows/new.json"),
        anchor.clone(),
    );
    run(&mut window, Command::OpenWorkspace(a.clone()));
    assert_eq!(window.remote_db.profiles, vec![config("a")]);
    assert!(window.remote_db.active.is_none());
    let mut restored = AppService::new_with_profile(anchor.clone(), anchor);
    run(&mut restored, Command::RestoreSession);
    assert_eq!(restored.workspace_path(), Some(a.as_path()));
    assert_eq!(restored.remote_db.profiles, vec![config("a")]);
    run(&mut restored, Command::CloseWorkspace);
    assert!(restored.remote_db.profiles.is_empty());
    assert!(restored.remote_db.console_bindings.is_empty());
    assert_eq!(fs::read_to_string(qa).unwrap(), "SELECT 1;\n");
    assert_eq!(fs::read_to_string(qb).unwrap(), "SELECT 1;\n");
}

#[test]
fn workspace_files_in_same_directory_keep_database_namespaces_separate() {
    let owned = tempfile::tempdir().unwrap();
    project(owned.path(), "a");
    project(owned.path(), "b");
    let a = owned.path().join("a.code-workspace");
    let b = owned.path().join("b.code-workspace");
    fs::write(&a, r#"{"folders":[{"path":"a"}]}"#).unwrap();
    fs::write(&b, r#"{"folders":[{"path":"b"}]}"#).unwrap();
    let mut app = AppService::new(owned.path().join("profile/session.json"));
    run(&mut app, Command::OpenWorkspace(a.clone()));
    save(&mut app, "workspace-a");
    run(&mut app, Command::OpenWorkspace(b));
    assert!(app.remote_db.profiles.is_empty());
    run(&mut app, Command::OpenWorkspace(a));
    assert_eq!(app.remote_db.profiles, vec![config("workspace-a")]);
}

#[test]
fn contaminated_legacy_session_never_assigns_databases_or_queries_until_explicit_claim() {
    let owned = tempfile::tempdir().unwrap();
    let a = project(owned.path(), "a");
    let b = project(owned.path(), "b");
    let anchor = owned.path().join("profile/session.json");
    fs::create_dir_all(anchor.parent().unwrap()).unwrap();
    let legacy = anchor.with_extension("datasources.json");
    let original = serde_json::to_vec(&vec![config("legacy-a")]).unwrap();
    fs::write(&legacy, &original).unwrap();
    let console = anchor.parent().unwrap().join("consoles/query-1.sql");
    fs::create_dir_all(console.parent().unwrap()).unwrap();
    fs::write(&console, "SELECT 'old A only';\n").unwrap();
    fs::write(
        legacy.with_extension("console-bindings.json"),
        serde_json::to_vec(&std::collections::BTreeMap::from([(
            console.clone(),
            "legacy-a",
        )]))
        .unwrap(),
    )
    .unwrap();
    let session = serde_json::to_vec(
        &serde_json::json!({"schema_version":2,"workspace":b,"tabs":[console],"active":console}),
    )
    .unwrap();
    fs::write(&anchor, &session).unwrap();
    let mut app = AppService::new(anchor.clone());
    run(&mut app, Command::RestoreSession);
    assert!(app.remote_db.legacy_available);
    assert!(app.remote_db.profiles.is_empty());
    assert!(app.documents.is_empty());
    assert!(app.status.contains("consultas antigas"));
    assert_eq!(fs::read(&anchor).unwrap(), session);
    assert_eq!(
        fs::read(anchor.with_extension("before-project-isolation.json")).unwrap(),
        session
    );
    run(&mut app, Command::OpenWorkspace(a.clone()));
    assert!(app.remote_db.profiles.is_empty());
    run(&mut app, Command::ClaimLegacyDatabase);
    assert!(!app.remote_db.legacy_available);
    assert_eq!(app.remote_db.profiles, vec![config("legacy-a")]);
    assert_eq!(
        app.remote_db.console_target(&console).unwrap(),
        Some("legacy-a")
    );
    assert!(app.remote_db.active.is_none());
    assert_eq!(fs::read(&legacy).unwrap(), original);
    run(&mut app, Command::OpenWorkspace(b));
    assert!(app.remote_db.profiles.is_empty());
    assert!(app.remote_db.console_bindings.is_empty());
    assert!(!app.remote_db.legacy_available);
    run(&mut app, Command::OpenWorkspace(a));
    assert_eq!(app.remote_db.profiles, vec![config("legacy-a")]);
    assert_eq!(
        fs::read_to_string(console).unwrap(),
        "SELECT 'old A only';\n"
    );
}

#[test]
fn invalid_legacy_association_is_preserved_and_does_not_expose_connections() {
    let owned = tempfile::tempdir().unwrap();
    let a = project(owned.path(), "a");
    let anchor = owned.path().join("profile/session.json");
    fs::create_dir_all(anchor.parent().unwrap()).unwrap();
    let marker = anchor.with_extension("database-scope.json");
    fs::write(&marker, "{invalid}").unwrap();
    let legacy = anchor.with_extension("datasources.json");
    fs::write(
        &legacy,
        serde_json::to_vec(&vec![config("keep-private")]).unwrap(),
    )
    .unwrap();
    let bytes = fs::read(&legacy).unwrap();
    let mut app = AppService::new(anchor);
    settle(&mut app);
    app.dispatch(Command::OpenWorkspace(a)).unwrap();
    settle(&mut app);
    assert!(app.remote_db.profiles.is_empty());
    assert!(app.remote_db.error.is_some());
    assert_eq!(fs::read_to_string(marker).unwrap(), "{invalid}");
    assert_eq!(fs::read(legacy).unwrap(), bytes);
}

#[test]
fn remote_transaction_and_accepted_write_prevent_project_switch() {
    let owned = tempfile::tempdir().unwrap();
    let a = project(owned.path(), "a");
    let b = project(owned.path(), "b");
    let mut app = AppService::new(owned.path().join("profile/session.json"));
    run(&mut app, Command::OpenWorkspace(a.clone()));
    app.remote_db.transaction = true; // Guard state only; no synthetic DB execution.
    assert!(app.dispatch(Command::OpenWorkspace(b.clone())).is_err());
    assert!(app.dispatch(Command::CloseWorkspace).is_err());
    assert_eq!(app.workspace_path(), Some(a.as_path()));
    app.remote_db.transaction = false;
    app.remote_db
        .dispatch(RemoteCommand::Save(config("accepted")))
        .unwrap();
    assert!(app.dispatch(Command::OpenWorkspace(b)).is_err());
    settle(&mut app);
    assert_eq!(app.remote_db.profiles, vec![config("accepted")]);
    assert_eq!(app.workspace_path(), Some(a.as_path()));
}

#[test]
fn two_windows_merge_distinct_database_and_query_entries_and_reject_same_entry_conflict() {
    let owned = tempfile::tempdir().unwrap();
    let a = project(owned.path(), "a");
    let anchor = owned.path().join("profile/session.json");
    let mut first = AppService::new_with_profile(anchor.clone(), anchor.clone());
    let mut second =
        AppService::new_with_profile(owned.path().join("other-window.json"), anchor.clone());
    run(&mut first, Command::OpenWorkspace(a.clone()));
    run(&mut second, Command::OpenWorkspace(a.clone()));
    save(&mut first, "one");
    save(&mut second, "two");
    assert_eq!(second.remote_db.profiles.len(), 2);
    let q1 = query(&mut first, "one");
    let q2 = query(&mut second, "two");
    assert_eq!(second.remote_db.console_bindings.len(), 2);
    assert_ne!(q1, q2);
    let mut stale =
        AppService::new_with_profile(owned.path().join("stale-window.json"), anchor.clone());
    run(&mut stale, Command::OpenWorkspace(a.clone()));
    let mut newer = config("one");
    newer.name = "newer name".into();
    run(
        &mut first,
        Command::RemoteDatabase(RemoteCommand::Save(newer.clone())),
    );
    stale
        .dispatch(Command::RemoteDatabase(RemoteCommand::Save(config("one"))))
        .unwrap();
    settle(&mut stale);
    assert!(
        stale
            .remote_db
            .error
            .as_ref()
            .is_some_and(|s| s.contains("outra janela"))
    );
    let mut verifier =
        AppService::new_with_profile(owned.path().join("verify-window.json"), anchor);
    run(&mut verifier, Command::OpenWorkspace(a));
    assert_eq!(
        verifier.remote_db.profiles.iter().find(|c| c.id == "one"),
        Some(&newer)
    );
    assert_eq!(verifier.remote_db.profiles.len(), 2);
    assert_eq!(verifier.remote_db.console_bindings.len(), 2);
    assert_eq!(fs::read_to_string(q1).unwrap(), "SELECT 1;\n");
    assert_eq!(fs::read_to_string(q2).unwrap(), "SELECT 1;\n");
}

#[test]
fn claiming_legacy_rejects_a_stale_window_write_and_preserves_all_original_files() {
    let owned = tempfile::tempdir().unwrap();
    let a = project(owned.path(), "a");
    let anchor = owned.path().join("profile/session.json");
    fs::create_dir_all(anchor.parent().unwrap()).unwrap();
    let legacy = anchor.with_extension("datasources.json");
    let original = serde_json::to_vec(&vec![config("old")]).unwrap();
    fs::write(&legacy, &original).unwrap();
    let mut first = AppService::new_with_profile(anchor.clone(), anchor.clone());
    let mut second = AppService::new_with_profile(owned.path().join("second.json"), anchor.clone());
    run(&mut first, Command::OpenWorkspace(a.clone()));
    run(&mut second, Command::OpenWorkspace(a.clone()));
    run(&mut first, Command::ClaimLegacyDatabase);
    second
        .dispatch(Command::RemoteDatabase(RemoteCommand::Save(config(
            "must-not-hide",
        ))))
        .unwrap();
    settle(&mut second);
    assert!(
        second
            .remote_db
            .error
            .as_ref()
            .is_some_and(|s| s.contains("vínculo dos bancos mudou"))
    );
    assert_eq!(fs::read(&legacy).unwrap(), original);
    let mut restored = AppService::new_with_profile(owned.path().join("third.json"), anchor);
    run(&mut restored, Command::OpenWorkspace(a));
    assert_eq!(restored.remote_db.profiles, vec![config("old")]);
}

#[test]
fn claiming_legacy_never_hides_existing_project_data() {
    let owned = tempfile::tempdir().unwrap();
    let a = project(owned.path(), "a");
    let anchor = owned.path().join("profile/session.json");
    fs::create_dir_all(anchor.parent().unwrap()).unwrap();
    let legacy = anchor.with_extension("datasources.json");
    fs::write(&legacy, serde_json::to_vec(&vec![config("old")]).unwrap()).unwrap();
    let mut app = AppService::new(anchor);
    run(&mut app, Command::OpenWorkspace(a));
    save(&mut app, "new");
    assert!(app.dispatch(Command::ClaimLegacyDatabase).is_err());
    assert_eq!(app.remote_db.profiles, vec![config("new")]);
    assert!(app.remote_db.legacy_available);
    assert_eq!(
        serde_json::from_slice::<Vec<ConnectionConfig>>(&fs::read(legacy).unwrap()).unwrap(),
        vec![config("old")]
    );
}

#[test]
fn empty_legacy_sidecars_do_not_advertise_recovery_but_real_data_does() {
    let owned = tempfile::tempdir().unwrap();
    let anchor = owned.path().join("profile/session.json");
    fs::create_dir_all(anchor.parent().unwrap()).unwrap();
    fs::write(anchor.with_extension("datasources.json"), b"[]").unwrap();
    fs::write(
        anchor.with_extension("datasources.console-bindings.json"),
        b"{}",
    )
    .unwrap();
    let mut empty = AppService::new(anchor.clone());
    settle(&mut empty);
    assert!(!empty.remote_db.legacy_available);
    drop(empty);
    fs::write(
        anchor.with_extension("datasources.json"),
        serde_json::to_vec(&vec![config("real-data")]).unwrap(),
    )
    .unwrap();
    let mut real = AppService::new(anchor);
    settle(&mut real);
    assert!(real.remote_db.legacy_available);
    assert!(real.remote_db.profiles.is_empty());
}

#[cfg(unix)]
#[test]
fn restore_via_profile_alias_omits_unassigned_legacy_query_and_requires_exact_binding() {
    let owned = tempfile::tempdir().unwrap();
    let a = project(owned.path(), "a");
    let b = project(owned.path(), "b");
    let directory = project(owned.path(), "profile");
    let alias = owned.path().join("profile-alias");
    std::os::unix::fs::symlink(&directory, &alias).unwrap();
    let anchor = alias.join("session.json");
    let own_query = directory.join("consoles/own.sql");
    let other_query = directory.join("consoles/other-profile.sql");
    fs::create_dir_all(own_query.parent().unwrap()).unwrap();
    fs::write(&own_query, "SELECT 'own';").unwrap();
    fs::write(&other_query, "SELECT 'other';").unwrap();
    fs::write(
        anchor.with_extension("datasources.console-bindings.json"),
        serde_json::to_vec(&std::collections::BTreeMap::from([(
            own_query.clone(),
            "old",
        )]))
        .unwrap(),
    )
    .unwrap();
    fs::write(&anchor, serde_json::to_vec(&serde_json::json!({"schema_version":1,"workspace":b,"tabs":[own_query,other_query],"active":own_query})).unwrap()).unwrap();
    let mut app = AppService::new(anchor.clone());
    run(&mut app, Command::RestoreSession);
    assert!(app.documents.is_empty());
    run(&mut app, Command::OpenWorkspace(a.clone()));
    run(&mut app, Command::ClaimLegacyDatabase);
    run(&mut app, Command::PersistSession);
    fs::write(&anchor, serde_json::to_vec(&serde_json::json!({"schema_version":1,"workspace":a,"tabs":[own_query,other_query],"active":own_query})).unwrap()).unwrap();
    drop(app);
    let mut restored = AppService::new(anchor);
    run(&mut restored, Command::RestoreSession);
    assert_eq!(restored.documents.len(), 1);
    assert_eq!(restored.documents[0].path, own_query);
    assert_eq!(restored.documents[0].text(), "SELECT 'own';");
    assert_eq!(fs::read_to_string(other_query).unwrap(), "SELECT 'other';");
}
