//! Provider policy is exercised through AppService; only the ignored test needs
//! PHP/TS. Missing executables below prove failures never become restart loops.
use codisseu_core::{AppService, Command, Language, ProviderConfig, ProviderState};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

fn idle(app: &mut AppService) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        app.poll();
        if !app.is_busy() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "{} {:?}",
            app.status,
            app.language.statuses
        );
        thread::sleep(Duration::from_millis(2));
    }
    assert!(app.last_error.is_none(), "{:?}", app.last_error);
}
fn missing(root: &Path) -> Vec<ProviderConfig> {
    vec![ProviderConfig {
        language: Language::TypeScript,
        program: root.join("never-installed-provider"),
        args: vec!["--stdio".into()],
    }]
}
fn start_policy(app: &mut AppService, profile: &Path, fallback: Option<&Path>) {
    app.dispatch(Command::EnableProviderAutostart {
        profile: profile.into(),
        fallback_config: fallback.map(Path::to_path_buf),
    })
    .unwrap();
    idle(app);
}
fn open(root: &Path, workspace: &Path, configs: Vec<ProviderConfig>) -> AppService {
    let mut app = AppService::new(root.join("session.json"));
    app.dispatch(Command::OpenWorkspace(workspace.into()))
        .unwrap();
    idle(&mut app);
    app.dispatch(Command::ConfigureProviders(configs)).unwrap();
    app
}
fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("project");
    fs::create_dir(&workspace).unwrap();
    let profile = root.path().join("profile/providers.json");
    (root, workspace, profile)
}

#[test]
fn default_service_never_starts_configured_providers() {
    let (root, workspace, _) = setup();
    let mut app = open(root.path(), &workspace, missing(root.path()));
    for _ in 0..10 {
        app.poll();
    }
    assert!(!app.provider_startup().configured);
    assert!(app.language.statuses.is_empty());
}

#[test]
fn trust_after_workspace_switch_does_not_start_or_authorize_the_previous_root() {
    let (root, first, profile) = setup();
    let second = root.path().join("new-project");
    fs::create_dir(&second).unwrap();
    let mut app = open(root.path(), &first, missing(root.path()));
    start_policy(&mut app, &profile, None);
    app.dispatch(Command::StartProviders).unwrap();
    idle(&mut app);
    assert_eq!(app.active_language_project(), Some(first.as_path()));
    app.dispatch(Command::OpenWorkspace(second.clone()))
        .unwrap();
    idle(&mut app);
    assert!(app.active_document().is_none());
    assert!(app.provider_startup().needs_trust);
    assert_eq!(
        app.language_project_root(),
        Some(second.clone()),
        "cached language root must belong to current workspace"
    );
    app.dispatch(Command::StartProviders).unwrap();
    idle(&mut app);
    assert!(app.provider_startup().trusted);
    assert!(!app.provider_startup().needs_trust);
    assert_eq!(app.active_language_project(), Some(second.as_path()));
    let policy: serde_json::Value = serde_json::from_slice(&fs::read(profile).unwrap()).unwrap();
    assert!(
        policy["approvals"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["workspace"] == second.to_str().unwrap())
    );
}

#[test]
fn stop_status_survives_pending_start_and_file_reply() {
    let (root, workspace, _) = setup();
    let path = workspace.join("late.ts");
    fs::write(&path, "export const value = 1;\n").unwrap();
    let mut app = open(root.path(), &workspace, missing(root.path()));
    app.dispatch(Command::StartProviders).unwrap();
    app.dispatch(Command::OpenFile(path)).unwrap();
    app.dispatch(Command::StopProviders).unwrap();
    assert_eq!(app.status, "Encerrando providers de linguagem");
    idle(&mut app);
    assert!(!app.providers_running());
    assert_eq!(
        app.status,
        "Providers parados; análise pausada até iniciar ou reabrir"
    );
    for _ in 0..100 {
        app.poll();
        assert!(!app.status.contains("Providers ativos"));
    }
}

#[test]
fn discovers_managed_config_but_new_project_requires_trust() {
    let (root, workspace, profile) = setup();
    let managed = root.path().join("managed.json");
    let configs = missing(root.path());
    fs::write(&managed, serde_json::to_vec(&configs).unwrap()).unwrap();
    fs::create_dir(workspace.join(".tools")).unwrap();
    fs::write(
        workspace.join(".tools/providers.json"),
        b"malicious project configuration",
    )
    .unwrap();
    let mut app = AppService::new(root.path().join("session.json"));
    app.dispatch(Command::OpenWorkspace(workspace)).unwrap();
    idle(&mut app);
    start_policy(&mut app, &profile, Some(&managed));
    assert_eq!(app.provider_configs, configs);
    assert!(app.provider_startup().enabled);
    assert!(app.provider_startup().needs_trust);
    assert!(!app.provider_startup().trusted);
    assert!(app.language.statuses.is_empty());
    assert!(
        !profile.exists(),
        "opening an unknown project must not grant/save trust"
    );
}

