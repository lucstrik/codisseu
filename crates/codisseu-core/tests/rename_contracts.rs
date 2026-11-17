//! Real-provider refactorings must remain complete across nominal contracts.
use codisseu_core::{AppService, Command, Coverage, ProviderConfig};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn idle(app: &mut AppService) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        app.poll();
        if !app.is_busy() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{} {:?}",
            app.status,
            app.rename.error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn fixture(root: &Path) -> BTreeMap<String, String> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/rename-contracts");
    let mut result = BTreeMap::new();
    for entry in fs::read_dir(&source).unwrap() {
        let path = entry.unwrap().path();
        let paths = if path.is_dir() {
            fs::create_dir(root.join(path.file_name().unwrap())).unwrap();
            fs::read_dir(path)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect::<Vec<_>>()
        } else {
            vec![path]
        };
        for path in paths {
            let name = path
                .strip_prefix(&source)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            let text = fs::read_to_string(path).unwrap();
            fs::write(root.join(&name), &text).unwrap();
            result.insert(name, text);
        }
    }
    let output = std::process::Command::new("composer")
        .args([
            "dump-autoload",
            "--no-scripts",
            "--no-plugins",
            "--no-interaction",
        ])
        .env("COMPOSER_DISABLE_NETWORK", "1")
        .current_dir(root)
        .output()
        .expect("Composer required");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    result
}
fn configs() -> Vec<ProviderConfig> {
    serde_json::from_slice(
        &fs::read(std::env::var_os("CODISSEU_PROVIDER_CONFIG").expect("provider config required"))
            .unwrap(),
    )
    .unwrap()
}
fn open(app: &mut AppService, path: &Path, dirty: bool) -> u64 {
    app.dispatch(Command::OpenFile(path.to_path_buf())).unwrap();
    idle(app);
    let doc = app.active_document().unwrap();
    let id = doc.id;
    if dirty {
        app.dispatch(Command::Edit {
            id,
            expected_version: doc.version,
            text: format!("{}// unsaved é 🌧\n", doc.text()),
        })
        .unwrap();
    }
    id
}
fn propose(app: &mut AppService, id: u64, new_name: &str) {
    app.dispatch(Command::Activate { id }).unwrap();
    let doc = app.active_document().unwrap();
    let position = doc.text()[..doc.text().find("rainfall").unwrap()]
        .chars()
        .count()
        + 2;
    app.dispatch(Command::PrepareRename {
        id,
        expected_version: doc.version,
        character_index: position,
    })
    .unwrap();
    idle(app);
    assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
    app.dispatch(Command::PreviewRename {
        new_name: new_name.into(),
    })
    .unwrap();
    idle(app);
    assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
}
fn assert_disk(root: &Path, files: &BTreeMap<String, String>) {
    for (name, text) in files {
        assert_eq!(
            fs::read_to_string(root.join(name)).unwrap(),
            *text,
            "{name}"
        );
    }
}
fn verify_runtime(root: &Path, php_entry: &str, configs: &[ProviderConfig]) {
    let php = std::process::Command::new("php")
        .arg(php_entry)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        php.status.success(),
        "{}",
        String::from_utf8_lossy(&php.stderr)
    );
    let ts = configs
        .iter()
        .find(|c| c.language == codisseu_core::Language::TypeScript)
        .unwrap();
    let compiler = Path::new(&ts.args[0])
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("typescript/bin/tsc");
    let output = std::process::Command::new(&ts.program)
        .arg(compiler)
        .args(["--noEmit", "--project", "tsconfig.json"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "real PHP/TS interface union, dirty buffers, history and runtime; CODISSEU_PROVIDER_CONFIG and Composer"]
fn real_interface_rename_from_all_origins_preserves_contract_calls_and_history() {
    let provider_configs = configs();
    for (main, implementation, contract) in [
        ("main.php", "src/Weather.php", "src/WeatherPort.php"),
        ("main.ts", "src/weather.ts", "src/weatherPort.ts"),
    ] {
        for origin in [main, implementation, contract] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let originals = fixture(&root);
            let mut app = AppService::new(root.join("session.json"));
            app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
            idle(&mut app);
            let main_id = open(&mut app, &root.join(main), true);
            let implementation_id = open(&mut app, &root.join(implementation), true);
            let origin_id = if origin == main {
                main_id
            } else if origin == implementation {
                implementation_id
            } else {
                open(&mut app, &root.join(contract), true)
            };
            let opened = app.documents.len();
            let before: Vec<_> = app
                .documents
                .iter()
                .map(|d| (d.id, d.version, d.text().to_owned(), d.selection()))
                .collect();
            app.dispatch(Command::ConfigureProviders(provider_configs.clone()))
                .unwrap();
            app.dispatch(Command::StartProviders).unwrap();
            idle(&mut app);
            let new_name = if origin == implementation {
                "precipitação"
            } else {
                "precipitation"
            };
            propose(&mut app, origin_id, new_name);
            let validation = app.rename.validation.as_ref().unwrap();
            eprintln!(
                "interface origin={origin} validation={}",
                serde_json::to_string(validation).unwrap()
            );
            assert_eq!(validation.coverage, Coverage::Verified, "{validation:?}");
            assert_eq!(
                app.documents.len(),
                opened,
                "closed query lease must not create editor tabs"
            );
            assert_eq!(app.rename.files.len(), 3);
            assert_eq!(
                app.rename
                    .files
                    .iter()
                    .map(|f| f.edits.len())
                    .sum::<usize>(),
                4,
                "both concrete and interface-typed calls required regardless of origin"
            );
            assert!(app.rename.choosing_methods());
            assert_eq!(app.rename.related_methods().len(), 2);
            let expected = app.rename.files.clone();
            let main_preview = expected.iter().find(|f| f.path == root.join(main)).unwrap();
            assert_eq!(main_preview.edits.len(), 2);
            assert!(main_preview.after.contains("// rainfall remains a comment"));
            assert!(main_preview.after.contains("'rainfall' remains a string"));
            assert_disk(&root, &originals);
            app.dispatch(Command::CancelRename).unwrap();
            idle(&mut app);
            for (id, version, text, selection) in &before {
                let d = app.documents.iter().find(|d| d.id == *id).unwrap();
                assert_eq!(
                    (d.version, d.text(), d.selection()),
                    (*version, text.as_str(), *selection)
                );
            }
            propose(&mut app, origin_id, new_name);
            let token = app.rename.context_token().unwrap();
            app.dispatch(Command::ConfirmRenameMethods { token })
                .unwrap();
            app.dispatch(Command::ApplyRename).unwrap();
            idle(&mut app);
            assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
            for f in &expected {
                assert_eq!(
                    app.documents
                        .iter()
                        .find(|d| d.path == f.path)
                        .unwrap()
                        .text(),
                    f.after.as_ref()
                );
            }
            app.dispatch(Command::Undo { id: main_id }).unwrap();
            for f in &expected {
                assert_eq!(
                    app.documents
                        .iter()
                        .find(|d| d.path == f.path)
                        .unwrap()
                        .text(),
                    f.before.as_ref()
                );
            }
            app.dispatch(Command::Redo {
                id: implementation_id,
            })
            .unwrap();
            assert_disk(&root, &originals);
            let mut saved = originals.clone();
            for f in &expected {
                let id = app.documents.iter().find(|d| d.path == f.path).unwrap().id;
                app.dispatch(Command::Save { id }).unwrap();
                idle(&mut app);
                saved.insert(
                    f.path.strip_prefix(&root).unwrap().to_str().unwrap().into(),
                    f.after.to_string(),
                );
            }
            assert_disk(&root, &saved);
            verify_runtime(&root, "main.php", &provider_configs);
            app.dispatch(Command::StopProviders).unwrap();
            idle(&mut app);
        }
    }
}

