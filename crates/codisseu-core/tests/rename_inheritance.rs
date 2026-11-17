use codisseu_core::{AppService, Command, ConflictKind, Coverage, ProviderConfig, ValidationScope};
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
fn propose(app: &mut AppService, id: u64, name: &str) {
    let d = app.documents.iter().find(|d| d.id == id).unwrap();
    let version = d.version;
    let caret = d.text()[..d.text().find("rainfall()").unwrap()]
        .chars()
        .count()
        + 2;
    app.dispatch(Command::Activate { id }).unwrap();
    app.dispatch(Command::PrepareRename {
        id,
        expected_version: version,
        character_index: caret,
    })
    .unwrap();
    idle(app);
    assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
    app.dispatch(Command::PreviewRename {
        new_name: name.into(),
    })
    .unwrap();
    idle(app);
    assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
}
#[test]
#[ignore = "real PHP-first/TS inherited calls and closed/dirty ancestors; CODISSEU_PROVIDER_CONFIG and Composer"]
fn inherited_calls_stay_in_lsp_proposal_and_closed_dirty_ancestor_conflicts_block_apply() {
    let configs: Vec<ProviderConfig> = serde_json::from_slice(
        &fs::read(std::env::var_os("CODISSEU_PROVIDER_CONFIG").expect("provider config")).unwrap(),
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir(root.join("src")).unwrap();
    let originals = [
        (
            "src/BaseWeather.php",
            "<?php\nnamespace StormFixture;\nclass BaseWeather { public function forecast(): string { return 'sun'; } }\n",
        ),
        (
            "src/Weather.php",
            "<?php\nnamespace StormFixture;\nclass Weather extends BaseWeather { public function rainfall(): string { return 'rainfall'; } }\n",
        ),
        (
            "src/WetWeather.php",
            "<?php\nnamespace StormFixture;\nclass WetWeather extends Weather {}\n",
        ),
        (
            "main.php",
            "<?php\nuse StormFixture\\Weather;\n$weather=new Weather();\necho '🌧 é'; $weather->rainfall();\n$message='rainfall'; // rainfall stays\n",
        ),
        (
            "other.php",
            "<?php\nuse StormFixture\\WetWeather;\n$weather=new WetWeather();\n$weather->rainfall();\n// rainfall stays\n",
        ),
        (
            "src/baseWeather.ts",
            "export class BaseWeather { forecast(): string { return 'sun'; } }\n",
        ),
        (
            "src/weather.ts",
            "import { BaseWeather } from './baseWeather';\nexport class Weather extends BaseWeather { rainfall(): string { return 'rainfall'; } }\n",
        ),
        (
            "src/wetWeather.ts",
            "import { Weather } from './weather';\nexport class WetWeather extends Weather {}\n",
        ),
        (
            "main.ts",
            "import { Weather } from './src/weather';\nconst weather=new Weather();\nconsole.log('🌧 é'); weather.rainfall();\nconst label='rainfall'; // rainfall stays\n",
        ),
        (
            "other.ts",
            "import { WetWeather } from './src/wetWeather';\nconst other=new WetWeather();\nother.rainfall();\n// rainfall stays\n",
        ),
    ];
    for (p, text) in originals {
        fs::write(root.join(p), text).unwrap();
    }
    fs::write(
        root.join("composer.json"),
        r#"{"name":"storm/inheritance-fixture","autoload":{"psr-4":{"StormFixture\\":"src/"}}}"#,
    )
    .unwrap();
    fs::write(root.join("tsconfig.json"),r#"{"compilerOptions":{"strict":true,"target":"ES2022","module":"commonjs"},"include":["*.ts","src/*.ts"]}"#).unwrap();
    let out = std::process::Command::new("composer")
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
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut app = AppService::new(root.join("session.json"));
    app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    idle(&mut app);
    let mut sources = vec![];
    for (decl, main) in [
        ("src/Weather.php", "main.php"),
        ("src/weather.ts", "main.ts"),
    ] {
        for path in [decl, main] {
            app.dispatch(Command::OpenFile(root.join(path))).unwrap();
            idle(&mut app);
            let d = app.active_document().unwrap();
            let id = d.id;
            let text = format!("{}// dirty {}\n", d.text(), path);
            app.dispatch(Command::Edit {
                id,
                expected_version: 1,
                text,
            })
            .unwrap();
        }
        sources.push(app.active.unwrap());
    }
    app.dispatch(Command::ConfigureProviders(configs)).unwrap();
    app.dispatch(Command::StartProviders).unwrap();
    idle(&mut app);
    for (index, (base, decl, other)) in [
        ("src/BaseWeather.php", "src/Weather.php", "other.php"),
        ("src/baseWeather.ts", "src/weather.ts", "other.ts"),
    ]
    .into_iter()
    .enumerate()
    {
        let id = sources[index];
        propose(&mut app, id, "forecast");
        let report = app.rename.validation.as_ref().unwrap();
        assert_eq!(report.coverage, Coverage::Rejected, "{report:?}");
        assert!(
            report
                .conflicts
                .iter()
                .any(|c| c.path == root.join(base) && c.kind == ConflictKind::InheritedMember)
        );
        assert!(!app.documents.iter().any(|d| d.path == root.join(base)));
        let token = app.rename.context_token();
        assert!(app.dispatch(Command::ApplyRename).is_err());
        assert_eq!(app.rename.context_token(), token);
        app.dispatch(Command::OpenFile(root.join(base))).unwrap();
        idle(&mut app);
        let base_id = app.active.unwrap();
        let dirty = app
            .active_document()
            .unwrap()
            .text()
            .replace("forecast", "stormy");
        app.dispatch(Command::Edit {
            id: base_id,
            expected_version: 1,
            text: dirty.clone(),
        })
        .unwrap();
        propose(&mut app, id, "stormy");
        let report = app.rename.validation.as_ref().unwrap();
        assert_eq!(report.coverage, Coverage::Rejected, "{report:?}");
        assert!(report.conflicts.iter().any(|c| c.path == root.join(base)));
        assert!(app.dispatch(Command::ApplyRename).is_err());
        assert_eq!(
            app.documents
                .iter()
                .find(|d| d.id == base_id)
                .unwrap()
                .text(),
            dirty
        );
        propose(&mut app, id, "precipitation");
        let report = app.rename.validation.as_ref().unwrap();
        assert_eq!(report.coverage, Coverage::Verified, "{report:?}");
        assert_eq!(report.scope, ValidationScope::KnownReceiverBindings);
        assert_eq!(app.rename.files.len(), 3, "{:?}", app.rename.files);
        assert_eq!(
            app.rename
                .files
                .iter()
                .map(|f| f.edits.len())
                .sum::<usize>(),
            3
        );
        assert!(
            app.rename.files.iter().any(|f| f.path == root.join(other)),
            "LSP omitted inherited call"
        );
        assert!(app.rename.files.iter().any(|f| f.path == root.join(decl)));
        let snapshot = app.rename.files.clone();
        for f in &snapshot {
            let expected = f.before.replace("rainfall()", "precipitation()");
            assert_eq!(
                f.after.as_ref(),
                expected,
                "LSP must preserve literal/comment spelling"
            );
        }
        app.dispatch(Command::ApplyRename).unwrap();
        idle(&mut app);
        assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
        for f in &snapshot {
            assert_eq!(
                app.documents
                    .iter()
                    .find(|d| d.path == f.path)
                    .unwrap()
                    .text(),
                f.after.as_ref()
            );
        }
        app.dispatch(Command::Undo { id }).unwrap();
        for f in &snapshot {
            assert_eq!(
                app.documents
                    .iter()
                    .find(|d| d.path == f.path)
                    .unwrap()
                    .text(),
                f.before.as_ref()
            );
        }
        app.dispatch(Command::Redo { id }).unwrap();
        for f in &snapshot {
            assert_eq!(
                app.documents
                    .iter()
                    .find(|d| d.path == f.path)
                    .unwrap()
                    .text(),
                f.after.as_ref()
            );
        }
        assert_eq!(
            app.documents
                .iter()
                .find(|d| d.id == base_id)
                .unwrap()
                .text(),
            dirty
        );
    }
    for (p, text) in originals {
        assert_eq!(
            fs::read_to_string(root.join(p)).unwrap(),
            text,
            "no implicit save: {p}"
        );
    }
    app.dispatch(Command::StopProviders).unwrap();
    idle(&mut app);
}
