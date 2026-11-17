use codisseu_language::{
    Language, LanguageCommand, LanguageService, ProviderConfig, ProviderState, utf16_byte_offset,
};
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
fn until(service: &mut LanguageService, check: impl Fn(&LanguageService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        service.poll();
        if check(service) {
            return;
        }
        assert!(
            service.last_error.is_none(),
            "Provider error: {:?}; status {:?}",
            service.last_error,
            service.statuses
        );
        assert!(
            Instant::now() < deadline,
            "Timed out: {:?}; diagnostics {:?}",
            service.statuses,
            service.diagnostics
        );
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn missing_provider_is_a_visible_failure_not_ready() {
    let dir = tempfile::tempdir().unwrap();
    let mut service = LanguageService::new();
    service
        .dispatch(LanguageCommand::Start {
            workspace: dir.path().into(),
            configs: vec![ProviderConfig {
                language: Language::Php,
                program: dir.path().join("missing-provider"),
                args: vec![],
            }],
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(service.last_error.is_some());
    assert_eq!(service.statuses[0].state, ProviderState::Failed);
}
#[test]
#[ignore = "explicit real PHP/TypeScript integration; requires CODISSEU_PROVIDER_CONFIG"]
fn real_php_and_typescript_diagnostics_and_clear_with_unicode() {
    let config_path = PathBuf::from(
        std::env::var_os("CODISSEU_PROVIDER_CONFIG")
            .expect("set CODISSEU_PROVIDER_CONFIG; this test never silently skips"),
    );
    let configs: Vec<ProviderConfig> =
        serde_json::from_slice(&fs::read(config_path).unwrap()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let php = root.join("ação script.php");
    let ts = root.join("ação script.ts");
    let php_bad = "<?php echo '🌧'; $broken = ;\n";
    let ts_bad = "export const emoji = '🌧'; export const count: number = 'wrong';\n";
    fs::write(&php, php_bad).unwrap();
    fs::write(&ts, ts_bad).unwrap();
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"strict":true,"target":"ES2022","noEmit":true},"include":["*.ts"]}"#,
    )
    .unwrap();
    let mut service = LanguageService::new();
    service
        .dispatch(LanguageCommand::Start {
            workspace: root,
            configs,
        })
        .unwrap();
    service
        .dispatch(LanguageCommand::Sync {
            path: php.clone(),
            version: 1,
            text: php_bad.into(),
        })
        .unwrap();
    service
        .dispatch(LanguageCommand::Sync {
            path: ts.clone(),
            version: 1,
            text: ts_bad.into(),
        })
        .unwrap();
    until(&mut service, |s| {
        s.statuses.len() == 2
            && s.statuses.iter().all(|s| s.state == ProviderState::Ready)
            && s.diagnostics
                .iter()
                .any(|d| d.path == php && d.severity == 1)
            && s.diagnostics
                .iter()
                .any(|d| d.path == ts && d.severity == 1)
    });
    for diagnostic in &service.diagnostics {
        eprintln!(
            "{:?} v{:?} {}:{} {}",
            diagnostic.language,
            diagnostic.version,
            diagnostic.start_line,
            diagnostic.start_character,
            diagnostic.message
        );
    }
    let diagnostic = service
        .diagnostics
        .iter()
        .find(|d| d.path == ts && d.severity == 1)
        .unwrap();
    let byte =
        utf16_byte_offset(ts_bad, diagnostic.start_line, diagnostic.start_character).unwrap();
    assert_eq!(
        &ts_bad[byte..byte + 5],
        "count",
        "UTF-16 range after emoji points to actual symbol"
    );
    assert_eq!(service.analyzed_versions.get(&ts), Some(&1));
    assert_eq!(service.analyzed_versions.get(&php), Some(&1));
    assert!(service.unversioned_paths.contains(&ts));
    assert!(!service.unversioned_paths.contains(&php));
    let php_good = "<?php echo '🌧';\n";
    let ts_good = "export const emoji = '🌧'; export const count: number = 42;\n";
    service
        .dispatch(LanguageCommand::Sync {
            path: php.clone(),
            version: 2,
            text: php_good.into(),
        })
        .unwrap();
    service
        .dispatch(LanguageCommand::Sync {
            path: ts.clone(),
            version: 2,
            text: ts_good.into(),
        })
        .unwrap();
    assert!(service.diagnostics.is_empty());
    assert!(service.analyzed_versions.is_empty());
    assert!(service.unversioned_paths.is_empty());
    assert!(
        service
            .dispatch(LanguageCommand::Sync {
                path: ts.clone(),
                version: 1,
                text: ts_bad.into()
            })
            .is_err()
    );
    until(&mut service, |s| {
        s.analyzed_versions.get(&php) == Some(&2)
            && s.analyzed_versions.get(&ts) == Some(&2)
            && s.diagnostics.iter().all(|d| d.severity > 2)
    });
    assert!(
        service.unversioned_paths.contains(&ts),
        "empty TS publications also lack a server version"
    );
    assert!(!service.unversioned_paths.contains(&php));
    assert_eq!(
        fs::read_to_string(&php).unwrap(),
        php_bad,
        "didChange must analyze unsaved text"
    );
    assert_eq!(
        fs::read_to_string(&ts).unwrap(),
        ts_bad,
        "didChange must analyze unsaved text"
    );
    service.dispatch(LanguageCommand::Stop).unwrap();
    assert!(service.unversioned_paths.is_empty());
    until(&mut service, |s| {
        !s.is_busy()
            && s.statuses.len() == 2
            && s.statuses.iter().all(|s| s.state == ProviderState::Stopped)
    });
}

#[test]
fn asynchronous_sync_failure_keeps_path_and_version_for_bounded_retry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not-created.ts");
    let mut service = LanguageService::new();
    service
        .dispatch(LanguageCommand::Sync {
            path: path.clone(),
            version: 7,
            text: "export const answer = 42;".into(),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    let failure = service
        .sync_failures
        .remove(&path)
        .expect("queued is not synchronized");
    assert_eq!(failure.version, 7);
    assert!(!failure.message.is_empty());
    assert!(!service.analyzed_versions.contains_key(&path));
    service
        .dispatch(LanguageCommand::Sync {
            path: path.clone(),
            version: 7,
            text: "export const answer = 42;".into(),
        })
        .expect("same-version retry remains allowed");
    service.dispatch(LanguageCommand::Close { path }).unwrap();
}

#[test]
fn invalid_workspace_terminates_starting_state() {
    let dir = tempfile::tempdir().unwrap();
    let mut service = LanguageService::new();
    service
        .dispatch(LanguageCommand::Start {
            workspace: dir.path().join("missing"),
            configs: vec![ProviderConfig {
                language: Language::Php,
                program: "php".into(),
                args: vec![],
            }],
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(service.last_error.is_some());
    assert_eq!(service.statuses[0].state, ProviderState::Failed);
}

#[test]
#[ignore = "explicit real PHP/TypeScript hover; requires CODISSEU_PROVIDER_CONFIG"]
fn real_hover_php_typescript_unsaved_unicode_and_empty() {
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
        r#"{"compilerOptions":{"strict":true},"include":["*.ts"]}"#,
    )
    .unwrap();
    let php = root.join("hover.php");
    let ts = root.join("hover.ts");
    let php_text =
        "<?php\nfunction stormValue(): string { return 'rain'; }\necho '🌧'; stormValue();\n";
    let ts_text = "export const emoji = '🌧'; export const stormValue: number = 42;\nstormValue;\n";
    fs::write(&php, "<?php\n").unwrap();
    fs::write(&ts, "export {};\n").unwrap();
    let mut service = LanguageService::new();
    service
        .dispatch(LanguageCommand::Start {
            workspace: root,
            configs,
        })
        .unwrap();
    for (path, text) in [(&php, php_text), (&ts, ts_text)] {
        service
            .dispatch(LanguageCommand::Sync {
                path: path.clone(),
                version: 1,
                text: text.into(),
            })
            .unwrap();
    }
    until(&mut service, |s| {
        s.statuses.len() == 2
            && s.statuses.iter().all(|s| s.state == ProviderState::Ready)
            && !s.is_busy()
    });
    for (path, line, prefix, expected) in [
        (&php, 2, "echo '🌧'; ", "stormValue"),
        (&ts, 0, "export const emoji = '🌧'; export const ", "number"),
    ] {
        service
            .dispatch(LanguageCommand::Hover {
                path: path.clone(),
                version: 1,
                line,
                character: prefix.encode_utf16().count() as u32 + 2,
            })
            .unwrap();
        assert!(service.hover_pending());
        until(&mut service, |s| !s.hover_pending());
        assert!(service.hover_error.is_none(), "{:?}", service.hover_error);
        let result = service.hover.as_ref().expect("hover reply");
        eprintln!("{:?}: {}", path, result.text);
        assert!(result.text.contains(expected));
        assert_eq!(result.version, 1);
    }
    service
        .dispatch(LanguageCommand::Hover {
            path: ts.clone(),
            version: 1,
            line: 1,
            character: 2,
        })
        .unwrap();
    service
        .dispatch(LanguageCommand::Sync {
            path: ts.clone(),
            version: 2,
            text: "export const changed: boolean = true;\n".into(),
        })
        .unwrap();
    assert!(!service.hover_pending());
    until(&mut service, |s| !s.is_busy());
    assert!(service.hover.is_none());
    assert!(
        service
            .dispatch(LanguageCommand::Hover {
                path: ts.clone(),
                version: 1,
                line: 1,
                character: 2
            })
            .is_err()
    );
    service
        .dispatch(LanguageCommand::Hover {
            path: ts.clone(),
            version: 2,
            line: 0,
            character: 18,
        })
        .unwrap();
    until(&mut service, |s| !s.hover_pending());
    assert!(service.hover.as_ref().unwrap().text.contains("boolean"));
    service
        .dispatch(LanguageCommand::Hover {
            path: ts.clone(),
            version: 2,
            line: 0,
            character: 0,
        })
        .unwrap();
    until(&mut service, |s| !s.hover_pending());
    assert_eq!(service.hover.as_ref().unwrap().text, "");
    service.dispatch(LanguageCommand::CancelHover).unwrap();
    assert!(service.hover.is_none());
    service.dispatch(LanguageCommand::Stop).unwrap();
    until(&mut service, |s| !s.is_busy());
    assert_eq!(fs::read_to_string(ts).unwrap(), "export {};\n");
    assert_eq!(fs::read_to_string(php).unwrap(), "<?php\n");
}

#[test]
#[ignore = "explicit real TypeScript unnecessary diagnostics; requires CODISSEU_PROVIDER_CONFIG"]
fn real_typescript_unnecessary_tag_unsaved_use_clears_decoration() {
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
    fs::write(root.join("tsconfig.json"), r#"{"compilerOptions":{"strict":true,"target":"ES2022","noEmit":true,"noUnusedLocals":true},"include":["*.ts"]}"#).unwrap();
    let path = root.join("unused.ts");
    let text = "export {};\nconst emoji = '🌧'; const unused = 42;\nconsole.log(emoji);\n";
    fs::write(&path, text).unwrap();
    let mut service = LanguageService::new();
    service
        .dispatch(LanguageCommand::Start {
            workspace: root,
            configs: configs
                .into_iter()
                .filter(|c| c.language == Language::TypeScript)
                .collect(),
        })
        .unwrap();
    service
        .dispatch(LanguageCommand::Sync {
            path: path.clone(),
            version: 1,
            text: text.into(),
        })
        .unwrap();
    until(&mut service, |s| {
        s.analyzed_versions.get(&path) == Some(&1)
            && s.diagnostics
                .iter()
                .any(|d| d.path == path && d.unnecessary)
    });
    let d = service
        .diagnostics
        .iter()
        .find(|d| d.path == path && d.unnecessary)
        .unwrap();
    let start = utf16_byte_offset(text, d.start_line, d.start_character).unwrap();
    let end = utf16_byte_offset(text, d.end_line, d.end_character).unwrap();
    assert_eq!(&text[start..end], "unused");
    eprintln!(
        "real TS unnecessary diagnostic: {}",
        serde_json::to_string(d).unwrap()
    );
    let used = text.replace("console.log(emoji);", "console.log(emoji, unused);");
    service
        .dispatch(LanguageCommand::Sync {
            path: path.clone(),
            version: 2,
            text: used,
        })
        .unwrap();
    assert!(service.diagnostics.is_empty());
    until(&mut service, |s| {
        s.analyzed_versions.get(&path) == Some(&2)
            && !s
                .diagnostics
                .iter()
                .any(|d| d.path == path && d.unnecessary)
    });
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        text,
        "using symbol stayed unsaved"
    );
    service.dispatch(LanguageCommand::Stop).unwrap();
    until(&mut service, |s| !s.is_busy());
}
