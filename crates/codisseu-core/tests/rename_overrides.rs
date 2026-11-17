use codisseu_core::{AppService, Command, Coverage, ProviderConfig};
use std::{
    fs, thread,
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
fn propose(app: &mut AppService, id: u64) {
    let d = app.documents.iter().find(|d| d.id == id).unwrap();
    let expected_version = d.version;
    let character_index = d.text()[..d.text().find("rainfall()").unwrap()]
        .chars()
        .count()
        + 2;
    app.dispatch(Command::Activate { id }).unwrap();
    app.dispatch(Command::PrepareRename {
        id,
        expected_version,
        character_index,
    })
    .unwrap();
    idle(app);
    assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
    app.dispatch(Command::PreviewRename {
        new_name: "precipitation".into(),
    })
    .unwrap();
    idle(app);
    assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
}
#[test]
#[ignore = "real PHP/TS full override rename; CODISSEU_PROVIDER_CONFIG and Composer required"]
fn real_complete_overrides_preserve_dirty_contracts_history_and_explicit_saves() {
    let configs: Vec<ProviderConfig> = serde_json::from_slice(
        &fs::read(std::env::var_os("CODISSEU_PROVIDER_CONFIG").expect("provider config")).unwrap(),
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir(root.join("src")).unwrap();
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/rename-overrides");
    let names = [
        "composer.json",
        "tsconfig.json",
        "main.php",
        "other.php",
        "main.ts",
        "other.ts",
        "src/BaseWeather.php",
        "src/Weather.php",
        "src/WetWeather.php",
        "src/baseWeather.ts",
        "src/weather.ts",
        "src/wetWeather.ts",
    ];
    let mut originals = std::collections::BTreeMap::new();
    for name in names {
        let text = fs::read_to_string(fixture.join(name)).unwrap();
        fs::write(root.join(name), &text).unwrap();
        originals.insert(name, text);
    }
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
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut app = AppService::new(root.join("session.json"));
    app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    idle(&mut app);
    let mut groups = vec![];
    for (main, decl, child) in [
        ("main.php", "src/Weather.php", "src/WetWeather.php"),
        ("main.ts", "src/weather.ts", "src/wetWeather.ts"),
    ] {
        let mut ids = vec![];
        for path in [main, decl, child] {
            app.dispatch(Command::OpenFile(root.join(path))).unwrap();
            idle(&mut app);
            let d = app.active_document().unwrap();
            let id = d.id;
            let text = format!("{}// dirty {path}\n", d.text());
            app.dispatch(Command::Edit {
                id,
                expected_version: 1,
                text,
            })
            .unwrap();
            ids.push(id);
        }
        groups.push(ids);
    }
    app.dispatch(Command::ConfigureProviders(configs)).unwrap();
    app.dispatch(Command::StartProviders).unwrap();
    idle(&mut app);
    for ids in groups {
        let main = ids[0];
        let child = ids[2];
        let d = app.documents.iter().find(|d| d.id == child).unwrap();
        let before_child = d.text().to_owned();
        let version = d.version;
        let divergent = before_child.replace(
            ": string",
            if d.path.extension().unwrap() == "php" {
                ": int"
            } else {
                ": number"
            },
        );
        app.dispatch(Command::Edit {
            id: child,
            expected_version: version,
            text: divergent.clone(),
        })
        .unwrap();
        propose(&mut app, main);
        let r = app.rename.validation.as_ref().unwrap();
        assert_eq!(r.coverage, Coverage::Unsupported, "{r:?}");
        let child_path = &app.documents.iter().find(|d| d.id == child).unwrap().path;
        assert!(
            r.issues.iter().any(|issue| {
                issue.path == *child_path
                    && issue.range.is_some()
                    && issue.kind == codisseu_core::IssueKind::UnsupportedClass
            }),
            "{r:?}"
        );
        assert!(app.dispatch(Command::ApplyRename).is_err());
        assert_eq!(
            app.documents.iter().find(|d| d.id == child).unwrap().text(),
            divergent
        );
        app.dispatch(Command::Undo { id: child }).unwrap();
        propose(&mut app, main);
        let r = app.rename.validation.as_ref().unwrap();
        assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
        assert_eq!(r.checked_override_pairs, 1);
        assert_eq!(r.checked_declarations, 2);
        assert_eq!(app.rename.files.len(), 4, "{:?}", app.rename.files);
        assert_eq!(
            app.rename
                .files
                .iter()
                .map(|f| f.edits.len())
                .sum::<usize>(),
            4
        );
        assert!(app.rename.choosing_methods());
        assert!(!app.rename.can_apply());
        assert_eq!(app.rename.related_methods().len(), 2);
        let token = app.rename.context_token().unwrap();
        assert!(app.dispatch(Command::ApplyRename).is_err());
        assert!(
            app.dispatch(Command::SelectRenameMethod {
                token: token + 1,
                index: 0,
                selected: false
            })
            .is_err()
        );
        assert!(
            app.dispatch(Command::SelectRenameMethod {
                token,
                index: 999,
                selected: false
            })
            .is_err()
        );
        app.dispatch(Command::SelectRenameMethod {
            token,
            index: 0,
            selected: false,
        })
        .unwrap();
        assert!(!app.rename.can_confirm_methods());
        assert!(
            app.dispatch(Command::ConfirmRenameMethods { token })
                .is_err()
        );
        assert!(app.dispatch(Command::ApplyRename).is_err());
        // Mutating public observations cannot change the private selection.
        app.rename
            .validation
            .as_mut()
            .unwrap()
            .related_methods
            .clear();
        assert!(!app.rename.can_confirm_methods());
        app.dispatch(Command::SelectRenameMethod {
            token,
            index: 0,
            selected: true,
        })
        .unwrap();
        app.dispatch(Command::ConfirmRenameMethods { token })
            .unwrap();
        assert!(app.rename.can_apply());
        // A compatible preview cannot survive a later edit of the related dirty buffer.
        let related = app.documents.iter().find(|d| d.id == child).unwrap();
        let version = related.version;
        let revised = format!("{}// edit after preview\n", related.text());
        app.dispatch(Command::Edit {
            id: child,
            expected_version: version,
            text: revised,
        })
        .unwrap();
        assert!(app.dispatch(Command::ApplyRename).is_err());
        app.dispatch(Command::Undo { id: child }).unwrap();
        propose(&mut app, main);
        let token = app.rename.context_token().unwrap();
        app.dispatch(Command::ConfirmRenameMethods { token })
            .unwrap();
        assert!(app.rename.can_apply());
        let proposal = app.rename.files.clone();
        for f in &proposal {
            assert_eq!(
                f.after.as_ref(),
                f.before.replace("rainfall()", "precipitation()")
            );
        }
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
        for (name, text) in &originals {
            assert_eq!(fs::read_to_string(root.join(name)).unwrap(), *text);
        }
        for f in &proposal {
            let id = app.documents.iter().find(|d| d.path == f.path).unwrap().id;
            app.dispatch(Command::Save { id }).unwrap();
            idle(&mut app);
            assert_eq!(fs::read_to_string(&f.path).unwrap(), f.after.as_ref());
            let name = f.path.strip_prefix(&root).unwrap().to_str().unwrap();
            *originals.get_mut(name).unwrap() = f.after.to_string();
        }
    }
    app.dispatch(Command::StopProviders).unwrap();
    idle(&mut app);
}
