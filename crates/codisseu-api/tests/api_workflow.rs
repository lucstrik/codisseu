use codisseu_api::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

fn await_job(job: &mut RequestJob) -> Result<ExecutionResult, String> {
    let start = Instant::now();
    loop {
        if let Some(result) = job.try_result() {
            return result;
        }
        assert!(start.elapsed() < Duration::from_secs(8));
        thread::sleep(Duration::from_millis(5));
    }
}
fn poll_until(service: &mut ApiService, condition: impl Fn(&ApiService) -> bool) {
    let start = Instant::now();
    loop {
        service.poll();
        if condition(service) {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "service timed out: {:?}",
            service.error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn fixture_server(count: usize) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(vec![]));
    let received = requests.clone();
    let worker = thread::spawn(move || {
        for _ in 0..count {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut buffer = [0; 4096];
            let mut data = vec![];
            loop {
                let n = socket.read(&mut buffer).unwrap();
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&buffer[..n]);
                if data.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&data).into_owned();
            let body = if request.starts_with("POST /token ") {
                r#"{"data":{"access_token":"synthetic-token"}}"#
            } else if request
                .to_ascii_lowercase()
                .contains("authorization: bearer synthetic-token")
            {
                r#"{"authorized":true}"#
            } else {
                r#"{"authorized":false}"#
            };
            received.lock().unwrap().push(request);
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).unwrap();
        }
    });
    (url, requests, worker)
}
#[test]
fn curl_roundtrip_preserves_unicode_repeated_params_json_and_quotes_without_execution() {
    let source = "curl --request POST 'https://example.test/api?a=1&a=2' -H 'Content-Type: application/json' --data-raw '{\"name\":\"João\",\"quote\":\"a'\\''b\",\"shell\":\"$(touch /tmp/no-shell)\"}'";
    let original = parse_curl(source).unwrap();
    assert_eq!(original.params.len(), 2);
    assert!(!original.settings.follow_redirects);
    let exported = export_curl(&original, &Variables::new()).unwrap();
    let imported = parse_curl(&exported).unwrap();
    assert_eq!(original.url, imported.url);
    assert_eq!(original.params, imported.params);
    assert_eq!(original.body, imported.body);
    assert_eq!(
        original.settings.follow_redirects,
        imported.settings.follow_redirects
    );
    assert!(parse_curl("curl 💩").is_err());
    assert!(parse_curl("curl --data @/etc/passwd https://example.test").is_err());
    assert!(parse_curl("curl https://example.test ; touch /tmp/no-shell").is_err());
    assert!(parse_curl("curl --data-urlencode 'plain value' https://example.test").is_err());
}
#[test]
fn import_export_nested_postman_collection_and_environment() {
    let source = r#"{"info":{"name":"Payments"},"auth":{"type":"bearer","bearer":[{"key":"token","value":"{{token}}"}]},"variable":[{"key":"base","value":"https://example.test"}],"item":[{"name":"v1","item":[{"name":"Create","request":{"method":"POST","url":{"raw":"{{base}}/items?a=1","query":[{"key":"a","value":"1"},{"key":"off","value":"x","disabled":true}]},"body":{"mode":"raw","raw":"{}","options":{"raw":{"language":"json"}}}},"event":[{"listen":"test","script":{"exec":["pm.environment.set('token', pm.response.json().token);"]}}]}]}]}"#;
    let collection = import_collection(source).unwrap();
    let exported = export_collection(&collection).unwrap();
    let again = import_collection(&exported).unwrap();
    assert_eq!(collection.name, again.name);
    assert_eq!(collection.auth, again.auth);
    let CollectionItem::Folder { items, .. } = &again.items[0] else {
        panic!()
    };
    let CollectionItem::Request(request) = &items[0] else {
        panic!()
    };
    assert_eq!(request.params.len(), 2);
    assert!(!request.params[1].enabled);
    assert!(request.post_response_script.contains("pm.environment.set"));
    let environment=import_environment(r#"{"name":"Local","values":[{"key":"token","value":"secret","type":"secret","enabled":true}]}"#).unwrap();
    assert_eq!(
        import_environment(&export_environment(&environment).unwrap())
            .unwrap()
            .variables,
        environment.variables
    );
    assert!(import_collection(r#"{"info":{"name":"bad"},"item":[{"name":"bad","request":{"url":"https://example.test","auth":{"type":"oauth2"}}}]}"#).is_err());
}
#[test]
fn project_storage_isolated_atomic_secret_values_removed_and_failure_preserves_original() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a");
    let b = directory.path().join("b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let storage = ApiStorage::new(directory.path().join("profile"));
    let mut workspace = ApiWorkspace::default();
    let collection = workspace.new_collection("Only A");
    let id = workspace.new_request(&collection, "Auth").unwrap();
    workspace.find_request_mut(&id).unwrap().auth = Auth::Bearer {
        token: "secret-literal-bearer".into(),
    };
    let environment = workspace.new_environment("Local");
    workspace.environments[0].variables = vec![
        KeyValue::new("base", "https://example.test"),
        KeyValue::new("access_token", "secret-auto-token"),
    ];
    workspace.apply_environment_updates(
        &environment,
        &Variables::from([("session".into(), "secret-script-token".into())]),
    );
    storage.save(&a, &workspace).unwrap();
    assert!(storage.load(&b).unwrap().collections.is_empty());
    let loaded = storage.load(&a).unwrap();
    assert_eq!(loaded.collections[0].name, "Only A");
    assert!(
        loaded.environments[0]
            .variables
            .iter()
            .filter(|r| r.key != "base")
            .all(|r| r.value.is_empty())
    );
    let saved = std::fs::read_to_string(storage.path_for(&a).unwrap()).unwrap();
    assert!(!saved.contains("secret-literal"));
    assert!(!saved.contains("secret-auto"));
    assert!(!saved.contains("secret-script"));
    let mut huge = workspace.clone();
    huge.collections[0].description = "x".repeat(17 * 1024 * 1024);
    assert!(storage.save(&a, &huge).is_err());
    assert_eq!(
        std::fs::read_to_string(storage.path_for(&a).unwrap()).unwrap(),
        saved
    );
}
#[test]
fn javascript_scopes_unset_tests_timeout_and_no_host_runtime() {
    let vars = Variables::from([("token".into(), "old".into())]);
    let result=run_script("const answer = 42; pm.environment.set('token', String(answer)); pm.environment.unset('obsolete'); pm.test('pass', () => pm.expect(answer).to.eql(42)); pm.test('fail', () => pm.expect(answer).to.eql(9)); pm.test('no host', () => { pm.expect(typeof process).to.eql('undefined'); pm.expect(typeof require).to.eql('undefined'); });",&vars,None).unwrap();
    assert_eq!(result.environment_updates["token"], "42");
    assert_eq!(result.environment_unsets, ["obsolete"]);
    assert_eq!(result.tests.iter().filter(|t| t.passed).count(), 2);
    assert!(!result.tests[1].passed);
    let result = run_script(
        "pm.test('async unsupported', async () => true);",
        &Variables::new(),
        None,
    )
    .unwrap();
    assert!(!result.tests[0].passed);
    let start = Instant::now();
    assert!(run_script("while (true) {}", &Variables::new(), None).is_err());
    assert!(start.elapsed() < Duration::from_secs(4));
    assert!(run_script("eval('2+2')", &Variables::new(), None).is_err());
    assert!(run_script("new Function('return process')()", &Variables::new(), None).is_err());
}
#[test]
fn malformed_or_large_script_is_rejected_and_allocator_limit_is_child_only() {
    assert!(run_script(&"x".repeat(65537), &Variables::new(), None).is_err());
    assert!(run_script("const x = ;", &Variables::new(), None).is_err());
    #[cfg(target_os = "linux")]
    assert!(
        run_script(
            "const x = new ArrayBuffer(512 * 1024 * 1024); console.log(x.byteLength);",
            &Variables::new(),
            None
        )
        .is_err()
    );
    assert_eq!(
        run_script(
            "pm.environment.set('still_alive', 'yes');",
            &Variables::new(),
            None
        )
        .unwrap()
        .environment_updates["still_alive"],
        "yes"
    );
}
#[test]
fn serial_collection_runner_extracts_token_and_sends_followup_with_inherited_auth() {
    let (url, received, server) = fixture_server(2);
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut service = ApiService::new(directory.path().join("profile"));
    service.set_project(project.clone()).unwrap();
    poll_until(&mut service, |s| !s.loading);
    let collection = service.new_collection("Microservices");
    let env = service.new_environment("Local");
    service.workspace.active_environment = Some(env.clone());
    service.workspace.environments[0]
        .variables
        .push(KeyValue::new("base", &url));
    service.workspace.collections[0]
        .variables
        .push(KeyValue::new("shared", "collection"));
    service.workspace.environments[0]
        .variables
        .push(KeyValue::new("shared", "environment"));
    service.workspace.collections[0].pre_request_script="const duplicate = 'collection'; pm.test('collection scope', () => pm.expect(pm.collectionVariables.get('shared')).to.eql('collection'));".into();
    let token = service.new_request(&collection, "Token").unwrap();
    let request = service.workspace.find_request_mut(&token).unwrap();
    request.method = "POST".into();
    request.url = "{{base}}/token".into();
    request.pre_request_script="const duplicate = 'request'; pm.test('environment scope', () => pm.expect(pm.environment.get('shared')).to.eql('environment'));".into();
    request.post_response_script="const json = pm.response.json(); pm.environment.set('token', json.data.access_token); pm.test('token status', () => pm.response.to.have.status(200));".into();
    let followup = service.new_request(&collection, "Protected").unwrap();
    let request = service.workspace.find_request_mut(&followup).unwrap();
    request.url = "{{base}}/protected".into();
    request.auth = Auth::Bearer {
        token: "{{token}}".into(),
    };
    request.post_response_script="pm.test('authorized', () => pm.expect(pm.response.json().authorized).to.be.true); pm.test('deliberate failure', () => pm.expect(1).to.eql(2));".into();
    service.run_collection(&collection, true).unwrap();
    poll_until(&mut service, |s| {
        s.runner.as_ref().is_some_and(|r| r.finished)
    });
    let runner = service.runner.as_ref().unwrap();
    assert_eq!(runner.completed, 2);
    assert_eq!(runner.tests_failed, 1);
    assert!(runner.error.is_none(), "{:?}", runner.error);
    assert_eq!(runner.results.len(), 2);
    assert_eq!(runner.results[0].request_id, token);
    assert_eq!(runner.results[1].request_id, followup);
    assert_eq!(service.responses[&followup].response.status, 200);
    assert!(
        service.workspace.environments[0]
            .variables
            .iter()
            .any(|r| r.key == "token" && r.value == "synthetic-token" && r.secret)
    );
    service.save().unwrap();
    poll_until(&mut service, |s| !s.saving);
    let file = ApiStorage::new(directory.path().join("profile"))
        .path_for(&project)
        .unwrap();
    assert!(
        !std::fs::read_to_string(file)
            .unwrap()
            .contains("synthetic-token")
    );
    server.join().unwrap();
    assert!(
        received.lock().unwrap()[1]
            .to_ascii_lowercase()
            .contains("authorization: bearer synthetic-token")
    );
    service.shutdown();
    poll_until(&mut service, |s| !s.is_busy());
}
#[test]
fn real_http_has_single_content_type_cancellation_and_response_body_cap() {
    let (url, received, server) = fixture_server(1);
    let mut request = parse_curl(&format!(
        "curl '{url}/token' -X POST -H 'Content-Type: application/json' --data-raw '{{}}'"
    ))
    .unwrap();
    let mut job = ApiClient::default()
        .start(request.clone(), Variables::new(), false)
        .unwrap();
    assert_eq!(await_job(&mut job).unwrap().response.status, 200);
    server.join().unwrap();
    assert_eq!(
        received.lock().unwrap()[0]
            .to_ascii_lowercase()
            .matches("content-type:")
            .count(),
        1
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    request.url = format!("http://{}/slow", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (_socket, _) = listener.accept().unwrap();
        thread::sleep(Duration::from_millis(200));
    });
    let mut job = ApiClient::default()
        .start(request, Variables::new(), false)
        .unwrap();
    thread::sleep(Duration::from_millis(30));
    job.cancel();
    assert_eq!(await_job(&mut job).unwrap_err(), "Request cancelled");
    server.join().unwrap();
    let mock = MockServer::start(MockDefinition {
        id: new_id(),
        name: "Large".into(),
        routes: vec![MockRoute {
            method: "GET".into(),
            path: "/large".into(),
            status: 200,
            headers: vec![],
            body: "x".repeat(5000),
        }],
    })
    .unwrap();
    let mut request = ApiRequest {
        url: format!("{}/large", mock.url),
        ..ApiRequest::default()
    };
    request.settings.max_response_bytes = 1024;
    let mut job = ApiClient::default()
        .start(request, Variables::new(), false)
        .unwrap();
    let result = await_job(&mut job).unwrap();
    assert_eq!(result.response.body.len(), 1024);
    assert!(result.response.truncated);
    mock.stop();
}
#[test]
fn openapi_yaml_generates_requests_and_auth_and_rejects_unsupported_security() {
    let source = "openapi: 3.1.0\ninfo:\n  title: Test API\nservers:\n  - url: https://example.test\nsecurity:\n  - bearerAuth: []\ncomponents:\n  securitySchemes:\n    bearerAuth:\n      type: http\n      scheme: bearer\npaths:\n  /users/{id}:\n    get:\n      summary: Read user\n      parameters:\n        - in: path\n          name: id\n          example: 42\n        - in: query\n          name: expand\n          example: roles\n";
    let (spec, collection) = import_openapi(source).unwrap();
    assert_eq!(spec.name, "Test API");
    assert_eq!(
        collection.auth,
        Auth::Bearer {
            token: "{{access_token}}".into()
        }
    );
    let CollectionItem::Request(request) = &collection.items[0] else {
        panic!()
    };
    assert_eq!(request.url, "{{base_url}}/users/{{id}}");
    assert_eq!(request.params[0].value, "roles");
    assert!(
        import_openapi(&source.replace("type: http\n      scheme: bearer", "type: oauth2"))
            .is_err()
    );
}
#[test]
fn switching_projects_saves_old_snapshot_and_never_shows_other_projects_collections() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a");
    let b = directory.path().join("b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let mut service = ApiService::new(directory.path().join("profile"));
    service.set_project(a.clone()).unwrap();
    poll_until(&mut service, |s| !s.loading);
    service.new_collection("Only A");
    service.set_project(b).unwrap();
    assert!(service.workspace.collections.is_empty());
    poll_until(&mut service, |s| !s.loading);
    assert!(service.workspace.collections.is_empty());
    service.new_collection("Only B");
    service.set_project(a).unwrap();
    poll_until(&mut service, |s| !s.loading);
    assert_eq!(service.workspace.collections[0].name, "Only A");
    service.clear_project().unwrap();
    assert!(service.workspace.collections.is_empty());
    service.shutdown();
    poll_until(&mut service, |s| !s.is_busy());
}

#[test]
fn unchanged_window_does_not_overwrite_another_windows_saved_collections() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a");
    let b = directory.path().join("b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let profile = directory.path().join("profile");
    let mut first = ApiService::new(profile.clone());
    let mut second = ApiService::new(profile.clone());
    for service in [&mut first, &mut second] {
        service.set_project(a.clone()).unwrap();
        poll_until(service, |s| !s.loading);
    }
    first.new_collection("First window saved collection");
    first.save().unwrap();
    poll_until(&mut first, |s| !s.saving);
    assert!(first.error.is_none());
    let storage = ApiStorage::new(profile);
    let path = storage.path_for(&a).unwrap();
    let saved = std::fs::read(&path).unwrap();
    second.save().unwrap();
    poll_until(&mut second, |s| !s.saving);
    assert!(second.error.is_none());
    second.set_project(b).unwrap();
    poll_until(&mut second, |s| !s.loading);
    assert!(second.failed_saves.is_empty());
    assert!(second.workspace.collections.is_empty());
    assert_eq!(std::fs::read(path).unwrap(), saved);
    for service in [&mut first, &mut second] {
        service.shutdown();
        poll_until(service, |s| !s.is_busy());
    }
}

#[test]
fn dirty_stale_window_retains_its_snapshot_and_never_retries_over_a_newer_profile() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a");
    let b = directory.path().join("b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let profile = directory.path().join("profile");
    let mut first = ApiService::new(profile.clone());
    let mut second = ApiService::new(profile.clone());
    for service in [&mut first, &mut second] {
        service.set_project(a.clone()).unwrap();
        poll_until(service, |s| !s.loading);
    }
    first.new_collection("First committed definition");
    first.save().unwrap();
    poll_until(&mut first, |s| !s.saving);
    let storage = ApiStorage::new(profile);
    let path = storage.path_for(&a).unwrap();
    let saved = std::fs::read(&path).unwrap();
    second.new_collection("Second retained definition");
    second.save().unwrap();
    poll_until(&mut second, |s| !s.saving);
    assert!(second.error.as_ref().unwrap().contains("another window"));
    assert_eq!(
        second.workspace.collections[0].name,
        "Second retained definition"
    );
    assert_eq!(second.failed_saves.len(), 1);
    assert_eq!(second.failed_saves[0].project, a);
    assert_eq!(
        second.failed_saves[0].workspace.collections[0].name,
        "Second retained definition"
    );
    assert_eq!(std::fs::read(&path).unwrap(), saved);
    second.set_project(b).unwrap();
    poll_until(&mut second, |s| !s.loading);
    assert!(second.workspace.collections.is_empty());
    assert!(second.error.as_ref().unwrap().contains("previous project"));
    second.set_project(a.clone()).unwrap();
    poll_until(&mut second, |s| !s.loading);
    assert_eq!(
        second.workspace.collections[0].name,
        "First committed definition"
    );
    second.retry_failed_saves().unwrap();
    poll_until(&mut second, |s| !s.saving);
    assert_eq!(second.failed_saves.len(), 1);
    assert_eq!(
        second.failed_saves[0].workspace.collections[0].name,
        "Second retained definition"
    );
    assert_eq!(std::fs::read(path).unwrap(), saved);
    for service in [&mut first, &mut second] {
        service.shutdown();
        poll_until(service, |s| !s.is_busy());
    }
}
#[test]
fn failed_profile_load_preserves_file_and_prevents_saving_but_can_exit() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("a");
    std::fs::create_dir(&project).unwrap();
    let storage = ApiStorage::new(directory.path().join("profile"));
    std::fs::create_dir(directory.path().join("profile")).unwrap();
    let path = storage.path_for(&project).unwrap();
    std::fs::write(&path, "invalid json original").unwrap();
    let mut service = ApiService::new(directory.path().join("profile"));
    service.set_project(project).unwrap();
    poll_until(&mut service, |s| !s.loading);
    assert!(service.error.is_some());
    assert!(!service.can_save());
    assert!(service.save().is_err());
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "invalid json original"
    );
    service.shutdown();
    poll_until(&mut service, |s| !s.is_busy());
}

fn capture_upload() -> (String, Arc<Mutex<Vec<u8>>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let data = Arc::new(Mutex::new(vec![]));
    let received = data.clone();
    let thread = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut all = vec![];
        let mut buffer = [0; 4096];
        loop {
            let n = socket.read(&mut buffer).unwrap();
            if n == 0 {
                break;
            }
            all.extend_from_slice(&buffer[..n]);
            if let Some(end) = all.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&all[..end]);
                let size = head
                    .lines()
                    .find_map(|l| {
                        l.split_once(':')
                            .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                            .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if all.len() >= end + 4 + size {
                    break;
                }
            }
        }
        *received.lock().unwrap() = all;
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
            .unwrap();
    });
    (url, data, thread)
}
#[test]
fn uploads_import_as_references_and_only_send_reads_binary_and_multipart_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("file with spaces.bin");
    let payload = b"synthetic-upload\x00\xff";
    std::fs::write(&path, payload).unwrap();
    let (url, received, server) = capture_upload();
    let mut request = parse_curl(&format!(
        "curl '{url}/upload' --data-binary '@{}'",
        path.display()
    ))
    .unwrap();
    assert_eq!(request.body, RequestBody::BinaryFile { path: path.clone() });
    let copy = parse_curl(&export_curl(&request, &Variables::new()).unwrap()).unwrap();
    assert_eq!(copy.body, request.body);
    let mut job = ApiClient::default()
        .start(request.clone(), Variables::new(), false)
        .unwrap();
    assert_eq!(await_job(&mut job).unwrap().response.status, 200);
    server.join().unwrap();
    assert!(received.lock().unwrap().ends_with(payload));
    let (url, received, server) = capture_upload();
    let mut row = KeyValue::new("document", "");
    row.file_path = Some(path.clone());
    request.url = format!("{url}/upload");
    request.body = RequestBody::FormData(vec![row, KeyValue::new("description", "fixture")]);
    let copy = parse_curl(&export_curl(&request, &Variables::new()).unwrap()).unwrap();
    assert_eq!(copy.body, request.body);
    let mut job = ApiClient::default()
        .start(request, Variables::new(), false)
        .unwrap();
    assert_eq!(await_job(&mut job).unwrap().response.status, 200);
    server.join().unwrap();
    let bytes = received.lock().unwrap();
    assert!(bytes.windows(payload.len()).any(|b| b == payload));
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("filename=\"file with spaces.bin\""));
    assert!(text.contains("fixture"));
    drop(bytes);
    let imported=import_collection(r#"{"info":{"name":"Files"},"item":[{"name":"Upload","request":{"method":"POST","url":"http://127.0.0.1:9/upload","body":{"mode":"file","file":{"src":"/does-not-exist-yet"}}}}]}"#).unwrap();
    let CollectionItem::Request(request) = &imported.items[0] else {
        panic!()
    };
    assert_eq!(
        request.body,
        RequestBody::BinaryFile {
            path: "/does-not-exist-yet".into()
        }
    );
    let mut job = ApiClient::default()
        .start(request.as_ref().clone(), Variables::new(), false)
        .unwrap();
    assert_eq!(
        await_job(&mut job).unwrap_err(),
        "Upload file could not be opened"
    );
    let huge = directory.path().join("large.bin");
    std::fs::File::create(&huge)
        .unwrap()
        .set_len(8 * 1024 * 1024 + 1)
        .unwrap();
    let mut request = request.as_ref().clone();
    request.body = RequestBody::BinaryFile { path: huge };
    let mut job = ApiClient::default()
        .start(request, Variables::new(), false)
        .unwrap();
    assert_eq!(
        await_job(&mut job).unwrap_err(),
        "Upload file exceeds 8 MiB"
    );
}

