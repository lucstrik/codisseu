//! Actual CEF OSR pixels and DOM, with an owned HTTP fixture/Xvfb profile.
//! Run through artifacts/browser-integration-20261102/ui/run.py.
use super::*;
use codisseu_core::browser::{DevToolsChannel, DevToolsEvent, EngineConfig, EngineState};
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Instant,
};
const DIRTY: &str = "// código preservado · ação 😀\nlet answer = 42;\n";
struct Http {
    origin: String,
    loads: Arc<AtomicUsize>,
    trigger: Arc<AtomicBool>,
    polling: Arc<AtomicBool>,
    completed: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    task: Option<thread::JoinHandle<()>>,
}
impl Http {
    fn new() -> Self {
        Self::mode(false)
    }
    fn with_trigger() -> Self {
        Self::mode(true)
    }
    fn mode(autotrigger: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let loads = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let trigger = Arc::new(AtomicBool::new(false));
        let polling = Arc::new(AtomicBool::new(false));
        let completed = Arc::new(AtomicUsize::new(0));
        let t = trigger.clone();
        let p = polling.clone();
        let done = completed.clone();
        let l = loads.clone();
        let s = stop.clone();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/browser");
        let task = thread::spawn(move || {
            while !s.load(Ordering::Acquire) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(3));
                    continue;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = [0u8; 8192];
                let count = stream.read(&mut request).unwrap_or(0);
                let first = String::from_utf8_lossy(&request[..count]);
                let target = first.split_whitespace().nth(1).unwrap_or("/");
                let path = target.split('?').next().unwrap();
                let (status, mime, body) = match path {
                    "/" | "/index.html" => {
                        l.fetch_add(1, Ordering::Relaxed);
                        let mut body = fs::read_to_string(root.join("index.html")).unwrap();
                        if autotrigger {
                            body = body.replace("</html>", "<script>setInterval(async()=>{const r=await fetch('/trigger');const d=await r.json();if(d.run){const result=compute(19,23);await fetch('/done?value='+result)}},20)</script></html>");
                        }
                        ("200 OK", "text/html; charset=utf-8", body.into_bytes())
                    }
                    "/login" => ("200 OK","text/html; charset=utf-8",br#"<!doctype html><title>Frontend local</title><style>body{font:16px sans-serif;background:#ddd}input,button{padding:12px;margin:12px}</style><form onsubmit='event.preventDefault()'><label>Usuario<input id=u autocomplete=username value='fixture@example.invalid'></label><label>Senha<input id=p type=password autocomplete=current-password value='owned-ui-login-only'></label><button>Entrar</button></form>"#.to_vec()),
                    "/second" => (
                        "200 OK",
                        "text/html; charset=utf-8",
                        fs::read(root.join("second.html")).unwrap(),
                    ),
                    "/app.js" => (
                        "200 OK",
                        "text/javascript; charset=utf-8",
                        fs::read(root.join("app.js")).unwrap(),
                    ),
                    "/style.css" => (
                        "200 OK",
                        "text/css; charset=utf-8",
                        fs::read(root.join("style.css")).unwrap(),
                    ),
                    "/trigger" => {
                        p.store(true, Ordering::Release);
                        (
                            "200 OK",
                            "application/json",
                            serde_json::to_vec(
                                &serde_json::json!({"run":t.swap(false,Ordering::AcqRel)}),
                            )
                            .unwrap(),
                        )
                    }
                    "/done" => {
                        if let Some(value) = target
                            .strip_prefix("/done?value=")
                            .and_then(|v| v.parse().ok())
                        {
                            done.store(value, Ordering::Release);
                        }
                        ("200 OK", "text/plain", b"ok".to_vec())
                    }
                    "/data" => ("200 OK", "application/json", br#"{"value":42}"#.to_vec()),
                    _ => ("404 Not Found", "text/plain", b"not found".to_vec()),
                };
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        Self {
            origin,
            loads,
            trigger,
            polling,
            completed,
            stop,
            task: Some(task),
        }
    }
}
impl Drop for Http {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.origin.trim_start_matches("http://"));
        if let Some(task) = self.task.take() {
            task.join().unwrap();
        }
    }
}
fn wait(h: &mut Harness<'_, CodisseuApp>, predicate: impl Fn(&CodisseuApp) -> bool) {
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        h.step();
        assert_ne!(
            h.state().service.browser.state,
            EngineState::Failed,
            "browser failed: {}",
            h.state().browser_capture_state()
        );
        if predicate(h.state()) {
            h.step();
            return;
        }
        assert!(
            Instant::now() < until,
            "browser={} error={:?}",
            h.state().browser_capture_state(),
            h.state().service.last_error
        );
        thread::sleep(Duration::from_millis(8));
    }
}
fn key(h: &mut Harness<'_, CodisseuApp>, m: Modifiers, k: Key) {
    // Match eframe's Linux input: physical Ctrl also sets logical command.
    let m = if m.ctrl { m | Modifiers::COMMAND } else { m };
    h.key_press_modifiers(m, k);

    h.step();

    h.step();
}
fn click(h: &mut Harness<'_, CodisseuApp>, p: egui::Pos2) {
    h.hover_at(p);
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.step();
}
fn capture(h: &mut Harness<'_, CodisseuApp>, scale: f32, compact: bool, name: &str) {
    let dir = std::env::var_os("CODISSEU_BROWSER_CAPTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../artifacts/browser-integration-20261102/ui")
        })
        .join(format!(
            "{}-{}",
            if scale == 1. { "100" } else { "125" },
            if compact { "compact" } else { "normal" }
        ));
    fs::create_dir_all(&dir).unwrap();
    h.render()
        .unwrap()
        .save(dir.join(format!("{name}.png")))
        .unwrap();
    fs::write(
        dir.join(format!("{name}.json")),
        serde_json::to_vec_pretty(&h.state().browser_capture_state()).unwrap(),
    )
    .unwrap();
}
fn setup(scale: f32, compact: bool) -> (tempfile::TempDir, Harness<'static, CodisseuApp>, PathBuf) {
    assert_eq!(
        std::env::var("CODISSEU_BROWSER_UI").as_deref(),
        Ok("1"),
        "use owned Xvfb launcher"
    );
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let file = root.join("main.js");
    fs::write(&file, "let answer = 0;\n").unwrap();
    let mut service = AppService::new(dir.path().join("profile/session.json"));
    service
        .dispatch(Command::OpenWorkspace(root.clone()))
        .unwrap();
    let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.tools/browser");
    service
        .browser
        .configure(EngineConfig {
            program: tools
                .join("bin/codisseu-browser-engine")
                .canonicalize()
                .unwrap(),
            runtime_dir: tools.join("cef").canonicalize().unwrap(),
        })
        .unwrap();
    let mut h = Harness::builder()
        .with_size(if compact { [900., 650.] } else { [1280., 800.] })
        .with_pixels_per_point(scale)
        .with_max_steps(180)
        .build_ui_state(
            |ui, a: &mut CodisseuApp| a.show(ui),
            CodisseuApp::new(service),
        );
    wait(&mut h, |a| !a.service.is_busy());
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(file.clone()))
        .unwrap();
    wait(&mut h, |a| !a.service.is_busy());
    let d = h.state().service.active_document().unwrap();
    let cmd = Command::Edit {
        id: d.id,
        expected_version: d.version,
        text: DIRTY.into(),
    };
    h.state_mut().service.dispatch(cmd).unwrap();
    h.step();
    assert_eq!(h.state().service.active_document().unwrap().text(), DIRTY);
    (dir, h, file)
}
fn eval(
    h: &mut Harness<'_, CodisseuApp>,
    channel: &DevToolsChannel,
    id: u64,
    expression: &str,
) -> serde_json::Value {
    channel.sender.send(serde_json::json!({"id":id,"method":"Runtime.evaluate","params":{"expression":expression,"returnByValue":true}}).to_string()).unwrap();
    let until = Instant::now() + Duration::from_secs(8);
    loop {
        h.step();
        match channel.events.recv_timeout(Duration::from_millis(10)) {
            Ok(DevToolsEvent::Message { json }) => {
                let v: serde_json::Value = serde_json::from_str(&json).unwrap();
                if v["id"] == id {
                    assert!(v.get("error").is_none(), "{v}");
                    return v["result"]["result"]["value"].clone();
                }
            }
            Ok(e) => panic!("CDP closed: {e:?}"),
            Err(_) => {}
        }
        assert!(
            Instant::now() < until,
            "actual CEF CDP timeout for {expression}; browser={} error={:?}",
            h.state().browser_capture_state(),
            h.state().service.browser.last_error
        );
    }
}
fn dom_wait(
    h: &mut Harness<'_, CodisseuApp>,
    c: &DevToolsChannel,
    expression: &str,
    expected: serde_json::Value,
) {
    let until = Instant::now() + Duration::from_secs(8);
    while eval(h, c, 500, expression) != expected {
        assert!(
            Instant::now() < until,
            "DOM predicate timeout: {expression}"
        );
        thread::sleep(Duration::from_millis(8));
    }
}
fn dom_click(h: &mut Harness<'_, CodisseuApp>, c: &DevToolsChannel, selector: &str) {
    let value = eval(
        h,
        c,
        600,
        &format!(
            "(()=>{{const e=document.querySelector({});e.scrollIntoView({{block:'center',inline:'center'}});const r=e.getBoundingClientRect();return [r.x+r.width/2,r.y+r.height/2]}})()",
            serde_json::to_string(selector).unwrap()
        ),
    );
    let rect = h.get_by_label("Página do navegador").rect();
    click(
        h,
        rect.min
            + egui::vec2(
                value[0].as_f64().unwrap() as f32,
                value[1].as_f64().unwrap() as f32,
            ),
    );
}
fn address(h: &mut Harness<'_, CodisseuApp>, url: &str) {
    key(h, Modifiers::CTRL, Key::L);
    h.event(egui::Event::Text(url.into()));
    h.step();
    key(h, Modifiers::NONE, Key::Enter);
    wait(h, |a| a.browser_capture_ready());
}
fn native_clipboard_roundtrip(h: &mut Harness<'_, CodisseuApp>, c: &DevToolsChannel) -> String {
    key(h, Modifiers::NONE, Key::Backspace);
    dom_wait(
        h,
        c,
        "document.querySelector('#name').value",
        serde_json::json!(""),
    );
    let value = eval(
        h,
        c,
        3,
        "(()=>{const r=document.querySelector('#name').getBoundingClientRect();return [r.x+r.width/2,r.y+r.height/2]})()",
    );
    let pane = h.get_by_label("Página do navegador").rect();
    let point = pane.min
        + egui::vec2(
            value[0].as_f64().unwrap() as f32,
            value[1].as_f64().unwrap() as f32,
        );
    h.hover_at(point);
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.step();
    h.get_by_label("Colar").hover();
    h.step();
    h.get_by_label("Colar").click();
    h.step();
    assert!(h.output().viewport_output.values().any(|v| {
        v.commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::RequestPaste))
    }));
    // Harness has no eframe event loop: emulate only its OS clipboard boundary,
    // using the actual CEF-owned X11 selection, then deliver the real Paste event.
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/browser-integration-20261102/ui/clipboard.py");
    let clipboard = std::process::Command::new("python3")
        .arg(script)
        .output()
        .unwrap();
    assert!(
        clipboard.status.success(),
        "{}",
        String::from_utf8_lossy(&clipboard.stderr)
    );
    let text = String::from_utf8(clipboard.stdout).unwrap();
    assert_eq!(text, "ação 😀 + colar 🦀");
    h.event(egui::Event::Paste(text.clone()));
    h.step();
    dom_wait(
        h,
        c,
        "document.querySelector('#name').value",
        serde_json::json!("ação 😀 + colar 🦀"),
    );
    text
}
#[test]
#[ignore = "actual private sandboxed CEF, owned Xvfb, WGPU, four scale/size cases"]
fn cef_browser_native_widgets_dom_input_history_tabs_split_and_dirty_source() {
    let http = Http::new();
    for (scale, compact) in [(1., false), (1.25, false), (1., true), (1.25, true)] {
        let (_profile, mut h, file) = setup(scale, compact);
        let browser = h.get_by_label("Abrir navegador integrado").rect();
        let build = h.get_by_label("Build Rust · Cargo build · Ctrl+F9").rect();
        assert!((browser.center().y - build.center().y).abs() < 1.);
        assert!((browser.right() - build.left()).abs() < 1.);
        assert!(browser.left() >= 0. && browser.top() >= 0.);
        assert!(
            !h.get_by_label("Abrir navegador integrado")
                .accesskit_node()
                .is_disabled()
        );
        capture(&mut h, scale, compact, "00-browser-toolbar");
        if compact {
            h.get_by_label("Abrir navegador integrado").click();
        } else {
            if h.query_by_label("Exibir").is_none() {
                h.get_by_label("Menu principal").click();
                h.step();
            }
            h.get_by_label("Exibir").hover();
            h.step();
            h.get_by_label("Exibir").click();
            h.step();
            h.step();
            h.get_by_label("Navegador integrado…").click();
        }
        h.step();
        wait(&mut h, |a| a.browser_capture_ready());
        address(&mut h, &http.origin);
        let id = h.state().browser_active().unwrap();
        let c = h.state_mut().service.browser.devtools(id, 100).unwrap();
        dom_wait(
            &mut h,
            &c,
            "document.querySelector('#fetch-result')?.textContent",
            serde_json::json!("Fetch: 42"),
        );
        assert!(
            h.state()
                .service
                .browser
                .versions
                .as_ref()
                .unwrap()
                .chromium
                .starts_with("156.")
        );
        assert_eq!(
            eval(&mut h, &c, 1, "document.title"),
            "Codisseu · Browser fixture"
        );
        capture(&mut h, scale, compact, "01-page-real-cef");
        dom_click(&mut h, &c, "#calculate");
        dom_wait(&mut h, &c, "window.lastResult", serde_json::json!(42));
        dom_click(&mut h, &c, "#name");
        h.event(egui::Event::Ime(egui::ImeEvent::Commit("ação 😀".into())));
        h.step();
        dom_wait(
            &mut h,
            &c,
            "document.querySelector('#name').value",
            serde_json::json!("ação 😀"),
        );
        h.event(egui::Event::Paste(" + colar 🦀".into()));
        h.step();
        dom_wait(
            &mut h,
            &c,
            "document.querySelector('#name').value",
            serde_json::json!("ação 😀 + colar 🦀"),
        );
        // Browser focus preserves the global settings shortcut and prevents a
        // document breakpoint shortcut from acting on the source behind it.
        let breakpoints = h.state().service.debugger.breakpoints.len();
        key(&mut h, Modifiers::CTRL, Key::F8);
        assert_eq!(h.state().service.debugger.breakpoints.len(), breakpoints);
        assert_eq!(h.state().service.active_document().unwrap().text(), DIRTY);
        key(&mut h, Modifiers::CTRL.plus(Modifiers::ALT), Key::S);
        assert!(h.state().preferences_dialog);
        h.get_by_label("Concluído").hover();
        h.step();
        h.get_by_label("Concluído").click();
        h.step();
        h.step();
        assert!(!h.state().preferences_dialog);
        dom_click(&mut h, &c, "#name");
        // Browser selection/copy is issued through the native egui event route.
        key(&mut h, Modifiers::CTRL, Key::A);
        h.event(egui::Event::Copy);
        h.step();
        assert_eq!(
            eval(
                &mut h,
                &c,
                2,
                "document.querySelector('#name').selectionEnd-document.querySelector('#name').selectionStart"
            ),
            "ação 😀 + colar 🦀".encode_utf16().count()
        );
        capture(&mut h, scale, compact, "02-unicode-input");
        dom_click(&mut h, &c, "#next");
        wait(&mut h, |a| {
            a.browser_capture_ready()
                && a.service
                    .browser
                    .tabs
                    .get(&id)
                    .is_some_and(|t| t.url.ends_with("/second"))
        });
        capture(&mut h, scale, compact, "03-history-second-page");
        h.get_by_label("Voltar no navegador (Alt+Left)").click();
        h.step();
        wait(&mut h, |a| {
            a.browser_capture_ready()
                && a.service
                    .browser
                    .tabs
                    .get(&id)
                    .is_some_and(|t| !t.url.ends_with("/second"))
        });
        h.get_by_label("Avançar no navegador (Alt+Right)").click();
        h.step();
        wait(&mut h, |a| {
            a.browser_capture_ready()
                && a.service
                    .browser
                    .tabs
                    .get(&id)
                    .is_some_and(|t| t.url.ends_with("/second"))
        });
        key(&mut h, Modifiers::ALT, Key::ArrowLeft);
        wait(&mut h, |a| {
            a.browser_capture_ready()
                && a.service
                    .browser
                    .tabs
                    .get(&id)
                    .is_some_and(|t| !t.url.ends_with("/second"))
        });
        let loads = http.loads.load(Ordering::Acquire);
        key(&mut h, Modifiers::NONE, Key::F5);
        wait(&mut h, |a| {
            a.browser_capture_ready() && http.loads.load(Ordering::Acquire) > loads
        });
        dom_wait(
            &mut h,
            &c,
            "document.querySelector('#fetch-result')?.textContent",
            serde_json::json!("Fetch: 42"),
        );
        let rect = h.get_by_label("Página do navegador").rect();
        h.hover_at(rect.center());
        h.step();
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            delta: egui::vec2(0., -480.),
            modifiers: Modifiers::NONE,
        });
        h.step();
        dom_wait(&mut h, &c, "window.scrollY > 0", serde_json::json!(true));
        capture(&mut h, scale, compact, "04-real-scroll");
        drop(c);
        h.get_by_label("Nova aba de navegador").click();
        h.step();
        wait(&mut h, |a| {
            a.browser_capture_ready() && a.service.browser.tabs.len() == 2
        });
        address(&mut h, &format!("{}/second", http.origin));
        let second = h.state().browser_active().unwrap();
        assert_ne!(id, second);
        key(&mut h, Modifiers::CTRL, Key::Tab);
        assert_eq!(h.state().service.active_document().unwrap().text(), DIRTY);
        // Native group menu creates a code/browser split without modifying either source.
        h.get_by_label("Mais ações do grupo").hover();
        h.step();
        h.get_by_label("Mais ações do grupo").click();
        h.step();
        h.step();
        h.get_by_label("Dividir à direita").click();
        h.step();
        h.step();
        assert_eq!(h.state().service.document_views().len(), 2);
        let first_editor = h
            .get_all_by_role(Role::MultilineTextInput)
            .map(|node| node.rect())
            .min_by(|a, b| a.left().total_cmp(&b.left()))
            .unwrap();
        click(&mut h, first_editor.min + egui::vec2(30., 15.));
        key(&mut h, Modifiers::CTRL, Key::Tab);
        wait(&mut h, |a| {
            a.browser_capture_ready() && a.browser_active() == Some(id)
        });
        assert!(h.query_by_label("Página do navegador").is_some());
        let inspect = h.state_mut().service.browser.devtools(id, 101).unwrap();
        eval(&mut h, &inspect, 800, "window.scrollTo(0,0)");
        dom_wait(&mut h, &inspect, "window.scrollY", serde_json::json!(0));
        let resized = h.state().browser_capture_state()["surfaces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["surface"] == "Page")
            .unwrap()["seq"]
            .as_u64()
            .unwrap();
        wait(&mut h, |a| {
            a.browser_capture_state()["surfaces"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["surface"] == "Page" && s["seq"].as_u64().unwrap_or(0) > resized)
        });
        drop(inspect);
        capture(&mut h, scale, compact, "05-code-browser-split");
        assert_eq!(h.state().service.active_document().unwrap().text(), DIRTY);
        assert_eq!(fs::read_to_string(file).unwrap(), "let answer = 0;\n");
        assert_eq!(
            h.ctx.memory(|m| m.focused()),
            Some(egui::Id::unique((
                "browser.surface",
                id,
                codisseu_core::browser::Surface::Page
            ))),
            "CtrlTab must focus the browser surface"
        );
        key(&mut h, Modifiers::CTRL, Key::W);
        assert!(
            !h.state().service.browser.tabs.contains_key(&id),
            "CtrlW did not close active browser; state={} confirm={}",
            h.state().browser_capture_state(),
            h.state().confirm.is_some()
        );
        assert_eq!(h.state().service.browser.tabs.len(), 1);
        key(&mut h, Modifiers::CTRL, Key::W);
        assert!(h.state().service.browser.tabs.is_empty());
        wait(&mut h, |a| {
            a.service.browser.state == EngineState::Stopped
                && a.service.browser.engine_pid().is_none()
        });
    }
}

