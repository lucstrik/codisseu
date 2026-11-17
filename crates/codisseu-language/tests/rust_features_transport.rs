use codisseu_language::{
    Language, LanguageCommand, LanguageService, ProviderConfig, ProviderState, RenameRange,
};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
fn wait(s: &mut LanguageService, check: impl Fn(&LanguageService) -> bool) {
    let end = Instant::now() + Duration::from_secs(8);
    loop {
        s.poll();
        if check(s) {
            return;
        }
        assert!(
            Instant::now() < end,
            "{:?} / {:?} / {:?}",
            s.last_error,
            s.code_actions_error,
            s.inlay_hints_error
        );
        thread::sleep(Duration::from_millis(3));
    }
}
fn range() -> RenameRange {
    RenameRange {
        start_line: 0,
        start_character: 0,
        end_line: 1,
        end_character: 15,
    }
}
fn setup(mode: &str) -> (tempfile::TempDir, LanguageService, PathBuf, PathBuf) {
    setup_language(mode, Language::Rust)
}
fn setup_language(
    mode: &str,
    language: Language,
) -> (tempfile::TempDir, LanguageService, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let path = root.join(match language {
        Language::Rust => "main.rs",
        Language::Python => "main.py",
        Language::Php => "main.php",
        Language::TypeScript => "main.ts",
    });
    let text = "// 😀\r\nlet number = 42;\r\n";
    fs::write(&path, text).unwrap();
    let outside = temp.path().join("outside.rs");
    fs::write(&outside, "outside").unwrap();
    let log = root.join("wire.jsonl");
    let mut s = LanguageService::new();
    s.dispatch(LanguageCommand::Start {
        workspace: root,
        configs: vec![ProviderConfig {
            language,
            program: "python3".into(),
            args: vec![
                format!(
                    "{}/tests/fixtures/rust_features_server.py",
                    env!("CARGO_MANIFEST_DIR")
                ),
                mode.into(),
                log.display().to_string(),
                format!("file://{}", outside.display()),
            ],
        }],
    })
    .unwrap();
    wait(&mut s, |s| {
        s.statuses.iter().any(|p| p.state == ProviderState::Ready)
    });
    s.dispatch(LanguageCommand::Sync {
        path: path.clone(),
        version: 7,
        text: text.into(),
    })
    .unwrap();
    wait(&mut s, |s| {
        !s.is_busy() && s.analyzed_versions.get(&path) == Some(&7)
    });
    (temp, s, path, log)
}
fn actions(s: &mut LanguageService, path: &Path) {
    s.dispatch(LanguageCommand::CodeActions {
        path: path.into(),
        version: 7,
        range: range(),
    })
    .unwrap();
    wait(s, |s| !s.code_actions_pending());
}
fn hints(s: &mut LanguageService, path: &Path) {
    s.dispatch(LanguageCommand::InlayHints {
        path: path.into(),
        version: 7,
        range: range(),
    })
    .unwrap();
    wait(s, |s| !s.inlay_hints_pending());
}
fn stop(s: &mut LanguageService) {
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(s, |s| !s.is_busy());
}
#[test]
fn actions_resolve_preserves_opaque_data_and_rejects_hostile_edits_without_writes() {
    for mode in [
        "safe",
        "command",
        "disabled",
        "escape",
        "resource",
        "overlap",
        "surrogate",
        "wrong-version",
        "resolve-command",
        "identity",
        "unsupported",
        "null",
        "crash",
    ] {
        let (_temp, mut s, path, log) = setup(mode);
        let before = fs::read(&path).unwrap();
        actions(&mut s, &path);
        if matches!(mode, "unsupported" | "crash") {
            assert!(s.code_actions_error.is_some(), "{mode}");
        } else if mode == "null" {
            assert!(s.code_actions.as_ref().unwrap().items.is_empty());
        } else {
            let item = s.code_actions.as_ref().unwrap().items[0].clone();
            if matches!(mode, "command" | "disabled") {
                assert!(item.disabled_reason.is_some());
            } else {
                s.dispatch(LanguageCommand::ResolveCodeAction { token: item.token })
                    .unwrap();
                wait(&mut s, |s| !s.code_actions_pending());
                if mode == "safe" {
                    let r = s.code_action_edit.as_ref().unwrap();
                    assert_eq!(r.token, item.token);
                    assert_eq!(r.changes.len(), 1);
                    assert!(r.changes[0].edits[0].new_text.contains("use std"));
                } else {
                    assert!(s.code_actions_error.is_some(), "{mode}");
                    assert!(s.code_action_edit.is_none());
                }
            }
        }
        assert_eq!(fs::read(&path).unwrap(), before);
        stop(&mut s);
        let messages: Vec<serde_json::Value> = fs::read_to_string(log)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let init = &messages[0];
        assert_eq!(
            init.pointer("/params/capabilities/textDocument/codeAction/dataSupport"),
            Some(&true.into())
        );
        assert!(
            init.pointer("/params/capabilities/textDocument/inlayHint")
                .is_some()
        );
        assert!(
            !messages
                .iter()
                .any(|m| m["method"] == "workspace/executeCommand")
        );
    }
}
#[test]
fn hints_validate_utf16_range_and_ignore_executable_label_part_metadata() {
    for mode in [
        "safe",
        "hint-surrogate",
        "hint-outside",
        "hint-control",
        "unsupported",
        "null",
    ] {
        let (_temp, mut s, path, _log) = setup(mode);
        let before = fs::read(&path).unwrap();
        hints(&mut s, &path);
        if mode == "safe" {
            let r = s.inlay_hints.as_ref().unwrap();
            assert_eq!(r.items[0].label, ": i32");
            assert!(r.items[0].padding_left);
        } else if mode == "null" {
            assert!(s.inlay_hints.as_ref().unwrap().items.is_empty());
        } else {
            assert!(s.inlay_hints_error.is_some(), "{mode}");
            assert!(s.inlay_hints.is_none());
        }
        assert_eq!(fs::read(&path).unwrap(), before);
        stop(&mut s);
    }
}
#[test]
fn sync_cancel_and_new_sessions_discard_late_results_and_resolve_tokens() {
    for hint in [false, true] {
        let (_temp, mut s, path, log) = setup("delay");
        let command = if hint {
            LanguageCommand::InlayHints {
                path: path.clone(),
                version: 7,
                range: range(),
            }
        } else {
            LanguageCommand::CodeActions {
                path: path.clone(),
                version: 7,
                range: range(),
            }
        };
        s.dispatch(command).unwrap();
        let end = Instant::now() + Duration::from_secs(4);
        loop {
            s.poll();
            let wire = fs::read_to_string(&log).unwrap_or_default();
            if wire.contains(if hint {
                "textDocument/inlayHint"
            } else {
                "textDocument/codeAction"
            }) {
                break;
            }
            assert!(Instant::now() < end);
            thread::sleep(Duration::from_millis(3));
        }
        s.dispatch(LanguageCommand::Sync {
            path: path.clone(),
            version: 8,
            text: "// 😀\r\nlet number = 43;\r\n".into(),
        })
        .unwrap();
        wait(&mut s, |s| !s.is_busy());
        thread::sleep(Duration::from_millis(180));
        s.poll();
        assert!(s.inlay_hints.is_none());
        assert!(s.code_actions.is_none());
        assert!(!s.code_actions_pending());
        assert!(!s.inlay_hints_pending());
        stop(&mut s);
        assert!(fs::read_to_string(log).unwrap().contains("$/cancelRequest"));
    }
    let (_temp, mut s, path, _log) = setup("safe");
    actions(&mut s, &path);
    let token = s.code_actions.as_ref().unwrap().items[0].token;
    s.dispatch(LanguageCommand::CancelCodeActions).unwrap();
    assert!(
        s.dispatch(LanguageCommand::ResolveCodeAction { token })
            .is_err()
    );
    stop(&mut s);
}
#[test]
fn inlay_wait_is_background_and_nonrust_documents_do_not_gain_rust_features() {
    let (_temp, mut s, path, _log) = setup("silent");
    s.dispatch(LanguageCommand::InlayHints {
        path: path.clone(),
        version: 7,
        range: range(),
    })
    .unwrap();
    wait(&mut s, |s| !s.is_busy());
    assert!(s.inlay_hints_pending());
    s.dispatch(LanguageCommand::CancelInlayHints).unwrap();
    wait(&mut s, |s| !s.is_busy());
    assert!(!s.inlay_hints_pending());
    assert!(
        s.dispatch(LanguageCommand::CodeActions {
            path: path.with_extension("php"),
            version: 7,
            range: range()
        })
        .is_err()
    );
    stop(&mut s);
}

