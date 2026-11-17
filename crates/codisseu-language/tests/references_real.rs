//! Explicit real references integration, with no query-induced saves.
use codisseu_language::{
    LanguageCommand, LanguageService, ProviderConfig, ProviderState, utf16_byte_offset,
};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn wait(s: &mut LanguageService, check: impl Fn(&LanguageService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        s.poll();
        if check(s) {
            return;
        }
        assert!(s.last_error.is_none(), "{:?}", s.last_error);
        assert!(
            Instant::now() < deadline,
            "{:?}, refs {:?}",
            s.statuses,
            s.references_error
        );
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
#[ignore = "real cross-file PHP/TS references; requires CODISSEU_PROVIDER_CONFIG and Composer"]
fn php_composer_and_typescript_references_track_cross_file_unsaved_unicode() {
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
        r#"{"name":"storm/references-fixture","autoload":{"psr-4":{"StormFixture\\":"src/"}}}"#,
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
    let mut s = LanguageService::new();
    s.dispatch(LanguageCommand::Start {
        workspace: root,
        configs,
    })
    .unwrap();
    for (path, text) in [
        (&php_decl, php_decl_text),
        (&php_use, php_use_text),
        (&php_other, php_other_text),
        (&ts_decl, ts_decl_text),
        (&ts_use, ts_use_text),
        (&ts_other, ts_other_text),
    ] {
        s.dispatch(LanguageCommand::Sync {
            path: path.clone(),
            version: 2,
            text: text.into(),
        })
        .unwrap();
    }
    wait(&mut s, |s| {
        s.statuses.len() == 2
            && s.statuses.iter().all(|p| p.state == ProviderState::Ready)
            && !s.is_busy()
    });
    for (decl, decl_text, use_path, use_text, other, other_text) in [
        (
            &php_decl,
            php_decl_text,
            &php_use,
            php_use_text,
            &php_other,
            php_other_text,
        ),
        (
            &ts_decl,
            ts_decl_text,
            &ts_use,
            ts_use_text,
            &ts_other,
            ts_other_text,
        ),
    ] {
        let offset = decl_text.find("rainfall").unwrap() + 2;
        let prefix = &decl_text[..offset];
        let line = prefix.bytes().filter(|b| *b == b'\n').count() as u32;
        let col = prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32;
        s.dispatch(LanguageCommand::References {
            path: decl.clone(),
            version: 2,
            line,
            character: col,
            include_declaration: false,
        })
        .unwrap();
        wait(&mut s, |s| !s.references_pending());
        assert!(s.references_error.is_none(), "{:?}", s.references_error);
        let result = s.references.as_ref().unwrap();
        eprintln!(
            "{decl:?}: {}",
            serde_json::to_string_pretty(result).unwrap()
        );
        assert!(!result.truncated);
        assert!(
            result.targets.iter().any(|t| t.path == *use_path),
            "expected primary use"
        );
        assert!(
            result.targets.iter().any(|t| t.path == *other),
            "expected cross-file second use"
        );
        let uses: Vec<_> = result
            .targets
            .iter()
            .filter(|t| t.path == *use_path)
            .collect();
        assert_eq!(uses.len(), 2, "unsaved additional call must be found");
        for target in &result.targets {
            let text = if target.path == *decl {
                decl_text
            } else if target.path == *use_path {
                use_text
            } else if target.path == *other {
                other_text
            } else {
                panic!("unexpected file {:?}", target.path)
            };
            let start = utf16_byte_offset(text, target.start_line, target.start_character).unwrap();
            let end = utf16_byte_offset(text, target.end_line, target.end_character).unwrap();
            assert_eq!(
                &text[start..end],
                "rainfall",
                "range must match actual buffer after Unicode/added line"
            );
        }
        assert!(
            !result.targets.iter().any(|t| t.path == *decl),
            "includeDeclaration false excludes declaration"
        );
        let usage_prefix = &use_text[..use_text.find("rainfall").unwrap() + 2];
        let usage_line = usage_prefix.bytes().filter(|b| *b == b'\n').count() as u32;
        let usage_col = usage_prefix
            .rsplit('\n')
            .next()
            .unwrap()
            .encode_utf16()
            .count() as u32;
        s.dispatch(LanguageCommand::References {
            path: use_path.clone(),
            version: 2,
            line: usage_line,
            character: usage_col,
            include_declaration: false,
        })
        .unwrap();
        wait(&mut s, |s| !s.references_pending());
        assert!(s.references_error.is_none(), "{:?}", s.references_error);
        eprintln!(
            "usage-query {use_path:?}: {}",
            serde_json::to_string_pretty(s.references.as_ref().unwrap()).unwrap()
        );
        assert!(
            !s.references
                .as_ref()
                .unwrap()
                .targets
                .iter()
                .any(|t| t.path == *decl),
            "usage query includeDeclaration false excludes declaration"
        );
        s.dispatch(LanguageCommand::References {
            path: decl.clone(),
            version: 2,
            line,
            character: col,
            include_declaration: true,
        })
        .unwrap();
        wait(&mut s, |s| !s.references_pending());
        assert!(s.references_error.is_none(), "{:?}", s.references_error);
        assert!(
            s.references
                .as_ref()
                .unwrap()
                .targets
                .iter()
                .any(|t| t.path == *decl),
            "includeDeclaration true must add declaration"
        );
        let updated = use_text.replace(
            "// unsaved extra line\n",
            "// unsaved extra line\n// moved again\n",
        );
        s.dispatch(LanguageCommand::Sync {
            path: use_path.clone(),
            version: 3,
            text: updated.clone(),
        })
        .unwrap();
        assert!(s.references.is_none());
        s.dispatch(LanguageCommand::References {
            path: decl.clone(),
            version: 2,
            line,
            character: col,
            include_declaration: false,
        })
        .unwrap();
        wait(&mut s, |s| !s.references_pending());
        assert!(s.references_error.is_none(), "{:?}", s.references_error);
        let result = s.references.as_ref().unwrap();
        let moved: Vec<_> = result
            .targets
            .iter()
            .filter(|t| t.path == *use_path)
            .collect();
        assert_eq!(
            moved.len(),
            2,
            "both unsaved usages remain after the second update"
        );
        for target in moved {
            let start =
                utf16_byte_offset(&updated, target.start_line, target.start_character).unwrap();
            assert!(updated[start..].starts_with("rainfall"));
        }
    }
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, |s| !s.is_busy());
    for (path, text) in [
        (&php_decl, php_decl_text),
        (&php_other, php_other_text),
        (&ts_decl, ts_decl_text),
        (&ts_other, ts_other_text),
    ] {
        assert_eq!(fs::read_to_string(path).unwrap(), text);
    }
    assert_eq!(fs::read_to_string(php_use).unwrap(), php_use_disk);
    assert_eq!(fs::read_to_string(ts_use).unwrap(), ts_use_disk);
}
