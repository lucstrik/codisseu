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
fn wait(s: &mut LanguageService, limit: u64, check: impl Fn(&LanguageService) -> bool) {
    let end = Instant::now() + Duration::from_secs(limit);
    loop {
        s.poll();
        if check(s) {
            return;
        }
        assert!(
            Instant::now() < end,
            "{:?} {:?}",
            s.last_error,
            s.definition_error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn records(p: &std::path::Path) -> Vec<Value> {
    fs::read_to_string(p)
        .unwrap_or_default()
        .lines()
        .filter_map(|s| serde_json::from_str(s).ok())
        .collect()
}
fn fixture(
    mode: &str,
) -> (
    tempfile::TempDir,
    LanguageService,
    PathBuf,
    PathBuf,
    PathBuf,
) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let source = root.join("main.ts");
    let target = root.join("target.ts");
    let outside = dir.path().join("outside.ts");
    let log = dir.path().join("messages.jsonl");
    for p in [&source, &target, &outside] {
        fs::write(p, "export const target = 42;\n").unwrap();
    }
    let mut s = LanguageService::new();
    s.dispatch(LanguageCommand::Start {
        workspace: root,
        configs: vec![ProviderConfig {
            language: Language::TypeScript,
            program: "python3".into(),
            args: vec![
                format!(
                    "{}/tests/fixtures/definition_server.py",
                    env!("CARGO_MANIFEST_DIR")
                ),
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
        text: "export const target = 42;\n".into(),
    })
    .unwrap();
    wait(&mut s, 5, |s| !s.is_busy());
    (dir, s, source, target, log)
}
fn query(s: &mut LanguageService, p: &std::path::Path) {
    s.dispatch(LanguageCommand::Definition {
        path: p.into(),
        version: 1,
        line: 0,
        character: 14,
    })
    .unwrap();
}
#[test]
fn capability_locations_links_null_and_invalid_targets_use_real_transport() {
    for mode in [
        "unsupported",
        "location",
        "link",
        "null",
        "outside",
        "remote",
    ] {
        let (_dir, mut s, p, target, log) = fixture(mode);
        query(&mut s, &p);
        wait(&mut s, 5, |s| !s.definition_pending());
        if matches!(mode, "unsupported" | "outside" | "remote") {
            assert!(s.definition_error.is_some(), "{mode}");
            assert!(s.definition.is_none());
        } else {
            let result = s.definition.as_ref().unwrap();
            assert_eq!(result.targets.len(), usize::from(mode != "null"));
            if mode != "null" {
                assert_eq!(result.targets[0].path, target);
                assert_eq!(result.targets[0].start_character, 13);
            }
        }
        s.dispatch(LanguageCommand::Stop).unwrap();
        wait(&mut s, 5, |s| !s.is_busy());
        if mode == "unsupported" {
            assert!(
                !records(&log)
                    .iter()
                    .any(|m| m["method"] == "textDocument/definition")
            );
        }
    }
}
#[test]
fn reordered_response_and_cancellation_keep_only_current_definition() {
    let (_dir, mut s, p, target, log) = fixture("reorder");
    query(&mut s, &p);
    wait(&mut s, 5, |_| {
        records(&log)
            .iter()
            .any(|m| m["method"] == "textDocument/definition")
    });
    query(&mut s, &p);
    wait(&mut s, 5, |s| !s.definition_pending());
    assert_eq!(s.definition.as_ref().unwrap().targets[0].path, target);
    s.dispatch(LanguageCommand::CancelDefinition).unwrap();
    assert!(s.definition.is_none());
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, 5, |s| !s.is_busy());
    let records = records(&log);
    let requests: Vec<_> = records
        .iter()
        .filter(|m| m["method"] == "textDocument/definition")
        .collect();
    let cancels: Vec<_> = records
        .iter()
        .filter(|m| m["method"] == "$/cancelRequest")
        .collect();
    assert_eq!(requests.len(), 2);
    assert!(!cancels.is_empty());
    assert_eq!(requests[0]["id"], cancels[0]["params"]["id"]);
}
#[test]
fn silent_definition_is_cancelled_and_times_out_on_actual_deadline() {
    let (_dir, mut s, p, _target, log) = fixture("silent");
    query(&mut s, &p);
    let started = Instant::now();
    wait(&mut s, 15, |s| !s.definition_pending());
    assert!(started.elapsed() >= Duration::from_secs(9));
    assert!(
        s.definition_error
            .as_ref()
            .unwrap()
            .contains("Timeout de definição")
    );
    wait(&mut s, 5, |_| {
        records(&log)
            .iter()
            .any(|m| m["method"] == "$/cancelRequest")
    });
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, 5, |s| !s.is_busy());
}

#[test]
fn implementations_negotiate_capability_validate_targets_and_cancel_the_shared_slot() {
    for mode in [
        "unsupported",
        "location",
        "link",
        "multi",
        "null",
        "outside",
        "remote",
    ] {
        let (_dir, mut s, source, target, log) = fixture(mode);
        s.dispatch(LanguageCommand::Implementation {
            path: source,
            version: 1,
            line: 0,
            character: 14,
        })
        .unwrap();
        wait(&mut s, 5, |s| !s.definition_pending());
        if matches!(mode, "unsupported" | "outside" | "remote" | "multi") {
            assert!(s.definition_error.is_some(), "{mode}");
            assert!(s.definition.is_none());
        } else {
            let result = s.definition.as_ref().unwrap();
            assert_eq!(result.targets.len(), usize::from(mode != "null"));
            if mode != "null" {
                assert_eq!(result.targets[0].path, target);
            }
        }
        s.dispatch(LanguageCommand::Stop).unwrap();
        wait(&mut s, 5, |s| !s.is_busy());
        assert_eq!(
            records(&log)
                .iter()
                .filter(|m| m["method"] == "textDocument/implementation")
                .count(),
            usize::from(mode != "unsupported")
        );
    }
    let (_dir, mut s, source, target, log) = fixture("reorder");
    s.dispatch(LanguageCommand::Implementation {
        path: source.clone(),
        version: 1,
        line: 0,
        character: 14,
    })
    .unwrap();
    wait(&mut s, 5, |_| {
        records(&log)
            .iter()
            .any(|m| m["method"] == "textDocument/implementation")
    });
    query(&mut s, &source);
    wait(&mut s, 5, |s| !s.definition_pending());
    assert_eq!(s.definition.as_ref().unwrap().targets[0].path, target);
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, 5, |s| !s.is_busy());
    let log = records(&log);
    let old = log
        .iter()
        .find(|m| m["method"] == "textDocument/implementation")
        .unwrap();
    assert!(
        log.iter()
            .any(|m| m["method"] == "$/cancelRequest" && m["params"]["id"] == old["id"])
    );
}
