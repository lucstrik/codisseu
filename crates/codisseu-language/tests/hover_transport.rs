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

fn wait(
    service: &mut LanguageService,
    timeout: Duration,
    condition: impl Fn(&LanguageService) -> bool,
) {
    let deadline = Instant::now() + timeout;
    loop {
        service.poll();
        if condition(service) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timeout; error {:?}, hover error {:?}, statuses {:?}",
            service.last_error,
            service.hover_error,
            service.statuses
        );
        thread::sleep(Duration::from_millis(10));
    }
}
fn messages(path: &std::path::Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}
fn fixture(mode: &str) -> (tempfile::TempDir, LanguageService, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = root.join("fixture.ts");
    fs::write(&path, "const name = 42;\n").unwrap();
    let log = root.join("messages.jsonl");
    let mut service = LanguageService::new();
    service
        .dispatch(LanguageCommand::Start {
            workspace: root,
            configs: vec![ProviderConfig {
                language: Language::TypeScript,
                program: "python3".into(),
                args: vec![
                    format!(
                        "{}/tests/fixtures/hover_server.py",
                        env!("CARGO_MANIFEST_DIR")
                    ),
                    mode.into(),
                    log.to_str().unwrap().into(),
                ],
            }],
        })
        .unwrap();
    wait(&mut service, Duration::from_secs(5), |s| {
        s.statuses.iter().any(|s| s.state == ProviderState::Ready)
    });
    service
        .dispatch(LanguageCommand::Sync {
            path: path.clone(),
            version: 1,
            text: "const name = 42;\n".into(),
        })
        .unwrap();
    wait(&mut service, Duration::from_secs(5), |s| !s.is_busy());
    (temp, service, path, log)
}
fn request(service: &mut LanguageService, path: &std::path::Path) {
    service
        .dispatch(LanguageCommand::Hover {
            path: path.into(),
            version: 1,
            line: 0,
            character: 7,
        })
        .unwrap();
}
#[test]
fn absent_hover_capability_rejects_without_sending_request() {
    let (_temp, mut service, path, log) = fixture("unsupported");
    request(&mut service, &path);
    wait(&mut service, Duration::from_secs(5), |s| !s.hover_pending());
    assert!(
        service
            .hover_error
            .as_ref()
            .unwrap()
            .contains("não oferece hover")
    );
    assert!(service.hover.is_none());
    service.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut service, Duration::from_secs(5), |s| !s.is_busy());
    assert!(
        !messages(&log)
            .iter()
            .any(|v| v["method"] == "textDocument/hover")
    );
}
#[test]
fn silent_provider_receives_cancel_and_real_deadline_finishes_pending_hover() {
    let (_temp, mut service, path, log) = fixture("silent");
    request(&mut service, &path);
    wait(&mut service, Duration::from_secs(5), |_| {
        messages(&log)
            .iter()
            .any(|v| v["method"] == "textDocument/hover")
    });
    service.dispatch(LanguageCommand::CancelHover).unwrap();
    assert!(!service.hover_pending());
    wait(&mut service, Duration::from_secs(5), |_| {
        messages(&log)
            .iter()
            .any(|v| v["method"] == "$/cancelRequest")
    });
    request(&mut service, &path);
    let started = Instant::now();
    wait(&mut service, Duration::from_secs(15), |s| {
        !s.hover_pending()
    });
    assert!(started.elapsed() >= Duration::from_secs(9));
    assert!(
        service
            .hover_error
            .as_ref()
            .unwrap()
            .contains("Timeout de hover após 10 segundos")
    );
    assert!(service.hover.is_none());
    service.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut service, Duration::from_secs(5), |s| !s.is_busy());
    let messages = messages(&log);
    let requests: Vec<_> = messages
        .iter()
        .filter(|v| v["method"] == "textDocument/hover")
        .collect();
    let cancellations: Vec<_> = messages
        .iter()
        .filter(|v| v["method"] == "$/cancelRequest")
        .collect();
    assert_eq!(requests.len(), 2);
    assert_eq!(cancellations.len(), 2);
    assert_ne!(requests[0]["id"], requests[1]["id"]);
    assert_eq!(requests[0]["id"], cancellations[0]["params"]["id"]);
    assert_eq!(requests[1]["id"], cancellations[1]["params"]["id"]);
}
