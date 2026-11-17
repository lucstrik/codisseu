//! A real PHP provider on an owned disposable Composer project. Never runs user code.
use codisseu_core::{AppService, Command, Language, ProviderConfig, ProviderState};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &to.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}
fn wait(app: &mut AppService, check: impl Fn(&AppService) -> bool) {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        app.poll();
        assert!(
            app.last_error.is_none(),
            "{:?} / {}",
            app.last_error,
            app.status
        );
        if check(app) {
            return;
        }
        assert!(
            Instant::now() < end,
            "{} / {:?} / {:?}",
            app.status,
            app.language.statuses,
            app.language.definition_error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn position(app: &AppService, needle: &str) -> (codisseu_core::DocumentId, u64, usize) {
    let d = app.active_document().unwrap();
    (
        d.id,
        d.version,
        d.text()[..d.text().rfind(needle).unwrap()].chars().count() + 1,
    )
}
fn open(app: &mut AppService, path: &Path) {
    app.dispatch(Command::OpenFile(path.into())).unwrap();
    wait(app, |a| !a.is_busy());
}

#[test]
#[ignore = "real PHP class/function/interface/factory navigation; requires CODISSEU_PROVIDER_CONFIG + Composer"]
fn real_php_navigation_preview_implementations_history_and_dirty_buffers() {
    let configs: Vec<ProviderConfig> = serde_json::from_slice(
        &fs::read(std::env::var_os("CODISSEU_PROVIDER_CONFIG").expect("provider config required"))
            .unwrap(),
    )
    .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    copy(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/php-navigation"),
        &root,
    );
    let root = root.canonicalize().unwrap();
    let mut original = Vec::new();
    for name in [
        "main.php",
        "src/ParcelGateway.php",
        "src/ParcelFactory.php",
        "src/CourierService.php",
        "src/LocalService.php",
        "src/BaseService.php",
        "src/helpers.php",
        "composer.json",
    ] {
        original.push((root.join(name), fs::read(root.join(name)).unwrap()));
    }
    let output = std::process::Command::new("composer")
        .args([
            "dump-autoload",
            "--no-scripts",
            "--no-plugins",
            "--no-interaction",
        ])
        .current_dir(&root)
        .env("COMPOSER_DISABLE_NETWORK", "1")
        .output()
        .expect("Composer required");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut app = AppService::new(temp.path().join("session.json"));
    app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    wait(&mut app, |a| !a.is_busy());
    open(&mut app, &root.join("main.php"));
    app.dispatch(Command::ConfigureProviders(
        configs
            .into_iter()
            .filter(|p| p.language == Language::Php)
            .collect(),
    ))
    .unwrap();
    app.dispatch(Command::StartProviders).unwrap();
    wait(&mut app, |a| {
        a.language
            .statuses
            .iter()
            .any(|s| s.state == ProviderState::Ready)
            && !a.is_busy()
    });
    for (needle, target) in [
        ("ParcelFactory::", "src/ParcelFactory.php"),
        ("create();", "src/ParcelFactory.php"),
        ("CourierService())", "src/CourierService.php"),
        ("inherited();", "src/BaseService.php"),
        ("quote_label();", "src/helpers.php"),
        ("deliver();", "src/ParcelGateway.php"),
    ] {
        open(&mut app, &root.join("main.php"));
        let (id, version, at) = position(&app, needle);
        let selection = app.active_document().unwrap().selection();
        let status = app.status.clone();
        let history = (app.navigation.can_back(), app.navigation.can_forward());
        app.dispatch(Command::PreviewDefinition {
            id,
            expected_version: version,
            character_index: at,
        })
        .unwrap();
        wait(&mut app, |a| !a.is_busy());
        let preview = app.navigation.preview.as_ref().expect("preview result");
        assert_eq!(preview.targets.len(), 1, "{needle}: {:?}", preview.targets);
        assert_eq!(preview.targets[0].path, root.join(target), "{needle}");
        assert_eq!(app.active, Some(id));
        assert_eq!(app.active_document().unwrap().selection(), selection);
        assert_eq!(app.status, status);
        assert_eq!(
            (app.navigation.can_back(), app.navigation.can_forward()),
            history
        );
        app.dispatch(Command::Definition {
            id,
            expected_version: version,
            character_index: at,
        })
        .unwrap();
        wait(&mut app, |a| !a.is_busy());
        assert_eq!(
            app.active_document().unwrap().path,
            root.join(target),
            "{needle}"
        );
        let d = app.active_document().unwrap();
        app.dispatch(Command::NavigateBack {
            id: d.id,
            expected_version: d.version,
            character_index: 0,
        })
        .unwrap();
        assert_eq!(app.active, Some(id));
        assert_eq!(app.navigation.selection.as_ref().unwrap().range, at..at);
        eprintln!("{needle} -> {target}; preview, definition, history preserved");
    }
    // An interface method resolves to both real implementations, requiring a choice.
    let (id, version, at) = position(&app, "deliver();");
    app.dispatch(Command::Implementation {
        id,
        expected_version: version,
        character_index: at,
    })
    .unwrap();
    wait(&mut app, |a| !a.navigation.is_pending() && !a.is_busy());
    let mut paths: Vec<_> = app
        .navigation
        .choices
        .iter()
        .map(|t| t.path.clone())
        .collect();
    paths.sort();
    assert_eq!(
        paths,
        vec![
            root.join("src/CourierService.php"),
            root.join("src/LocalService.php")
        ]
    );
    assert_eq!(app.active, Some(id));
    assert!(app.navigation.is_implementation());
    let labels = app
        .navigation
        .presentation
        .as_ref()
        .expect("AST labels for the chooser");
    assert_eq!(labels.source_symbol, "deliver");
    assert_eq!(
        labels
            .targets
            .iter()
            .map(|l| l.as_ref().unwrap().name.as_str())
            .collect::<Vec<_>>(),
        vec!["CourierService", "LocalService"]
    );
    assert!(labels.targets.iter().all(|l| {
        l.as_ref()
            .is_some_and(|l| l.namespace == "\\CodisseuNavigation" && l.kind == 'm' && l.public)
    }));
    app.dispatch(Command::ChooseDefinition { index: 0 })
        .unwrap();
    wait(&mut app, |a| !a.is_busy());
    assert!(paths.contains(&app.active_document().unwrap().path));
    let d = app.active_document().unwrap();
    app.dispatch(Command::NavigateBack {
        id: d.id,
        expected_version: d.version,
        character_index: 0,
    })
    .unwrap();
    // Comments and unresolved functions must not receive a false link.
    open(&mut app, &root.join("main.php"));
    let d = app.active_document().unwrap();
    let id = d.id;
    let version = d.version;
    let negative = [
        d.text()[..d.text().find("missing_symbol()").unwrap()]
            .chars()
            .count()
            + 1,
        d.text()[..d.text().rfind("missing_symbol()").unwrap()]
            .chars()
            .count()
            + 1,
    ];
    for at in negative {
        app.dispatch(Command::PreviewDefinition {
            id,
            expected_version: version,
            character_index: at,
        })
        .unwrap();
        wait(&mut app, |a| !a.is_busy());
        assert!(
            app.navigation
                .preview
                .as_ref()
                .is_some_and(|p| p.targets.is_empty())
        );
        assert_eq!(app.active, Some(id));
    }
    // Dirty target: provider and opening must reuse the in-memory version.
    open(&mut app, &root.join("src/ParcelFactory.php"));
    let d = app.active_document().unwrap();
    let target_id = d.id;
    let dirty = format!("\n{}", d.text());
    app.dispatch(Command::Edit {
        id: target_id,
        expected_version: d.version,
        text: dirty.clone(),
    })
    .unwrap();
    open(&mut app, &root.join("main.php"));
    let (id, version, at) = position(&app, "create();");
    app.dispatch(Command::PreviewDefinition {
        id,
        expected_version: version,
        character_index: at,
    })
    .unwrap();
    wait(&mut app, |a| !a.is_busy());
    assert!(app.navigation.preview.is_some());
    let d = app.documents.iter().find(|d| d.id == target_id).unwrap();
    assert!(d.is_dirty());
    // Edit another buffer invalidates a resolved hover without changing source version.
    app.dispatch(Command::Edit {
        id: target_id,
        expected_version: d.version,
        text: format!("\n{dirty}"),
    })
    .unwrap();
    assert!(app.navigation.preview.is_none());
    app.dispatch(Command::Definition {
        id,
        expected_version: version,
        character_index: at,
    })
    .unwrap();
    wait(&mut app, |a| !a.is_busy());
    assert_eq!(app.active, Some(target_id));
    assert_eq!(app.active_document().unwrap().text(), format!("\n{dirty}"));
    app.dispatch(Command::StopProviders).unwrap();
    wait(&mut app, |a| !a.is_busy());
    for (path, bytes) in original {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    assert!(!root.join("HOOK_EXECUTED").exists());
}
