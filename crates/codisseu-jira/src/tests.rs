use super::*;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
};

#[derive(Clone, Debug)]
pub(super) struct HttpRequest {
    pub method: String,
    pub path: String,
    pub body: Value,
}
pub(super) struct Server {
    pub base: String,
    pub requests: Arc<Mutex<Vec<HttpRequest>>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}
impl Server {
    pub(super) fn new(
        f: impl Fn(&HttpRequest) -> (u16, String, String) + Send + Sync + 'static,
    ) -> Self {
        Self::with_keep_alive(false, f)
    }
    pub(super) fn keep_alive(
        f: impl Fn(&HttpRequest) -> (u16, String, String) + Send + Sync + 'static,
    ) -> Self {
        Self::with_keep_alive(true, f)
    }
    fn with_keep_alive(
        keep_alive: bool,
        f: impl Fn(&HttpRequest) -> (u16, String, String) + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(vec![]));
        let seen = requests.clone();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done = stop.clone();
        let f = Arc::new(f);
        let thread = thread::spawn(move || {
            let mut clients = Vec::new();
            while !done.load(std::sync::atomic::Ordering::Relaxed) {
                let (mut s, _) = match listener.accept() {
                    Ok(s) => s,
                    Err(_) => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                };
                let seen = seen.clone();
                let f = f.clone();
                clients.push(thread::spawn(move || {
                s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                loop {
                let mut b = vec![];
                let mut buf = [0; 4096];
                let mut length = None;
                loop {
                    let n = s.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    b.extend_from_slice(&buf[..n]);
                    if let Some(i) = b.windows(4).position(|w| w == b"\r\n\r\n") {
                        let h = String::from_utf8_lossy(&b[..i]);
                        let size = h
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        length = Some((i + 4, size));
                        if b.len() >= i + 4 + size {
                            break;
                        }
                    }
                }
                let Some((i, n)) = length else { return };
                let head = String::from_utf8_lossy(&b[..i]);
                let first: Vec<_> = head.lines().next().unwrap().split_whitespace().collect();
                let r = HttpRequest {
                    method: first[0].into(),
                    path: first[1].into(),
                    body: if n > 0 {
                        serde_json::from_slice(&b[i..i + n]).unwrap()
                    } else {
                        Value::Null
                    },
                };
                seen.lock().unwrap().push(r.clone());
                let (status, headers, body) = f(&r);
                let message = format!(
                    "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nConnection: {}\r\nContent-Length: {}\r\n{headers}\r\n{body}",
                    if keep_alive { "keep-alive" } else { "close" }, body.len()
                );
                if s.write_all(message.as_bytes()).is_err() || !keep_alive { return; }
                }
                }));
            }
            for client in clients {
                client.join().unwrap();
            }
        });
        Self {
            base,
            requests,
            stop,
            thread: Some(thread),
        }
    }
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
pub(super) fn wait(s: &mut Service) {
    let end = Instant::now() + Duration::from_secs(5);
    while s.is_busy() {
        s.poll();
        assert!(Instant::now() < end, "{:?}", s.error);
        thread::sleep(Duration::from_millis(4))
    }
}
#[test]
fn filters_are_bounded_literal_and_default_mine() {
    let f = Filters::default();
    assert_eq!(
        f.jql(None).unwrap(),
        "assignee = currentUser() AND statusCategory != Done ORDER BY updated DESC, key ASC"
    );
    let f = Filters {
        mine: false,
        status: Status::All,
        ..Default::default()
    };
    assert!(f.jql(None).unwrap().contains("created >="));
    let f = Filters {
        text: "x\" OR assignee = admin +(*?)".into(),
        project: Some(Choice {
            id: "42\" OR project = X".into(),
            name: "safe".into(),
        }),
        ..Default::default()
    };
    let q = f.jql(None).unwrap();
    assert!(q.contains("project = \"42\\\" OR project = X\""));
    assert!(q.contains("\\\\+"));
    let mut f = Filters {
        team: Some(Choice {
            id: "team-123".into(),
            name: "Squad".into(),
        }),
        ..Default::default()
    };
    assert!(f.jql(None).is_err());
    assert!(
        f.jql(Some("customfield_10001"))
            .unwrap()
            .contains("cf[10001] = \"team-123\"")
    );
    assert!(f.jql(Some("bad OR true")).is_err());
    f.text = "abc-123".into();
    assert!(
        f.jql(Some("customfield_10001"))
            .unwrap()
            .contains("key = \"ABC-123\"")
    );
}
#[test]
fn connection_rejects_credentials_redirect_urls_and_gateway_confusion() {
    for site in [
        "http://example.com",
        "https://user:pass@example.com",
        "https://example.com?secret=1",
        "https://example.com/a",
    ] {
        let mut c = Connection {
            site: site.into(),
            email: "me@example.com".into(),
            api: String::new(),
        };
        assert!(c.validate().is_err())
    }
    let mut c = Connection {
        site: "https://example.atlassian.net/".into(),
        email: "me@example.com".into(),
        api: "https://api.atlassian.com/ex/jira/cloud-123".into(),
    };
    c.validate().unwrap();
    assert_eq!(c.site, "https://example.atlassian.net");
    c.api = "https://evil.example/ex/jira/id".into();
    assert!(c.validate().is_err());
}
#[test]
fn adf_formats_hierarchy_code_tables_mentions_and_blocks_scripts() {
    let v = json!({"type":"doc","content":[{"type":"heading","attrs":{"level":2},"content":[{"type":"text","text":"Olá"}]},{"type":"paragraph","content":[{"type":"text","text":"importante","marks":[{"type":"strong"}]},{"type":"mention","attrs":{"text":"@Pessoa"}},{"type":"text","text":"unsafe","marks":[{"type":"link","attrs":{"href":"javascript:alert(1)"}}]}]},{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"Um"}]}]}]},{"type":"codeBlock","content":[{"type":"text","text":"echo `x`;"}]},{"type":"table","content":[{"type":"tableRow","content":[{"type":"tableHeader","content":[{"type":"text","text":"A"}]}]},{"type":"tableRow","content":[{"type":"tableCell","content":[{"type":"text","text":"B"}]}]}]},{"type":"media"}]});
    let m = content::markdown(&v);
    assert!(m.contains("## Olá"));
    assert!(m.contains("**importante**@Pessoa"));
    assert!(m.contains("- Um"));
    assert!(m.contains("```\necho `x`;"));
    assert!(m.contains("| --- |"));
    assert!(!m.contains("javascript:"));
    assert!(m.contains("[Anexo disponível na seção Anexos do card]"));
    assert_eq!(
        content::markdown(&json!("# Markdown\n**preservado**")),
        "# Markdown\n**preservado**"
    );
}
#[test]
fn profile_persists_no_secret_and_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("jira.json");
    let mut s = Service::new(path.clone());
    wait(&mut s);
    assert!(!s.profile.enabled);
    let mut p = s.profile.clone();
    p.enabled = true;
    p.filters.text = "busca".into();
    p.views.push(View {
        name: "Squad".into(),
        filters: p.filters.clone(),
    });
    s.set_profile(p.clone()).unwrap();
    wait(&mut s);
    drop(s);
    let bytes = std::fs::read_to_string(&path).unwrap();
    assert!(!bytes.contains("token"));
    assert!(bytes.contains("busca"));
    let mut s = Service::new(path);
    wait(&mut s);
    assert_eq!(s.profile.filters, p.filters);
    assert_eq!(s.profile.views.len(), 1);
    assert!(!s.connected);
}
#[test]
fn invalid_profile_is_preserved_and_not_overwritten() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("jira.json");
    std::fs::write(&path, b"{ broken").unwrap();
    let mut s = Service::new(path.clone());
    wait(&mut s);
    assert!(s.error.is_some());
    assert!(s.set_profile(Profile::default()).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"{ broken");
}
#[test]
fn http_client_reads_cursor_details_people_comments_and_hours() {
    let server = Server::new(|r| {
        let v = if r.path.ends_with("/myself") {
            json!({"accountId":"me","displayName":"Fixture","active":true})
        } else if r.path.ends_with("/field") {
            json!([{"id":"customfield_10001","schema":{"custom":"com.atlassian.teams:rm-teams-custom-field-team"}}])
        } else if r.path.ends_with("/search/jql") {
            assert_eq!(r.method, "POST");
            assert_eq!(r.body["maxResults"], 50);
            assert_eq!(r.body["fields"].as_array().unwrap().len(), 8);
            if r.body["nextPageToken"].is_null() {
                json!({"issues":[{"key":"DEMO-1","fields":{"summary":"First","assignee":{"displayName":"Pessoa","accountId":"one"}}}],"nextPageToken":"cursor-two","isLast":false})
            } else {
                assert_eq!(r.body["nextPageToken"], "cursor-two");
                json!({"issues":[],"isLast":true})
            }
        } else if r.path.contains("/comment?") {
            assert!(r.path.contains("maxResults=20"));
            json!({"comments":[{"author":{"displayName":"Pessoa"},"body":{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Comentário"}]}]}}],"total":41})
        } else if r.path.contains("/worklog?") {
            json!({"worklogs":[{"author":{"displayName":"Pessoa"},"timeSpentSeconds":1800}],"total":1})
        } else if r.path.contains("/issue/DEMO-1?") {
            json!({"key":"DEMO-1","fields":{"summary":"First","description":"# Markdown","parent":{"key":"DEMO-9","fields":{"summary":"Epic"}},"reporter":{"displayName":"Reporter"}}})
        } else {
            panic!("Unexpected path {}", r.path)
        };
        (200, String::new(), v.to_string())
    });
    runtime().block_on(async {
        let mut c = JiraClient::fixture(server.base.clone());
        assert_eq!(
            c.connect().await.unwrap().team_field.as_deref(),
            Some("customfield_10001")
        );
        let p = c.search(&Filters::default(), None).await.unwrap();
        assert_eq!(p.issues[0].assignee.name, "Pessoa");
        let p = c.search(&Filters::default(), p.next).await.unwrap();
        assert!(p.next.is_none());
        let d = c.detail("DEMO-1").await.unwrap();
        assert_eq!(d.issue.relations[0].key, "DEMO-9");
        assert_eq!(d.issue.description, "# Markdown");
        assert_eq!(c.comments("DEMO-1", 0).await.unwrap().1, 41);
        assert_eq!(c.worklogs("DEMO-1", 0).await.unwrap().0[0].seconds, 1800);
    });
    assert_eq!(server.requests.lock().unwrap().len(), 7);
}
#[test]
fn http_errors_are_sanitized_and_never_retried() {
    for status in [401, 403, 404, 429, 500, 302] {
        let server = Server::new(move |_| {
            (
                status,
                "Retry-After: 120\r\nLocation: http://127.0.0.1:9/leak\r\n".into(),
                "{\"error\":\"secret-never-print\"}".into(),
            )
        });
        let c = JiraClient::fixture(server.base.clone());
        let error = runtime()
            .block_on(c.search(&Filters::default(), None))
            .unwrap_err();
        assert!(!error.contains("secret-never-print"));
        if status == 429 {
            assert!(error.contains("120"))
        }
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }
}
#[test]
fn worklog_validates_payload_and_posts_exactly_once_with_estimate_unchanged() {
    let d = WorklogDraft {
        key: "DEMO-1".into(),
        minutes: 90,
        started: "2026-11-03T09:00:00-03:00".into(),
        comment: "Trabalho feito".into(),
    };
    assert_eq!(d.body().unwrap()["timeSpentSeconds"], 5400);
    assert_eq!(d.body().unwrap()["started"], "2026-11-03T09:00:00.000-0300");
    let server = Server::new(|r| {
        assert_eq!(r.method, "POST");
        assert!(r.path.contains("adjustEstimate=leave"));
        assert_eq!(r.body["timeSpentSeconds"], 5400);
        (201, String::new(), "{\"id\":\"one\"}".into())
    });
    let c = JiraClient::fixture(server.base.clone());
    runtime().block_on(c.post_worklog(&d)).unwrap();
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    let mut invalid = d.clone();
    invalid.minutes = 0;
    assert!(invalid.body().is_err());
    invalid.minutes = 1441;
    assert!(invalid.body().is_err());
    invalid.minutes = 30;
    invalid.started = "yesterday".into();
    assert!(invalid.body().is_err());
    let server = Server::new(|_| (500, String::new(), "{}".into()));
    let c = JiraClient::fixture(server.base.clone());
    assert!(
        runtime()
            .block_on(c.post_worklog(&d))
            .unwrap_err()
            .contains("Confira")
    );
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}
#[test]
fn http_timeout_limits_and_retry_after_gate() {
    let server = Server::new(|_| {
        thread::sleep(Duration::from_millis(100));
        (200, String::new(), "{\"issues\":[]}".into())
    });
    let c = JiraClient::fixture_timeout(server.base.clone(), Duration::from_millis(30));
    assert!(
        runtime()
            .block_on(c.search(&Filters::default(), None))
            .is_err()
    );
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    let server = Server::new(|_| (429, "Retry-After: 60\r\n".into(), "{}".into()));
    let c = JiraClient::fixture(server.base.clone());
    let rt = runtime();
    assert!(rt.block_on(c.search(&Filters::default(), None)).is_err());
    assert!(
        rt.block_on(c.search(&Filters::default(), None))
            .unwrap_err()
            .contains("aguarde")
    );
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    let server = Server::new(|_| {
        (200,String::new(),json!({"issues":(0..51).map(|_|json!({"key":"DEMO-1"})).collect::<Vec<_>>(),"isLast":true}).to_string())
    });
    let c = JiraClient::fixture(server.base.clone());
    assert!(
        rt.block_on(c.search(&Filters::default(), None))
            .unwrap_err()
            .contains("tamanho de página")
    );
}
#[test]
fn stale_responses_cannot_repopulate_changed_filter_card_or_connection() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Service::new(dir.path().join("jira.json"));
    wait(&mut s);
    s.tx = None;
    let (tx, rx) = mpsc::sync_channel(1);
    s.rx = rx;
    s.profile.enabled = true;
    s.connected = true;
    s.query = 3;
    s.selected = Some("DEMO-2".into());
    tx.send(Response {
        epoch: s.epoch,
        result: Ok(Reply::Page(
            Page {
                issues: vec![Issue {
                    key: "DEMO-1".into(),
                    ..Default::default()
                }],
                next: None,
            },
            0,
            2,
        )),
        write: false,
        scope: Scope::Search(2),
    })
    .unwrap();
    s.poll();
    assert!(s.page.issues.is_empty());
    tx.send(Response {
        epoch: s.epoch,
        result: Err("Obsolete error".into()),
        write: false,
        scope: Scope::Card("DEMO-1".into()),
    })
    .unwrap();
    s.poll();
    assert!(s.error.is_none());
    let old = s.epoch;
    s.disconnect();
    tx.send(Response {
        epoch: old,
        result: Ok(Reply::Detail(Box::new(Detail {
            issue: Issue {
                key: "DEMO-2".into(),
                ..Default::default()
            },
            ..Default::default()
        }))),
        write: false,
        scope: Scope::Card("DEMO-2".into()),
    })
    .unwrap();
    s.poll();
    assert!(s.detail.is_none());
    assert!(!s.connected);
}
#[test]
#[ignore = "requires explicit disposable Secret Service; never run against a personal vault"]
fn jira_vault_round_trip() {
    assert_eq!(
        std::env::var("CODISSEU_BROWSER_VAULT_TEST").as_deref(),
        Ok("1")
    );
    let c = Connection {
        site: "https://fixture.atlassian.net".into(),
        api: "https://fixture.atlassian.net".into(),
        email: format!("fixture-{}@example.invalid", std::process::id()),
    };
    let e = vault(&c).unwrap();
    e.set_password("jira-fixture-canary").unwrap();
    assert_eq!(load_token(&c).unwrap().expose(), "jira-fixture-canary");
    e.delete_credential().unwrap();
    assert!(load_token(&c).is_err());
    assert!(!format!("{:?}", Secret::new("jira-fixture-canary".into())).contains("canary"));
}
