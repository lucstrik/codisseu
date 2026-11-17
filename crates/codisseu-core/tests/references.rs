use codisseu_core::{AppService, Command, ProviderConfig, ProviderState};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn idle(app: &mut AppService) {
    let until = Instant::now() + Duration::from_secs(45);
    loop {
        app.poll();
        if !app.is_busy() {
            return;
        }
        assert!(
            Instant::now() < until,
            "{} {:?}",
            app.status,
            app.references.error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
#[test]
#[ignore = "real PHP/TS references application service; requires CODISSEU_PROVIDER_CONFIG and Composer"]
fn real_references_preserve_unsaved_php_ts_and_navigate_both_usages() {
    let configs: Vec<ProviderConfig> = serde_json::from_slice(
        &fs::read(
            std::env::var_os("CODISSEU_PROVIDER_CONFIG")
                .expect("CODISSEU_PROVIDER_CONFIG required"),
        )
        .unwrap(),
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"strict":true,"target":"ES2022"},"include":["*.ts"]}"#,
    )
    .unwrap();
    let php = root.join("main.php");
    fs::create_dir(root.join("src")).unwrap();
    let php_decl = root.join("src/Weather.php");
    fs::write(&php_decl,"<?php\nnamespace StormFixture;\nclass Weather { public function rainfall(): string { return 'rain'; } }\n").unwrap();
    let ts = root.join("weather.ts");
    let php_disk =
        "<?php\nuse StormFixture\\Weather;\n$weather = new Weather();\n$weather->rainfall();\n";
    let ts_disk = "export class Weather { rainfall(): string { return 'rain'; } }\nconst weather = new Weather();\nweather.rainfall();\n";
    fs::write(&php, php_disk).unwrap();
    fs::write(&ts, ts_disk).unwrap();
    fs::write(
        root.join("composer.json"),
        r#"{"name":"storm/references-fixture","autoload":{"psr-4":{"StormFixture\\":"src/"}}}"#,
    )
    .unwrap();
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
        .expect("Composer required");
    assert!(
        composer.status.success(),
        "{}",
        String::from_utf8_lossy(&composer.stderr)
    );
    let mut app = AppService::new(root.join("session.json"));
    app.dispatch(Command::OpenWorkspace(root)).unwrap();
    idle(&mut app);
    app.dispatch(Command::OpenFile(php_decl)).unwrap();
    idle(&mut app);
    let mut docs = Vec::new();
    for (path, disk, extra) in [
        (&php, php_disk, "echo '🌧 e\u{301}'; $weather->rainfall();\n"),
        (
            &ts,
            ts_disk,
            "console.log('🌧 e\u{301}'); weather.rainfall();\n",
        ),
    ] {
        app.dispatch(Command::OpenFile(path.clone())).unwrap();
        idle(&mut app);
        let id = app.active.unwrap();
        let dirty = format!("{disk}{extra}");
        app.dispatch(Command::Edit {
            id,
            expected_version: 1,
            text: dirty.clone(),
        })
        .unwrap();
        docs.push((id, path.clone(), dirty));
    }
    app.dispatch(Command::ConfigureProviders(configs)).unwrap();
    app.dispatch(Command::StartProviders).unwrap();
    idle(&mut app);
    assert_eq!(app.language.statuses.len(), 2);
    assert!(
        app.language
            .statuses
            .iter()
            .all(|s| s.state == ProviderState::Ready)
    );
    for (id, path, dirty) in &docs {
        app.dispatch(Command::Activate { id: *id }).unwrap();
        let caret = dirty[..dirty.find("rainfall").unwrap()].chars().count() + 2;
        app.dispatch(Command::References {
            id: *id,
            expected_version: 2,
            character_index: caret,
        })
        .unwrap();
        idle(&mut app);
        assert!(app.references.error.is_none(), "{:?}", app.references.error);
        assert_eq!(
            app.references.items.len(),
            2,
            "{path:?}: {:?}",
            app.references.items
        );
        assert!(app.references.items.iter().all(|r| r.target.path == *path));
        for index in 0..2 {
            app.dispatch(Command::ChooseReference {
                index,
                id: *id,
                expected_version: 2,
                character_index: caret,
            })
            .unwrap();
            idle(&mut app);
            assert!(app.references.is_open());
            let selected = app.navigation.selection.as_ref().unwrap();
            assert_eq!(
                dirty
                    .chars()
                    .skip(selected.range.start)
                    .take(selected.range.len())
                    .collect::<String>(),
                "rainfall"
            );
            assert_eq!(app.active_document().unwrap().text(), dirty);
        }
        app.dispatch(Command::NavigateBack {
            id: *id,
            expected_version: 2,
            character_index: caret,
        })
        .unwrap();
        assert_eq!(app.active_document().unwrap().text(), dirty);
        app.dispatch(Command::Undo { id: *id }).unwrap();
        assert_eq!(
            app.active_document().unwrap().text(),
            if path == &php { php_disk } else { ts_disk }
        );
        app.dispatch(Command::Redo { id: *id }).unwrap();
    }
    app.dispatch(Command::StopProviders).unwrap();
    idle(&mut app);
    assert_eq!(fs::read_to_string(php).unwrap(), php_disk);
    assert_eq!(fs::read_to_string(ts).unwrap(), ts_disk);
}