#[test]
#[ignore = "actual CEF system clipboard under owned Xvfb; independent regression"]
fn cef_native_clipboard_copy_and_context_paste_roundtrip() {
    let http = Http::new();
    for (scale, compact) in [(1., false), (1.25, false), (1., true), (1.25, true)] {
        let (_profile, mut h, _file) = setup(scale, compact);
        h.state_mut().open_browser_url(&http.origin);
        wait(&mut h, |a| a.browser_capture_ready());
        let id = h.state().browser_active().unwrap();
        let c = h.state_mut().service.browser.devtools(id, 1).unwrap();
        dom_wait(
            &mut h,
            &c,
            "document.querySelector('#name') !== null",
            serde_json::json!(true),
        );
        dom_click(&mut h, &c, "#name");
        h.event(egui::Event::Ime(egui::ImeEvent::Commit(
            "ação 😀 + colar 🦀".into(),
        )));
        h.step();
        dom_wait(
            &mut h,
            &c,
            "document.querySelector('#name').value",
            serde_json::json!("ação 😀 + colar 🦀"),
        );
        key(&mut h, Modifiers::CTRL, Key::A);
        h.event(egui::Event::Copy);
        h.step();
        let clipboard = native_clipboard_roundtrip(&mut h, &c);
        capture(&mut h, scale, compact, "06-native-clipboard-roundtrip");
        // A real backend reply arriving after the originating tab closes must not
        // become an edit in the code view that receives focus next.
        let pane = h.get_by_label("Página do navegador").rect();
        let point = pane.center();
        h.hover_at(point);
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            h.step();
        }
        h.step();
        h.get_by_label("Colar").hover();
        h.step();
        h.get_by_label("Colar").click();
        h.step();
        assert!(h.output().viewport_output.values().any(|v| {
            v.commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::RequestPaste))
        }));
        drop(c);
        h.state_mut().browser_close(id);
        h.step();
        h.step();
        h.get_by_role(Role::MultilineTextInput).focus();
        h.step();
        h.event(egui::Event::Paste(clipboard));
        h.step();
        h.step();
        assert_eq!(
            h.state().service.active_document().unwrap().text(),
            DIRTY,
            "late browser clipboard response must not edit source"
        );
        wait(&mut h, |a| a.service.browser.state == EngineState::Stopped);
    }
}

