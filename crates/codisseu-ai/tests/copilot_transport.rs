#![cfg(unix)]
use codisseu_ai::copilot::*;
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
fn fixture(options: Value) -> (tempfile::TempDir, PathBuf, Client) {
    let d = tempfile::tempdir().unwrap();
    let program = d.path().join("server.py");
    fs::write(&program, include_str!("fixtures/copilot_server.py")).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(program.with_extension("json"), options.to_string()).unwrap();
    let c = Client::start(program.clone(), vec![d.path().into()]).unwrap();
    (d, program, c)
}
fn wait(c: &mut Client, f: impl Fn(&Client) -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        c.poll();
        if f(c) {
            break;
        }
        assert!(Instant::now() < end, "timeout: {:?}", c.error);
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn doc(v: u64, text: &str) -> Document {
    Document {
        uri: "file:///owned.php".into(),
        version: v,
        language: "php".into(),
        text: Arc::from(text),
        position: position(text, text.chars().count()).unwrap(),
    }
}
fn records(p: &std::path::Path) -> Vec<Value> {
    fs::read_to_string(p.with_extension("log"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
#[test]
fn copilot_protocol_sync_cancels_stale_results_and_reports_only_explicit_acceptance() {
    let (_d, p, mut c) = fixture(json!({"old_delay":0.25,"delay":0.02}));
    wait(&mut c, |c| c.ready);
    c.sign_in().unwrap();
    wait(&mut c, |c| c.authenticated);
    c.complete(doc(1, "😀 before"), false).unwrap();
    std::thread::sleep(Duration::from_millis(30));
    let ticket = c.complete(doc(2, "😀 after"), true).unwrap();
    wait(&mut c, |c| c.suggestion.is_some());
    assert_eq!(c.suggestion.as_ref().unwrap().ticket, ticket);
    std::thread::sleep(Duration::from_millis(300));
    c.poll();
    assert_eq!(c.suggestion.as_ref().unwrap().document.version, 2);
    let s = c.suggestion.clone().unwrap();
    c.shown(s.item.clone());
    c.accepted(json!({"command":{"command":"arbitrary.command","arguments":["unsafe"]}}));
    std::thread::sleep(Duration::from_millis(20));
    assert!(
        !records(&p)
            .iter()
            .any(|v| v["method"] == "workspace/executeCommand")
    );
    c.accepted(s.item);
    std::thread::sleep(Duration::from_millis(30));
    let rows = records(&p);
    assert_eq!(
        rows.iter().find(|v| v.get("method").is_some()).unwrap()["method"],
        "initialize"
    );
    assert!(rows.iter().any(|v| v["method"] == "$/cancelRequest"));
    assert!(
        rows.iter()
            .any(|v| v["synced"] == "file:///owned.php" && v["text"] == "😀 after")
    );
    assert!(
        rows.iter()
            .any(|v| v["method"] == "textDocument/didShowCompletion")
    );
    assert_eq!(
        rows.iter()
            .filter(|v| v["method"] == "workspace/executeCommand")
            .count(),
        1
    );
    c.document(Document {
        uri: "file:///owned.md".into(),
        language: "markdown".into(),
        ..doc(3, "# Title")
    })
    .unwrap();
    std::thread::sleep(Duration::from_millis(30));
    assert!(
        records(&p)
            .iter()
            .any(|v| v["method"] == "textDocument/didClose")
    );
}
#[test]
fn copilot_device_flow_messages_and_sign_out_are_native_client_actions() {
    let (_d, p, mut c) = fixture(json!({"login":true,"message":true}));
    wait(&mut c, |c| c.ready && !c.messages.is_empty());
    c.reply(c.messages[0].id.clone(), json!({"title":"OK"}));
    c.sign_in().unwrap();
    wait(&mut c, |c| c.login.is_some());
    assert_eq!(c.login.as_ref().unwrap().code, "TEST-CODE");
    assert!(!c.authenticated);
    c.finish_login().unwrap();
    wait(&mut c, |c| c.authenticated && c.open_url.is_some());
    let (id, url) = c.open_url.take().unwrap();
    assert_eq!(url, "https://github.com/login/device");
    c.reply(id, json!({"success":true}));
    c.sign_out().unwrap();
    wait(&mut c, |c| {
        !c.authenticated && c.status == "Conta Copilot desconectada"
    });
    assert!(records(&p).iter().any(|v| v["method"] == "signOut"));
}

#[test]
fn copilot_device_flow_waits_for_browser_request_and_suppresses_duplicate_continue() {
    let (_d, p, mut c) = fixture(json!({"login":true,"finish_after_show_document_reply":true}));
    wait(&mut c, |c| c.ready);
    c.sign_in().unwrap();
    wait(&mut c, |c| c.login.is_some());
    c.finish_login().unwrap();
    c.finish_login().unwrap();
    assert!(c.login_in_progress);
    assert!(c.status.contains("navegador"));
    wait(&mut c, |c| c.open_url.is_some());
    assert!(!c.authenticated);
    let (id, url) = c.open_url.take().unwrap();
    assert_eq!(url, "https://github.com/login/device");
    assert_eq!(
        records(&p)
            .iter()
            .filter(|r| r["method"] == "workspace/executeCommand")
            .count(),
        1
    );
    c.reply(id, json!({"success":true}));
    wait(&mut c, |c| c.authenticated);
    assert!(!c.login_in_progress);
    assert!(c.login.is_none());
}
#[test]
fn copilot_drop_reaps_its_owned_process_group() {
    let (_d, p, mut c) = fixture(json!({"spawn_child":true}));
    wait(&mut c, |c| c.ready);
    let rows = records(&p);
    let pids = rows
        .iter()
        .filter_map(|v| v["pid"].as_u64().or_else(|| v["child"].as_u64()))
        .collect::<Vec<_>>();
    assert_eq!(pids.len(), 2);
    drop(c);
    let end = Instant::now() + Duration::from_secs(3);
    for pid in pids {
        loop {
            let state = fs::read_to_string(format!("/proc/{pid}/stat")).ok();
            if state
                .as_ref()
                .is_none_or(|s| s.split(") ").nth(1).is_some_and(|s| s.starts_with("Z ")))
            {
                break;
            }
            assert!(Instant::now() < end, "owned process survived: {pid}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[test]
fn copilot_does_not_confuse_a_running_server_or_invalid_login_with_account_access() {
    for result in [
        json!({}),
        json!({"status":"NotAuthorized"}),
        json!({"userCode":"TEST","command":{"command":"arbitrary.command"}}),
    ] {
        let (_d, _p, mut c) = fixture(json!({"login_result":result}));
        wait(&mut c, |c| c.ready);
        assert!(!c.authenticated);
        assert!(!c.status.contains("conectado"));
        c.sign_in().unwrap();
        wait(&mut c, |c| c.error.is_some());
        assert!(!c.authenticated);
        assert!(c.login.is_none());
        assert_eq!(c.kind, "Error");
    }
    let (_d, _p, mut c) = fixture(json!({"login_error":true}));
    wait(&mut c, |c| c.ready);
    c.sign_in().unwrap();
    wait(&mut c, |c| c.error.is_some());
    assert!(!c.authenticated);
    assert_eq!(c.status, "Copilot não autenticado");
}

#[test]
fn copilot_finish_command_rechecks_account_instead_of_assuming_success() {
    let (_d, p, mut c) = fixture(json!({"login":true,"finish_no_auth":true}));
    wait(&mut c, |c| c.ready);
    c.sign_in().unwrap();
    wait(&mut c, |c| c.login.is_some());
    c.finish_login().unwrap();
    wait(&mut c, |c| c.open_url.is_some());
    let until = Instant::now() + Duration::from_secs(3);
    while records(&p)
        .iter()
        .filter(|r| r["method"] == "signIn")
        .count()
        < 2
    {
        c.poll();
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(5));
    }
    c.poll();
    assert!(!c.authenticated);
    assert!(c.login.is_some());
}

#[test]
fn copilot_semantic_context_is_optional_and_expired_account_drops_code_preview() {
    let (_d, p, mut c) = fixture(json!({"delay":0.02,"revoke_after_completion":true}));
    wait(&mut c, |c| c.ready);
    c.sign_in().unwrap();
    wait(&mut c, |c| c.authenticated);
    let selected = json!({"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}},"text":"return"});
    c.complete_with_context(doc(2, "ret"), false, Some(selected.clone()))
        .unwrap();
    wait(&mut c, |c| c.kind == "Error");
    assert!(!c.authenticated);
    assert!(!c.busy);
    assert!(c.suggestion.is_none());
    assert_eq!(
        records(&p)
            .iter()
            .find(|r| r["method"] == "textDocument/inlineCompletion")
            .unwrap()["params"]["context"]["selectedCompletionInfo"],
        selected
    );
}

#[test]
fn copilot_cycles_returned_alternatives_and_partial_acceptance_preserves_only_the_remainder() {
    let (_d, p, mut c) =
        fixture(json!({"suggestions":["first 😀word\nnext","second","third"],"delay":0.02}));
    wait(&mut c, |c| c.ready);
    c.sign_in().unwrap();
    wait(&mut c, |c| c.authenticated);
    c.complete(doc(2, "ret"), true).unwrap();
    wait(&mut c, |c| c.suggestion.is_some());
    assert_eq!((c.suggestion_index, c.suggestion_count), (0, 3));
    assert!(c.cycle(1));
    assert_eq!(c.suggestion.as_ref().unwrap().suffix, "second");
    assert!(c.cycle(-2));
    assert_eq!(c.suggestion_index, 2);
    assert!(c.cycle(1));
    assert_eq!(c.suggestion_index, 0);
    assert!(!c.cycle(0));
    let mut remainder = c.suggestion.clone().unwrap();
    let accepted = "first 😀";
    let length = accepted.encode_utf16().count() as u32;
    c.partially_accepted(remainder.item.clone(), length);
    remainder.document = doc(3, &format!("ret{accepted}"));
    remainder.start = remainder.document.text.chars().count();
    remainder.end = remainder.start;
    remainder.insert = "word\nnext".into();
    remainder.suffix = remainder.insert.clone();
    let mut wrong = remainder.clone();
    wrong.document.uri = "file:///different.php".into();
    assert!(!c.continue_suggestion(wrong));
    let mut wrong = remainder.clone();
    wrong.suffix = "not the insert delta".into();
    assert!(!c.continue_suggestion(wrong));
    let mut wrong = remainder.clone();
    wrong.insert = "different code".into();
    wrong.suffix = wrong.insert.clone();
    assert!(!c.continue_suggestion(wrong));
    assert!(c.continue_suggestion(remainder.clone()));
    assert_eq!((c.suggestion_index, c.suggestion_count), (0, 1));
    assert!(!c.cycle(1));
    wait(&mut c, |_| {
        records(&p)
            .iter()
            .any(|r| r["method"] == "textDocument/didPartiallyAcceptCompletion")
    });
    let rows = records(&p);
    let notification = rows
        .iter()
        .find(|r| r["method"] == "textDocument/didPartiallyAcceptCompletion")
        .unwrap();
    assert_eq!(notification["params"]["acceptedLength"], length);
    assert_eq!(notification["params"]["item"], remainder.item);
    assert!(
        !rows
            .iter()
            .any(|r| r["method"] == "workspace/executeCommand")
    );
    assert_eq!(
        rows.iter()
            .filter(|r| r["method"] == "textDocument/inlineCompletion")
            .count(),
        1
    );
    c.cancel();
    assert_eq!((c.suggestion_index, c.suggestion_count), (0, 0));
    assert!(c.suggestion.is_none());
    assert!(!c.cycle(1));
    assert!(!c.continue_suggestion(remainder));
}

#[test]
fn copilot_alternatives_are_bounded_and_stale_responses_cannot_repopulate_the_list() {
    let options = json!({"suggestions":(0..12).map(|i|format!("alternative {i}")).collect::<Vec<_>>(),"old_delay":0.25,"delay":0.02});
    let (_d, _p, mut c) = fixture(options);
    wait(&mut c, |c| c.ready);
    c.sign_in().unwrap();
    wait(&mut c, |c| c.authenticated);
    c.complete(doc(1, "old"), false).unwrap();
    std::thread::sleep(Duration::from_millis(30));
    let ticket = c.complete(doc(2, "new"), false).unwrap();
    wait(&mut c, |c| c.suggestion.is_some());
    assert_eq!(c.suggestion_count, 8);
    assert!(c.cycle(-1));
    assert_eq!(c.suggestion_index, 7);
    assert_eq!(c.suggestion.as_ref().unwrap().ticket, ticket);
    assert_eq!(c.suggestion.as_ref().unwrap().document.version, 2);
    c.cancel();
    std::thread::sleep(Duration::from_millis(300));
    c.poll();
    assert_eq!(c.suggestion_count, 0);
    assert!(c.suggestion.is_none());
}

#[test]
fn copilot_revalidates_once_after_authenticated_error_then_normal_status() {
    let mut kinds = vec!["Error", "Error"];
    kinds.extend(std::iter::repeat_n("Normal", 20));
    let (_d, p, mut c) = fixture(json!({"statuses_after_completion":kinds,"recovery_delay":0.04}));
    wait(&mut c, |c| c.ready);
    c.sign_in().unwrap();
    wait(&mut c, |c| c.authenticated);
    c.complete(doc(2, "ret"), false).unwrap();
    wait(&mut c, |c| {
        c.authenticated
            && records(&p)
                .iter()
                .filter(|r| r["method"] == "signIn")
                .count()
                == 2
    });
    std::thread::sleep(Duration::from_millis(100));
    c.poll();
    assert!(c.authenticated);
    assert!(
        c.suggestion.is_none(),
        "expired preview must remain discarded after account recovery"
    );
    assert_eq!(
        records(&p)
            .iter()
            .filter(|r| r["method"] == "signIn")
            .count(),
        2
    );
}

#[test]
fn copilot_status_revalidation_does_not_loop_on_challenge_or_failure() {
    for challenge in [true, false] {
        let mut options = json!({"statuses_after_completion":["Error","Normal","Normal","Normal"],"recovery_delay":0.02});
        if challenge {
            options["recovery_login_result"] = json!({"userCode":"TEST-CODE","command":{"command":"github.copilot.finishDeviceFlow","arguments":[]}});
        } else {
            options["recovery_login_error"] = json!(true);
        }
        let (_d, p, mut c) = fixture(options);
        wait(&mut c, |c| c.ready);
        c.sign_in().unwrap();
        wait(&mut c, |c| c.authenticated);
        c.complete(doc(2, "ret"), false).unwrap();
        wait(&mut c, |c| {
            if challenge {
                c.login.is_some()
            } else {
                c.error.is_some()
            }
        });
        std::thread::sleep(Duration::from_millis(100));
        c.poll();
        assert!(!c.authenticated);
        assert!(c.suggestion.is_none());
        assert_eq!(
            records(&p)
                .iter()
                .filter(|r| r["method"] == "signIn")
                .count(),
            2
        );
        assert!(
            !records(&p)
                .iter()
                .any(|r| r["method"] == "workspace/executeCommand")
        );
    }
}

#[test]
fn copilot_auxiliary_error_cannot_finish_a_newer_completion() {
    let (_d, p, mut c) = fixture(
        json!({"old_delay":0.02,"delay":0.4,"accept_error":true,"accept_error_delay":0.06}),
    );
    wait(&mut c, |c| c.ready);
    c.sign_in().unwrap();
    wait(&mut c, |c| c.authenticated);
    c.complete(doc(1, "first"), false).unwrap();
    wait(&mut c, |c| c.suggestion.is_some());
    c.accepted(c.suggestion.as_ref().unwrap().item.clone());
    let ticket = c.complete(doc(2, "second"), false).unwrap();
    wait(&mut c, |c| c.error.is_some());
    assert!(
        c.busy,
        "telemetry failure must not finish a different request"
    );
    assert!(c.authenticated);
    wait(&mut c, |c| c.suggestion.is_some());
    assert_eq!(c.suggestion.as_ref().unwrap().ticket, ticket);
    assert!(!c.busy);
    assert!(
        c.error.is_none(),
        "successful current result clears a prior auxiliary error"
    );
    assert_eq!(
        records(&p)
            .iter()
            .filter(|v| v["method"] == "textDocument/inlineCompletion")
            .count(),
        2
    );
}

#[test]
fn copilot_current_completion_error_finishes_only_its_request_and_recovers_on_edit() {
    let (_d, _p, mut c) =
        fixture(json!({"completion_error_versions":[1],"old_delay":0.02,"delay":0.02}));
    wait(&mut c, |c| c.ready);
    c.sign_in().unwrap();
    wait(&mut c, |c| c.authenticated);
    c.complete(doc(1, "first"), false).unwrap();
    wait(&mut c, |c| c.error.is_some());
    assert!(!c.busy);
    assert!(c.authenticated && c.ready);
    assert!(c.suggestion.is_none());
    let ticket = c.complete(doc(2, "second"), false).unwrap();
    wait(&mut c, |c| c.suggestion.is_some());
    assert_eq!(c.suggestion.as_ref().unwrap().ticket, ticket);
    assert!(c.error.is_none());
}
