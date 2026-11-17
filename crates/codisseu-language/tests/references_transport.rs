use codisseu_language::{
    Language, LanguageCommand, LanguageService, ProviderConfig, ProviderState,
};
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
fn wait(s: &mut LanguageService, secs: u64, check: impl Fn(&LanguageService) -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        s.poll();
        if check(s) {
            return;
        }
        assert!(
            Instant::now() < end,
            "{:?} {:?}",
            s.last_error,
            s.references_error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn records(p: &std::path::Path) -> Vec<Value> {
    fs::read_to_string(p)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}
fn fixture(mode: &str) -> (tempfile::TempDir, LanguageService, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let source = root.join("source.ts");
    let target = root.join("target.ts");
    let outside = dir.path().join("outside.ts");
    let log = dir.path().join("protocol.jsonl");
    fs::write(&source, "😀target").unwrap();
    fs::write(&target, "x".repeat(600)).unwrap();
    fs::write(&outside, "target").unwrap();
    let fixture_script = if mode.starts_with("adapter_") {
        let lib = dir.path().join("lib");
        fs::create_dir(&lib).unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"name":"typescript-language-server","version":"controlled"}"#,
        )
        .unwrap();
        let script = lib.join("cli.py");
        fs::copy(
            format!(
                "{}/tests/fixtures/references_server.py",
                env!("CARGO_MANIFEST_DIR")
            ),
            &script,
        )
        .unwrap();
        script.display().to_string()
    } else {
        format!(
            "{}/tests/fixtures/references_server.py",
            env!("CARGO_MANIFEST_DIR")
        )
    };
    let mut s = LanguageService::new();
    s.dispatch(LanguageCommand::Start {
        workspace: root,
        configs: vec![ProviderConfig {
            language: Language::TypeScript,
            program: "python3".into(),
            args: vec![
                fixture_script,
                mode.into(),
                log.to_str().unwrap().into(),
                target.to_str().unwrap().into(),
                outside.to_str().unwrap().into(),
            ],
        }],
    })
    .unwrap();
    wait(&mut s, 5, |s| {
        s.statuses.iter().any(|p| p.state == ProviderState::Ready)
    });
    s.dispatch(LanguageCommand::Sync {
        path: source.clone(),
        version: 1,
        text: "😀target".into(),
    })
    .unwrap();
    wait(&mut s, 5, |s| !s.is_busy());
    (dir, s, source, log)
}
fn query(s: &mut LanguageService, p: &std::path::Path, include: bool) {
    s.dispatch(LanguageCommand::References {
        path: p.into(),
        version: 1,
        line: 0,
        character: 4,
        include_declaration: include,
    })
    .unwrap();
}
#[test]
fn references_capability_strict_locations_null_truncation_and_errors_use_transport() {
    for mode in [
        "unsupported",
        "safe",
        "options",
        "null",
        "many",
        "outside",
        "remote",
        "link",
        "object",
        "bad_range",
        "error",
    ] {
        let (_dir, mut s, path, log) = fixture(mode);
        query(&mut s, &path, false);
        wait(&mut s, 5, |s| !s.references_pending());
        if matches!(mode, "safe" | "options" | "null" | "many") {
            let r = s.references.as_ref().unwrap();
            assert!(s.references_error.is_none());
            assert_eq!(
                r.targets.len(),
                if mode == "null" {
                    0
                } else if mode == "many" {
                    256
                } else {
                    1
                }
            );
            assert_eq!(r.truncated, mode == "many");
        } else {
            assert!(s.references.is_none(), "{mode}");
            assert!(s.references_error.is_some(), "{mode}");
        }
        s.dispatch(LanguageCommand::Stop).unwrap();
        wait(&mut s, 5, |s| !s.is_busy());
        let records = records(&log);
        let requests: Vec<_> = records
            .iter()
            .filter(|v| v["method"] == "textDocument/references")
            .collect();
        if mode == "unsupported" {
            assert!(requests.is_empty());
        } else {
            assert_eq!(
                requests[0]["params"]["context"]["includeDeclaration"],
                false
            );
        }
    }
}
#[test]
fn references_include_declaration_context_and_reordered_ids_are_correlated() {
    let (_dir, mut s, path, log) = fixture("reorder");
    query(&mut s, &path, false);
    wait(&mut s, 5, |_| {
        records(&log)
            .iter()
            .any(|v| v["method"] == "textDocument/references")
    });
    query(&mut s, &path, true);
    wait(&mut s, 5, |s| !s.references_pending());
    assert_eq!(s.references.as_ref().unwrap().targets.len(), 1);
    s.dispatch(LanguageCommand::CancelReferences).unwrap();
    assert!(s.references.is_none());
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, 5, |s| !s.is_busy());
    let records = records(&log);
    let req: Vec<_> = records
        .iter()
        .filter(|v| v["method"] == "textDocument/references")
        .collect();
    assert_eq!(req.len(), 2);
    assert_eq!(req[0]["params"]["context"]["includeDeclaration"], false);
    assert_eq!(req[1]["params"]["context"]["includeDeclaration"], true);
    let cancel = records
        .iter()
        .find(|v| v["method"] == "$/cancelRequest")
        .unwrap();
    assert_eq!(cancel["params"]["id"], req[0]["id"]);
}
#[test]
fn references_source_edit_discards_a_late_response() {
    let (_dir, mut s, path, log) = fixture("on_change");
    query(&mut s, &path, false);
    wait(&mut s, 5, |_| {
        records(&log)
            .iter()
            .any(|v| v["method"] == "textDocument/references")
    });
    s.dispatch(LanguageCommand::Sync {
        path: path.clone(),
        version: 2,
        text: "😀changed".into(),
    })
    .unwrap();
    assert!(!s.references_pending());
    wait(&mut s, 5, |_| {
        records(&log).iter().any(|v| v.get("replySent").is_some())
    });
    wait(&mut s, 5, |s| !s.is_busy());
    assert!(s.references.is_none());
    assert!(s.references_error.is_none());
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, 5, |s| !s.is_busy());
}
#[test]
fn references_real_deadline_cancels_silent_request() {
    let (_dir, mut s, path, log) = fixture("silent");
    query(&mut s, &path, false);
    let start = Instant::now();
    wait(&mut s, 15, |s| !s.references_pending());
    assert!(start.elapsed() >= Duration::from_secs(9));
    assert!(
        s.references_error
            .as_ref()
            .unwrap()
            .contains("Timeout de referências")
    );
    assert!(s.references.is_none());
    wait(&mut s, 5, |_| {
        records(&log)
            .iter()
            .any(|v| v["method"] == "$/cancelRequest")
    });
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, 5, |s| !s.is_busy());
}