#[test]
fn manual_start_remembers_exact_workspace_and_configuration_and_failure_does_not_loop() {
    let (root, workspace, profile) = setup();
    let configs = missing(root.path());
    let mut app = open(root.path(), &workspace, configs.clone());
    start_policy(&mut app, &profile, None);
    app.dispatch(Command::StartProviders).unwrap();
    idle(&mut app);
    assert!(app.provider_startup().trusted);
    assert!(profile.exists());
    assert_eq!(app.language.statuses[0].state, ProviderState::Failed);
    let failed = app.language.last_error.clone();
    for _ in 0..100 {
        app.poll();
        assert!(!app.is_busy());
    }
    assert_eq!(app.language.last_error, failed);
    app.dispatch(Command::StopProviders).unwrap();
    idle(&mut app);
    assert!(app.provider_startup().suspended);
    for _ in 0..100 {
        app.poll();
        assert!(!app.providers_running());
    }
    drop(app);

    let mut restored = open(root.path(), &workspace, configs);
    start_policy(&mut restored, &profile, None);
    assert!(restored.provider_startup().trusted);
    assert_eq!(
        restored.language.statuses[0].state,
        ProviderState::Failed,
        "a remembered project automatically attempted the real launch"
    );
    for _ in 0..100 {
        restored.poll();
        assert!(!restored.is_busy());
    }
    restored.dispatch(Command::StopProviders).unwrap();
    idle(&mut restored);

    let other = root.path().join("other");
    fs::create_dir(&other).unwrap();
    restored.dispatch(Command::OpenWorkspace(other)).unwrap();
    idle(&mut restored);
    assert!(!restored.provider_startup().trusted);
    assert!(restored.provider_startup().needs_trust);
    assert!(!restored.providers_running());
}

#[test]
fn changed_arguments_revoke_automatic_permission_and_disabled_preference_persists() {
    let (root, workspace, profile) = setup();
    let configs = missing(root.path());
    let mut app = open(root.path(), &workspace, configs.clone());
    start_policy(&mut app, &profile, None);
    app.dispatch(Command::StartProviders).unwrap();
    idle(&mut app);
    app.dispatch(Command::StopProviders).unwrap();
    idle(&mut app);
    drop(app);
    let mut changed = configs.clone();
    changed[0].args.push("changed".into());
    let mut app = open(root.path(), &workspace, changed);
    start_policy(&mut app, &profile, None);
    assert!(app.provider_startup().needs_trust);
    assert!(!app.provider_startup().trusted);
    assert!(app.language.statuses.is_empty());
    app.dispatch(Command::SetProvidersAutomatic(false)).unwrap();
    idle(&mut app);
    drop(app);
    let mut app = open(root.path(), &workspace, configs);
    start_policy(&mut app, &profile, None);
    assert!(app.provider_startup().trusted);
    assert!(!app.provider_startup().enabled);
    assert!(!app.provider_startup().needs_trust);
    assert!(app.language.statuses.is_empty());
    app.dispatch(Command::SetProvidersAutomatic(true)).unwrap();
    idle(&mut app);
    assert_eq!(app.language.statuses[0].state, ProviderState::Failed);
}

#[test]
fn profile_inside_project_cannot_authorize_automatic_execution() {
    let (root, workspace, _) = setup();
    let configs = missing(root.path());
    let profile = workspace.join("forged-policy.json");
    fs::write(
        &profile,
        serde_json::to_vec(&serde_json::json!({
            "schema_version":1, "automatic":true, "configs":configs,
            "approvals":[{"workspace":workspace, "configs":configs}]
        }))
        .unwrap(),
    )
    .unwrap();
    let bytes = fs::read(&profile).unwrap();
    let mut app = open(root.path(), &workspace, configs);
    start_policy(&mut app, &profile, None);
    assert!(!app.provider_startup().trusted);
    assert!(
        app.provider_startup()
            .error
            .as_ref()
            .unwrap()
            .contains("fora do projeto")
    );
    assert!(app.language.statuses.is_empty());
    assert!(app.dispatch(Command::SetProvidersAutomatic(false)).is_err());
    assert_eq!(bytes, fs::read(profile).unwrap());
}

#[test]
fn invalid_profile_is_a_visible_failure_without_process_or_rewrite() {
    let (root, workspace, profile) = setup();
    fs::create_dir_all(profile.parent().unwrap()).unwrap();
    fs::write(&profile, b"{invalid").unwrap();
    let mut app = open(root.path(), &workspace, missing(root.path()));
    start_policy(&mut app, &profile, None);
    assert!(app.provider_startup().error.is_some());
    assert!(!app.provider_startup().needs_trust);
    assert!(app.language.statuses.is_empty());
    assert_eq!(b"{invalid", fs::read(profile).unwrap().as_slice());
}

