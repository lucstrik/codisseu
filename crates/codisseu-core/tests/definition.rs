//! AppService integration against the configured real PHP/TS processes.
use codisseu_core::{AppService, Command, ProviderState};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

fn wait(app: &mut AppService, condition: impl Fn(&AppService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        app.poll();
        assert!(
            app.last_error.is_none(),
            "{:?} / {:?}",
            app.last_error,
            app.navigation.error
        );
        assert!(
            app.language.last_error.is_none(),
            "{:?}",
            app.language.last_error
        );
        if condition(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{} / {:?}",
            app.status,
            app.language.statuses
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "real cross-file PHP/TS definitions; requires CODISSEU_PROVIDER_CONFIG and Composer"]
fn real_definition_reuses_dirty_targets_and_restores_source_caret_without_saving() {
    let config = std::env::var_os("CODISSEU_PROVIDER_CONFIG")
        .expect("explicit provider configuration required");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(
        root.join("composer.json"),
        r#"{"name":"storm/navigation-fixture","autoload":{"psr-4":{"StormFixture\\":"src/"}}}"#,
    )
    .unwrap();
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"strict":true,"target":"ES2022"},"include":["*.ts"]}"#,
    )
    .unwrap();
    let cases = [
        (
            "main.php",
            "src/Rain.php",
            "<?php\nuse StormFixture\\Rain;\nnew Rain();\n",
            "<?php\nnamespace StormFixture;\nclass Rain {}\n",
            "<?php\nuse StormFixture\\Rain;\necho '🌧 e\u{301}'; new Rain();\n// dirty source",
            "<?php\nnamespace StormFixture;\n// dirty target\nclass Rain {}\n",
            "Rain",
            "class Rain",
        ),
        (
            "main.ts",
            "rain.ts",
            "import { rain } from './rain';\nrain();\n",
            "export function rain() { return 0; }\n",
            "import { rain } from './rain';\nconsole.log('🌧 e\u{301}'); rain();\n// dirty source",
            "/* 🌧 e\u{301} */ export function rain() { return 42; }\n// dirty target",
            "rain",
            "rain",
        ),
    ];
    for (source, target, source_disk, target_disk, ..) in cases {
        fs::write(root.join(source), source_disk).unwrap();
        fs::write(root.join(target), target_disk).unwrap();
    }
    let composer = std::process::Command::new("composer")
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
        composer.status.success(),
        "{}",
        String::from_utf8_lossy(&composer.stderr)
    );
    let mut app = AppService::new(root.join("session.json"));
    app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    wait(&mut app, |a| !a.is_busy());
    for (source, target, _, _, source_text, target_text, ..) in cases {
        for (name, text) in [(source, source_text), (target, target_text)] {
            app.dispatch(Command::OpenFile(root.join(name))).unwrap();
            wait(&mut app, |a| !a.is_busy());
            let id = app.active.unwrap();
            app.dispatch(Command::Edit {
                id,
                expected_version: 1,
                text: text.into(),
            })
            .unwrap();
        }
    }
    app.dispatch(Command::LoadProviderConfig(config.into()))
        .unwrap();
    wait(&mut app, |a| !a.is_busy());
    app.dispatch(Command::StartProviders).unwrap();
    wait(&mut app, |a| {
        a.language.statuses.len() == 2
            && a.language
                .statuses
                .iter()
                .all(|s| s.state == ProviderState::Ready)
            && !a.is_busy()
    });
    for (source, target, source_disk, target_disk, source_text, target_text, symbol, expected) in
        cases
    {
        let source_id = app
            .documents
            .iter()
            .find(|d| d.path == root.join(source))
            .unwrap()
            .id;
        let target_id = app
            .documents
            .iter()
            .find(|d| d.path == root.join(target))
            .unwrap()
            .id;
        app.dispatch(Command::Activate { id: source_id }).unwrap();
        let byte = source_text.rfind(&format!("{symbol}();")).unwrap();
        let index = source_text[..byte].chars().count() + 1;
        app.dispatch(Command::Definition {
            id: source_id,
            expected_version: 2,
            character_index: index,
        })
        .unwrap();
        wait(&mut app, |a| !a.is_busy());
        assert_eq!(
            app.active,
            Some(target_id),
            "{} / {:?}",
            app.status,
            app.language.definition_error
        );
        let selection = app.navigation.selection.as_ref().unwrap();
        let byte = target_text
            .char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(target_text.len()))
            .nth(selection.range.start)
            .unwrap();
        assert!(
            target_text[byte..].starts_with(expected),
            "{}",
            &target_text[byte..]
        );
        assert_eq!(app.active_document().unwrap().text(), target_text);
        app.dispatch(Command::NavigateBack {
            id: target_id,
            expected_version: 2,
            character_index: selection.range.start,
        })
        .unwrap();
        assert_eq!(app.active, Some(source_id));
        assert_eq!(app.active_document().unwrap().text(), source_text);
        assert_eq!(
            app.navigation.selection.as_ref().unwrap().range,
            index..index
        );
        app.dispatch(Command::NavigateForward {
            id: source_id,
            expected_version: 2,
            character_index: index,
        })
        .unwrap();
        assert_eq!(app.active, Some(target_id));
        assert_eq!(app.documents.len(), 4);
        assert_eq!(fs::read_to_string(root.join(source)).unwrap(), source_disk);
        assert_eq!(fs::read_to_string(root.join(target)).unwrap(), target_disk);
    }
    assert!(app.documents.iter().all(|d| d.is_dirty() && d.version == 2));
    app.dispatch(Command::StopProviders).unwrap();
    wait(&mut app, |a| !a.is_busy());
}
