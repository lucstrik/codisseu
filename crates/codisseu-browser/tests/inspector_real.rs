//! Real, private CEF156/CDP inspector. No browser implementation is simulated.
use codisseu_browser::{BrowserService, Command, EngineConfig, EngineState, Surface, inspector::*};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, Command as Process, Stdio},
    thread,
    time::{Duration, Instant},
};
struct Http {
    child: Child,
    origin: String,
}
impl Http {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../");
        let mut child = Process::new(codisseu_runtime::python().unwrap())
            .args(["-I"])
            .arg(root.join("fixtures/browser-inspector/server.py"))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let origin = serde_json::from_str::<serde_json::Value>(&line).unwrap()["origin"]
            .as_str()
            .unwrap()
            .to_owned();
        Self { child, origin }
    }
}
impl Drop for Http {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn wait(
    b: &mut BrowserService,
    i: &mut InspectorService,
    p: impl Fn(&BrowserService, &Snapshot) -> bool,
) {
    let until = Instant::now() + Duration::from_secs(25);
    loop {
        b.poll();
        i.poll();
        assert_ne!(b.state, EngineState::Failed, "{:?}", b.last_error);
        assert_ne!(i.state.lifecycle, Lifecycle::Failed, "{:?}", i.state.error);
        if p(b, &i.state) {
            return;
        }
        assert!(
            Instant::now() < until,
            "timeout snapshot={:?} error={:?} pending={} epoch={}",
            i.state,
            i.state.error,
            i.state.busy,
            i.state.document_epoch
        );
        thread::sleep(Duration::from_millis(3));
    }
}
fn evaluate(b: &mut BrowserService, i: &mut InspectorService, expression: &str, expected: &str) {
    i.dispatch(Action::Evaluate {
        expression: expression.into(),
    })
    .unwrap();
    wait(b, i, |_, s| {
        s.evaluation.as_ref().is_some_and(|e| {
            e.expression == expression
                && e.value
                    .as_ref()
                    .is_some_and(|v| v.description.contains(expected))
        })
    });
}
#[test]
#[ignore = "actual CEF, own authenticated Xvfb and private HTTP fixture"]
fn real_inspector_dom_network_storage_properties_pause_and_cleanup() {
    assert_eq!(std::env::var("CODISSEU_INSPECTOR_REAL").as_deref(), Ok("1"));
    let http = Http::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../");
    let mut b = BrowserService::new();
    b.configure(EngineConfig {
        program: root
            .join(".tools/browser/bin/codisseu-browser-engine")
            .canonicalize()
            .unwrap(),
        runtime_dir: root.join(".tools/browser/cef").canonicalize().unwrap(),
    })
    .unwrap();
    let mut i = InspectorService::new();
    b.dispatch(Command::Open {
        id: 1,
        url: http.origin.clone(),
    })
    .unwrap();
    b.dispatch(Command::Resize {
        id: 1,
        surface: Surface::Page,
        width: 1000,
        height: 700,
        scale: 1.,
    })
    .unwrap();
    b.dispatch(Command::Visible {
        id: 1,
        surface: Surface::Page,
        visible: true,
    })
    .unwrap();
    wait(&mut b, &mut i, |b, _| {
        b.state == EngineState::Ready
            && b.tabs[&1].title.contains("Inspector")
            && !b.tabs[&1].loading
    });
    let pid = b.engine_pid().unwrap();
    let channel = b.devtools(1, 1).unwrap();
    i.start(
        Session {
            tab: 1,
            epoch: 1,
            engine_pid: pid,
            url: http.origin.clone(),
            origin: String::new(),
        },
        channel,
    )
    .unwrap();
    wait(&mut b, &mut i, |_, s| {
        s.lifecycle == Lifecycle::Ready
            && s.dom.iter().any(|n| n.custom_element)
            && s.cookies.iter().any(|c| c.http_only)
            && !s.local_storage.is_empty()
    });
    assert!(i.state.dom.iter().any(|n| n.shadow_root));
    let node = i
        .state
        .dom
        .iter()
        .find(|n| n.local_name == "fixture-counter")
        .unwrap()
        .id;
    i.dispatch(Action::SelectNode { node_id: node }).unwrap();
    wait(&mut b, &mut i, |_, s| {
        s.selected_node == Some(node) && s.styles.iter().any(|(k, _)| k == "display")
    });
    let shadow = i.state.dom.iter().find(|n| n.shadow_root).unwrap().id;
    let before_errors = i.state.errors.len();
    i.dispatch(Action::SelectNode { node_id: shadow }).unwrap();
    wait(&mut b, &mut i, |_, s| {
        s.selected_node == Some(shadow) && s.attributes.is_empty() && s.styles.is_empty() && !s.busy
    });
    assert_eq!(i.state.errors.len(), before_errors);
    let expression =
        "({answer:42,nested:{value:7},get secret(){window.getterInvoked=true;return 'bad'}})";
    i.dispatch(Action::Evaluate {
        expression: expression.into(),
    })
    .unwrap();
    wait(&mut b, &mut i, |_, s| {
        s.evaluation.as_ref().is_some_and(|e| {
            e.expression == expression && e.value.as_ref().is_some_and(|v| v.object_id.is_some())
        })
    });
    let object = i
        .state
        .evaluation
        .as_ref()
        .unwrap()
        .value
        .as_ref()
        .unwrap()
        .object_id
        .clone()
        .unwrap();
    i.dispatch(Action::GetProperties { object_id: object })
        .unwrap();
    wait(&mut b, &mut i, |_, s| {
        s.properties
            .iter()
            .any(|p| p.name == "answer" && p.value.description == "42")
    });
    assert!(
        i.state
            .properties
            .iter()
            .any(|p| p.name == "secret" && p.getter)
    );
    evaluate(&mut b, &mut i, "typeof window.getterInvoked", "undefined");
    evaluate(&mut b, &mut i, "sendProbe()", "Promise");
    wait(&mut b, &mut i, |_, s| {
        s.network.iter().any(|r| {
            r.url.ends_with("/api/echo")
                && r.finished
                && r.status == Some(201.)
                && r.decoded_bytes.is_some()
        })
    });
    let request = i
        .state
        .network
        .iter()
        .find(|r| r.url.ends_with("/api/echo"))
        .unwrap();
    assert_eq!(request.method, "POST");
    assert!(request.post_data.as_ref().unwrap().contains("19"));
    assert!(
        request
            .request_headers
            .iter()
            .any(|(n, v)| n.eq_ignore_ascii_case("authorization") && v.contains("fixture-bearer"))
    );
    let id = request.id.clone();
    i.dispatch(Action::GetResponseBody {
        request_id: id.clone(),
    })
    .unwrap();
    wait(&mut b, &mut i, |_, s| {
        s.bodies
            .get(&id)
            .is_some_and(|b| b.text.contains("42") && b.error.is_none())
    });
    let response: serde_json::Value = serde_json::from_str(&i.state.bodies[&id].text).unwrap();
    assert_eq!(response["answer"], 42);
    i.dispatch(Action::SetStorage {
        kind: StorageKind::Local,
        key: "access_token".into(),
        value: "edited-local-43".into(),
    })
    .unwrap();
    wait(&mut b, &mut i, |_, s| {
        s.local_storage
            .iter()
            .any(|(k, v)| k == "access_token" && v == "edited-local-43")
    });
    evaluate(
        &mut b,
        &mut i,
        "localStorage.getItem('access_token')",
        "edited-local-43",
    );
    i.dispatch(Action::DeleteStorage {
        kind: StorageKind::Session,
        key: "session_token".into(),
    })
    .unwrap();
    wait(&mut b, &mut i, |_, s| {
        !s.session_storage.iter().any(|(k, _)| k == "session_token")
    });
    evaluate(
        &mut b,
        &mut i,
        "sessionStorage.getItem('session_token')",
        "null",
    );
    let mut cookie = i
        .state
        .cookies
        .iter()
        .find(|c| c.name == "fixture_edit")
        .unwrap()
        .clone();
    cookie.value = "edited-cookie-43".into();
    i.dispatch(Action::SetCookie { cookie }).unwrap();
    wait(&mut b, &mut i, |_, s| {
        s.cookies
            .iter()
            .any(|c| c.name == "fixture_edit" && c.value == "edited-cookie-43")
    });
    let cookie = i
        .state
        .cookies
        .iter()
        .find(|c| c.name == "fixture_edit")
        .unwrap()
        .clone();
    i.dispatch(Action::DeleteCookie {
        name: cookie.name,
        domain: cookie.domain,
        path: cookie.path,
    })
    .unwrap();
    wait(&mut b, &mut i, |_, s| {
        !s.cookies.iter().any(|c| c.name == "fixture_edit")
    });
    i.dispatch(Action::SetWatches(vec![
        "result".into(),
        "left+right".into(),
    ]))
    .unwrap();
    i.dispatch(Action::Evaluate {
        expression: "setTimeout(()=>computeProbe(),0)".into(),
    })
    .unwrap();
    wait(&mut b, &mut i, |_, s| {
        s.paused.is_some()
            && s.scope_values
                .values()
                .flatten()
                .any(|p| p.name == "result" && p.value.description == "42")
            && s.watches
                .iter()
                .any(|w| w.value.as_ref().is_some_and(|v| v.description == "42"))
    });
    assert!(
        i.state.paused.as_ref().unwrap().frames[0]
            .function_name
            .contains("computeProbe")
    );
    i.dispatch(Action::StepOver).unwrap();
    let line = i.state.paused.as_ref().unwrap().frames[0].line;
    wait(&mut b, &mut i, |_, s| {
        s.paused.as_ref().is_some_and(|p| p.frames[0].line > line)
    });
    i.dispatch(Action::Resume).unwrap();
    wait(&mut b, &mut i, |b, s| {
        s.paused.is_none() && b.tabs[&1].title == "InspectorCompute:42"
    });
    let old_node = i.state.dom.first().unwrap().id;
    evaluate(
        &mut b,
        &mut i,
        "(()=>{for(let n=0;n<20000;n++){const e=document.createElement('div');e.textContent='bounded😀';document.body.append(e)}return document.querySelectorAll('*').length})()",
        "200",
    );
    i.dispatch(Action::RefreshDom).unwrap();
    wait(&mut b, &mut i, |_, s| {
        s.dom.len() == 500 && s.dom_truncated && !s.busy
    });
    assert!(!i.state.dom.iter().any(|n| n.id == old_node));
    evaluate(
        &mut b,
        &mut i,
        "(()=>{document.body.replaceChildren();let p=document.body;for(let n=0;n<6;n++){const e=document.createElement('div');p.append(e);p=e}const f=document.createDocumentFragment();for(let n=0;n<20000;n++){const e=document.createElement('span');e.textContent='😀'.repeat(200)+'\\ud800';f.append(e)}p.append(f);return 'deep bounded'})()",
        "deep bounded",
    );
    let previous_tree = i.state.dom[0].id;
    let deep_start = Instant::now();
    i.dispatch(Action::RefreshDom).unwrap();
    wait(&mut b, &mut i, |_, s| {
        s.dom.first().is_some_and(|n| n.id != previous_tree) && s.dom_truncated && !s.busy
    });
    assert!(i.state.dom.len() < 30);
    assert!(deep_start.elapsed() < Duration::from_secs(3));
    let old_document = i.state.document_epoch;
    let second = Http::new();
    b.dispatch(Command::Navigate {
        id: 1,
        url: second.origin.clone(),
    })
    .unwrap();
    wait(&mut b, &mut i, |b, s| {
        !b.tabs[&1].loading
            && s.document_epoch > old_document
            && s.session
                .as_ref()
                .is_some_and(|v| v.origin == second.origin)
            && !s.dom.is_empty()
            && !s.busy
    });
    assert!(i.state.bodies.is_empty());
    assert!(i.state.evaluation.is_none());
    assert!(i.state.paused.is_none());
    assert!(!i.state.dom.iter().any(|n| n.id == old_node));
    assert!(!format!("{:?}", i.state).contains("fixture-cookie-secret"));
    i.stop();
    b.end_devtools(1);
    assert_eq!(i.state.lifecycle, Lifecycle::Stopped);
    assert!(i.state.cookies.is_empty() && i.state.local_storage.is_empty());
    b.shutdown();
    wait(&mut b, &mut i, |b, _| {
        b.state == EngineState::Stopped && b.engine_pid().is_none()
    });
    let out = root.join("artifacts/browser-inspector-20261102/backend");
    fs::create_dir_all(&out).unwrap();
    fs::write(out.join("real-result.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","helper_pid_reaped":pid,"dom_limit":500,"large_dom":20000,"network":"POST JSON201/body42","storage":"local/session mutation+page verification","cookies":"HttpOnly read/edit/delete","debug":"pause/locals42/watches/step/resume","properties":"own getters not invoked","wire":2})).unwrap()).unwrap();
}
