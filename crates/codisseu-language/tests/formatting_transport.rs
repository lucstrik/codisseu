use codisseu_language::{
    Language, LanguageCommand, LanguageService, ProviderConfig, ProviderState, RenameRange,
};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
fn wait(s: &mut LanguageService, seconds: u64, check: impl Fn(&LanguageService) -> bool) {
    let end = Instant::now() + Duration::from_secs(seconds);
    loop {
        s.poll();
        if check(s) {
            return;
        }
        assert!(
            Instant::now() < end,
            "{:?}/{:?}",
            s.last_error,
            s.formatting_error
        );
        thread::sleep(Duration::from_millis(3));
    }
}
fn fixture(mode: &str) -> (tempfile::TempDir, LanguageService, PathBuf, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    let p = root.join("format.ts");
    let log = root.join("wire.jsonl");
    let text = "// 😀\r\nlet x=1;\r\nlet y=2;\r\n";
    fs::write(&p, text).unwrap();
    let mut s = LanguageService::new();
    s.dispatch(LanguageCommand::Start {
        workspace: root,
        configs: vec![ProviderConfig {
            language: Language::TypeScript,
            program: "python3".into(),
            args: vec![
                format!(
                    "{}/tests/fixtures/formatting_server.py",
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
        path: p.clone(),
        version: 7,
        text: text.into(),
    })
    .unwrap();
    wait(&mut s, 5, |s| !s.is_busy());
    (d, s, p, log)
}
fn query(s: &mut LanguageService, p: &Path, range: bool) {
    s.dispatch(LanguageCommand::Formatting {
        path: p.into(),
        version: 7,
        range: range.then_some(RenameRange {
            start_line: 1,
            start_character: 0,
            end_line: 1,
            end_character: 8,
        }),
        tab_size: 4,
        insert_spaces: true,
    })
    .unwrap();
}
fn stop(s: &mut LanguageService) {
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(s, 5, |s| !s.is_busy());
}
#[test]
fn transport_negotiates_both_modes_and_rejects_untrusted_edits_without_mutation() {
    for mode in [
        "safe",
        "unsupported",
        "null",
        "surrogate",
        "overlap",
        "scope",
        "large",
        "error",
        "crash",
        "missing",
    ] {
        let (_d, mut s, p, log) = fixture(mode);
        let before = fs::read(&p).unwrap();
        query(&mut s, &p, false);
        wait(&mut s, 5, |s| !s.formatting_pending());
        if mode == "safe" {
            assert_eq!(s.formatting.as_ref().unwrap().edits.len(), 2);
        } else if mode == "null" {
            assert!(s.formatting.as_ref().unwrap().edits.is_empty());
        } else {
            assert!(s.formatting_error.is_some(), "{mode}");
            assert!(s.formatting.is_none(), "{mode}");
        }
        assert_eq!(fs::read(&p).unwrap(), before);
        stop(&mut s);
        let messages: Vec<serde_json::Value> = fs::read_to_string(log)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        if mode == "unsupported" {
            assert!(
                !messages
                    .iter()
                    .any(|v| v["method"] == "textDocument/formatting")
            );
        }
    }
}
#[test]
fn range_unicode_validation_cancellation_and_wire_options_are_real() {
    let (_d, mut s, p, log) = fixture("safe");
    assert!(s.formatting_capabilities[&Language::TypeScript].range);
    query(&mut s, &p, true);
    wait(&mut s, 5, |s| !s.formatting_pending());
    assert_eq!(s.formatting.as_ref().unwrap().edits.len(), 1);
    s.dispatch(LanguageCommand::Formatting {
        path: p.clone(),
        version: 7,
        range: Some(RenameRange {
            start_line: 0,
            start_character: 4,
            end_line: 0,
            end_character: 4,
        }),
        tab_size: 4,
        insert_spaces: true,
    })
    .unwrap();
    wait(&mut s, 5, |s| !s.formatting_pending());
    assert!(s.formatting_error.is_some());
    s.dispatch(LanguageCommand::CancelFormatting).unwrap();
    s.dispatch(LanguageCommand::CancelFormatting).unwrap();
    stop(&mut s);
    let records = fs::read_to_string(log).unwrap();
    let m = records
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .find(|v| v["method"] == "textDocument/rangeFormatting")
        .unwrap();
    assert_eq!(
        m["params"]["options"],
        serde_json::json!({"tabSize":4,"insertSpaces":true})
    );
    assert_eq!(m["params"]["range"]["start"]["line"], 1);
}
#[test]
fn silent_request_receives_real_cancel_on_deadline_and_stale_version_is_refused() {
    let (_d, mut s, p, log) = fixture("silent");
    assert!(
        s.dispatch(LanguageCommand::Formatting {
            path: p.clone(),
            version: 6,
            range: None,
            tab_size: 4,
            insert_spaces: true
        })
        .is_err()
    );
    query(&mut s, &p, false);
    wait(&mut s, 12, |s| !s.formatting_pending());
    assert!(s.formatting_error.as_ref().unwrap().contains("Timeout"));
    stop(&mut s);
    assert!(fs::read_to_string(log).unwrap().contains("$/cancelRequest"));
}