#[test]
fn independent_feature_deadlines_end_pending_state_without_false_result() {
    let (_temp, mut s, path, _log) = setup("silent");
    let before = fs::read(&path).unwrap();
    s.dispatch(LanguageCommand::CodeActions {
        path: path.clone(),
        version: 7,
        range: range(),
    })
    .unwrap();
    s.dispatch(LanguageCommand::InlayHints {
        path: path.clone(),
        version: 7,
        range: range(),
    })
    .unwrap();
    let end = Instant::now() + Duration::from_secs(13);
    while s.code_actions_pending() || s.inlay_hints_pending() {
        s.poll();
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(s.code_actions_error.as_deref().unwrap().contains("Timeout"));
    assert!(s.inlay_hints_error.as_deref().unwrap().contains("Timeout"));
    assert!(s.code_actions.is_none() && s.inlay_hints.is_none());
    assert_eq!(fs::read(path).unwrap(), before);
    stop(&mut s);
}

#[test]
fn php_typescript_actions_resolve_and_reject_hostile_edits_with_no_commands() {
    for language in [Language::Php, Language::TypeScript] {
        for mode in [
            "safe",
            "command",
            "disabled",
            "escape",
            "resource",
            "overlap",
            "surrogate",
            "wrong-version",
            "resolve-command",
            "identity",
            "unsupported",
            "null",
            "crash",
        ] {
            let (_temp, mut service, path, log) = setup_language(mode, language);
            let before = fs::read(&path).unwrap();
            actions(&mut service, &path);
            if mode == "null" {
                assert!(service.code_actions.as_ref().unwrap().items.is_empty());
            } else if matches!(mode, "unsupported" | "crash") {
                assert!(service.code_actions_error.is_some());
            } else {
                let item = service.code_actions.as_ref().unwrap().items[0].clone();
                if matches!(mode, "command" | "disabled") {
                    assert!(item.disabled_reason.is_some());
                } else {
                    service
                        .dispatch(LanguageCommand::ResolveCodeAction { token: item.token })
                        .unwrap();
                    wait(&mut service, |s| !s.code_actions_pending());
                    if mode == "safe" {
                        assert!(
                            service.code_action_edit.is_some(),
                            "{language:?}: {:?}",
                            service.code_actions_error
                        );
                    } else {
                        assert!(service.code_actions_error.is_some(), "{language:?}/{mode}");
                    }
                }
            }
            assert_eq!(fs::read(&path).unwrap(), before);
            stop(&mut service);
            let messages: Vec<serde_json::Value> = fs::read_to_string(log)
                .unwrap()
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect();
            assert_eq!(
                messages[0].pointer("/params/capabilities/textDocument/codeAction/dataSupport"),
                Some(&true.into())
            );
            assert!(
                messages[0]
                    .pointer("/params/capabilities/textDocument/inlayHint")
                    .is_none()
            );
            assert!(
                !messages
                    .iter()
                    .any(|m| m["method"] == "workspace/executeCommand")
            );
            if let Some(query) = messages
                .iter()
                .find(|m| m["method"] == "textDocument/codeAction")
            {
                assert_eq!(
                    query.pointer("/params/context/diagnostics/0/code"),
                    Some(&2339.into())
                );
                assert_eq!(
                    query.pointer("/params/context/diagnostics/0/data/opaque"),
                    Some(&"kept".into())
                );
            }
        }
    }
}

#[test]
fn php_and_typescript_resolve_tokens_route_to_the_owning_provider() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let configs = [(Language::Php, "php"), (Language::TypeScript, "ts")]
        .into_iter()
        .map(|(language, ext)| ProviderConfig {
            language,
            program: "python3".into(),
            args: vec![
                format!(
                    "{}/tests/fixtures/rust_features_server.py",
                    env!("CARGO_MANIFEST_DIR")
                ),
                "safe".into(),
                root.join(format!("{ext}.wire")).display().to_string(),
                format!("file://{}/outside.rs", root.display()),
            ],
        })
        .collect();
    let mut service = LanguageService::new();
    service
        .dispatch(LanguageCommand::Start {
            workspace: root.clone(),
            configs,
        })
        .unwrap();
    wait(&mut service, |s| {
        s.statuses.len() == 2 && s.statuses.iter().all(|p| p.state == ProviderState::Ready)
    });
    for ext in ["php", "ts"] {
        let path = root.join(format!("main.{ext}"));
        let text = "// 😀\r\nlet number = 42;\r\n";
        fs::write(&path, text).unwrap();
        service
            .dispatch(LanguageCommand::Sync {
                path: path.clone(),
                version: 7,
                text: text.into(),
            })
            .unwrap();
        wait(&mut service, |s| {
            !s.is_busy() && s.analyzed_versions.get(&path) == Some(&7)
        });
        actions(&mut service, &path);
        let token = service.code_actions.as_ref().unwrap().items[0].token;
        service
            .dispatch(LanguageCommand::ResolveCodeAction { token })
            .unwrap();
        wait(&mut service, |s| !s.code_actions_pending());
        assert_eq!(
            service.code_action_edit.as_ref().unwrap().path,
            path,
            "{:?}",
            service.code_actions_error
        );
        assert_eq!(fs::read_to_string(path).unwrap(), text);
    }
    stop(&mut service);
    for ext in ["php", "ts"] {
        let wire = fs::read_to_string(root.join(format!("{ext}.wire"))).unwrap();
        assert!(wire.contains("codeAction/resolve"));
        assert!(!wire.contains("workspace/executeCommand"));
    }
}