#[test]
fn workspace_file_storage_identity_uses_its_declared_folder_for_relative_uploads() {
    let directory = tempfile::tempdir().unwrap();
    let carrier = directory.path().join("carrier");
    let declared = directory.path().join("declared");
    std::fs::create_dir(&carrier).unwrap();
    std::fs::create_dir(&declared).unwrap();
    let workspace_file = carrier.join("project.code-workspace");
    std::fs::write(&workspace_file, "{}").unwrap();
    std::fs::write(carrier.join("payload.bin"), "wrong carrier bytes").unwrap();
    let payload = b"declared project upload\0";
    std::fs::write(declared.join("payload.bin"), payload).unwrap();
    let mut service = ApiService::new(directory.path().join("profile"));
    service
        .set_project_with_root(workspace_file.clone(), Some(declared))
        .unwrap();
    poll_until(&mut service, |s| !s.loading);
    let collection = service.new_collection("Uploads");
    let id = service
        .workspace
        .new_request(&collection, "Upload")
        .unwrap();
    let (url, received, server) = capture_upload();
    let request = service.workspace.find_request_mut(&id).unwrap();
    request.url = url;
    request.method = "POST".into();
    request.body = RequestBody::BinaryFile {
        path: "payload.bin".into(),
    };
    service.send(&id, false).unwrap();
    poll_until(&mut service, |s| s.running_request.is_none());
    assert!(service.error.is_none(), "{:?}", service.error);
    assert_eq!(service.responses[&id].response.status, 200);
    assert_eq!(service.project, Some(workspace_file));
    server.join().unwrap();
    assert!(received.lock().unwrap().ends_with(payload));
    service.shutdown();
    poll_until(&mut service, |s| !s.is_busy());

    let mut request = ApiRequest {
        url: "http://127.0.0.1:9/never-sent".into(),
        body: RequestBody::BinaryFile {
            path: "payload.bin".into(),
        },
        ..Default::default()
    };
    request.upload_root = None;
    let mut job = ApiClient::default()
        .start(request, Variables::new(), false)
        .unwrap();
    assert_eq!(
        await_job(&mut job).unwrap_err(),
        "Relative upload requires an available project folder"
    );
}
#[test]
fn generated_auth_replaces_manual_header_and_query_key_instead_of_duplicating() {
    let mut request = ApiRequest {
        url: "https://example.test?api_key=old".into(),
        headers: vec![
            KeyValue::new("authorization", "old"),
            KeyValue::new("Authorization", "another"),
        ],
        auth: Auth::Bearer {
            token: "new".into(),
        },
        ..ApiRequest::default()
    };
    let (_, headers) = resolve_request(&request, &Variables::new()).unwrap();
    assert_eq!(
        headers
            .iter()
            .filter(|h| h.key.eq_ignore_ascii_case("Authorization"))
            .count(),
        1
    );
    assert_eq!(headers.last().unwrap().value, "Bearer new");
    request.auth = Auth::ApiKey {
        key: "api_key".into(),
        value: "new".into(),
        in_query: true,
    };
    let (url, _) = resolve_request(&request, &Variables::new()).unwrap();
    assert_eq!(
        url.query_pairs().collect::<Vec<_>>(),
        vec![("api_key".into(), "new".into())]
    );
}
#[test]
fn cancelled_collection_stops_remaining_requests_and_pending_mock_start_can_be_stopped() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut service = ApiService::new(directory.path().join("profile"));
    service.set_project(project).unwrap();
    poll_until(&mut service, |s| !s.loading);
    let collection = service.new_collection("Cancel");
    for name in ["One", "Two"] {
        let id = service.new_request(&collection, name).unwrap();
        let request = service.workspace.find_request_mut(&id).unwrap();
        request.url = "http://1270.0.0.1:9/never".into();
        request.pre_request_script = "while (true) {}".into();
    }
    service.run_collection(&collection, true).unwrap();
    service.cancel();
    poll_until(&mut service, |s| !s.is_busy());
    let runner = service.runner.as_ref().unwrap();
    assert!(runner.cancelled && runner.finished);
    assert_eq!(runner.completed, 0);
    assert!(service.history.is_empty());
    let id = new_id();
    service.workspace.mocks.push(MockDefinition {
        id: id.clone(),
        name: "Stopped".into(),
        routes: vec![],
    });
    service.start_mock(&id).unwrap();
    service.stop_mock();
    poll_until(&mut service, |s| !s.is_busy());
    assert!(service.mock_url.is_none());
    service.shutdown();
    poll_until(&mut service, |s| !s.is_busy());
}
#[test]
fn failed_old_project_save_retains_snapshot_and_can_retry_after_filesystem_repair() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a");
    let b = directory.path().join("b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let profile = directory.path().join("profile");
    std::fs::write(&profile, "fixture prevents directory creation").unwrap();
    let mut service = ApiService::new(profile.clone());
    service.set_project(a.clone()).unwrap();
    poll_until(&mut service, |s| !s.loading);
    service.new_collection("A retained draft");
    service.set_project(b).unwrap();
    poll_until(&mut service, |s| !s.loading);
    assert_eq!(service.failed_saves.len(), 1);
    assert_eq!(
        service.failed_saves[0].workspace.collections[0].name,
        "A retained draft"
    );
    assert!(service.workspace.collections.is_empty());
    std::fs::remove_file(&profile).unwrap();
    service.retry_failed_saves().unwrap();
    poll_until(&mut service, |s| !s.saving);
    assert!(service.failed_saves.is_empty());
    assert_eq!(
        ApiStorage::new(profile).load(&a).unwrap().collections[0].name,
        "A retained draft"
    );
    service.shutdown();
    poll_until(&mut service, |s| !s.is_busy());
}