#[test]
fn explicit_empty_configuration_does_not_rediscover_tools() {
    let (root, workspace, profile) = setup();
    let managed = root.path().join("managed.json");
    fs::write(&managed, serde_json::to_vec(&missing(root.path())).unwrap()).unwrap();
    let mut app = open(root.path(), &workspace, vec![]);
    start_policy(&mut app, &profile, Some(&managed));
    assert!(app.provider_configs.is_empty());
    assert!(!app.provider_startup().needs_trust);
    assert!(app.language.statuses.is_empty());
}

#[test]
fn nonexistent_parent_and_dotdot_cannot_hide_profile_inside_project() {
    let (root, workspace, _) = setup();
    let profile = root.path().join("not-created/../project/policy.json");
    let mut app = open(root.path(), &workspace, missing(root.path()));
    start_policy(&mut app, &profile, None);
    assert!(
        app.provider_startup()
            .error
            .as_ref()
            .unwrap()
            .contains("fora do projeto")
    );
    assert!(!app.provider_startup().trusted);
    assert!(app.language.statuses.is_empty());
    assert!(!profile.exists());
}

#[cfg(unix)]
#[test]
fn symlinked_profile_is_rejected_and_target_is_preserved() {
    let (root, workspace, profile) = setup();
    fs::create_dir_all(profile.parent().unwrap()).unwrap();
    let target = root.path().join("untouched.json");
    fs::write(&target, b"{}").unwrap();
    std::os::unix::fs::symlink(&target, &profile).unwrap();
    let mut app = open(root.path(), &workspace, missing(root.path()));
    start_policy(&mut app, &profile, None);
    assert!(app.provider_startup().error.is_some());
    assert!(app.language.statuses.is_empty());
    assert_eq!(fs::read(target).unwrap(), b"{}");
}

#[test]
#[ignore = "real PHP/TS automatic restart; requires CODISSEU_PROVIDER_CONFIG"]
fn real_php_ts_start_once_then_restore_automatically_with_dirty_buffers_preserved() {
    let config = std::env::var_os("CODISSEU_PROVIDER_CONFIG")
        .expect("CODISSEU_PROVIDER_CONFIG required; never skip silently");
    let configs: Vec<ProviderConfig> = serde_json::from_slice(&fs::read(config).unwrap()).unwrap();
    let (root, workspace, profile) = setup();
    fs::write(
        workspace.join("tsconfig.json"),
        r#"{"compilerOptions":{"strict":true},"include":["*.ts"]}"#,
    )
    .unwrap();
    let php = workspace.join("index.php");
    let ts = workspace.join("app.ts");
    fs::write(&php, "<?php $value = ;\n").unwrap();
    fs::write(&ts, "export const value: number = 'wrong';\n").unwrap();
    let mut app = open(root.path(), &workspace, configs);
    start_policy(&mut app, &profile, None);
    for file in [&php, &ts] {
        app.dispatch(Command::OpenFile(file.into())).unwrap();
    }
    idle(&mut app);
    app.dispatch(Command::StartProviders).unwrap();
    idle(&mut app);
    assert!(
        app.language
            .statuses
            .iter()
            .all(|s| s.state == ProviderState::Ready)
    );
    app.dispatch(Command::StartProviders).unwrap();
    assert!(!app.language.is_busy(), "repeat Start is idempotent");
    idle(&mut app);
    // A real asynchronous file acknowledgement must not restore the previous
    // active-provider report after the user's Stop command.
    app.dispatch(Command::OpenFile(php.clone())).unwrap();
    app.dispatch(Command::StopProviders).unwrap();
    assert_eq!(app.status, "Encerrando providers de linguagem");
    idle(&mut app);
    assert_eq!(
        app.status,
        "Providers parados; análise pausada até iniciar ou reabrir"
    );
    app.dispatch(Command::PersistSession).unwrap();
    idle(&mut app);
    drop(app);
    let mut app = AppService::new(root.path().join("session.json"));
    app.dispatch(Command::RestoreSession).unwrap();
    idle(&mut app);
    let id = app.documents.iter().find(|d| d.path == php).unwrap().id;
    app.dispatch(Command::Edit {
        id,
        expected_version: 1,
        text: "<?php $value = 'não salvo 🌧';\n".into(),
    })
    .unwrap();
    start_policy(&mut app, &profile, None);
    assert!(
        app.language
            .statuses
            .iter()
            .all(|s| s.state == ProviderState::Ready)
    );
    assert!(
        app.documents
            .iter()
            .find(|d| d.id == id)
            .unwrap()
            .is_dirty()
    );
    assert_eq!(fs::read_to_string(&php).unwrap(), "<?php $value = ;\n");
    assert_eq!(
        app.documents.iter().find(|d| d.id == id).unwrap().text(),
        "<?php $value = 'não salvo 🌧';\n"
    );
    app.dispatch(Command::StopProviders).unwrap();
    idle(&mut app);
}
