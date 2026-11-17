//! A real protocol barrier, with unrelated and out-of-order progress rejected.
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
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        s.poll();
        if check(s) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{:?} {:?}",
            s.statuses,
            s.last_error
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
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    let log = root.join("wire.jsonl");
    let release = root.join("release");
    let source = root.join("file.php");
    fs::write(&source, "<?php // saved").unwrap();
    let mut s = LanguageService::new();
    s.dispatch(LanguageCommand::Start {
        workspace: root,
        configs: vec![ProviderConfig {
            language: Language::Php,
            program: "python3".into(),
            args: vec![
                format!(
                    "{}/tests/fixtures/index_initialization_server.py",
                    env!("CARGO_MANIFEST_DIR")
                ),
                mode.into(),
                log.display().to_string(),
                release.display().to_string(),
            ],
        }],
    })
    .unwrap();
    s.dispatch(LanguageCommand::Sync {
        path: source,
        version: 2,
        text: "<?php // dirty 🌧 e\u{301}".into(),
    })
    .unwrap();
    (d, s, log, release)
}
#[test]
fn phpactor_waits_for_matching_index_end_and_flushes_dirty_documents_before_ready() {
    let (_d, mut s, log, release) = fixture("release");
    wait(&mut s, 5, |_| {
        records(&log).iter().any(|v| v["waiting"] == true)
    });
    assert!(s.statuses.iter().all(|p| p.state != ProviderState::Ready));
    assert!(s.is_busy());
    let wire = records(&log);
    assert_eq!(
        wire[0].pointer("/params/capabilities/window/workDoneProgress"),
        Some(&Value::Bool(true))
    );
    assert!(!wire.iter().any(|v| v["method"] == "textDocument/didOpen"));
    fs::write(release, "go").unwrap();
    wait(&mut s, 5, |s| {
        s.statuses.iter().any(|p| p.state == ProviderState::Ready)
    });
    wait(&mut s, 5, |_| {
        records(&log)
            .iter()
            .any(|v| v["method"] == "textDocument/didOpen")
    });
    let wire = records(&log);
    let open = wire
        .iter()
        .find(|v| v["method"] == "textDocument/didOpen")
        .unwrap();
    assert_eq!(
        open.pointer("/params/textDocument/version"),
        Some(&Value::from(2))
    );
    assert!(
        open.pointer("/params/textDocument/text")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("dirty 🌧 e\u{301}")
    );
}
#[test]
fn generic_provider_stays_compatible_and_stop_discards_initial_index() {
    let (_d, mut s, _, _) = fixture("generic");
    wait(&mut s, 5, |s| {
        s.statuses.iter().any(|p| p.state == ProviderState::Ready)
    });
    let (_d, mut s, log, release) = fixture("release");
    wait(&mut s, 5, |_| {
        records(&log).iter().any(|v| v["waiting"] == true)
    });
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, 5, |s| {
        s.statuses.iter().all(|p| p.state == ProviderState::Stopped)
    });
    fs::write(release, "late").unwrap();
    s.poll();
    assert!(!s.statuses.iter().any(|p| p.state == ProviderState::Ready));
}
#[test]
fn progressing_index_longer_than_old_deadline_stays_alive_and_becomes_ready() {
    let (_d, mut s, log, release) = fixture("progress");
    wait(&mut s, 5, |_| {
        records(&log).iter().any(|v| v["waiting"] == true)
    });
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(32) {
        s.poll();
        assert!(
            s.statuses
                .iter()
                .all(|p| p.state == ProviderState::Starting)
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert!(s.statuses.iter().any(|p| p.detail.contains("500/1000")));
    assert!(
        !records(&log)
            .iter()
            .any(|v| v["method"] == "textDocument/didOpen")
    );
    fs::write(release, "go").unwrap();
    wait(&mut s, 5, |s| {
        s.statuses.iter().any(|p| p.state == ProviderState::Ready)
    });
}

#[test]
fn missing_initial_index_end_fails_after_inactivity_without_false_ready() {
    let (_d, mut s, log, _) = fixture("timeout");
    wait(&mut s, 5, |_| {
        records(&log).iter().any(|v| v["waiting"] == true)
    });
    wait(&mut s, 125, |s| {
        s.statuses.iter().any(|p| p.state == ProviderState::Failed)
    });
    assert!(
        s.statuses
            .iter()
            .any(|p| p.detail.contains("sem progresso por 120 segundos"))
    );
    assert!(
        !records(&log)
            .iter()
            .any(|v| v["method"] == "textDocument/didOpen")
    );
    assert!(!s.statuses.iter().any(|p| p.state == ProviderState::Ready));
}
