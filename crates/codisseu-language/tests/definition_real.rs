//! Explicit provider integration: no simulated definitions and no disk writes by queries.
use codisseu_language::{
    LanguageCommand, LanguageService, ProviderConfig, ProviderState, utf16_byte_offset,
};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn wait(service: &mut LanguageService, condition: impl Fn(&LanguageService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        service.poll();
        if condition(service) {
            return;
        }
        assert!(service.last_error.is_none(), "{:?}", service.last_error);
        assert!(
            Instant::now() < deadline,
            "statuses {:?}, def error {:?}, analyzed {:?}, unversioned {:?}",
            service.statuses,
            service.definition_error,
            service.analyzed_versions,
            service.unversioned_paths
        );
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
#[ignore = "real Composer PHP + TypeScript definition; requires CODISSEU_PROVIDER_CONFIG"]
fn php_composer_and_typescript_cross_file_definitions_resolve_unsaved_unicode_buffers() {
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
        r#"{"name":"storm/definition-fixture","autoload":{"psr-4":{"StormFixture\\":"src/"}}}"#,
    )
    .unwrap();
    fs::write(root.join("tsconfig.json"),r#"{"compilerOptions":{"strict":true,"target":"ES2022","module":"commonjs"},"include":["*.ts"]}"#).unwrap();
    let php_source = root.join("main.php");
    let php_target = root.join("src/Rain.php");
    let ts_source = root.join("main.ts");
    let ts_target = root.join("rain.ts");
    let php_source_text =
        "<?php\nuse StormFixture\\Rain;\necho '🌧 e\u{301}'; $value = new Rain();\n";
    let php_target_text = "<?php\nnamespace StormFixture;\n// unsaved line before definition\nclass Rain { public function label(): string { return 'rain'; } }\n";
    let ts_source_text = "import { rain } from './rain';\nconsole.log('🌧 e\u{301}'); rain();\n";
    let ts_target_text = "/* 🌧 */ export function rain(): string { return 'rain'; }\n";
    let php_disk = "<?php\nuse StormFixture\\Rain;\nnew Rain();\n";
    let php_target_disk = "<?php\nnamespace StormFixture;\nclass Rain {}\n";
    let ts_disk = "import { rain } from './rain';\nrain();\n";
    let ts_target_disk = "export function rain(): string { return ''; }\n";
    for (path, text) in [
        (&php_source, php_disk),
        (&php_target, php_target_disk),
        (&ts_source, ts_disk),
        (&ts_target, ts_target_disk),
    ] {
        fs::write(path, text).unwrap();
    }
    // Controlled fixture only: generate Composer's real PSR-4 maps without
    // loading plugins, running scripts, installing dependencies or using network.
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
        .expect("Composer is required for this explicit Composer integration test");
    assert!(
        composer.status.success(),
        "{}",
        String::from_utf8_lossy(&composer.stderr)
    );
    let mut service = LanguageService::new();
    service
        .dispatch(LanguageCommand::Start {
            workspace: root,
            configs,
        })
        .unwrap();
    for (path, text) in [
        (&php_source, php_source_text),
        (&php_target, php_target_text),
        (&ts_source, ts_source_text),
        (&ts_target, ts_target_text),
    ] {
        service
            .dispatch(LanguageCommand::Sync {
                path: path.clone(),
                version: 2,
                text: text.into(),
            })
            .unwrap();
    }
    wait(&mut service, |s| {
        s.statuses.len() == 2
            && s.statuses.iter().all(|p| p.state == ProviderState::Ready)
            && !s.is_busy()
    });
    for (source, target, text, target_text, line, prefix, symbol) in [
        (
            &php_source,
            &php_target,
            php_source_text,
            php_target_text,
            2,
            "echo '🌧 e\u{301}'; $value = new ",
            "Rain",
        ),
        (
            &ts_source,
            &ts_target,
            ts_source_text,
            ts_target_text,
            1,
            "console.log('🌧 e\u{301}'); ",
            "rain",
        ),
    ] {
        service
            .dispatch(LanguageCommand::Definition {
                path: source.clone(),
                version: 2,
                line,
                character: prefix.encode_utf16().count() as u32 + 1,
            })
            .unwrap();
        wait(&mut service, |s| !s.definition_pending());
        assert!(
            service.definition_error.is_none(),
            "{:?}",
            service.definition_error
        );
        let result = service.definition.as_ref().unwrap();
        eprintln!("{source:?} -> {:?}", result.targets);
        assert_eq!(result.path, *source);
        assert_eq!(result.version, 2);
        assert_eq!(result.targets.len(), 1);
        let found = &result.targets[0];
        assert_eq!(found.path, *target);
        let start =
            utf16_byte_offset(target_text, found.start_line, found.start_character).unwrap();
        let expected_start = if symbol == "Rain" {
            "class Rain"
        } else {
            symbol
        };
        assert!(
            target_text[start..].starts_with(expected_start),
            "target offset points to symbol: {}",
            &target_text[start..]
        );
        let source_offset =
            utf16_byte_offset(text, line, prefix.encode_utf16().count() as u32).unwrap();
        assert!(text[source_offset..].starts_with(symbol));
    }
    // A second revision moves a PHP target while source version stays the same.
    service
        .dispatch(LanguageCommand::Sync {
            path: php_target.clone(),
            version: 3,
            text: format!("\n{php_target_text}"),
        })
        .unwrap();
    service
        .dispatch(LanguageCommand::Definition {
            path: php_source.clone(),
            version: 2,
            line: 2,
            character: "echo '🌧 e\u{301}'; $value = new ".encode_utf16().count() as u32 + 1,
        })
        .unwrap();
    wait(&mut service, |s| !s.definition_pending());
    assert!(
        service.definition_error.is_none(),
        "{:?}",
        service.definition_error
    );
    assert_eq!(
        service.definition.as_ref().unwrap().targets[0].start_line,
        4
    );
    // A second revision moves a target while the source version stays the same.
    service
        .dispatch(LanguageCommand::Sync {
            path: ts_target.clone(),
            version: 3,
            text: format!("\n{ts_target_text}"),
        })
        .unwrap();
    assert!(service.definition.is_none());
    service
        .dispatch(LanguageCommand::Definition {
            path: ts_source.clone(),
            version: 2,
            line: 1,
            character: "console.log('🌧 e\u{301}'); ".encode_utf16().count() as u32 + 1,
        })
        .unwrap();
    wait(&mut service, |s| !s.definition_pending());
    assert_eq!(
        service.definition.as_ref().unwrap().targets[0].start_line,
        1
    );
    service.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut service, |s| !s.is_busy());
    for (path, text) in [
        (&php_source, php_disk),
        (&php_target, php_target_disk),
        (&ts_source, ts_disk),
        (&ts_target, ts_target_disk),
    ] {
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            text,
            "queries must preserve disk bytes"
        );
    }
}