#[test]
fn absent_environment_mutations_fail_before_send_or_show_post_error_but_collection_scripts_work() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut service = ApiService::new(directory.path().join("profile"));
    service.set_project(project).unwrap();
    poll_until(&mut service, |s| !s.loading);
    let collection = service.new_collection("Scopes");
    let id = service.new_request(&collection, "No environment").unwrap();
    let request = service.workspace.find_request_mut(&id).unwrap();
    request.url = "http://127.0.0.1:9/must-not-send".into();
    request.pre_request_script = "pm.environment.set('token', 'temporary');".into();
    service.send(&id, true).unwrap();
    poll_until(&mut service, |s| s.running_request.is_none());
    assert!(
        service
            .error
            .as_ref()
            .unwrap()
            .contains("Select an environment")
    );
    assert!(!service.responses.contains_key(&id));
    let (url, _, server) = fixture_server(2);
    let request = service.workspace.find_request_mut(&id).unwrap();
    request.url = format!("{url}/token");
    request.method = "POST".into();
    request.pre_request_script.clear();
    request.post_response_script =
        "pm.environment.set('token', pm.response.json().data.access_token);".into();
    service.send(&id, true).unwrap();
    poll_until(&mut service, |s| s.running_request.is_none());
    assert_eq!(service.responses[&id].response.status, 200);
    assert!(
        service.responses[&id]
            .script_error
            .as_ref()
            .unwrap()
            .contains("Select an environment")
    );
    let request = service.workspace.find_request_mut(&id).unwrap();
    request.url = "{{base}}/protected".into();
    request.pre_request_script = format!("pm.collectionVariables.set('base', '{url}');");
    request.post_response_script = "pm.collectionVariables.set('success', 'yes');".into();
    service.send(&id, true).unwrap();
    poll_until(&mut service, |s| s.running_request.is_none());
    assert!(service.responses[&id].script_error.is_none());
    assert_eq!(service.responses[&id].response.status, 200);
    assert!(
        service.workspace.collections[0]
            .variables
            .iter()
            .any(|r| r.key == "success" && r.value == "yes")
    );
    server.join().unwrap();
    service.shutdown();
    poll_until(&mut service, |s| !s.is_busy());
}
#[test]
fn missing_json_token_or_null_variable_does_not_overwrite_valid_credentials() {
    for script in [
        "pm.environment.set('token', undefined);",
        "pm.environment.set('token', null);",
        "pm.collectionVariables.set('token', undefined);",
        "pm.collectionVariables.set('token', null);",
    ] {
        assert!(
            run_script(script, &Variables::new(), None)
                .unwrap_err()
                .contains("null or undefined")
        );
    }
    let (url, _, server) = fixture_server(1);
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut service = ApiService::new(directory.path().join("profile"));
    service.set_project(project).unwrap();
    poll_until(&mut service, |s| !s.loading);
    let collection = service.new_collection("Token");
    let env = service.new_environment("Local");
    service.workspace.active_environment = Some(env);
    service.workspace.environments[0]
        .variables
        .push(KeyValue::new("token", "existing-valid-token"));
    let id = service
        .new_request(&collection, "Missing token field")
        .unwrap();
    let request = service.workspace.find_request_mut(&id).unwrap();
    request.url = format!("{url}/token");
    request.method = "POST".into();
    request.post_response_script =
        "pm.environment.set('token', pm.response.json().data.nonexistent);".into();
    service.send(&id, true).unwrap();
    poll_until(&mut service, |s| s.running_request.is_none());
    assert!(
        service.responses[&id]
            .script_error
            .as_ref()
            .unwrap()
            .contains("null or undefined")
    );
    assert_eq!(
        service.workspace.environments[0].variables[0].value,
        "existing-valid-token"
    );
    server.join().unwrap();
    service.shutdown();
    poll_until(&mut service, |s| !s.is_busy());
}

