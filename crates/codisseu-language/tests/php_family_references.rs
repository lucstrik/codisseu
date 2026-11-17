use codisseu_language::{
    Language, LanguageCommand, LanguageService, ProviderConfig, ProviderState,
};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
fn wait(s: &mut LanguageService, f: impl Fn(&LanguageService) -> bool) {
    let until = Instant::now() + Duration::from_secs(8);
    while !f(s) {
        s.poll();
        thread::sleep(Duration::from_millis(2));
        assert!(
            Instant::now() < until,
            "{:?} {:?}",
            s.last_error,
            s.references_error
        );
    }
}
fn records(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
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
    let root = dir.path().join("root");
    fs::create_dir(&root).unwrap();
    let source = root.join("Concrete.php");
    let base = root.join("Base.php");
    let log = dir.path().join("protocol.jsonl");
    let usage = root.join("usage.php");
    let usage2 = root.join("usage2.php");
    fs::write(
        &source,
        "<?php\nclass Concrete { public function deliver(): string { return ''; } }\n",
    )
    .unwrap();
    fs::write(
        &base,
        "<?php\ninterface Contract { public function deliver(): string; }\n",
    )
    .unwrap();
    fs::write(&usage, "target\n").unwrap();
    fs::write(&usage2, "target\n").unwrap();
    if mode == "legacy" {
        fs::write(
            &base,
            b"<?php\n/* \xe7 */ interface Contract { public function deliver(): string; }\n",
        )
        .unwrap();
    }
    if mode == "oversized" {
        fs::write(&base, vec![b'x'; 1024 * 1024 + 1]).unwrap();
    }
    let mut s = LanguageService::new();
    s.dispatch(LanguageCommand::Start {
        workspace: root,
        configs: vec![ProviderConfig {
            language: Language::Php,
            program: "python3".into(),
            args: vec![
                format!(
                    "{}/tests/fixtures/php_family_references_server.py",
                    env!("CARGO_MANIFEST_DIR")
                ),
                mode.into(),
                log.display().to_string(),
                base.display().to_string(),
                usage.display().to_string(),
                usage2.display().to_string(),
            ],
        }],
    })
    .unwrap();
    wait(&mut s, |s| {
        s.statuses.iter().any(|p| p.state == ProviderState::Ready)
    });
    s.dispatch(LanguageCommand::Sync {
        path: source.clone(),
        version: 7,
        text: fs::read_to_string(&source).unwrap(),
    })
    .unwrap();
    wait(&mut s, |s| !s.is_busy());
    (dir, s, source, base, log)
}
fn query(s: &mut LanguageService, source: &Path) {
    s.dispatch(LanguageCommand::References {
        path: source.into(),
        version: 7,
        line: 1,
        character: 35,
        include_declaration: false,
    })
    .unwrap();
}
#[test]
fn base_contract_union_retains_source_version_and_closes_transient() {
    let (_dir, mut s, source, base, log) = fixture("safe");
    query(&mut s, &source);
    wait(&mut s, |s| !s.references_pending());
    let result = s.references.as_ref().unwrap();
    assert_eq!(result.path, source);
    assert_eq!(result.version, 7);
    assert_eq!((result.line, result.character), (1, 35));
    assert_eq!(result.targets.len(), 2);
    assert!(!result.truncated);
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, |s| !s.is_busy());
    let wire = records(&log);
    let base_uri = format!("file://{}", base.display());
    assert!(
        wire.iter().any(|m| m["method"] == "textDocument/didOpen"
            && m["params"]["textDocument"]["uri"] == base_uri)
    );
    assert!(wire.iter().any(|m| m["method"] == "textDocument/didClose"
        && m["params"]["textDocument"]["uri"] == base_uri));
    assert_eq!(
        wire.iter()
            .filter(|m| m["method"] == "textDocument/references")
            .count(),
        2
    );
}
#[test]
fn failed_or_uncertified_base_keeps_direct_results_and_marks_partial() {
    for mode in [
        "legacy",
        "oversized",
        "wrong_range",
        "ambiguous",
        "base_error",
    ] {
        let (_dir, mut s, source, _base, _log) = fixture(mode);
        query(&mut s, &source);
        wait(&mut s, |s| !s.references_pending());
        let result = s.references.as_ref().unwrap();
        assert_eq!(result.targets.len(), 1, "{mode}");
        assert!(result.truncated, "{mode}");
        assert!(s.references_error.is_none());
        s.dispatch(LanguageCommand::Stop).unwrap();
        wait(&mut s, |s| !s.is_busy());
    }
}
#[test]
fn cancel_base_discards_late_response_and_closes_only_owned_lease() {
    let (_dir, mut s, source, base, log) = fixture("hold_base");
    query(&mut s, &source);
    wait(&mut s, |_| {
        records(&log).iter().any(|m| {
            m["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("references-family-base-"))
        })
    });
    s.dispatch(LanguageCommand::CancelReferences).unwrap();
    wait(&mut s, |s| !s.is_busy());
    thread::sleep(Duration::from_millis(20));
    s.poll();
    assert!(s.references.is_none());
    assert!(s.references_error.is_none());
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, |s| !s.is_busy());
    let wire = records(&log);
    let uri = format!("file://{}", base.display());
    assert!(wire.iter().any(
        |m| m["method"] == "textDocument/didClose" && m["params"]["textDocument"]["uri"] == uri
    ));
    assert!(wire.iter().any(|m| {
        m["method"] == "$/cancelRequest"
            && m["params"]["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("references-family-base-"))
    }));
}

#[test]
fn server_choice_in_base_phase_keeps_original_request_owner_and_echoes_inert_item() {
    let (_dir, mut s, source, _base, log) = fixture("choice");
    query(&mut s, &source);
    wait(&mut s, |s| !s.server_choices.is_empty());
    let choice = s.server_choices[0].clone();
    assert!(s.references_pending());
    assert_eq!(choice.items, vec!["Continue"]);
    s.answer_server_choice(choice.token, Some(0)).unwrap();
    wait(&mut s, |s| !s.references_pending());
    assert_eq!(s.references.as_ref().unwrap().targets.len(), 2);
    assert_eq!(s.references.as_ref().unwrap().path, source);
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, |s| !s.is_busy());
    let wire = records(&log);
    assert!(
        wire.iter()
            .any(|m| m["id"] == "family-choice" && m["result"]["opaque"] == "preserve")
    );
}
