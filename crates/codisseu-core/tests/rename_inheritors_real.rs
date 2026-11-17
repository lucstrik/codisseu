//! Real provider lifecycle around independently proved PHP class/file choices.
use codisseu_core::{
    AppService, Command, Language, ProviderConfig, RenameInheritorKind, RenameOptions,
};
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
fn copy(source: &Path, dest: &Path) {
    fs::create_dir_all(dest).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let path = entry.unwrap().path();
        let target = dest.join(path.file_name().unwrap());
        if path.is_dir() {
            copy(&path, &target);
        } else {
            fs::copy(path, target).unwrap();
        }
    }
}
fn preview(app: &mut AppService, id: u64, case: &serde_json::Value) {
    app.dispatch(Command::Activate { id }).unwrap();
    let doc = app.active_document().unwrap();
    let offset = doc.text().find(case["old"].as_str().unwrap()).unwrap();
    app.dispatch(Command::PrepareRename {
        id,
        expected_version: doc.version,
        character_index: doc.text()[..offset].chars().count() + 1,
    })
    .unwrap();
    idle(app);
    assert!(app.rename.is_prepared(), "{:?}", app.rename.error);
    app.dispatch(Command::SetRenameOptions {
        options: RenameOptions {
            rename_inheritors: case["options"]["inheritors"].as_bool().unwrap(),
            ..Default::default()
        },
    })
    .unwrap();
    app.dispatch(Command::PreviewRename {
        new_name: case["new"].as_str().unwrap().into(),
    })
    .unwrap();
    idle(app);
    assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
    if app.rename.choosing_inheritors() {
        let token = app.rename.context_token().unwrap();
        if let Some(selected) = case["inheritors"].as_array() {
            let rows = app.rename.inheritor_choices().to_vec();
            for (index, row) in rows.iter().enumerate() {
                let requested = selected.iter().find(|r| {
                    r["old"].as_str() == Some(row.old_name.as_str())
                        && r["kind"].as_str()
                            == Some(match row.kind {
                                RenameInheritorKind::Class => "class",
                                RenameInheritorKind::File => "file",
                            })
                });
                app.dispatch(Command::UpdateRenameInheritor {
                    token,
                    index,
                    selected: requested.is_some(),
                    new_name: requested
                        .map(|r| r["new"].as_str().unwrap().to_owned())
                        .unwrap_or_else(|| row.new_name.clone()),
                })
                .unwrap();
            }
        }
        assert!(app.rename.can_confirm_inheritors());
        app.dispatch(Command::ConfirmRenameInheritors { token })
            .unwrap();
        idle(app);
    }
}

