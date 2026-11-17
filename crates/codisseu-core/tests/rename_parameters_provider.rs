//! Real provider lexical proposals plus independent PHP named-argument completion.
use codisseu_core::{AppService, Command, Coverage, ProviderConfig};
use std::{
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
fn configs() -> Vec<ProviderConfig> {
    serde_json::from_slice(
        &fs::read(
            std::env::var_os("CODISSEU_PROVIDER_CONFIG")
                .expect("CODISSEU_PROVIDER_CONFIG required"),
        )
        .unwrap(),
    )
    .unwrap()
}
fn propose(app: &mut AppService, id: u64, old: &str, new_name: &str) {
    app.dispatch(Command::Activate { id }).unwrap();
    let d = app.active_document().unwrap();
    let position = d.text()[..d.text().find(old).unwrap()].chars().count() + 1;
    app.dispatch(Command::PrepareRename {
        id,
        expected_version: d.version,
        character_index: position,
    })
    .unwrap();
    idle(app);
    assert!(
        app.rename.error.is_none(),
        "prepare {old}: {:?}",
        app.rename.error
    );
    app.dispatch(Command::PreviewRename {
        new_name: new_name.into(),
    })
    .unwrap();
    idle(app);
    assert!(
        app.rename.error.is_none(),
        "preview {old}->{new_name}: {:?}",
        app.rename.error
    );
    if app.rename.choosing_inheritors() {
        let token = app.rename.context_token().unwrap();
        assert!(!app.rename.can_apply());
        assert!(app.rename.can_confirm_inheritors());
        app.dispatch(Command::ConfirmRenameInheritors { token })
            .unwrap();
        idle(app);
        assert!(
            app.rename.error.is_none(),
            "inheritors: {:?}",
            app.rename.error
        );
    }
    if app.rename.choosing_methods() {
        let token = app.rename.context_token().unwrap();
        app.dispatch(Command::SelectRenameMethod {
            token,
            index: 0,
            selected: false,
        })
        .unwrap();
        assert!(
            app.dispatch(Command::ConfirmRenameMethods { token })
                .is_err()
        );
        app.dispatch(Command::SelectRenameMethod {
            token,
            index: 0,
            selected: true,
        })
        .unwrap();
        app.dispatch(Command::ConfirmRenameMethods { token })
            .unwrap();
    }
}
fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let path = entry.unwrap().path();
        let dest = target.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_tree(&path, &dest);
        } else {
            fs::copy(path, dest).unwrap();
        }
    }
}
fn exercise_fixture(kind: &str, fixture_name: &str) {
    let providers = configs();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/{fixture_name}"));
    let cases: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.join("cases.json")).unwrap()).unwrap();
    for case in cases
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["id"].as_str().unwrap().ends_with(kind))
    {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        copy_tree(&fixture, &root);
        let output = std::process::Command::new("composer")
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
        assert!(output.status.success(), "{:?}", output);
        let mut app = AppService::new(dir.path().join("session.json"));
        app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
        idle(&mut app);
        let paths: Vec<_> = case["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let mut ids = Vec::new();
        let mut before = std::collections::BTreeMap::new();
        let mut expected = std::collections::BTreeMap::new();
        let mut disk = std::collections::BTreeMap::new();
        for (i, path) in paths.iter().enumerate() {
            let original = fs::read_to_string(root.join(path)).unwrap();
            disk.insert(*path, original.clone());
            let suffix = if i < 2 {
                "// unsaved participant 😀 é\n"
            } else {
                ""
            };
            before.insert(*path, format!("{original}{suffix}"));
            expected.insert(
                *path,
                format!("{}{suffix}", case["expected"][path].as_str().unwrap()),
            );
            if i < 2 {
                app.dispatch(Command::OpenFile(root.join(path))).unwrap();
                idle(&mut app);
                let d = app.active_document().unwrap();
                let id = d.id;
                app.dispatch(Command::Edit {
                    id,
                    expected_version: d.version,
                    text: before[path].clone(),
                })
                .unwrap();
                ids.push(id);
            }
        }
        app.dispatch(Command::ConfigureProviders(providers.clone()))
            .unwrap();
        app.dispatch(Command::StartProviders).unwrap();
        idle(&mut app);
        let id = ids[0];
        let old = case["old"].as_str().unwrap();
        let new = case["new"].as_str().unwrap();
        propose(&mut app, id, old, new);
        let report = app.rename.validation.as_ref().unwrap();
        eprintln!("{} {}", case["id"], serde_json::to_string(report).unwrap());
        assert_eq!(report.coverage, Coverage::Verified, "{report:#?}");
        assert_eq!(app.rename.files.len(), paths.len());
        for file in &app.rename.files {
            let path = file.path.strip_prefix(&root).unwrap().to_str().unwrap();
            assert_eq!(file.before.as_ref(), before[path], "before {path}");
            assert_eq!(file.after.as_ref(), expected[path], "after {path}");
        }
        app.dispatch(Command::CancelRename).unwrap();
        idle(&mut app);
        for path in &paths {
            assert_eq!(fs::read_to_string(root.join(path)).unwrap(), disk[path]);
        }
        propose(&mut app, id, old, new);
        if app.rename.validation.as_ref().unwrap().scope
            == codisseu_core::ValidationScope::CompilerBindings
        {
            let config_path = root.join("tsconfig.json");
            let configuration = fs::read_to_string(&config_path).unwrap();
            fs::write(&config_path, format!("{configuration}\n")).unwrap();
            app.rename.validation.as_mut().unwrap().coverage = Coverage::Verified;
            app.dispatch(Command::ApplyRename).unwrap();
            idle(&mut app);
            assert!(!app.rename.can_apply());
            for path in &paths {
                assert_eq!(fs::read_to_string(root.join(path)).unwrap(), disk[path]);
            }
            for d in &app.documents {
                let path = d.path.strip_prefix(&root).unwrap().to_str().unwrap();
                if let Some(original) = before.get(path) {
                    assert_eq!(d.text(), original);
                }
            }
            fs::write(&config_path, configuration).unwrap();
            propose(&mut app, id, old, new);
        }
        app.dispatch(Command::ApplyRename).unwrap();
        idle(&mut app);
        assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
        let current = |path: &str| -> String {
            if case["move"].is_array() && case["move"][0].as_str() == Some(path) {
                case["move"][1].as_str().unwrap().into()
            } else {
                path.into()
            }
        };
        for path in &paths {
            assert_eq!(
                app.documents
                    .iter()
                    .find(|d| d.path == root.join(current(path)))
                    .unwrap()
                    .text(),
                expected[path]
            );
            assert_eq!(
                fs::read_to_string(root.join(current(path))).unwrap(),
                disk[path]
            );
        }
        app.dispatch(Command::Undo { id }).unwrap();
        idle(&mut app);
        for path in &paths {
            assert_eq!(
                app.documents
                    .iter()
                    .find(|d| d.path == root.join(path))
                    .unwrap()
                    .text(),
                before[path]
            );
        }
        app.dispatch(Command::Redo { id }).unwrap();
        idle(&mut app);
        for path in &paths {
            let d = app
                .documents
                .iter()
                .find(|d| d.path == root.join(current(path)))
                .unwrap();
            assert_eq!(d.text(), expected[path]);
            app.dispatch(Command::Save { id: d.id }).unwrap();
            idle(&mut app);
            assert_eq!(
                fs::read_to_string(root.join(current(path))).unwrap(),
                expected[path]
            );
        }
        app.dispatch(Command::StopProviders).unwrap();
        idle(&mut app);
        // Rebuild the fixture's own class map only after explicit saves.
        let output = std::process::Command::new("composer")
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
        assert!(output.status.success());
        for folder in cases
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| c["origin"].as_str())
            .filter(|p| p.ends_with(".php"))
            .map(|p| Path::new(p).parent().unwrap().to_str().unwrap())
        {
            let output = std::process::Command::new("php")
                .args(["-d", "xdebug.mode=off", "-d"])
                .arg(format!(
                    "auto_prepend_file={}",
                    root.join("vendor/autoload.php").display()
                ))
                .arg(root.join(folder).join("main.php"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let node = providers
            .iter()
            .find(|c| c.language == codisseu_core::Language::TypeScript)
            .unwrap();
        let output = std::process::Command::new(&node.program)
            .arg(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.tools/node_modules/typescript/bin/tsc"),
            )
            .args(["--noEmit", "--project"])
            .arg(root.join("tsconfig.json"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
#[ignore = "real PHP/TS parameter proposals; CODISSEU_PROVIDER_CONFIG"]
fn real_named_parameters_preserve_callees_homonyms_dirty_and_group_history() {
    exercise_fixture("parameters", "rename-parameters-complete");
}
