//! The application must complete inferred JS references without test-side LSP calls.
use codisseu_core::{AppService, Command, Coverage, Language, ProviderConfig, ProviderState};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn until(app: &mut AppService, condition: impl Fn(&AppService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        app.poll();
        if condition(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "status={} rename_error={:?} validation={:?} providers={:?}",
            app.status,
            app.rename.error,
            app.rename.validation,
            app.language.statuses
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn settle(app: &mut AppService) {
    until(app, |app| !app.is_busy() && !app.rename.is_pending());
}

#[test]
#[ignore = "actual inferred JS project; requires explicit CODISSEU_PROVIDER_CONFIG"]
fn inferred_javascript_method_rename_automatically_includes_closed_callers_and_group_history() {
    let configs: Vec<ProviderConfig> = serde_json::from_slice(
        &fs::read(
            std::env::var_os("CODISSEU_PROVIDER_CONFIG")
                .expect("explicit provider configuration required"),
        )
        .unwrap(),
    )
    .unwrap();
    let configs = configs
        .into_iter()
        .filter(|config| config.language == Language::TypeScript)
        .collect::<Vec<_>>();
    assert_eq!(configs.len(), 1);
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/rename-web-audit/javascript");
    for entry in fs::read_dir(fixture).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() != "jsconfig.json" {
            fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
        }
    }
    assert!(!root.join("jsconfig.json").exists() && !root.join("tsconfig.json").exists());
    let disk = fs::read_dir(&root)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            (path.clone(), fs::read_to_string(path).unwrap())
        })
        .collect::<BTreeMap<_, _>>();
    let caller = root.join("main.js");
    let closed = root.join("closed.js");
    let declaration = root.join("weather.js");
    let other = root.join("other.js");
    let mut app = AppService::new(root.join("profile/session.json"));
    app.dispatch(Command::SetAutoSave {
        enabled: false,
        delay_ms: 1000,
    })
    .unwrap();
    app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    settle(&mut app);
    app.dispatch(Command::OpenFile(caller.clone())).unwrap();
    settle(&mut app);
    let document = app.active_document().unwrap();
    let (id, version) = (document.id, document.version);
    let dirty = format!(
        "// unsaved 😀 e\u{301}\n{}\nweather.rainfall();\n",
        document.text()
    );
    app.dispatch(Command::Edit {
        id,
        expected_version: version,
        text: dirty.clone(),
    })
    .unwrap();
    app.dispatch(Command::ConfigureProviders(configs)).unwrap();
    app.dispatch(Command::StartProviders).unwrap();
    settle(&mut app);
    until(&mut app, |app| {
        app.language.statuses.len() == 1
            && app.language.statuses[0].state == ProviderState::Ready
            && app.language.analyzed_versions.get(&caller)
                == Some(&app.active_document().unwrap().version)
    });
    let version = app.active_document().unwrap().version;
    let character_index = dirty[..dirty.find("rainfall()").unwrap() + 2]
        .chars()
        .count();
    app.dispatch(Command::PrepareRename {
        id,
        expected_version: version,
        character_index,
    })
    .unwrap();
    settle(&mut app);
    assert!(app.rename.is_prepared(), "prepare {:?}", app.rename.error);
    app.dispatch(Command::PreviewRename {
        new_name: "forecast".into(),
    })
    .unwrap();
    settle(&mut app);
    if app.rename.choosing_methods() {
        let token = app.rename.context_token().unwrap();
        app.dispatch(Command::ConfirmRenameMethods { token })
            .unwrap();
        settle(&mut app);
    }
    assert!(
        app.rename.can_apply(),
        "automatic preview {:?} {:?}",
        app.rename.error,
        app.rename.validation
    );
    assert_eq!(
        app.rename.validation.as_ref().unwrap().coverage,
        Coverage::Verified
    );
    assert_eq!(app.rename.files.len(), 3);
    assert_eq!(
        app.rename
            .files
            .iter()
            .map(|file| file.edits.len())
            .sum::<usize>(),
        4
    );
    assert!(app.rename.files.iter().any(|file| file.path == closed));
    assert!(app.rename.files.iter().any(|file| file.path == declaration));
    assert!(!app.rename.files.iter().any(|file| file.path == other));
    assert!(
        !app.documents.iter().any(|document| document.path == closed),
        "proof queries must not create an editor document"
    );
    let before = app
        .rename
        .files
        .iter()
        .map(|file| (file.path.clone(), file.before.clone()))
        .collect::<BTreeMap<_, _>>();
    let after = app
        .rename
        .files
        .iter()
        .map(|file| (file.path.clone(), file.after.clone()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(before[&caller].as_ref(), dirty);
    assert!(after[&caller].contains("// rainfall is a comment"));
    assert!(after[&caller].contains("\"rainfall\""));
    for (path, text) in &disk {
        assert_eq!(fs::read_to_string(path).unwrap(), *text);
    }
    app.dispatch(Command::ApplyRename).unwrap();
    settle(&mut app);
    assert!(app.rename.error.is_none(), "apply {:?}", app.rename.error);
    for (path, text) in &after {
        assert_eq!(
            app.documents
                .iter()
                .find(|document| &document.path == path)
                .unwrap()
                .text(),
            text.as_ref()
        );
    }
    for (path, text) in &disk {
        assert_eq!(fs::read_to_string(path).unwrap(), *text);
    }
    app.dispatch(Command::Activate { id }).unwrap();
    app.dispatch(Command::Undo { id }).unwrap();
    settle(&mut app);
    for (path, text) in &before {
        assert_eq!(
            app.documents
                .iter()
                .find(|document| &document.path == path)
                .unwrap()
                .text(),
            text.as_ref()
        );
    }
    app.dispatch(Command::Redo { id }).unwrap();
    settle(&mut app);
    for (path, text) in &after {
        assert_eq!(
            app.documents
                .iter()
                .find(|document| &document.path == path)
                .unwrap()
                .text(),
            text.as_ref()
        );
    }
    let ids = app
        .documents
        .iter()
        .filter(|document| after.contains_key(&document.path))
        .map(|document| document.id)
        .collect::<Vec<_>>();
    for id in ids {
        app.dispatch(Command::Save { id }).unwrap();
        settle(&mut app);
    }
    for (path, text) in &after {
        assert_eq!(fs::read_to_string(path).unwrap(), text.as_ref());
    }
    assert_eq!(
        fs::read_to_string(other).unwrap(),
        disk[&root.join("other.js")]
    );
    app.dispatch(Command::StopProviders).unwrap();
    settle(&mut app);
}
