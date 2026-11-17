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
            s.completion_error
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
    let root = dir.path().canonicalize().unwrap();
    let path = root.join("fixture.ts");
    let log = root.join("protocol.jsonl");
    fs::write(&path, "😀foo").unwrap();
    let mut s = LanguageService::new();
    s.dispatch(LanguageCommand::Start {
        workspace: root,
        configs: vec![ProviderConfig {
            language: Language::TypeScript,
            program: "python3".into(),
            args: vec![
                format!(
                    "{}/tests/fixtures/completion_server.py",
                    env!("CARGO_MANIFEST_DIR")
                ),
                mode.into(),
                log.to_str().unwrap().into(),
            ],
        }],
    })
    .unwrap();
    wait(&mut s, 5, |s| {
        s.statuses.iter().any(|p| p.state == ProviderState::Ready)
    });
    s.dispatch(LanguageCommand::Sync {
        path: path.clone(),
        version: 1,
        text: "😀foo".into(),
    })
    .unwrap();
    wait(&mut s, 5, |s| !s.is_busy());
    (dir, s, path, log)
}
fn query(s: &mut LanguageService, p: &std::path::Path) {
    s.dispatch(LanguageCommand::Completion {
        path: p.into(),
        version: 1,
        line: 0,
        character: 5,
    })
    .unwrap();
}
#[test]
fn capability_safe_items_snippets_commands_and_null_use_real_transport() {
    for mode in ["unsupported", "safe", "snippet", "command", "null"] {
        let (_dir, mut s, path, log) = fixture(mode);
        query(&mut s, &path);
        wait(&mut s, 5, |s| !s.completion_pending());
        if mode == "unsupported" {
            assert!(
                s.completion_error
                    .as_ref()
                    .unwrap()
                    .contains("não oferece completion")
            );
            assert!(s.completion.is_none());
        } else {
            let result = s.completion.as_ref().unwrap();
            assert_eq!(result.items.len(), usize::from(mode != "null"));
            if mode != "null" {
                let item = &result.items[0];
                if mode == "safe" {
                    assert!(item.unsupported_reason.is_none());
                    assert_eq!(item.new_text, "food");
                } else {
                    assert!(item.unsupported_reason.is_some());
                    assert!(item.text_edit.is_none());
                    assert!(item.new_text.is_empty());
                }
            }
        }
        s.dispatch(LanguageCommand::Stop).unwrap();
        wait(&mut s, 5, |s| !s.is_busy());
        let records = records(&log);
        assert!(
            !records
                .iter()
                .any(|m| m["method"] == "workspace/executeCommand"
                    || m["method"] == "completionItem/resolve")
        );
        let initialize = records
            .iter()
            .find(|m| m["method"] == "initialize")
            .unwrap();
        assert_eq!(
            initialize["params"]["capabilities"]["textDocument"]["completion"]["completionItem"]["snippetSupport"],
            false
        );
        if mode == "unsupported" {
            assert!(
                !records
                    .iter()
                    .any(|m| m["method"] == "textDocument/completion")
            );
        }
    }
}
#[test]
fn reordered_responses_preserve_only_current_id_and_send_cancel() {
    let (_dir, mut s, path, log) = fixture("reorder");
    query(&mut s, &path);
    wait(&mut s, 5, |_| {
        records(&log)
            .iter()
            .any(|m| m["method"] == "textDocument/completion")
    });
    query(&mut s, &path);
    wait(&mut s, 5, |s| !s.completion_pending());
    assert_eq!(s.completion.as_ref().unwrap().items[0].new_text, "food");
    s.dispatch(LanguageCommand::CancelCompletion).unwrap();
    assert!(s.completion.is_none());
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, 5, |s| !s.is_busy());
    assert!(
        records(&log)
            .iter()
            .any(|m| m["method"] == "$/cancelRequest")
    );
}
#[test]
fn source_edit_discards_a_late_server_response_without_repopulating_items() {
    let (_dir, mut s, path, log) = fixture("on_change");
    query(&mut s, &path);
    wait(&mut s, 5, |_| {
        records(&log)
            .iter()
            .any(|m| m["method"] == "textDocument/completion")
    });
    s.dispatch(LanguageCommand::Sync {
        path: path.clone(),
        version: 2,
        text: "😀updated".into(),
    })
    .unwrap();
    assert!(!s.completion_pending());
    wait(&mut s, 5, |_| {
        records(&log).iter().any(|m| m.get("replySent").is_some())
    });
    wait(&mut s, 5, |s| !s.is_busy());
    assert!(s.completion.is_none());
    assert!(s.completion_error.is_none());
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, 5, |s| !s.is_busy());
}
#[test]
fn silent_completion_times_out_and_cancels_actual_request() {
    let (_dir, mut s, path, log) = fixture("silent");
    query(&mut s, &path);
    let start = Instant::now();
    wait(&mut s, 15, |s| !s.completion_pending());
    assert!(start.elapsed() >= Duration::from_secs(9));
    assert!(
        s.completion_error
            .as_ref()
            .unwrap()
            .contains("Timeout de completion")
    );
    assert!(s.completion.is_none());
    wait(&mut s, 5, |_| {
        records(&log)
            .iter()
            .any(|m| m["method"] == "$/cancelRequest")
    });
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, 5, |s| !s.is_busy());
}