#[test]
fn folder_moves_duplicates_order_and_failed_cycles_preserve_all_definitions() {
    let mut w = ApiWorkspace::default();
    let a = w.new_collection("Billing");
    let b = w.new_collection("Shipping");
    let version = w.new_folder(&a, "8.4").unwrap();
    let dev = w.new_folder(&version, "DEV").unwrap();
    let request = w.new_request(&a, "Ping").unwrap();
    w.move_item(&request, &dev).unwrap();
    assert_eq!(w.request_path(&request), ["Billing", "8.4", "DEV", "Ping"]);
    let before = w.clone();
    assert!(w.move_item(&version, &dev).is_err());
    assert_eq!(w, before);
    assert!(w.move_item(&request, "missing").is_err());
    assert_eq!(w, before);
    let copy = w.duplicate_item(&version).unwrap();
    assert_ne!(copy, version);
    let duplicate = match w.item(&copy).unwrap() {
        CollectionItem::Folder { items, .. } => items[0].clone(),
        _ => panic!(),
    };
    assert_ne!(duplicate.id(), dev);
    assert_eq!(w.parent_id(&copy), Some(a.clone()));
    w.move_item(&version, &b).unwrap();
    assert_eq!(w.request_path(&request), ["Shipping", "8.4", "DEV", "Ping"]);
    assert!(
        w.destinations(Some(&version))
            .iter()
            .all(|(id, _, _)| id != &version && id != &dev)
    );
    let sibling = w.new_request(&b, "Other").unwrap();
    assert!(w.reorder_item(&sibling, false));
    assert!(!w.reorder_item(&sibling, false));
    let restored = import_collection(&export_collection(&w.collections[1]).unwrap()).unwrap();
    assert_eq!(restored.items.len(), 2);
    assert!(matches!(&restored.items[1],CollectionItem::Folder{name,..} if name=="8.4"));
}