#[test]
fn typescript_adapter_excludes_only_actual_definition_ranges_and_fails_closed() {
    for mode in [
        "adapter_safe",
        "adapter_missing",
        "adapter_error",
        "adapter_null",
    ] {
        let (_d, mut s, p, log) = fixture(mode);
        query(&mut s, &p, false);
        wait(&mut s, 5, |s| !s.references_pending());
        let wire = records(&log);
        if mode == "adapter_safe" {
            let result = s.references.as_ref().unwrap();
            assert_eq!(result.targets.len(), 1);
            assert_eq!(result.targets[0].start_character, 6);
            assert_eq!(result.targets[0].end_character, 12);
            let methods: Vec<_> = wire
                .iter()
                .filter_map(|v| v["method"].as_str())
                .filter(|m| *m == "textDocument/definition" || *m == "textDocument/references")
                .collect();
            assert_eq!(
                methods,
                vec!["textDocument/definition", "textDocument/references"]
            );
        } else {
            assert!(s.references_error.is_some());
            assert!(
                !wire
                    .iter()
                    .any(|v| v["method"] == "textDocument/references")
            );
        }
    }
}
#[test]
fn private_definition_and_reference_phases_cancel_stale_and_reordered_responses() {
    for mode in ["adapter_holddef", "adapter_holdrefs"] {
        let (_d, mut s, p, log) = fixture(mode);
        query(&mut s, &p, false);
        let method = if mode == "adapter_holddef" {
            "textDocument/definition"
        } else {
            "textDocument/references"
        };
        wait(&mut s, 5, |_| {
            records(&log).iter().any(|v| v["method"] == method)
        });
        s.dispatch(LanguageCommand::Sync {
            path: p.clone(),
            version: 2,
            text: "😀target changed".into(),
        })
        .unwrap();
        wait(&mut s, 5, |_| {
            records(&log)
                .iter()
                .any(|v| v["method"] == "textDocument/didChange")
        });
        s.poll();
        assert!(s.references.is_none());
        assert!(s.references_error.is_none());
        let wire = records(&log);
        let canceled = wire
            .iter()
            .find(|v| v["method"] == "$/cancelRequest")
            .unwrap()["params"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(canceled.starts_with(if mode == "adapter_holddef" {
            "references-definition-"
        } else {
            "references-"
        }));
        if mode == "adapter_holddef" {
            assert!(
                !wire
                    .iter()
                    .any(|v| v["method"] == "textDocument/references")
            );
        }
    }
    let (_d, mut s, p, log) = fixture("adapter_reorderdef");
    query(&mut s, &p, false);
    wait(&mut s, 5, |_| {
        records(&log)
            .iter()
            .any(|v| v["method"] == "textDocument/definition")
    });
    query(&mut s, &p, false);
    wait(&mut s, 5, |s| !s.references_pending());
    assert_eq!(s.references.as_ref().unwrap().targets.len(), 1);
    assert_eq!(
        records(&log)
            .iter()
            .filter(|v| v["method"] == "textDocument/references")
            .count(),
        1
    );
}

#[test]
fn private_definition_deadline_cancels_its_own_id() {
    let (_d, mut s, p, log) = fixture("adapter_holddef");
    query(&mut s, &p, false);
    wait(&mut s, 15, |s| !s.references_pending());
    assert!(
        s.references_error
            .as_ref()
            .unwrap()
            .contains("Timeout de referências")
    );
    wait(&mut s, 5, |_| {
        records(&log)
            .iter()
            .any(|v| v["method"] == "$/cancelRequest")
    });
    let wire = records(&log);
    assert!(
        wire.iter()
            .find(|v| v["method"] == "$/cancelRequest")
            .unwrap()["params"]["id"]
            .as_str()
            .unwrap()
            .starts_with("references-definition-")
    );
    assert!(
        !wire
            .iter()
            .any(|v| v["method"] == "textDocument/references")
    );
}
