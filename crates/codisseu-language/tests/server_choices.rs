use codisseu_language::{
    Language, LanguageCommand, LanguageService, ProviderConfig, ProviderState,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
fn wait(s: &mut LanguageService, check: impl Fn(&LanguageService) -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
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
        thread::sleep(Duration::from_millis(3));
    }
}
fn fixture(mode: &str) -> (tempfile::TempDir, LanguageService, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let source = root.join("main.ts");
    let target = root.join("target.ts");
    for p in [&source, &target] {
        fs::write(p, "export const target = 42;\n").unwrap();
    }
    let log = dir.path().join("messages.jsonl");
    let mut service = LanguageService::new();
    service
        .dispatch(LanguageCommand::Start {
            workspace: root,
            configs: vec![ProviderConfig {
                language: Language::TypeScript,
                program: "python3".into(),
                args: vec![
                    format!(
                        "{}/tests/fixtures/server_choice.py",
                        env!("CARGO_MANIFEST_DIR")
                    ),
                    mode.into(),
                    log.display().to_string(),
                    target.display().to_string(),
                ],
            }],
        })
        .unwrap();
    wait(&mut service, |s| {
        s.statuses.iter().any(|p| p.state == ProviderState::Ready)
    });
    service
        .dispatch(LanguageCommand::Sync {
            path: source.clone(),
            version: 1,
            text: "export const target = 42;\n".into(),
        })
        .unwrap();
    wait(&mut service, |s| !s.is_busy());
    (dir, service, source, log)
}
fn query(s: &mut LanguageService, path: &Path) {
    s.dispatch(LanguageCommand::Definition {
        path: path.into(),
        version: 1,
        line: 0,
        character: 14,
    })
    .unwrap();
}
fn records(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}
#[test]
fn explicit_choice_echoes_opaque_action_and_other_requests_remain_rejected() {
    let (_dir, mut s, path, log) = fixture("concurrent");
    query(&mut s, &path);
    wait(&mut s, |s| !s.server_choices.is_empty());
    let choice = s.server_choices[0].clone();
    assert_eq!(choice.items, ["First", "Second"]);
    assert_eq!(choice.message, "Choose inferred factory type");
    assert!(s.answer_server_choice(choice.token, Some(2)).is_err());
    assert_eq!(s.server_choices.len(), 1);
    s.answer_server_choice(choice.token, Some(1)).unwrap();
    assert!(s.server_choices.is_empty());
    wait(&mut s, |s| s.definition.is_some());
    let rows = records(&log);
    assert!(rows.iter().any(|r|r["result"]==json!({"title":"Second","opaque":{"command":"never-execute","arguments":["untrusted"]}})));
    assert!(
        rows.iter().any(|r| r["id"] == "extra-question"
            && r["result"].is_null()
            && r.get("error").is_none())
    );
    assert!(
        rows.iter()
            .any(|r| r["id"] == "unknown-request" && r["error"]["code"] == -32601)
    );
    assert!(s.answer_server_choice(choice.token, Some(0)).is_err());
}
#[test]
fn cancel_sync_epoch_and_stale_answers_never_choose_a_new_request() {
    let (_dir, mut s, path, log) = fixture("valid");
    query(&mut s, &path);
    wait(&mut s, |s| !s.server_choices.is_empty());
    let first = s.server_choices[0].token;
    s.dispatch(LanguageCommand::CancelDefinition).unwrap();
    assert!(s.server_choices.is_empty());
    wait(&mut s, |s| !s.is_busy());
    assert!(s.answer_server_choice(first, Some(0)).is_err());
    query(&mut s, &path);
    wait(&mut s, |s| !s.server_choices.is_empty());
    let second = s.server_choices[0].token;
    assert_ne!(first, second);
    s.dispatch(LanguageCommand::Sync {
        path: path.clone(),
        version: 2,
        text: "export const target = 43;\n".into(),
    })
    .unwrap();
    wait(&mut s, |s| !s.is_busy());
    assert!(s.server_choices.is_empty());
    assert!(s.answer_server_choice(second, Some(0)).is_err());
    s.dispatch(LanguageCommand::Definition {
        path,
        version: 2,
        line: 0,
        character: 14,
    })
    .unwrap();
    wait(&mut s, |s| !s.server_choices.is_empty());
    let third = s.server_choices[0].token;
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, |s| !s.is_busy());
    assert!(s.server_choices.is_empty());
    assert!(s.answer_server_choice(third, Some(0)).is_err());
    assert!(records(&log).iter().any(|r| {
        r["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("question-"))
            && r.get("result") == Some(&Value::Null)
    }));
}
#[test]
fn malformed_and_oversized_questions_are_bounded_without_auto_selecting() {
    for (mode, code) in [("malformed", Some(-32602)), ("overflow", None)] {
        let (_dir, mut s, path, log) = fixture(mode);
        query(&mut s, &path);
        wait(&mut s, |s| !s.definition_pending());
        assert!(s.server_choices.is_empty());
        let response = records(&log)
            .into_iter()
            .find(|r| {
                r["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("question-"))
            })
            .unwrap();
        if let Some(code) = code {
            assert_eq!(response["error"]["code"], code);
        } else {
            assert_eq!(response.get("result"), Some(&Value::Null));
        }
    }
}