#[test]
fn url_query_templates_repeated_unicode_fragments_and_path_values_resolve_once() {
    let mut r = ApiRequest::default();
    r.params.push(KeyValue {
        enabled: false,
        ..KeyValue::new("off", "x")
    });
    r.set_url("{{base}}/customers/:id?a=1&a=2&name=Jo%C3%A3o&value={{value}}#view");
    assert_eq!(r.params.len(), 5);
    assert_eq!(r.path_variables[0].key, "id");
    r.path_variables[0].value = "A/B ?".into();
    r.params[0].description = "retained".into();
    let display = r.display_url();
    assert!(display.contains("a=1&a=2"));
    assert!(display.contains("value={{value}}"));
    assert!(display.ends_with("#view"));
    r.set_url(&display);
    assert_eq!(r.params[0].description, "retained");
    assert_eq!(r.path_variables[0].value, "A/B ?");
    let vars = Variables::from([
        ("base".into(), "https://example.test".into()),
        ("value".into(), "A&B".into()),
    ]);
    let (url, _) = resolve_request(&r, &vars).unwrap();
    assert_eq!(url.path(), "/customers/A%2FB%20%3F");
    assert_eq!(url.query_pairs().filter(|(key, _)| key == "a").count(), 2);
    assert_eq!(
        url.query_pairs().find(|(key, _)| key == "value").unwrap().1,
        "A&B"
    );
    assert!(!url.as_str().contains("off="));
    let mut w = ApiWorkspace::default();
    let c = w.new_collection("Path");
    w.collections[0]
        .items
        .push(CollectionItem::Request(Box::new(r)));
    let imported = import_collection(&export_collection(&w.collections[0]).unwrap()).unwrap();
    let CollectionItem::Request(r) = &imported.items[0] else {
        panic!()
    };
    assert_eq!(r.path_variables[0].value, "A/B ?");
    assert!(r.url.ends_with("#view"));
    assert_eq!(c, w.collections[0].id);
    let dynamic =
        interpolate("{{$timestamp}} {{$guid}} {{$randomInt}}", &Variables::new()).unwrap();
    let values = dynamic.split_whitespace().collect::<Vec<_>>();
    assert!(values[0].parse::<u64>().unwrap() > 0);
    assert_eq!(values[1].len(), 36);
    assert!(values[2].parse::<u16>().unwrap() <= 1000);
}

