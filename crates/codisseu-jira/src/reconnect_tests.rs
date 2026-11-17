use super::*;
use crate::tests::{Server, wait};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn connection() -> Connection {
    Connection {
        site: "https://fixture.atlassian.net".into(),
        api: "https://fixture.atlassian.net".into(),
        email: format!("restore-{}@example.invalid", std::process::id()),
    }
}
fn controlled(path: PathBuf, p: Profile) -> (Service, SyncSender<Response>, Receiver<Request>) {
    let mut s = Service::new(path);
    wait(&mut s);
    let (requests, jobs) = mpsc::sync_channel(1);
    let (replies, responses) = mpsc::sync_channel(1);
    s.tx = Some(requests);
    s.rx = responses;
    s.pending = true;
    replies
        .send(Response {
            epoch: s.epoch,
            result: Ok(Reply::Loaded(Box::new(p))),
            write: false,
            scope: Scope::Global,
        })
        .unwrap();
    s.poll();
    (s, replies, jobs)
}
#[test]
fn startup_restores_once_only_when_enabled_configured_and_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let legacy: Profile =
        serde_json::from_value(json!({"enabled":true,"connection":connection()})).unwrap();
    assert!(legacy.auto_connect);
    for mode in 0..4 {
        let mut p = legacy.clone();
        match mode {
            1 => p.enabled = false,
            2 => p.connection = Connection::default(),
            3 => p.auto_connect = false,
            _ => (),
        }
        let (mut s, replies, jobs) = controlled(dir.path().join(format!("{mode}.json")), p);
        if mode == 0 {
            let request = jobs.try_recv().unwrap();
            assert!(matches!(request.op, Op::Restore(_)));
            assert!(s.restoring());
            replies
                .send(Response {
                    epoch: request.epoch,
                    result: Ok(Reply::NoSavedToken),
                    write: false,
                    scope: Scope::Global,
                })
                .unwrap();
        } else {
            assert!(!s.restoring());
        }
        for _ in 0..100 {
            s.poll();
        }
        assert!(jobs.try_recv().is_err());
        assert!(!s.connected && !s.restoring() && !s.main_busy());
        assert!(s.error.is_none());
    }
}
#[test]
fn restore_errors_do_not_loop_or_erase_saved_connection() {
    let dir = tempfile::tempdir().unwrap();
    for message in [
        "Cofre bloqueado",
        "Jira HTTP 401",
        "Tempo de conexão excedido",
    ] {
        let p = Profile {
            enabled: true,
            connection: connection(),
            ..Default::default()
        };
        let (mut s, replies, jobs) = controlled(dir.path().join("jira.json"), p);
        let request = jobs.try_recv().unwrap();
        replies
            .send(Response {
                epoch: request.epoch,
                result: Err(message.into()),
                write: false,
                scope: Scope::Global,
            })
            .unwrap();
        for _ in 0..100 {
            s.poll();
        }
        assert_eq!(s.error.as_deref(), Some(message));
        assert!(s.profile.auto_connect && !s.connected && !s.restoring());
        assert!(jobs.try_recv().is_err());
    }
}
#[test]
fn disconnect_during_restore_rejects_late_success() {
    let dir = tempfile::tempdir().unwrap();
    let p = Profile {
        enabled: true,
        connection: connection(),
        ..Default::default()
    };
    let (mut s, replies, jobs) = controlled(dir.path().join("jira.json"), p);
    let request = jobs.try_recv().unwrap();
    s.disconnect();
    let client = JiraClient::fixture("http://127.0.0.1:1".into());
    replies
        .send(Response {
            epoch: request.epoch,
            result: Ok(Reply::Connected(
                Identity {
                    account: "fixture".into(),
                    name: "Fixture".into(),
                    avatar: String::new(),
                    team_field: None,
                    team_fields: vec![],
                },
                None,
                None,
                Box::new(client),
                true,
            )),
            write: false,
            scope: Scope::Global,
        })
        .unwrap();
    s.poll();
    assert!(!s.connected && !s.profile.auto_connect && !s.restoring());
    assert!(s.identity.is_none() && s.page.issues.is_empty());
    assert!(matches!(jobs.try_recv().unwrap().op, Op::Disconnect));
}
fn private_vault() {
    assert_eq!(
        std::env::var("CODISSEU_BROWSER_VAULT_TEST").as_deref(),
        Ok("1")
    );
}
fn service(path: &Path, base: &str) -> Service {
    let base = base.to_string();
    Service::with_client(path.to_path_buf(), move |_, token| {
        assert_eq!(token.expose(), "fixture-token");
        Ok(JiraClient::fixture(base.clone()))
    })
}
fn server(expired: Arc<AtomicBool>) -> Server {
    Server::new(move |r| {
        if expired.load(Ordering::SeqCst) {
            return (401, String::new(), "{}".into());
        }
        let body = if r.path.ends_with("/myself") {
            json!({"accountId":"fixture","displayName":"Fixture","active":true})
        } else if r.path.ends_with("/field") {
            json!([])
        } else {
            assert!(r.path.ends_with("/search/jql") && r.method == "POST");
            assert_eq!(r.body["maxResults"], 50);
            json!({"issues":[],"isLast":true})
        };
        (200, String::new(), body.to_string())
    })
}
#[test]
#[ignore = "requires explicit disposable Secret Service; never run against a personal vault"]
fn jira_vault_automatic_restart_disconnect_and_forget() {
    private_vault();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("jira.json");
    let server = server(Arc::new(AtomicBool::new(false)));
    let c = connection();
    let mut s = service(&path, &server.base);
    wait(&mut s);
    let mut p = s.profile.clone();
    p.enabled = true;
    s.set_profile(p).unwrap();
    s.connect(c.clone(), Some(Secret::new("fixture-token".into())), true)
        .unwrap();
    wait(&mut s);
    assert!(s.connected && s.profile.auto_connect && s.notice.is_none());
    drop(s);
    // Legacy profiles have no auto_connect field, but their saved token must work.
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    legacy.as_object_mut().unwrap().remove("auto_connect");
    std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let requests = server.requests.lock().unwrap().len();
    let mut s = service(&path, &server.base);
    wait(&mut s);
    assert!(s.connected && s.identity.as_ref().unwrap().account == "fixture");
    assert_eq!(server.requests.lock().unwrap().len() - requests, 3);
    s.disconnect();
    wait(&mut s);
    drop(s);
    let requests = server.requests.lock().unwrap().len();
    let mut s = service(&path, &server.base);
    wait(&mut s);
    assert!(!s.connected && !s.profile.auto_connect);
    assert_eq!(server.requests.lock().unwrap().len(), requests);
    // Explicitly reconnect using only the vault, never retype a token.
    s.connect(c.clone(), None, false).unwrap();
    wait(&mut s);
    assert!(s.connected && s.profile.auto_connect);
    s.forget();
    wait(&mut s);
    assert!(s.error.is_none());
    drop(s);
    assert!(saved_token(&c).unwrap().is_none());
    let mut s = service(&path, &server.base);
    wait(&mut s);
    assert!(!s.connected && !s.profile.auto_connect);
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains("fixture-token")
    );
}
#[test]
#[ignore = "requires explicit disposable Secret Service; never run against a personal vault"]
fn jira_vault_missing_expired_and_session_only_never_retry() {
    private_vault();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("jira.json");
    let expired = Arc::new(AtomicBool::new(false));
    let server = server(expired.clone());
    let c = connection();
    let p = Profile {
        enabled: true,
        connection: c.clone(),
        ..Default::default()
    };
    std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
    let mut s = service(&path, &server.base);
    wait(&mut s);
    assert!(!s.connected && s.error.is_none());
    assert!(server.requests.lock().unwrap().is_empty());
    s.connect(c.clone(), Some(Secret::new("fixture-token".into())), false)
        .unwrap();
    wait(&mut s);
    assert!(s.connected && !s.profile.auto_connect);
    drop(s);
    assert!(saved_token(&c).unwrap().is_none());
    let requests = server.requests.lock().unwrap().len();
    let mut s = service(&path, &server.base);
    wait(&mut s);
    assert!(!s.connected);
    assert_eq!(server.requests.lock().unwrap().len(), requests);
    s.connect(c.clone(), Some(Secret::new("fixture-token".into())), true)
        .unwrap();
    wait(&mut s);
    drop(s);
    expired.store(true, Ordering::SeqCst);
    let requests = server.requests.lock().unwrap().len();
    let mut s = service(&path, &server.base);
    wait(&mut s);
    assert!(!s.connected && s.error.is_some());
    for _ in 0..100 {
        s.poll();
    }
    assert_eq!(server.requests.lock().unwrap().len() - requests, 1);
    assert!(saved_token(&c).unwrap().is_some());
    expired.store(false, Ordering::SeqCst);
    s.connect(c.clone(), None, false).unwrap();
    wait(&mut s);
    assert!(s.connected);
    s.forget();
    wait(&mut s);
}
