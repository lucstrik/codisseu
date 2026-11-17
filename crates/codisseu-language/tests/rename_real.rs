//! Explicit real rename integration, with no query-induced saves.
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
            s.rename_error
        );
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
#[ignore = "real cross-file PHP/TS rename; requires CODISSEU_PROVIDER_CONFIG and Composer"]
fn php_first_and_typescript_rename_methods_semantically_in_dirty_unicode_snapshots() {
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
    let mut s = LanguageService::new();
    s.dispatch(LanguageCommand::Start {
        workspace: root,
        configs,
    })
    .unwrap();
    for (path, text) in [
        (&php_decl, php_decl_text.as_str()),
        (&php_use, php_use_text.as_str()),
        (&ts_decl, ts_decl_text.as_str()),
        (&ts_use, ts_use_text.as_str()),
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
            php_decl_text.as_str(),
            &php_use,
            php_use_text.as_str(),
            &php_other,
            php_other_text,
        ),
        (
            &ts_decl,
            ts_decl_text.as_str(),
            &ts_use,
            ts_use_text.as_str(),
            &ts_other,
            ts_other_text,
        ),
    ] {
        let offset = use_text.find("rainfall").unwrap() + 2;
        let prefix = &use_text[..offset];
        let line = prefix.bytes().filter(|b| *b == b'\n').count() as u32;
        let col = prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32;
        s.dispatch(LanguageCommand::PrepareRename {
            path: use_path.clone(),
            version: 2,
            line,
            character: col,
        })
        .unwrap();
        wait(&mut s, |s| !s.rename_pending());
        assert!(s.rename_error.is_none(), "{:?}", s.rename_error);
        eprintln!(
            "prepare {use_path:?}: {}",
            serde_json::to_string_pretty(s.prepare_rename.as_ref().unwrap()).unwrap()
        );
        let prepare = s.prepare_rename.as_ref().unwrap();
        assert!(!prepare.used_fallback);
        let r = prepare
            .range
            .as_ref()
            .expect("actual semantic prepare range");
        let start = utf16_byte_offset(use_text, r.start_line, r.start_character).unwrap();
        let end = utf16_byte_offset(use_text, r.end_line, r.end_character).unwrap();
        assert_eq!(&use_text[start..end], "rainfall");
        s.dispatch(LanguageCommand::Rename {
            path: use_path.clone(),
            version: 2,
            line,
            character: col,
            new_name: "forecast".into(),
        })
        .unwrap();
        wait(&mut s, |s| !s.rename_pending());
        assert!(s.rename_error.is_none(), "{:?}", s.rename_error);
        let result = s.rename.as_ref().unwrap();
        eprintln!(
            "rename {use_path:?}: {}",
            serde_json::to_string_pretty(result).unwrap()
        );
        assert_eq!(result.changes.len(), 3);
        let mut total = 0;
        for file in &result.changes {
            if file.path == *other {
                assert_eq!(
                    file.version,
                    if decl.extension().is_some_and(|e| e == "php") {
                        Some(0)
                    } else {
                        None
                    },
                    "closed target version must remain exactly as provider reports"
                );
            }

            let text = if file.path == *decl {
                decl_text
            } else if file.path == *use_path {
                use_text
            } else if file.path == *other {
                other_text
            } else {
                panic!("unexpected file {:?}", file.path)
            };
            let mut updated = text.to_owned();
            for edit in file.edits.iter().rev() {
                let a = utf16_byte_offset(text, edit.start_line, edit.start_character).unwrap();
                let b = utf16_byte_offset(text, edit.end_line, edit.end_character).unwrap();
                assert_eq!(&text[a..b], "rainfall");
                assert_eq!(edit.new_text, "forecast");
                updated.replace_range(a..b, &edit.new_text);
                total += 1;
            }
            if file.path == *use_path {
                assert_eq!(file.edits.len(), 2);
                assert!(updated.contains("// rainfall is a comment"));
                assert!(updated.contains("'rainfall'"));
                assert!(updated.contains("🌧 e\u{301}"));
            }
        }
        assert_eq!(total, 4, "declaration + three actual calls only");
        let updated = use_text.replace(
            "// unsaved extra line\n",
            "// unsaved extra line\n// another dirty revision\n",
        );
        s.dispatch(LanguageCommand::Sync {
            path: use_path.clone(),
            version: 3,
            text: updated.clone(),
        })
        .unwrap();
        assert!(s.rename.is_none() && s.prepare_rename.is_none());
        s.dispatch(LanguageCommand::Rename {
            path: use_path.clone(),
            version: 3,
            line: line + 1,
            character: col,
            new_name: "forecast".into(),
        })
        .unwrap();
        wait(&mut s, |s| !s.rename_pending());
        assert!(s.rename_error.is_none(), "{:?}", s.rename_error);
        let revision = s.rename.as_ref().unwrap();
        let file = revision
            .changes
            .iter()
            .find(|f| f.path == *use_path)
            .unwrap();
        assert_eq!(file.edits.len(), 2);
        for edit in &file.edits {
            let a = utf16_byte_offset(&updated, edit.start_line, edit.start_character).unwrap();
            let b = utf16_byte_offset(&updated, edit.end_line, edit.end_character).unwrap();
            assert_eq!(&updated[a..b], "rainfall");
            assert_eq!(edit.new_text, "forecast");
            assert!(edit.start_line > line);
        }

        // Characterize existing-name collisions without applying any proposal.
        let collision_decl = if decl.extension().is_some_and(|e| e == "php") {
            decl_text.replace(
                "return 'rain'; } }",
                "return 'rain'; } public function forecast(): string { return 'sun'; } }",
            )
        } else {
            decl_text.replace(
                "return 'rain'; } }",
                "return 'rain'; } forecast(): string { return 'sun'; } }",
            )
        };
        assert_ne!(
            collision_decl, decl_text,
            "collision fixture must actually introduce an existing method"
        );
        s.dispatch(LanguageCommand::Sync {
            path: decl.clone(),
            version: 3,
            text: collision_decl.clone(),
        })
        .unwrap();
        s.dispatch(LanguageCommand::PrepareRename {
            path: use_path.clone(),
            version: 3,
            line: line + 1,
            character: col,
        })
        .unwrap();
        wait(&mut s, |s| !s.rename_pending());
        eprintln!(
            "collision prepare {use_path:?}: result={} error={:?}",
            serde_json::to_string(&s.prepare_rename).unwrap(),
            s.rename_error
        );
        assert!(
            s.prepare_rename.is_some() && s.rename_error.is_none(),
            "recorded providers do not refuse the existing-name collision"
        );
        s.dispatch(LanguageCommand::Rename {
            path: use_path.clone(),
            version: 3,
            line: line + 1,
            character: col,
            new_name: "forecast".into(),
        })
        .unwrap();
        wait(&mut s, |s| !s.rename_pending());
        eprintln!(
            "collision rename {use_path:?}: result={} error={:?}",
            serde_json::to_string(&s.rename).unwrap(),
            s.rename_error
        );
        assert!(
            s.rename.is_some() && s.rename_error.is_none(),
            "recorded providers return a proposal despite the collision; this is a limitation, not approval to apply"
        );
        if let Some(proposal) = &s.rename {
            assert_eq!(proposal.changes.len(), 3);
            let count: usize = proposal.changes.iter().map(|f| f.edits.len()).sum();
            assert_eq!(count, 4);
            for file in &proposal.changes {
                let text = if file.path == *decl {
                    collision_decl.as_str()
                } else if file.path == *use_path {
                    updated.as_str()
                } else {
                    other_text
                };
                for edit in &file.edits {
                    let a = utf16_byte_offset(text, edit.start_line, edit.start_character).unwrap();
                    let b = utf16_byte_offset(text, edit.end_line, edit.end_character).unwrap();
                    assert_eq!(&text[a..b], "rainfall");
                }
            }
        }
    }
    for (path, text) in [
        (&php_decl, php_decl_disk),
        (&php_use, php_use_disk),
        (&php_other, php_other_text),
        (&ts_decl, ts_decl_disk),
        (&ts_use, ts_use_disk),
        (&ts_other, ts_other_text),
    ] {
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            text,
            "rename query must not write"
        );
    }
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, |s| !s.is_busy());
}