#[test]
#[ignore = "real PHP provider, composer/php executable and CODISSEU_PROVIDER_CONFIG"]
fn real_inheritor_choices_preserve_dirty_closed_files_and_save_undo_redo() {
    let config = std::env::var_os("CODISSEU_PROVIDER_CONFIG").expect("provider config required");
    let providers: Vec<ProviderConfig> =
        serde_json::from_slice(&fs::read(config).unwrap()).unwrap();
    let php: Vec<_> = providers
        .into_iter()
        .filter(|p| p.language == Language::Php)
        .collect();
    assert!(!php.is_empty());
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/rename-inheritors-complete");
    let cases: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.join("cases.json")).unwrap()).unwrap();
    for case in cases
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["id"].as_str().unwrap().starts_with("php-"))
    {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        copy(&fixture, &root);
        let composer = std::process::Command::new("composer")
            .args([
                "dump-autoload",
                "--no-scripts",
                "--no-plugins",
                "--no-interaction",
            ])
            .env("COMPOSER_DISABLE_NETWORK", "1")
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(composer.status.success(), "{:?}", composer);
        let mut app = AppService::new(root.join("session.json"));
        app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
        idle(&mut app);
        let paths: Vec<_> = case["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap())
            .collect();
        let mut originals = BTreeMap::new();
        let mut before = BTreeMap::new();
        let mut expected = BTreeMap::new();
        let mut origin_id = None;
        for path in &paths {
            let original = fs::read_to_string(root.join(path)).unwrap();
            let open = *path == case["origin"].as_str().unwrap() || path.ends_with("/main.php");
            let suffix = if open { "// unsaved 😀 é\n" } else { "" };
            originals.insert(*path, original.clone());
            before.insert(*path, format!("{original}{suffix}"));
            expected.insert(
                *path,
                format!("{}{suffix}", case["expected"][path].as_str().unwrap()),
            );
            if open {
                app.dispatch(Command::OpenFile(root.join(path))).unwrap();
                idle(&mut app);
                let doc = app.active_document().unwrap();
                let id = doc.id;
                app.dispatch(Command::Edit {
                    id,
                    expected_version: doc.version,
                    text: before[path].clone(),
                })
                .unwrap();
                if *path == case["origin"].as_str().unwrap() {
                    origin_id = Some(id);
                }
            }
        }
        let id = origin_id.unwrap();
        app.dispatch(Command::ConfigureProviders(php.clone()))
            .unwrap();
        app.dispatch(Command::StartProviders).unwrap();
        idle(&mut app);
        preview(&mut app, id, case);
        app.dispatch(Command::CancelRename).unwrap();
        idle(&mut app);
        for path in &paths {
            assert_eq!(
                fs::read_to_string(root.join(path)).unwrap(),
                originals[path]
            );
        }
        preview(&mut app, id, case);
        if case["expect_blocked"].as_bool() == Some(true) {
            assert!(!app.rename.can_apply());
            assert!(app.rename.semantic_blocked());
            app.dispatch(Command::CancelRename).unwrap();
            for path in &paths {
                assert_eq!(
                    fs::read_to_string(root.join(path)).unwrap(),
                    originals[path]
                );
            }
            assert_eq!(
                app.documents.iter().find(|d| d.id == id).unwrap().text(),
                before[case["origin"].as_str().unwrap()]
            );
            app.dispatch(Command::StopProviders).unwrap();
            idle(&mut app);
            continue;
        }
        assert!(
            app.rename.can_apply(),
            "{} {:?} {:?}",
            case["id"],
            app.rename.error,
            app.rename.validation
        );
        let current = |path: &str| -> String {
            case["moves"]
                .as_array()
                .and_then(|moves| moves.iter().find(|m| m[0].as_str() == Some(path)))
                .map(|m| m[1].as_str().unwrap().to_owned())
                .unwrap_or_else(|| path.into())
        };
        app.dispatch(Command::ApplyRename).unwrap();
        idle(&mut app);
        assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
        for path in &paths {
            assert_eq!(
                fs::read_to_string(root.join(current(path))).unwrap(),
                originals[path]
            );
            if let Some(doc) = app
                .documents
                .iter()
                .find(|d| d.path == root.join(current(path)))
            {
                assert_eq!(doc.text(), expected[path], "{} {path}", case["id"]);
            } else {
                assert_eq!(expected[path], before[path]);
            }
        }
        app.dispatch(Command::Undo { id }).unwrap();
        idle(&mut app);
        for path in &paths {
            assert_eq!(
                fs::read_to_string(root.join(path)).unwrap(),
                originals[path]
            );
        }
        assert_eq!(
            app.documents.iter().find(|d| d.id == id).unwrap().text(),
            before[case["origin"].as_str().unwrap()]
        );
        app.dispatch(Command::Redo { id }).unwrap();
        idle(&mut app);
        for path in &paths {
            if let Some(doc) = app
                .documents
                .iter()
                .find(|d| d.path == root.join(current(path)))
            {
                app.dispatch(Command::Save { id: doc.id }).unwrap();
                idle(&mut app);
            }
            assert_eq!(
                fs::read_to_string(root.join(current(path))).unwrap(),
                expected[path]
            );
        }
        // Undo after Save moves the newly saved bytes; saved_text must remain truthful.
        app.dispatch(Command::Undo { id }).unwrap();
        idle(&mut app);
        for path in &paths {
            assert_eq!(fs::read_to_string(root.join(path)).unwrap(), expected[path]);
        }
        app.dispatch(Command::Redo { id }).unwrap();
        idle(&mut app);
        for path in &paths {
            assert_eq!(
                fs::read_to_string(root.join(current(path))).unwrap(),
                expected[path]
            );
        }
        app.dispatch(Command::StopProviders).unwrap();
        idle(&mut app);
        let main = paths.iter().find(|p| p.ends_with("/main.php")).unwrap();
        let output = std::process::Command::new("php")
            .args(["-d", "xdebug.mode=off"])
            .arg(root.join(main))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{} {}",
            case["id"],
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