#[test]
#[ignore = "real PHP trait, consuming-class collision and explicit save; CODISSEU_PROVIDER_CONFIG and Composer"]
fn real_trait_rename_preserves_unrelated_contracts_and_blocks_dirty_consumer_collision() {
    let provider_configs = configs();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let originals = fixture(&root);
    let mut app = AppService::new(root.join("session.json"));
    app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    idle(&mut app);
    let main = open(&mut app, &root.join("trait.php"), true);
    open(&mut app, &root.join("src/RainProvider.php"), true);
    let consumer = open(&mut app, &root.join("src/TraitWeather.php"), false);
    let before = app.active_document().unwrap().text().to_owned();
    let collision = before.replace(
        "use RainProvider;",
        "use RainProvider; public function precipitation(string $city): string { return $city; }",
    );
    app.dispatch(Command::Edit {
        id: consumer,
        expected_version: 1,
        text: collision.clone(),
    })
    .unwrap();
    app.dispatch(Command::ConfigureProviders(provider_configs.clone()))
        .unwrap();
    app.dispatch(Command::StartProviders).unwrap();
    idle(&mut app);
    propose(&mut app, main, "precipitation");
    assert_ne!(
        app.rename.validation.as_ref().unwrap().coverage,
        Coverage::Verified
    );
    assert!(app.dispatch(Command::ApplyRename).is_err());
    assert_disk(&root, &originals);
    assert_eq!(
        app.documents
            .iter()
            .find(|d| d.id == consumer)
            .unwrap()
            .text(),
        collision
    );
    app.dispatch(Command::Undo { id: consumer }).unwrap();
    propose(&mut app, main, "precipitação");
    assert_eq!(
        app.rename.validation.as_ref().unwrap().coverage,
        Coverage::Verified,
        "{:?}",
        app.rename.validation
    );
    assert_eq!(app.rename.files.len(), 2);
    assert_eq!(
        app.rename
            .files
            .iter()
            .map(|f| f.edits.len())
            .sum::<usize>(),
        2
    );
    let proposal = app.rename.files.clone();
    if app.rename.choosing_methods() {
        let token = app.rename.context_token().unwrap();
        app.dispatch(Command::ConfirmRenameMethods { token })
            .unwrap();
    }
    app.dispatch(Command::ApplyRename).unwrap();
    idle(&mut app);
    assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
    app.dispatch(Command::Undo { id: main }).unwrap();
    for f in &proposal {
        assert_eq!(
            app.documents
                .iter()
                .find(|d| d.path == f.path)
                .unwrap()
                .text(),
            f.before.as_ref()
        );
    }
    app.dispatch(Command::Redo { id: main }).unwrap();
    assert_disk(&root, &originals);
    let mut saved = originals.clone();
    for f in &proposal {
        let id = app.documents.iter().find(|d| d.path == f.path).unwrap().id;
        app.dispatch(Command::Save { id }).unwrap();
        idle(&mut app);
        saved.insert(
            f.path.strip_prefix(&root).unwrap().to_str().unwrap().into(),
            f.after.to_string(),
        );
    }
    assert_disk(&root, &saved);
    verify_runtime(&root, "trait.php", &provider_configs);
    app.dispatch(Command::StopProviders).unwrap();
    idle(&mut app);
}