#[test]
#[ignore = "actual CEF and private Xvfb, native debug configuration modal, four cases"]
fn cef_browser_debug_review_modal_blocks_page_and_first_click_cancel() {
    let http = Http::new();
    for (scale, compact) in [(1., false), (1.25, false), (1., true), (1.25, true)] {
        let (_profile, mut h, _file) = setup(scale, compact);
        h.state_mut().open_browser_url(&http.origin);
        wait(&mut h, |a| a.browser_capture_ready());
        let id = h.state().browser_active().unwrap();
        let c = h.state_mut().service.browser.devtools(id, 1).unwrap();
        dom_wait(
            &mut h,
            &c,
            "document.querySelector('#name') !== null",
            serde_json::json!(true),
        );
        dom_click(&mut h, &c, "#name");
        h.get_by_label("Depurar esta aba na IDE").hover();
        h.step();
        h.get_by_label("Depurar esta aba na IDE").click();
        h.step();
        h.step();
        assert!(h.state().debugger_ui.dialog);
        assert!(
            h.get_by_label("Start Debug Configuration")
                .accesskit_node()
                .is_disabled()
        );
        let before = eval(&mut h, &c, 700, "document.querySelector('#name').value");
        h.event(egui::Event::Text("modal owns keyboard".into()));
        h.step();
        assert_eq!(
            eval(&mut h, &c, 701, "document.querySelector('#name').value"),
            before
        );
        assert_eq!(h.state().service.active_document().unwrap().text(), DIRTY);
        assert!(h.state().service.debugger.sessions.is_empty());
        capture(&mut h, scale, compact, "07-debug-review-modal");
        let button = h.get_by_label("Cancel").rect();
        assert!(button.min.x >= 0. && button.min.y >= 0.);
        assert!(button.max.x <= if compact { 900. } else { 1280. });
        assert!(button.max.y <= if compact { 650. } else { 800. });
        h.get_by_label("Cancel").hover();
        h.step();
        assert_eq!(button, h.get_by_label("Cancel").rect());
        h.get_by_label("Cancel").click();
        h.step();
        h.step();
        assert!(
            !h.state().debugger_ui.dialog,
            "first Cancel click must close review"
        );
        assert!(h.state().service.debugger.sessions.is_empty());
        dom_click(&mut h, &c, "#name");
        key(&mut h, Modifiers::SHIFT, Key::F9);
        assert!(h.state().debugger_ui.dialog);
        assert!(
            h.state()
                .debugger_ui
                .draft
                .as_ref()
                .is_some_and(|d| d.integrated_browser && d.browser_tab == Some(id))
        );
        h.get_by_label("Cancel").hover();
        h.step();
        h.get_by_label("Cancel").click();
        h.step();
        h.step();
        assert!(!h.state().debugger_ui.dialog);
        assert!(h.state().service.debugger.sessions.is_empty());
        drop(c);
        h.state_mut().service.browser.shutdown();
        wait(&mut h, |a| a.service.browser.state == EngineState::Stopped);
    }
}
#[test]
#[ignore = "actual CEF SDK DevTools OSR frontend, private page gateway, four cases"]
fn cef_browser_f12_actual_frontend_surface_first_click_close_and_focus() {
    let http = Http::new();
    for (scale, compact) in [(1., false), (1.25, false), (1., true), (1.25, true)] {
        let (_profile, mut h, _file) = setup(scale, compact);
        h.state_mut().open_browser_url(&http.origin);
        wait(&mut h, |a| a.browser_capture_ready());
        let id = h.state().browser_active().unwrap();
        // No test CDP lease: only the app's explicit F12 gateway owns this page.
        key(&mut h, Modifiers::NONE, Key::F12);
        wait(&mut h, |a| {
            a.browser_capture_state()["surfaces"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["surface"] == "DevTools" && s["seq"].as_u64().unwrap_or(0) > 0)
        });
        assert!(h.query_by_label("DevTools do navegador").is_some());
        wait(&mut h, |a| {
            a.browser_capture_state()["devtools_state"]["gateways"]
                .as_array()
                .is_some_and(|gateways| {
                    gateways
                        .iter()
                        .any(|g| g["id"] == id && g["connected"] == true)
                })
        });
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            h.step();
            let rect = h.get_by_label("DevTools do navegador").rect();
            let image = h.render().unwrap();
            let mut dark = 0;
            let mut total_samples: usize = 0;
            let mut shades = std::collections::BTreeSet::new();
            for y in (rect.top() as u32 + 8..rect.bottom() as u32 - 8).step_by(8) {
                for x in (rect.left() as u32 + 8..rect.right() as u32 - 8).step_by(8) {
                    let p = image
                        .get_pixel((x as f32 * scale) as u32, (y as f32 * scale) as u32)
                        .0;
                    total_samples += 1;
                    dark += usize::from(p[0] < 180 && p[1] < 180 && p[2] < 180);
                    shades.insert([p[0], p[1], p[2]]);
                }
            }
            // Compact SDK viewports contain fewer sampled pixels (and may show
            // Chromium's locale banner). Preserve the normal50 ceiling while
            // requiring real text/controls in at least about2% of this viewport.
            let threshold_dark = total_samples.div_ceil(50).clamp(8, 50);
            if dark > threshold_dark && shades.len() > 20 {
                break;
            }
            if Instant::now() >= until
                && let Some(directory) = std::env::var_os("CODISSEU_F12_DIAGNOSTICS")
            {
                let directory = PathBuf::from(directory);
                fs::create_dir_all(&directory).unwrap();
                let name = format!(
                    "f12-{}-{}-timeout",
                    if scale == 1. { "100" } else { "125" },
                    if compact { "compact" } else { "normal" }
                );
                // Persist the exact image already sampled, without another render/poll.
                image.save(directory.join(format!("{name}.png"))).unwrap();
                fs::write(
                    directory.join(format!("{name}.json")),
                    serde_json::to_vec_pretty(&serde_json::json!({
                        "scale":scale,"compact":compact,"dark":dark,
                        "shades":shades.len(),"totalSamples":total_samples,"threshold_dark":threshold_dark,
                        "rect":{"min":[rect.min.x,rect.min.y],"max":[rect.max.x,rect.max.y]},
                        "image_width":image.width(),"image_height":image.height(),
                        "browser_capture_state":h.state().browser_capture_state()
                    }))
                    .unwrap(),
                )
                .unwrap();
            }
            assert!(
                Instant::now() < until,
                "SDK frontend did not draw actual UI"
            );
            thread::sleep(Duration::from_millis(10));
        }
        capture(&mut h, scale, compact, "08-devtools-sdk-real");
        let pane = h.get_by_label("DevTools do navegador").rect();
        // Focusing the surface must not activate the SDK's locale-banner button.
        click(&mut h, egui::pos2(pane.left() + 8., pane.top() + 8.));
        // Official SDK156 console.toggle (Ctrl+Backtick) opens Console and
        // focuses its real prompt without clicking locale/settings controls.
        key(&mut h, Modifiers::CTRL, Key::Backtick);
        let module_until = Instant::now() + Duration::from_millis(600);
        while Instant::now() < module_until {
            h.step();
            thread::sleep(Duration::from_millis(10));
        }
        h.event(egui::Event::Ime(egui::ImeEvent::Commit(
            "document.title = 'Console ' + (19 + 23)".into(),
        )));
        h.step();
        thread::sleep(Duration::from_millis(80));
        h.step();
        key(&mut h, Modifiers::NONE, Key::Enter);
        let until = Instant::now() + Duration::from_secs(8);
        loop {
            h.step();
            if h.state()
                .service
                .browser
                .tabs
                .get(&id)
                .is_some_and(|t| t.title == "Console 42")
            {
                break;
            }
            if Instant::now() >= until {
                capture(&mut h, scale, compact, "08-console-failed-diagnostic");
                panic!("real DevTools Console did not change real page title");
            }
            thread::sleep(Duration::from_millis(10));
        }
        capture(&mut h, scale, compact, "08-console42-real-page-effect");
        let button = h.get_by_label("Ferramentas do desenvolvedor (F12)").rect();
        h.get_by_label("Ferramentas do desenvolvedor (F12)").hover();
        h.step();
        assert_eq!(
            button,
            h.get_by_label("Ferramentas do desenvolvedor (F12)").rect()
        );
        h.get_by_label("Ferramentas do desenvolvedor (F12)").click();
        h.step();
        h.step();
        wait(&mut h, |a| {
            a.service
                .browser
                .tabs
                .get(&id)
                .is_some_and(|t| !t.devtools_open)
        });
        assert!(h.query_by_label("DevTools do navegador").is_none());
        let c = h.state_mut().service.browser.devtools(id, 100).unwrap();
        dom_click(&mut h, &c, "#name");
        h.event(egui::Event::Ime(egui::ImeEvent::Commit(
            "foco devolvido 😀".into(),
        )));
        h.step();
        dom_wait(
            &mut h,
            &c,
            "document.querySelector('#name').value",
            serde_json::json!("foco devolvido 😀"),
        );
        let painted = h.state().browser_capture_state()["surfaces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["surface"] == "Page")
            .unwrap()["seq"]
            .as_u64()
            .unwrap();
        wait(&mut h, |a| {
            a.browser_capture_state()["surfaces"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["surface"] == "Page" && s["seq"].as_u64().unwrap_or(0) > painted)
        });
        capture(&mut h, scale, compact, "09-devtools-closed-page-focus");
        assert_eq!(h.state().service.active_document().unwrap().text(), DIRTY);
        drop(c);
        let close = h.get_by_label("Fechar navegador Console 42").rect();
        h.get_by_label("Fechar navegador Console 42").hover();
        h.step();
        assert_eq!(close, h.get_by_label("Fechar navegador Console 42").rect());
        h.get_by_label("Fechar navegador Console 42").click();
        h.step();

        h.step();

        assert!(
            h.state().service.browser.tabs.is_empty(),
            "first close click must close last browser tab"
        );
        wait(&mut h, |a| {
            a.service.browser.state == EngineState::Stopped
                && a.service.browser.engine_pid().is_none()
        });
    }
}