#[test]
fn python_negotiates_inert_hints_and_environment_change_obsoletes_only_python_requests() {
    let (_temp, mut s, path, log) = setup_language("delay", Language::Python);
    assert!(s.rust_capabilities[&Language::Python].inlay_hints);
    s.dispatch(LanguageCommand::InlayHints {
        path: path.clone(),
        version: 7,
        range: range(),
    })
    .unwrap();
    s.dispatch(LanguageCommand::PythonEnvironment {
        python: Some("/unexecuted/venv/bin/python".into()),
    })
    .unwrap();
    wait(&mut s, |s| !s.is_busy() && !s.inlay_hints_pending());
    let end = Instant::now() + Duration::from_millis(250);
    while Instant::now() < end {
        s.poll();
        thread::sleep(Duration::from_millis(3));
    }
    assert!(
        s.inlay_hints.is_none(),
        "old-environment response cannot become current"
    );
    s.dispatch(LanguageCommand::InlayHints {
        path: path.clone(),
        version: 7,
        range: range(),
    })
    .unwrap();
    wait(&mut s, |s| !s.inlay_hints_pending());
    assert!(s.inlay_hints_error.is_none(), "{:?}", s.inlay_hints_error);
    assert_eq!(s.inlay_hints.as_ref().unwrap().items[0].label, ": i32");
    let wire = fs::read_to_string(log).unwrap();
    assert!(wire.contains("pythonPath") && wire.contains("/unexecuted/venv/bin/python"));
    assert!(wire.contains("inlayHint") && wire.contains("languageId\": \"python"));
    assert!(!wire.contains("workspace/executeCommand"));
}
#[test]
fn python_hostile_action_commands_escapes_and_hint_surrogates_are_rejected() {
    for mode in [
        "resolve-command",
        "resource",
        "escape",
        "wrong-version",
        "identity",
    ] {
        let (_temp, mut s, path, _log) = setup_language(mode, Language::Python);
        s.dispatch(LanguageCommand::CodeActions {
            path,
            version: 7,
            range: range(),
        })
        .unwrap();
        wait(&mut s, |s| !s.code_actions_pending());
        let token = s.code_actions.as_ref().unwrap().items[0].token;
        s.dispatch(LanguageCommand::ResolveCodeAction { token })
            .unwrap();
        wait(&mut s, |s| !s.code_actions_pending());
        assert!(
            s.code_action_edit.is_none() && s.code_actions_error.is_some(),
            "{mode}"
        );
    }
    let (_temp, mut s, path, _) = setup_language("hint-surrogate", Language::Python);
    s.dispatch(LanguageCommand::InlayHints {
        path,
        version: 7,
        range: range(),
    })
    .unwrap();
    wait(&mut s, |s| !s.inlay_hints_pending());
    assert!(s.inlay_hints.is_none() && s.inlay_hints_error.is_some());
}