#[test]
#[ignore = "real TS two implementations inside/outside tsconfig; CODISSEU_PROVIDER_CONFIG and Composer"]
fn real_multiple_implementations_require_complete_provider_project_context() {
    let provider_configs = configs();
    for included in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let mut originals = fixture(&root);
        if !included {
            let mut config: serde_json::Value =
                serde_json::from_str(&originals["tsconfig.json"]).unwrap();
            config["include"] = serde_json::json!(["*.ts", "src/*.ts"]);
            let text = serde_json::to_string(&config).unwrap();
            fs::write(root.join("tsconfig.json"), &text).unwrap();
            originals.insert("tsconfig.json".into(), text);
        }
        let mut app = AppService::new(root.join("session.json"));
        app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
        idle(&mut app);
        let contract = open(&mut app, &root.join("multiple/forecast.ts"), true);
        let before = app.active_document().unwrap().text().to_owned();
        let version = app.active_document().unwrap().version;
        app.dispatch(Command::ConfigureProviders(provider_configs.clone()))
            .unwrap();
        app.dispatch(Command::StartProviders).unwrap();
        idle(&mut app);
        propose(&mut app, contract, "precipitation");
        eprintln!(
            "multiple implementations included={included} validation={}",
            serde_json::to_string(app.rename.validation.as_ref().unwrap()).unwrap()
        );
        assert_eq!(
            app.documents.len(),
            1,
            "temporary queries must not open editor tabs"
        );
        assert_disk(&root, &originals);
        if !included {
            assert_eq!(
                app.rename.validation.as_ref().unwrap().coverage,
                Coverage::Rejected
            );
            assert!(
                app.rename
                    .validation
                    .as_ref()
                    .unwrap()
                    .conflicts
                    .iter()
                    .all(|c| matches!(
                        c.kind,
                        codisseu_core::ConflictKind::OverrideChanged
                            | codisseu_core::ConflictKind::MissingReference
                    ))
            );
            assert!(!app.rename.choosing_methods());
            assert!(app.dispatch(Command::ApplyRename).is_err());
            app.dispatch(Command::CancelRename).unwrap();
            assert_eq!(
                (
                    app.active_document().unwrap().version,
                    app.active_document().unwrap().text()
                ),
                (version, before.as_str())
            );
            assert_disk(&root, &originals);
        } else {
            assert_eq!(
                app.rename.validation.as_ref().unwrap().coverage,
                Coverage::Verified,
                "{:?}",
                app.rename.validation
            );
            assert_eq!(app.rename.related_methods().len(), 3);
            assert_eq!(app.rename.files.len(), 4);
            assert_eq!(
                app.rename
                    .files
                    .iter()
                    .map(|f| f.edits.len())
                    .sum::<usize>(),
                4
            );
            let proposal = app.rename.files.clone();
            let token = app.rename.context_token().unwrap();
            app.dispatch(Command::ConfirmRenameMethods { token })
                .unwrap();
            app.dispatch(Command::ApplyRename).unwrap();
            idle(&mut app);
            assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
            for f in &proposal {
                assert_eq!(
                    app.documents
                        .iter()
                        .find(|d| d.path == f.path)
                        .unwrap()
                        .text(),
                    f.after.as_ref()
                );
            }
            app.dispatch(Command::Undo { id: contract }).unwrap();
            for f in &proposal {
                assert_eq!(
                    app.documents
                        .iter()
                        .find(|d| d.path == f.path)
                        .unwrap()
                        .text(),
                    f.before.as_ref()
                );
            }
            app.dispatch(Command::Redo { id: contract }).unwrap();
            assert_disk(&root, &originals);
            let mut saved = originals.clone();
            for f in &proposal {
                let id = app.documents.iter().find(|d| d.path == f.path).unwrap().id;
                app.dispatch(Command::Save { id }).unwrap();
                idle(&mut app);
                saved.insert(
                    f.path.strip_prefix(&root).unwrap().to_str().unwrap().into(),
                    f.after.to_string(),
                );
            }
            assert_disk(&root, &saved);
            verify_runtime(&root, "main.php", &provider_configs);
        }
        app.dispatch(Command::StopProviders).unwrap();
        idle(&mut app);
    }
}