#[test]
#[ignore = "actual UI -> Core -> CEF page gateway -> js-debug DAP, own HTTP fixture"]
fn cef_browser_ui_debug_review_start_breakpoint_locals_and_stop_keeps_page() {
    use codisseu_core::debugger::DebugState;
    let http = Http::with_trigger();
    let (_profile, mut h, main) = setup(1., false);
    let root = main.parent().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/browser/app.js");
    let source = fs::read_to_string(fixture).unwrap();
    let app = root.join("app.js");
    fs::write(&app, &source).unwrap();
    let id = h.state().service.active_document().unwrap().id;
    h.state_mut()
        .service
        .dispatch(Command::Save { id })
        .unwrap();
    wait(&mut h, |a| !a.service.is_busy());
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(app.clone()))
        .unwrap();
    wait(&mut h, |a| {
        !a.service.is_busy() && a.service.active_document().is_some_and(|d| d.path == app)
    });
    let breakpoint = format!("Add breakpoint: {}:4", app.display());
    h.get_by_label(&breakpoint).hover();
    h.step();
    h.get_by_label(&breakpoint).click();
    h.step();
    h.step();
    assert!(
        h.state()
            .service
            .debugger
            .breakpoints
            .iter()
            .any(|b| b.path == app && b.line == 4)
    );
    h.state_mut().open_browser_url(&http.origin);
    wait(&mut h, |a| {
        a.browser_capture_ready() && http.polling.load(Ordering::Acquire)
    });
    let tab = h.state().browser_active().unwrap();
    h.get_by_label("Depurar esta aba na IDE").hover();
    h.step();
    h.get_by_label("Depurar esta aba na IDE").click();
    h.step();
    h.step();
    assert!(h.state().debugger_ui.dialog);
    assert!(
        h.get_by_label("Start Debug Configuration")
            .accesskit_node()
            .is_disabled()
    );
    h.get_by_label("I trust this configuration and authorize executing its code")
        .click();
    h.step();
    assert!(
        !h.get_by_label("Start Debug Configuration")
            .accesskit_node()
            .is_disabled()
    );
    capture(&mut h, 1., false, "10-debug-reviewed-trusted");
    let start = h.get_by_label("Start Debug Configuration").rect();
    h.get_by_label("Start Debug Configuration").hover();
    h.step();
    assert_eq!(start, h.get_by_label("Start Debug Configuration").rect());
    h.get_by_label("Start Debug Configuration").click();
    h.step();
    h.step();
    assert!(
        !h.state().debugger_ui.dialog,
        "first Start click must launch reviewed configuration"
    );
    wait(&mut h, |a| {
        a.service
            .debugger
            .sessions
            .iter()
            .any(|s| s.state == DebugState::Running && s.pending == 0)
    });
    let session = h.state().service.debugger.sessions.last().unwrap().id;
    assert_eq!(h.state().debugger_ui.selected_session, Some(session));
    capture(&mut h, 1., false, "11-debug-running-real-page");
    http.trigger.store(true, Ordering::Release);
    wait(&mut h, |a| {
        for s in &a.service.debugger.sessions {
            assert_ne!(s.state, DebugState::Failed, "{:?}", s.last_error);
        }
        a.service
            .debugger
            .sessions
            .iter()
            .find(|s| s.id == session)
            .is_some_and(|s| {
                s.state == DebugState::Stopped
                    && s.frames
                        .first()
                        .is_some_and(|f| f.path.as_ref() == Some(&app) && f.line == 4)
                    && s.variables
                        .values()
                        .flatten()
                        .any(|v| v.name == "result" && v.value == "42")
            })
    });
    let stopped = h
        .state()
        .service
        .debugger
        .sessions
        .iter()
        .find(|s| s.id == session)
        .unwrap();
    assert!(
        stopped
            .breakpoints
            .iter()
            .any(|b| b.line == 4 && b.verified)
    );
    assert!(
        stopped
            .variables
            .values()
            .flatten()
            .any(|v| v.name == "left" && v.value == "19")
    );
    assert!(
        stopped
            .variables
            .values()
            .flatten()
            .any(|v| v.name == "right" && v.value == "23")
    );
    capture(&mut h, 1., false, "12-debug-ui-paused-locals42");
    h.get_by_label("Stop Debug Session  Shift+F5").hover();
    h.step();
    h.get_by_label("Stop Debug Session  Shift+F5").click();
    h.step();
    wait(&mut h, |a| {
        a.service
            .debugger
            .sessions
            .iter()
            .find(|s| s.id == session)
            .is_some_and(|s| !s.state.is_active())
            && http.completed.load(Ordering::Acquire) == 42
    });
    assert_eq!(h.state().service.browser.state, EngineState::Ready);
    assert!(h.state().service.browser.tabs.contains_key(&tab));
    assert_eq!(fs::read_to_string(&app).unwrap(), source);
    assert_eq!(fs::read_to_string(&main).unwrap(), DIRTY);
    capture(&mut h, 1., false, "13-debug-stop-page-alive-source-intact");
    h.state_mut().service.browser.shutdown();
    wait(&mut h, |a| {
        a.service.browser.state == EngineState::Stopped && a.service.browser.engine_pid().is_none()
    });
}