#[test]
fn python_environment_keeps_php_action_request_and_negotiation_independent() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let php = root.join("main.php");
    let py = root.join("main.py");
    let text = "// 😀\r\nlet number = 42;\r\n";
    for path in [&php, &py] {
        fs::write(path, text).unwrap();
    }
    let configs = [(Language::Php, "php"), (Language::Python, "python")]
        .into_iter()
        .map(|(language, name)| ProviderConfig {
            language,
            program: "python3".into(),
            args: vec![
                format!(
                    "{}/tests/fixtures/rust_features_server.py",
                    env!("CARGO_MANIFEST_DIR")
                ),
                "delay".into(),
                root.join(format!("{name}.jsonl")).display().to_string(),
                format!("file://{}", php.display()),
            ],
        })
        .collect();
    let mut s = LanguageService::new();
    s.dispatch(LanguageCommand::Start {
        workspace: root.clone(),
        configs,
    })
    .unwrap();
    wait(&mut s, |s| {
        s.statuses.len() == 2 && s.statuses.iter().all(|p| p.state == ProviderState::Ready)
    });
    for path in [&php, &py] {
        s.dispatch(LanguageCommand::Sync {
            path: path.clone(),
            version: 7,
            text: text.into(),
        })
        .unwrap();
    }
    wait(&mut s, |s| !s.is_busy());
    s.dispatch(LanguageCommand::CodeActions {
        path: php.clone(),
        version: 7,
        range: range(),
    })
    .unwrap();
    s.dispatch(LanguageCommand::InlayHints {
        path: py,
        version: 7,
        range: range(),
    })
    .unwrap();
    s.dispatch(LanguageCommand::PythonEnvironment {
        python: Some("/unexecuted/bin/python".into()),
    })
    .unwrap();
    wait(&mut s, |s| !s.is_busy() && !s.code_actions_pending());
    assert_eq!(
        s.code_actions
            .as_ref()
            .expect("PHP result survives Python environment")
            .path,
        php
    );
    assert!(s.inlay_hints.is_none());
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, |s| !s.is_busy());
    let init = |name: &str| {
        let wire = fs::read_to_string(root.join(format!("{name}.jsonl"))).unwrap();
        wire.lines()
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
            .find(|v| v["method"] == "initialize")
            .unwrap()
    };
    assert!(
        init("php")
            .pointer("/params/capabilities/textDocument/inlayHint")
            .is_none()
    );
    assert!(
        init("python")
            .pointer("/params/capabilities/textDocument/inlayHint")
            .is_some()
    );
    assert!(
        s.dispatch(LanguageCommand::PythonEnvironment {
            python: Some("relative/python".into())
        })
        .is_err()
    );
}

