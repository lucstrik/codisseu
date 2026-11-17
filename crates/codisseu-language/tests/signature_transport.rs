use codisseu_language::{
    Language, LanguageCommand, LanguageService, ProviderConfig, ProviderState, SignatureHelpTrigger,
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
            s.signature_help_error
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
                    "{}/tests/fixtures/signature_server.py",
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
    s.dispatch(LanguageCommand::SignatureHelp {
        path: p.into(),
        version: 1,
        line: 0,
        character: 5,
        trigger: SignatureHelpTrigger::Invoked,
        is_retrigger: false,
    })
    .unwrap();
}
#[test]
fn negotiated_capabilities_inert_offsets_null_errors_and_crash() {
    for mode in ["safe", "unsupported", "null", "malformed", "error", "crash"] {
        let (_dir, mut s, path, log) = fixture(mode);
        query(&mut s, &path);
        wait(&mut s, 5, |s| !s.signature_help_pending());
        match mode {
            "safe" => {
                let r = s.signature_help.as_ref().unwrap();
                assert_eq!(r.active_parameter, Some(1));
                assert_eq!(
                    &r.signatures[0].label[r.signatures[0].parameters[0].label_range.clone()],
                    "🙂: string"
                );
                assert_eq!(
                    s.signature_help_capabilities[&Language::TypeScript].trigger_characters,
                    ["(", ","]
                );
            }
            "null" => assert!(s.signature_help.as_ref().unwrap().signatures.is_empty()),
            _ => {
                assert!(s.signature_help_error.is_some(), "{mode}");
                assert!(s.signature_help.is_none());
            }
        }
        s.dispatch(LanguageCommand::Stop).unwrap();
        wait(&mut s, 5, |s| !s.is_busy());
        let records = records(&log);
        let init = records
            .iter()
            .find(|m| m["method"] == "initialize")
            .unwrap();
        assert_eq!(
            init["params"]["capabilities"]["textDocument"]["signatureHelp"]["signatureInformation"]
                ["parameterInformation"]["labelOffsetSupport"],
            true
        );
        if mode == "unsupported" {
            assert!(
                !records
                    .iter()
                    .any(|m| m["method"] == "textDocument/signatureHelp")
            );
        }
    }
}
#[test]
fn retriggers_are_negotiated_and_positions_reject_surrogate_halves() {
    let (_dir, mut s, path, log) = fixture("safe");
    for (character, trigger, is_retrigger, valid) in [
        (5, ")", true, true),
        (5, ")", false, false),
        (1, "(", false, false),
    ] {
        s.dispatch(LanguageCommand::SignatureHelp {
            path: path.clone(),
            version: 1,
            line: 0,
            character,
            trigger: SignatureHelpTrigger::Character(trigger.into()),
            is_retrigger,
        })
        .unwrap();
        wait(&mut s, 5, |s| !s.signature_help_pending());
        assert_eq!(s.signature_help.is_some(), valid);
        assert_eq!(s.signature_help_error.is_some(), !valid);
    }
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, 5, |s| !s.is_busy());
    let records = records(&log);
    let requests: Vec<_> = records
        .iter()
        .filter(|m| m["method"] == "textDocument/signatureHelp")
        .collect();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]["params"]["context"],
        serde_json::json!({"triggerKind":2,"triggerCharacter":")","isRetrigger":true})
    );
}
#[test]
fn reorder_and_edit_invalidation_cancel_the_actual_wire_id() {
    for mode in ["reorder", "on_change"] {
        let (_dir, mut s, path, log) = fixture(mode);
        query(&mut s, &path);
        wait(&mut s, 5, |_| {
            records(&log)
                .iter()
                .any(|m| m["method"] == "textDocument/signatureHelp")
        });
        if mode == "reorder" {
            query(&mut s, &path);
        } else {
            s.dispatch(LanguageCommand::Sync {
                path: path.clone(),
                version: 2,
                text: "😀updated".into(),
            })
            .unwrap();
        }
        wait(&mut s, 5, |_| {
            records(&log).iter().any(|m| m.get("replySent").is_some())
        });
        wait(&mut s, 5, |s| !s.is_busy());
        assert_eq!(s.signature_help.is_some(), mode == "reorder");
        assert!(s.signature_help_error.is_none());
        s.dispatch(LanguageCommand::Stop).unwrap();
        wait(&mut s, 5, |s| !s.is_busy());
        let records = records(&log);
        let first = records
            .iter()
            .find(|m| m["method"] == "textDocument/signatureHelp")
            .unwrap();
        assert!(
            records
                .iter()
                .any(|m| m["method"] == "$/cancelRequest" && m["params"]["id"] == first["id"])
        );
    }
}
#[test]
fn silent_signature_times_out_then_cancels_the_wire_request() {
    let (_dir, mut s, path, log) = fixture("silent");
    query(&mut s, &path);
    wait(&mut s, 15, |s| !s.signature_help_pending());
    assert!(
        s.signature_help_error
            .as_ref()
            .unwrap()
            .contains("Timeout de assinatura")
    );
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, 5, |s| !s.is_busy());
    let records = records(&log);
    let first = records
        .iter()
        .find(|m| m["method"] == "textDocument/signatureHelp")
        .unwrap();
    assert!(
        records
            .iter()
            .any(|m| m["method"] == "$/cancelRequest" && m["params"]["id"] == first["id"])
    );
}