#[test]
fn folder_runner_data_locals_globals_headers_and_assertions_use_real_http_and_node() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut service = ApiService::new(dir.path().join("profile"));
    service.set_project(project.clone()).unwrap();
    poll_until(&mut service, |s| !s.loading);
    let (url, received, worker) = fixture_server(4);
    let collection = service.new_collection("Microservices");
    let folder = service.workspace.new_folder(&collection, "DEV").unwrap();
    service
        .workspace
        .globals
        .push(KeyValue::new("scope", "global"));
    let env = service.new_environment("Local");
    service.workspace.active_environment = Some(env);
    service.workspace.environments[0]
        .variables
        .push(KeyValue::new("scope", "environment"));
    service.workspace.collections[0]
        .variables
        .push(KeyValue::new("scope", "collection"));
    service.workspace.collections[0].pre_request_script="pm.variables.set('n', String(Number(pm.variables.get('n') || 0) + 1)); pm.request.headers.upsert({key:'X-Scope',value:pm.variables.get('scope')}); pm.request.headers.upsert({key:'X-Local',value:pm.variables.get('n')}); pm.request.headers.upsert({key:'X-Iteration',value:pm.info.iteration}); pm.globals.set('last_run','ephemeral');".into();
    let mut ids = vec![];
    for name in ["Login", "Protected"] {
        let id = service.new_request(&collection, name).unwrap();
        service.workspace.move_item(&id, &folder).unwrap();
        let r = service.workspace.find_request_mut(&id).unwrap();
        r.set_url(&format!("{url}/customers/:id?a=1&a=2"));
        r.path_variables[0].value = "{{customer}}".into();
        r.pre_request_script="pm.request.url.query.upsert({key:'data',value:pm.iterationData.get('scope')});pm.test('path context',()=>pm.expect(pm.execution.location).to.eql(['Microservices','DEV',pm.info.requestName]));".into();
        r.post_response_script="pm.test('response',()=>{pm.response.to.have.status(200);pm.response.to.have.header('content-type');pm.expect(pm.response.json()).to.have.property('authorized',false);pm.expect(pm.response.responseTime).to.be.at.least(0);pm.expect([1,2,3]).to.include(2);pm.expect({b:2,a:1}).to.deep.equal({a:1,b:2});pm.expect('ok').to.not.be.empty;});".into();
        ids.push(id);
    }
    let outside = service
        .new_request(&collection, "Must not run outside folder")
        .unwrap();
    service.workspace.find_request_mut(&outside).unwrap().url = "invalid-outside".into();
    let options = RunOptions::from_json(
        1,
        r#"[{"customer":"A/B","scope":"D1"},{"customer":"C","scope":"D2"}]"#,
    )
    .unwrap();
    service.run_with_options(&folder, true, options).unwrap();
    poll_until(&mut service, |s| {
        s.runner.as_ref().is_some_and(|r| r.finished)
    });
    worker.join().unwrap();
    let run = service.runner.as_ref().unwrap();
    assert_eq!(run.completed, 4);
    assert_eq!(run.tests_passed, 8);
    assert_eq!(run.tests_failed, 0);
    assert_eq!(run.error, None);
    let requests = received.lock().unwrap();
    assert_eq!(requests.len(), 4);
    for (index, request) in requests.iter().enumerate() {
        let text = request.to_ascii_lowercase();
        assert!(text.contains(&format!("x-local: {}", index + 1)));
        assert!(text.contains(&format!("x-iteration: {}", index / 2)));
        assert!(text.contains(if index < 2 {
            "x-scope: d1"
        } else {
            "x-scope: d2"
        }));
        assert!(request.starts_with(if index < 2 {
            "GET /customers/A%2FB?a=1&a=2&data=D1 "
        } else {
            "GET /customers/C?a=1&a=2&data=D2 "
        }));
    }
    assert!(
        service
            .workspace
            .find_request(&ids[0])
            .unwrap()
            .headers
            .is_empty()
    );
    assert!(!service.workspace.variables_for(&ids[0]).contains_key("n"));
    let snapshot = persistent_snapshot(&service.workspace);
    assert_eq!(
        snapshot
            .globals
            .iter()
            .find(|r| r.key == "last_run")
            .unwrap()
            .value,
        ""
    );
    assert!(RunOptions::from_json(1, "{}").is_err());
    assert!(
        service
            .run_with_options(
                &folder,
                false,
                RunOptions {
                    iterations: 256,
                    data: vec![]
                }
            )
            .is_err()
    );
    service.shutdown();
    poll_until(&mut service, |s| !s.is_busy());
}