#[test]
#[ignore = "actual CEF DevTools rapid coordinator close/reopen before observing old acknowledgement"]
fn cef_browser_rapid_f12_close_reopen_preserves_new_frontend() {
    use codisseu_core::browser::Command as B;
    let http = Http::new();
    let (_profile, mut h, _) = setup(1., false);
    h.state_mut().open_browser_url(&http.origin);
    wait(&mut h, |a| a.browser_capture_ready());
    let id = h.state().browser_active().unwrap();
    key(&mut h, Modifiers::NONE, Key::F12);
    let connected = |a: &CodisseuApp| {
        a.browser_capture_state()["devtools_state"]["gateways"]
            .as_array()
            .is_some_and(|g| g.iter().any(|v| v["id"] == id && v["connected"] == true))
    };
    wait(&mut h, |a| {
        connected(a)
            && a.browser_capture_state()["surfaces"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["surface"] == "DevTools")
    });
    let first_seq = h.state().browser_capture_state()["surfaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["surface"] == "DevTools")
        .unwrap()["seq"]
        .as_u64()
        .unwrap();
    // Use the exact native UI command sink, without an intervening app poll:
    // old Closed can be queued while the next opening is already authorized.
    h.state_mut().browser_send(B::DevTools { id, open: false });
    let began = Instant::now();
    thread::sleep(Duration::from_millis(10));
    h.state_mut().browser_send(B::DevTools { id, open: true });
    let gap = began.elapsed().as_micros();
    wait(&mut h, |a| {
        connected(a)
            && a.service
                .browser
                .tabs
                .get(&id)
                .is_some_and(|t| t.devtools_open)
            && a.browser_capture_state()["surfaces"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["surface"] == "DevTools" && s["seq"].as_u64().unwrap_or(0) > first_seq)
    });
    let until = Instant::now() + Duration::from_millis(500);
    while Instant::now() < until {
        h.step();
        assert!(
            connected(h.state()),
            "old close acknowledgement stopped the new gateway"
        );
        assert!(
            h.state()
                .service
                .browser
                .tabs
                .get(&id)
                .is_some_and(|t| t.devtools_open)
        );
        thread::sleep(Duration::from_millis(5));
    }
    capture(&mut h, 1., false, "14-rapid-devtools-reopen-stable");
    let out = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/browser-integration-20261102/ui/rapid-reopen-timing.json");
    fs::write(out,serde_json::to_vec_pretty(&serde_json::json!({"requested_gap_ms":10,"actual_gap_us":gap,"app_poll_between_close_and_open":false,"first_seq":first_seq,"after":h.state().browser_capture_state()})).unwrap()).unwrap();
    h.state_mut().service.shutdown_browser();
    wait(&mut h, |a| {
        a.service.browser.state == EngineState::Stopped && a.service.browser.engine_pid().is_none()
    });
}

#[test]
#[ignore = "explicit real CEF + private vault, four native UI scale/size cases"]
fn cef_browser_favorites_and_saved_access_keep_dirty_source_and_restart_metadata() {
    let http = Http::new();
    for (scale, compact) in [(1., false), (1.25, false), (1., true), (1.25, true)] {
        let (profile, mut h, file) = setup(scale, compact);
        let url = format!("{}/login", http.origin);
        h.state_mut().open_browser_url(&url);
        wait(&mut h, |a| {
            a.service
                .browser
                .tabs
                .values()
                .any(|t| t.url == url && !t.loading)
        });
        for label in [
            "Salvar página nos favoritos",
            "Favoritos do navegador",
            "Acessos e senhas deste site",
        ] {
            let rect = h.get_by_label(label).rect();
            assert!(rect.width() > 0. && rect.right() <= if compact { 900. } else { 1280. });
        }
        h.get_by_label("Salvar página nos favoritos").click();
        settle_saved_modal(&mut h);
        h.get_by_role_and_label(Role::TextInput, "Nome do favorito")
            .click();
        key(&mut h, Modifiers::CTRL, Key::A);
        h.event(egui::Event::Text("Meu frontend local".into()));
        h.step();
        assert_eq!(
            h.get_by_role_and_label(Role::TextInput, "Nome do favorito")
                .value()
                .as_deref(),
            Some("Meu frontend local")
        );
        capture(&mut h, scale, compact, "favorite-before-save");
        h.get_by_role_and_label(Role::Button, "Salvar favorito")
            .click();
        h.step();
        capture(&mut h, scale, compact, "favorite-after-save-click");
        wait(&mut h, |a| {
            a.service.browser_library.state.bookmarks.len() == 1
                && !a.service.browser_library.is_busy()
        });
        capture(&mut h, scale, compact, "saved-favorite");
        h.get_by_label("Concluído").click();
        settle_saved_modal(&mut h);
        h.get_by_label("Acessos e senhas deste site").click();
        settle_saved_modal(&mut h);
        let heading = h.get_by_label("Acessos do navegador").rect();
        let footer = h.get_by_label("Concluído").rect();
        assert!(heading.top() >= 16., "dialog heading clipped: {heading:?}");
        assert!(
            footer.bottom() <= if compact { 634. } else { 784. },
            "dialog footer clipped: {footer:?}"
        );
        h.get_by_label("Ler formulário").click();
        wait(&mut h, |a| {
            !a.service.browser_login_busy() && a.browser_ui.library.open
        });
        assert!(
            h.state()
                .service
                .browser_login_notice()
                .unwrap()
                .contains("Confira"),
            "{:?}",
            h.state().service.browser_login_notice()
        );
        assert!(h.query_by_role(Role::PasswordInput).is_some());
        assert!(
            !h.get_by_label("Senha do acesso")
                .value()
                .unwrap_or_default()
                .contains("owned-ui-login-only")
        );
        h.get_by_label("Salvar no cofre").click();
        wait(&mut h, |a| {
            !a.service.browser_library.is_busy()
                && a.service.browser_library.state.logins.len() == 1
        });
        show_saved_access(&mut h, compact);
        capture(&mut h, scale, compact, "saved-access-masked");
        h.get_by_label("Concluído").click();
        settle_saved_modal(&mut h);
        h.get_by_label("Favoritos do navegador").click();
        settle_saved_modal(&mut h);
        h.get_by_label("Meu frontend local").click();
        wait(&mut h, |a| {
            a.service.browser.tabs.len() == 2
                && a.browser_active() == Some(2)
                && a.service.browser.tabs.get(&2).is_some_and(|t| !t.loading)
        });
        h.get_by_label("Acessos e senhas deste site").click();
        settle_saved_modal(&mut h);
        show_saved_access(&mut h, compact);
        assert!(!h.get_by_label("Preencher").accesskit_node().is_disabled());
        h.get_by_label("Preencher").click();
        settle_saved_modal(&mut h);
        wait(&mut h, |a| !a.service.browser_login_busy());
        capture(&mut h, scale, compact, "fill-attempt-diagnostics");
        assert!(
            h.state()
                .service
                .browser_login_notice()
                .unwrap_or("")
                .contains("preenchido"),
            "{:?} / {:?}",
            h.state().service.browser_login_notice(),
            h.state().browser_ui.library.notice
        );
        capture(&mut h, scale, compact, "saved-access-filled");
        h.get_by_label("Excluir acesso").click();
        wait(&mut h, |a| {
            !a.service.browser_library.is_busy()
                && a.service.browser_library.state.logins.is_empty()
        });
        h.get_by_label("Concluído").click();
        settle_saved_modal(&mut h);
        assert_eq!(h.state().service.active_document().unwrap().text(), DIRTY);
        assert!(h.state().service.active_document().unwrap().is_dirty());
        assert_eq!(fs::read_to_string(&file).unwrap(), "let answer = 0;\n");
        h.state_mut().service.shutdown_browser();
        wait(&mut h, |a| a.service.browser.state == EngineState::Stopped);
        drop(h);
        let mut restored = AppService::new(profile.path().join("profile/session.json"));
        let end = Instant::now() + Duration::from_secs(10);
        while restored.browser_library.is_busy() {
            restored.poll();
            assert!(Instant::now() < end);
            thread::sleep(Duration::from_millis(3));
        }
        assert_eq!(restored.browser_library.state.bookmarks.len(), 1);
        assert!(restored.browser_library.state.logins.is_empty());
        assert_eq!(restored.browser.state, EngineState::Stopped);
    }
}

fn show_saved_access(h: &mut Harness<'_, CodisseuApp>, compact: bool) {
    if compact {
        // Wheel over the form scrolls the outer body, outside the nested list.
        h.get_by_label("Senha do acesso").hover();
        h.step();
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            delta: egui::vec2(0., -240.),
            modifiers: Modifiers::NONE,
        });
        for _ in 0..20 {
            h.step();
            thread::sleep(Duration::from_millis(5));
        }
    }
    settle_saved_modal(h);
    let button = h.get_by_label("Preencher").rect();
    assert!(button.bottom() < h.get_by_label("Concluído").rect().top() - 70.);
}

fn settle_saved_modal(h: &mut Harness<'_, CodisseuApp>) {
    let mut last = (None, None);
    let mut stable = 0;
    let until = Instant::now() + Duration::from_secs(5);
    while stable < 5 {
        h.step();
        let rect = (
            h.query_by_role_and_label(Role::Button, "Concluído")
                .map(|n| n.rect()),
            h.query_by_role_and_label(Role::Button, "Preencher")
                .map(|n| n.rect()),
        );
        stable = if rect == last { stable + 1 } else { 0 };
        last = rect;
        assert!(Instant::now() < until, "saved dialog did not settle");
        thread::sleep(Duration::from_millis(3));
    }
}
