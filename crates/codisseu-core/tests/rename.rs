use codisseu_core::{AppService, Command, Coverage, ProviderConfig, ProviderState};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn idle(s: &mut AppService) {
    let end = Instant::now() + Duration::from_secs(45);
    loop {
        s.poll();
        if !s.is_busy() {
            return;
        }
        assert!(Instant::now() < end, "{} {:?}", s.status, s.rename.error);
        thread::sleep(Duration::from_millis(5));
    }
}
#[test]
#[ignore = "real PHP-first/TS core rename; CODISSEU_PROVIDER_CONFIG and Composer"]
fn php_first_ts_dirty_open_closed_grouped_undo_no_save() {
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
    fs::create_dir(root.join("src")).unwrap();
    fs::write(
        root.join("composer.json"),
        r#"{"name":"storm/rename-fixture","autoload":{"psr-4":{"StormFixture\\":"src/"}}}"#,
    )
    .unwrap();
    fs::write(root.join("tsconfig.json"),r#"{"compilerOptions":{"strict":true,"target":"ES2022","module":"commonjs"},"include":["*.ts"]}"#).unwrap();
    let php_decl = root.join("src/Weather.php");
    let php_use = root.join("main.php");
    let php_other = root.join("other.php");
    let ts_decl = root.join("weather.ts");
    let ts_use = root.join("main.ts");
    let ts_other = root.join("other.ts");
    let php_decl_text = "<?php\nnamespace StormFixture;\nclass Weather { /* 🌧 e\u{301} */ public function rainfall(): string { return 'rain'; } }\n";
    let php_use_disk =
        "<?php\nuse StormFixture\\Weather;\n$weather = new Weather();\n$weather->rainfall();\n";
    let php_use_text = "<?php\nuse StormFixture\\Weather;\n$weather = new Weather();\n// unsaved extra line\necho '🌧 e\u{301}'; $weather->rainfall();\n$weather->rainfall();\n";
    let php_other_text =
        "<?php\nuse StormFixture\\Weather;\n$weather = new Weather();\n$weather->rainfall();\n";
    let ts_decl_text =
        "export class Weather { /* 🌧 e\u{301} */ rainfall(): string { return 'rain'; } }\n";
    let ts_use_disk = "import { Weather } from './weather';\nconst weather = new Weather();\nweather.rainfall();\n";
    let ts_use_text = "import { Weather } from './weather';\nconst weather = new Weather();\n// unsaved extra line\nconsole.log('🌧 e\u{301}'); weather.rainfall();\nweather.rainfall();\n";
    let ts_other_text =
        "import { Weather } from './weather';\nconst other = new Weather();\nother.rainfall();\n";
    for (path, text) in [
        (&php_decl, php_decl_text),
        (&php_use, php_use_disk),
        (&php_other, php_other_text),
        (&ts_decl, ts_decl_text),
        (&ts_use, ts_use_disk),
        (&ts_other, ts_other_text),
    ] {
        fs::write(path, text).unwrap();
    }
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
        .expect("Composer required for explicit test");
    assert!(
        composer.status.success(),
        "{}",
        String::from_utf8_lossy(&composer.stderr)
    );
    // Querying rename must preserve unrelated strings/comments with this spelling.
    let php_use_text = &format!("{php_use_text}// rainfall is a comment\necho 'rainfall';\n");
    let ts_use_text = &format!("{ts_use_text}// rainfall is a comment\nconsole.log('rainfall');\n");
    let php_decl_disk = php_decl_text;
    let ts_decl_disk = ts_decl_text;
    let php_decl_text =
        &php_decl_text.replace("class Weather", "// dirty declaration\nclass Weather");
    let ts_decl_text = &format!("// dirty declaration\n{ts_decl_text}");

    let mut app = AppService::new(root.join("session.json"));
    app.dispatch(Command::OpenWorkspace(root)).unwrap();
    idle(&mut app);
    for (path, text) in [
        (&php_decl, php_decl_text.as_str()),
        (&php_use, php_use_text.as_str()),
        (&ts_decl, ts_decl_text.as_str()),
        (&ts_use, ts_use_text.as_str()),
    ] {
        app.dispatch(Command::OpenFile(path.clone())).unwrap();
        idle(&mut app);
        app.dispatch(Command::Edit {
            id: app.active.unwrap(),
            expected_version: 1,
            text: text.into(),
        })
        .unwrap();
    }
    app.dispatch(Command::ConfigureProviders(configs)).unwrap();
    app.dispatch(Command::StartProviders).unwrap();
    idle(&mut app);
    assert!(
        app.language
            .statuses
            .iter()
            .all(|s| s.state == ProviderState::Ready)
    );
    for (decl, use_path, other, use_text) in [
        (&php_decl, &php_use, &php_other, php_use_text.as_str()),
        (&ts_decl, &ts_use, &ts_other, ts_use_text.as_str()),
    ] {
        let d = app.documents.iter().find(|d| &d.path == use_path).unwrap();
        let id = d.id;
        let version = d.version;
        let caret = use_text[..use_text.find("rainfall").unwrap()]
            .chars()
            .count()
            + 2;
        // Both real providers return a collision proposal: local AST must block it.
        let decl_doc = app.documents.iter().find(|d| &d.path == decl).unwrap();
        let decl_id = decl_doc.id;
        let decl_version = decl_doc.version;
        let before = decl_doc.text().to_owned();
        let mut colliding = before.clone();
        let insert = colliding.rfind('}').unwrap();
        colliding.insert_str(
            insert,
            if decl.extension().is_some_and(|e| e == "php") {
                " public function ForeCast(): string { return 'other'; } "
            } else {
                " forecast(): string { return 'other'; } "
            },
        );
        app.dispatch(Command::Edit {
            id: decl_id,
            expected_version: decl_version,
            text: colliding.clone(),
        })
        .unwrap();
        idle(&mut app);
        app.dispatch(Command::Activate { id }).unwrap();
        app.dispatch(Command::PrepareRename {
            id,
            expected_version: version,
            character_index: caret,
        })
        .unwrap();
        idle(&mut app);
        app.dispatch(Command::PreviewRename {
            new_name: "forecast".into(),
        })
        .unwrap();
        idle(&mut app);
        assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
        assert_eq!(
            app.rename.validation.as_ref().unwrap().coverage,
            Coverage::Rejected,
            "{:?}",
            app.rename.validation
        );
        let token = app.rename.context_token();
        assert!(app.dispatch(Command::ApplyRename).is_err());
        assert_eq!(app.rename.context_token(), token);
        assert_eq!(
            app.documents
                .iter()
                .find(|d| d.id == decl_id)
                .unwrap()
                .text(),
            colliding
        );
        assert!(!app.documents.iter().any(|d| &d.path == other));
        assert_eq!(
            fs::read_to_string(decl).unwrap(),
            if decl.extension().is_some_and(|e| e == "php") {
                php_decl_disk
            } else {
                ts_decl_disk
            }
        );
        app.dispatch(Command::Undo { id: decl_id }).unwrap();
        assert_eq!(
            app.documents
                .iter()
                .find(|d| d.id == decl_id)
                .unwrap()
                .text(),
            before
        );
        idle(&mut app);
        app.dispatch(Command::Activate { id }).unwrap();
        app.dispatch(Command::PrepareRename {
            id,
            expected_version: version,
            character_index: caret,
        })
        .unwrap();
        idle(&mut app);
        assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
        assert_eq!(app.rename.placeholder.as_deref(), Some("rainfall"));
        app.dispatch(Command::PreviewRename {
            new_name: "forecast".into(),
        })
        .unwrap();
        idle(&mut app);
        assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
        assert_eq!(app.rename.files.len(), 3);
        assert_eq!(
            app.rename.validation.as_ref().unwrap().coverage,
            Coverage::Verified,
            "{:?}",
            app.rename.validation
        );
        assert_eq!(
            app.rename
                .files
                .iter()
                .map(|f| f.edits.len())
                .sum::<usize>(),
            4
        );
        let snapshots = app.rename.files.clone();
        app.dispatch(Command::ApplyRename).unwrap();
        idle(&mut app);
        assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
        for f in &snapshots {
            let d = app.documents.iter().find(|d| d.path == f.path).unwrap();
            assert_eq!(d.text(), f.after.as_ref());
            assert!(d.is_dirty());
        }
        let d = app.documents.iter().find(|d| d.id == id).unwrap();
        assert!(d.text().contains("// rainfall is a comment"));
        assert!(d.text().contains("'rainfall'"));
        assert!(d.text().contains("🌧 e\u{301}"));
        app.dispatch(Command::Undo { id }).unwrap();
        for f in &snapshots {
            assert_eq!(
                app.documents
                    .iter()
                    .find(|d| d.path == f.path)
                    .unwrap()
                    .text(),
                f.before.as_ref()
            );
        }
        app.dispatch(Command::OpenFile(other.clone())).unwrap();
        idle(&mut app);
        app.dispatch(Command::Activate { id }).unwrap();
        app.dispatch(Command::Redo { id }).unwrap();
        for f in &snapshots {
            assert_eq!(
                app.documents
                    .iter()
                    .find(|d| d.path == f.path)
                    .unwrap()
                    .text(),
                f.after.as_ref()
            );
        }
        assert!(app.documents.iter().any(|d| &d.path == decl));
    }
    for (path, text) in [
        (&php_decl, php_decl_disk),
        (&php_use, php_use_disk),
        (&php_other, php_other_text),
        (&ts_decl, ts_decl_disk),
        (&ts_use, ts_use_disk),
        (&ts_other, ts_other_text),
    ] {
        assert_eq!(fs::read_to_string(path).unwrap(), text);
    }
    app.dispatch(Command::StopProviders).unwrap();
    idle(&mut app);
}