#[test]
fn scripts_skip_unsent_requests_route_by_name_and_stop_without_running_outside_folder() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut s = ApiService::new(dir.path().join("profile"));
    s.set_project(project).unwrap();
    poll_until(&mut s, |s| !s.loading);
    let (url, received, worker) = fixture_server(4);
    let c = s.new_collection("Workflows");
    let f = s.workspace.new_folder(&c, "DEV").unwrap();
    let skip = s.new_request(&c, "Skip").unwrap();
    let first = s.new_request(&c, "First").unwrap();
    let bypass = s.new_request(&c, "Bypass").unwrap();
    let last = s.new_request(&c, "Last").unwrap();
    let outside = s.new_request(&c, "Outside").unwrap();
    for id in [&skip, &first, &bypass, &last] {
        s.workspace.move_item(id, &f).unwrap();
    }
    for (id, path) in [(&first, "first"), (&last, "last")] {
        s.workspace.find_request_mut(id).unwrap().url = format!("{url}/{path}");
    }
    s.workspace
        .find_request_mut(&skip)
        .unwrap()
        .pre_request_script = "pm.execution.skipRequest();throw Error('must not execute');".into();
    s.workspace
        .find_request_mut(&first)
        .unwrap()
        .post_response_script = "pm.execution.setNextRequest('Last');".into();
    s.workspace
        .find_request_mut(&last)
        .unwrap()
        .post_response_script = "pm.execution.setNextRequest(null);".into();
    s.run_collection(&f, true).unwrap();
    poll_until(&mut s, |s| s.runner.as_ref().is_some_and(|r| r.finished));
    let run = s.runner.as_ref().unwrap();
    assert_eq!(run.error, None);
    assert_eq!(run.completed, 3);
    assert_eq!(run.results[0].status, None);
    assert!(s.responses[&skip].script_result.skipped);
    // A folder workflow must not escape to a sibling/root request.
    s.workspace
        .find_request_mut(&last)
        .unwrap()
        .post_response_script = format!("pm.execution.setNextRequest('{outside}');");
    s.run_collection(&f, true).unwrap();
    poll_until(&mut s, |s| s.runner.as_ref().is_some_and(|r| r.finished));
    worker.join().unwrap();
    assert!(
        s.runner
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("outside")
    );
    assert_eq!(received.lock().unwrap().len(), 4);
    assert!(
        received
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.starts_with("GET /first ") || r.starts_with("GET /last "))
    );
    s.shutdown();
    poll_until(&mut s, |s| !s.is_busy());
}