#[test]
fn python_failure_does_not_clear_a_rust_hint_request_or_accepted_response() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let rust = root.join("main.rs");
    let py = root.join("main.py");
    let text = "// 😀\r\nlet number = 42;\r\n";
    for path in [&rust, &py] {
        fs::write(path, text).unwrap();
    }
    let configs = [(Language::Rust, "delay"), (Language::Python, "crash")]
        .into_iter()
        .map(|(language, mode)| ProviderConfig {
            language,
            program: "python3".into(),
            args: vec![
                format!(
                    "{}/tests/fixtures/rust_features_server.py",
                    env!("CARGO_MANIFEST_DIR")
                ),
                mode.into(),
                root.join(format!("{language:?}.jsonl"))
                    .display()
                    .to_string(),
                format!("file://{}", py.display()),
            ],
        })
        .collect();
    let mut s = LanguageService::new();
    s.dispatch(LanguageCommand::Start {
        workspace: root,
        configs,
    })
    .unwrap();
    wait(&mut s, |s| {
        s.statuses.len() == 2 && s.statuses.iter().all(|p| p.state == ProviderState::Ready)
    });
    for path in [&rust, &py] {
        s.dispatch(LanguageCommand::Sync {
            path: path.clone(),
            version: 7,
            text: text.into(),
        })
        .unwrap();
    }
    wait(&mut s, |s| !s.is_busy());
    s.dispatch(LanguageCommand::InlayHints {
        path: rust.clone(),
        version: 7,
        range: range(),
    })
    .unwrap();
    s.dispatch(LanguageCommand::CodeActions {
        path: py,
        version: 7,
        range: range(),
    })
    .unwrap();
    wait(&mut s, |s| {
        !s.inlay_hints_pending()
            && s.statuses
                .iter()
                .any(|p| p.language == Language::Python && p.state == ProviderState::Failed)
    });
    assert_eq!(
        s.inlay_hints
            .as_ref()
            .expect("Rust hint survives unrelated Python crash")
            .path,
        rust
    );
    assert!(s.inlay_hints_error.is_none());
}