#[test]
fn project_globals_migrate_from_old_profiles_and_remain_isolated_and_secret_redacted() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let mut s = ApiService::new(dir.path().join("profile"));
    s.set_project(a.clone()).unwrap();
    poll_until(&mut s, |s| !s.loading);
    s.workspace.globals = vec![
        KeyValue::new("base_url", "http://a.test"),
        KeyValue {
            secret: true,
            ..KeyValue::new("private", "fixture-secret")
        },
    ];
    let c = s.new_collection("A");
    let r = s.new_request(&c, "Only A").unwrap();
    assert_eq!(s.workspace.variables_for(&r)["base_url"], "http://a.test");
    s.set_project(b).unwrap();
    poll_until(&mut s, |s| !s.loading);
    assert!(s.workspace.globals.is_empty());
    assert!(s.workspace.collections.is_empty());
    s.set_project(a).unwrap();
    poll_until(&mut s, |s| !s.loading);
    assert_eq!(s.workspace.globals[0].value, "http://a.test");
    assert!(s.workspace.globals[1].value.is_empty());
    let old: ApiWorkspace =
        serde_json::from_str(r#"{"schema_version":1,"collections":[],"environments":[]}"#).unwrap();
    assert!(old.globals.is_empty());
    s.shutdown();
    poll_until(&mut s, |s| !s.is_busy());
}

#[test]
fn workflow_budget_prevents_a_loop_from_scheduling_more_than_256_sends() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut s = ApiService::new(dir.path().join("profile"));
    s.set_project(project).unwrap();
    poll_until(&mut s, |s| !s.loading);
    let (url, received, worker) = fixture_server(1);
    let c = s.new_collection("Budget");
    let first = s.new_request(&c, "First").unwrap();
    let r = s.workspace.find_request_mut(&first).unwrap();
    r.url = url;
    r.post_response_script = "pm.execution.setNextRequest(pm.info.requestId);".into();
    for _ in 0..255 {
        s.new_request(&c, "Never sent").unwrap();
    }
    s.run_collection(&c, true).unwrap();
    poll_until(&mut s, |s| s.runner.as_ref().is_some_and(|r| r.finished));
    worker.join().unwrap();
    assert!(
        s.runner
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("256")
    );
    assert_eq!(received.lock().unwrap().len(), 1);
    s.shutdown();
    poll_until(&mut s, |s| !s.is_busy());
}

#[test]
fn common_json_assertions_fail_on_invalid_bodies_and_negated_checks_are_real() {
    let response = ApiResponse {
        status: 200,
        headers: vec![KeyValue::new("Content-Type", "application/json")],
        body: b"not-json".to_vec(),
        elapsed_ms: 1,
        size: 8,
        truncated: false,
    };
    let result=run_script("pm.test('JSON must fail',()=>{pm.response.to.be.json;});pm.test('wrong equality must fail',()=>pm.expect(2).to.equal(3));pm.test('negative must fail',()=>pm.expect('x').to.not.equal('x'));pm.test('valid checks',()=>{pm.response.to.be.ok;pm.expect('a').to.not.be.empty;pm.expect(3).to.be.within(1,4);});",&Variables::new(),Some(&response)).unwrap();
    assert_eq!(result.tests.iter().filter(|t| t.passed).count(), 1);
    assert_eq!(result.tests.iter().filter(|t| !t.passed).count(), 3);
    let response = ApiResponse {
        body: br#"{"valid":true}"#.to_vec(),
        ..response
    };
    let result=run_script("pm.test('JSON must pass',()=>{pm.response.to.be.json;pm.expect(pm.response.json()).to.deep.equal({valid:true});});",&Variables::new(),Some(&response)).unwrap();
    assert!(result.tests[0].passed);
    let collection=import_collection(r#"{"info":{"name":"Existing OAuth token"},"auth":{"type":"oauth2","oauth2":[{"key":"accessToken","value":"{{token}}"}]},"item":[]}"#).unwrap();
    assert_eq!(
        collection.auth,
        Auth::Bearer {
            token: "{{token}}".into()
        }
    );
}
