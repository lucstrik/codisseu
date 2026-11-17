//! Real CEF page -> private IPC -> authenticated routing bridge -> official js-debug.
//! Run explicitly with the installed CEF helper and an isolated authenticated X11 display.
use codisseu_browser::{
    BrowserService, Command as BrowserCommand, EngineConfig, EngineState, KeyKind, Modifiers,
    Surface,
};
use codisseu_debug::*;
use serde_json::json;
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

struct Site {
    url: String,
    stop: Arc<AtomicBool>,
    trigger: Arc<AtomicUsize>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Site {
    fn new(root: &Path) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let root = root.to_path_buf();
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let trigger = Arc::new(AtomicUsize::new(0));
        let requested = trigger.clone();
        let worker = thread::spawn(move || {
            while !signal.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_millis(200)))
                            .unwrap();
                        let mut request = Vec::new();
                        while request.len() < 8192 {
                            let mut part = [0; 1024];
                            let n = stream.read(&mut part).unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            request.extend_from_slice(&part[..n]);
                            if request.windows(4).any(|v| v == b"\r\n\r\n") {
                                break;
                            }
                        }
                        let request = String::from_utf8_lossy(&request);
                        let path = request.split_whitespace().nth(1).unwrap_or("/");
                        let name = match path {
                            "/app.js" => "app.js",
                            "/app.js.map" => "app.js.map",
                            "/app.ts" => "app.ts",
                            _ => "index.html",
                        };
                        let data = if path == "/data" {
                            br#"{"answer":42}"#.to_vec()
                        } else if path == "/trigger" {
                            json!({"requested":requested.load(Ordering::Acquire)})
                                .to_string()
                                .into_bytes()
                        } else if path == "/other" {
                            b"<!doctype html><title>Unrelated CEF page</title><p>Other page remains independent</p>".to_vec()
                        } else {
                            fs::read(root.join(name)).unwrap_or_default()
                        };
                        let kind = if matches!(path, "/trigger" | "/data") {
                            "application/json"
                        } else if name.ends_with(".js") {
                            "text/javascript"
                        } else if name.ends_with(".map") {
                            "application/json"
                        } else {
                            "text/html; charset=utf-8"
                        };
                        let header = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
                            data.len()
                        );
                        let _ = stream.write_all(header.as_bytes());
                        let _ = stream.write_all(&data);
                    }
                    Err(_) => thread::sleep(Duration::from_millis(3)),
                }
            }
        });
        Self {
            url,
            stop,
            trigger,
            worker: Some(worker),
        }
    }
}

fn frontend_pump(browser: &mut BrowserService, millis: u64) {
    let end = Instant::now() + Duration::from_millis(millis);
    while Instant::now() < end {
        browser.poll();
        thread::sleep(Duration::from_millis(5));
    }
}
fn frontend_key(browser: &mut BrowserService, key: i32, ctrl: bool, shift: bool) {
    for kind in [KeyKind::RawDown, KeyKind::Up] {
        browser
            .dispatch(BrowserCommand::Key {
                id: 1,
                surface: Surface::DevTools,
                kind,
                windows_key_code: key,
                native_key_code: 0,
                modifiers: Modifiers {
                    ctrl,
                    shift,
                    ..Default::default()
                },
                character: if key == 80 {
                    Some('P')
                } else if key == 13 {
                    Some('\r')
                } else {
                    None
                },
                unmodified: if key == 80 {
                    Some('p')
                } else if key == 13 {
                    Some('\r')
                } else {
                    None
                },
            })
            .unwrap();
    }
}
fn frontend_command(browser: &mut BrowserService, command: &str) {
    browser
        .dispatch(BrowserCommand::Focus {
            id: 1,
            surface: Surface::DevTools,
            focused: true,
        })
        .unwrap();
    frontend_key(browser, 80, true, true);
    frontend_pump(browser, 120);
    browser
        .dispatch(BrowserCommand::Text {
            id: 1,
            surface: Surface::DevTools,
            text: command.into(),
        })
        .unwrap();
    frontend_pump(browser, 80);
    frontend_key(browser, 13, false, false);
    frontend_pump(browser, 150);
}
fn frontend_console(browser: &mut BrowserService, expression: &str) {
    frontend_command(browser, "Show Console");
    frontend_capture(browser, "frontend-console-opened-diagnostic");
    browser
        .dispatch(BrowserCommand::Text {
            id: 1,
            surface: Surface::DevTools,
            text: expression.into(),
        })
        .unwrap();
    frontend_key(browser, 13, false, false);
    frontend_capture(browser, "frontend-console-input-diagnostic");
}
fn frontend_capture(browser: &mut BrowserService, name: &str) {
    // SDK panels fetch source/DOM over CDP after selection. Let those real
    // asynchronous responses paint before recording the visual evidence.
    frontend_pump(browser, 600);
    if let Some(directory) = std::env::var_os("CODISSEU_BROWSER_DEBUG_RESULTS")
        && let Some(frame) = browser.frames.get(&(1, Surface::DevTools))
    {
        let directory = PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(format!("{name}.bgra")), &*frame.bgra).unwrap();
        fs::write(
            directory.join(format!("{name}.json")),
            json!({"width":frame.width,"height":frame.height,"seq":frame.seq}).to_string(),
        )
        .unwrap();
    }
}

#[test]
#[ignore = "real original SDK DevTools frontend, CEF helper and private DISPLAY required"]
fn real_cef_original_devtools_frontend_dom_console_network_sources() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("index.html"),"<!doctype html><title>Frontend inspected page</title><h1 id='marker'>Real DOM42</h1><button onclick=\"document.title='clicked'\">Click</button><script src='/app.js'></script>").unwrap();
    fs::write(root.join("app.js"), "window.frontendCompute = function frontendCompute() {\n  const left = 19;\n  const right = 23;\n  const result = left + right;\n  debugger;\n  document.title = 'Sources:' + result;\n};\n").unwrap();
    let site = Site::new(root);
    let mut browser = BrowserService::new();
    let (program, runtime_dir) = codisseu_runtime::browser_engine().unwrap();
    browser
        .configure(EngineConfig {
            program,
            runtime_dir,
        })
        .unwrap();
    browser
        .dispatch(BrowserCommand::Open {
            id: 1,
            url: site.url.clone(),
        })
        .unwrap();
    browser
        .dispatch(BrowserCommand::Visible {
            id: 1,
            surface: Surface::Page,
            visible: true,
        })
        .unwrap();
    let mut debug = DebugService::new();
    wait(&mut browser, &mut debug, None, |b, _| {
        b.state == EngineState::Ready
            && b.tabs[&1].title == "Frontend inspected page"
            && !b.tabs[&1].loading
    });
    let engine_pid = browser.engine_pid().unwrap();
    let mut bridge = BrowserDebugBridge::start_page(browser.devtools(1, 1).unwrap(), 1, 1).unwrap();
    let port = bridge.endpoint().port;
    let frontend = bridge.frontend_url().unwrap();
    browser.open_devtools_frontend(1, frontend).unwrap();
    browser
        .dispatch(BrowserCommand::Resize {
            id: 1,
            surface: Surface::DevTools,
            width: 1000,
            height: 700,
            scale: 1.,
        })
        .unwrap();
    browser
        .dispatch(BrowserCommand::Visible {
            id: 1,
            surface: Surface::DevTools,
            visible: true,
        })
        .unwrap();
    wait(&mut browser, &mut debug, Some(&bridge), |b, _| {
        b.frames.get(&(1, Surface::DevTools)).is_some_and(|f| {
            f.width == 1000
                && f.height == 700
                && f.bgra.chunks_exact(4).any(|p| p[..3] != f.bgra[..3])
        })
    });
    wait(&mut browser, &mut debug, Some(&bridge), |_, _| {
        bridge.is_connected()
    });
    frontend_capture(&mut browser, "frontend-connected-elements");
    frontend_console(&mut browser, "document.title='Console42:'+String(6*7)");
    wait(&mut browser, &mut debug, Some(&bridge), |b, _| {
        b.tabs[&1].title == "Console42:42"
    });
    frontend_capture(&mut browser, "frontend-console42");
    frontend_console(&mut browser, "inspect(document.getElementById('marker'))");
    frontend_command(&mut browser, "Show Elements");
    frontend_capture(&mut browser, "frontend-elements-inspected-marker");
    frontend_console(
        &mut browser,
        "fetch('/data').then(r=>r.json()).then(v=>document.title='Network:'+v.answer)",
    );
    wait(&mut browser, &mut debug, Some(&bridge), |b, _| {
        b.tabs[&1].title == "Network:42"
    });
    frontend_command(&mut browser, "Show Network");
    frontend_capture(&mut browser, "frontend-network42");
    let paused = bridge.paused_events();
    frontend_console(&mut browser, "frontendCompute()");
    wait(&mut browser, &mut debug, Some(&bridge), |_, _| {
        bridge.paused_events() > paused
    });
    frontend_capture(&mut browser, "frontend-sources-paused42");
    assert_eq!(browser.tabs[&1].title, "Network:42");
    frontend_key(&mut browser, 119, false, false);
    wait(&mut browser, &mut debug, Some(&bridge), |b, _| {
        b.tabs[&1].title == "Sources:42"
    });
    frontend_capture(&mut browser, "frontend-sources-continued");
    browser
        .dispatch(BrowserCommand::DevTools { id: 1, open: false })
        .unwrap();
    wait(&mut browser, &mut debug, Some(&bridge), |b, _| {
        !b.tabs[&1].devtools_open
    });
    assert!(bridge.shutdown(Duration::from_secs(1)));
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err());
    assert_eq!(browser.engine_pid(), Some(engine_pid));
    assert_eq!(browser.tabs[&1].title, "Sources:42");
    browser.shutdown();
    wait(&mut browser, &mut debug, None, |b, _| {
        b.state == EngineState::Stopped && b.engine_pid().is_none()
    });
    if let Some(directory) = std::env::var_os("CODISSEU_BROWSER_DEBUG_RESULTS") {
        fs::write(PathBuf::from(directory).join("real-cef-frontend-results.json"),serde_json::to_vec_pretty(&json!({"result":"passed","original_sdk_frontend_osr":true,"authenticated_page_websocket":true,"console_evaluation42":true,"dom_inspect_command":true,"real_fetch_network42":true,"sources_paused_event":true,"sources_f8_continue42":true,"close_frontend_preserves_page":true,"port_reclaimed":true})).unwrap()).unwrap();
    }
}
impl Drop for Site {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
fn wait(
    browser: &mut BrowserService,
    debug: &mut DebugService,
    bridge: Option<&BrowserDebugBridge>,
    predicate: impl Fn(&BrowserService, &DebugService) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        browser.poll();
        debug.poll();
        assert!(
            browser.state != EngineState::Failed,
            "browser: {:?}",
            browser.last_error
        );
        assert!(
            !debug.sessions.iter().any(|s| s.state == DebugState::Failed),
            "debug: {:?}; bridge: {:?}",
            debug.sessions,
            bridge.and_then(|b| b.error())
        );
        if predicate(browser, debug) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timeout: browser={:?} {:?}, tabs={:?}, debug={:?}, bridge={:?}",
            browser.state,
            browser.last_error,
            browser.tabs,
            debug.sessions,
            bridge.and_then(|b| b.error())
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn descendants(root: u32) -> Vec<u32> {
    let mut rows = Vec::new();
    for entry in fs::read_dir("/proc").unwrap().flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if let Ok(status) = fs::read_to_string(entry.path().join("status"))
            && let Some(parent) = status.lines().find_map(|s| {
                s.strip_prefix("PPid:")
                    .and_then(|s| s.trim().parse::<u32>().ok())
            })
        {
            rows.push((pid, parent));
        }
    }
    let mut owned = vec![root];
    for _ in 0..10 {
        let more = rows
            .iter()
            .filter_map(|(pid, ppid)| owned.contains(ppid).then_some(*pid))
            .collect::<Vec<_>>();
        let before = owned.len();
        for pid in more {
            if !owned.contains(&pid) {
                owned.push(pid);
            }
        }
        if before == owned.len() {
            break;
        }
    }
    owned
}

#[test]
#[ignore = "real installed CEF helper and private DISPLAY required"]
fn real_cef_javascript_typescript_breakpoint_locals_step_continue_and_stop_preserve_pages() {
    let mut reports = Vec::new();
    for typescript in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let original = "function compute(): number {\n  const left: number = 19;\n  const right: number = 23;\n  const result: number = left + right;\n  document.title = 'answer=' + result + ':' + (++window.calls);\n  return result;\n}\nwindow.compute = compute;\n";
        let js = original.replace(": number", "");
        let source = root.join(if typescript { "app.ts" } else { "app.js" });
        if typescript {
            fs::write(&source, original).unwrap();
            fs::write(root.join("app.js.map"),json!({"version":3,"file":"app.js","sources":["app.ts"],"sourcesContent":[original],"names":[],"mappings":"AAAA;AACA;AACA;AACA;AACA;AACA;AACA;AACA"}).to_string()).unwrap();
        }
        fs::write(
            root.join("app.js"),
            format!(
                "{js}{}",
                if typescript {
                    "//# sourceMappingURL=app.js.map\n"
                } else {
                    ""
                }
            ),
        )
        .unwrap();
        fs::write(root.join("index.html"),"<!doctype html><meta charset='utf-8'><title>Owned CEF debug ready</title><style>button{position:absolute;left:40px;top:40px;width:160px;height:40px}</style><button onclick='compute()'>Compute42</button><script>window.calls=0;</script><script src='/app.js'></script><script>window.requested=0;setInterval(async()=>{const v=await fetch('/trigger').then(r=>r.json());if(v.requested>window.requested){window.requested=v.requested;compute();}},40);</script>").unwrap();
        let site = Site::new(&root);
        let mut browser = BrowserService::new();
        let (program, runtime_dir) = codisseu_runtime::browser_engine().unwrap();
        browser
            .configure(EngineConfig {
                program,
                runtime_dir,
            })
            .unwrap();
        browser
            .dispatch(BrowserCommand::Open {
                id: 1,
                url: site.url.clone(),
            })
            .unwrap();
        browser
            .dispatch(BrowserCommand::Resize {
                id: 1,
                surface: Surface::Page,
                width: 800,
                height: 600,
                scale: 1.,
            })
            .unwrap();
        browser
            .dispatch(BrowserCommand::Visible {
                id: 1,
                surface: Surface::Page,
                visible: true,
            })
            .unwrap();
        browser
            .dispatch(BrowserCommand::Focus {
                id: 1,
                surface: Surface::Page,
                focused: true,
            })
            .unwrap();
        let mut debug = DebugService::new();
        wait(&mut browser, &mut debug, None, |b, _| {
            b.state == EngineState::Ready
                && b.tabs
                    .get(&1)
                    .is_some_and(|t| !t.loading && t.title == "Owned CEF debug ready")
        });
        browser
            .dispatch(BrowserCommand::Open {
                id: 2,
                url: format!("{}other", site.url),
            })
            .unwrap();
        wait(&mut browser, &mut debug, None, |b, _| {
            b.tabs
                .get(&2)
                .is_some_and(|t| !t.loading && t.title == "Unrelated CEF page")
        });
        let engine_pid = browser.engine_pid().unwrap();
        let owned = descendants(engine_pid);
        let mut bridge = BrowserDebugBridge::start(
            browser.devtools(1, if typescript { 2 } else { 1 }).unwrap(),
            1,
            if typescript { 2 } else { 1 },
        )
        .unwrap();
        let endpoint = bridge.endpoint();
        let port = endpoint.port;
        debug
            .execute(DebugCommand::SetBreakpoints {
                path: source.clone(),
                lines: vec![5],
            })
            .unwrap();
        let config = DebugConfiguration {
            runtime: DebugRuntime::Chrome,
            request: DebugRequest::Attach,
            name: format!(
                "CEF {}",
                if typescript {
                    "TypeScript"
                } else {
                    "JavaScript"
                }
            ),
            cwd: root.clone(),
            url: site.url.clone(),
            source_maps: typescript,
            integrated_browser: true,
            browser_tab: Some(1),
            browser_inspect_uri: Some(endpoint.inspect_uri),
            port,
            trusted: true,
            ..Default::default()
        };
        let id = debug.execute(DebugCommand::Start(config)).unwrap().unwrap();
        wait(&mut browser, &mut debug, Some(&bridge), |_, d| {
            d.sessions[0].state == DebugState::Running
                && d.sessions[0].pending == 0
                && (!typescript
                    || d.sessions[0]
                        .breakpoints
                        .iter()
                        .any(|point| point.path == source && point.line == 5 && point.verified))
        });
        site.trigger.store(1, Ordering::Release);
        wait(&mut browser, &mut debug, Some(&bridge), |_, d| {
            d.sessions[0].state == DebugState::Stopped
                && !d.sessions[0].frames.is_empty()
                && d.sessions[0]
                    .variables
                    .values()
                    .flatten()
                    .any(|v| v.name == "result" && v.value == "42")
        });
        assert_eq!(debug.sessions[0].frames[0].path.as_ref(), Some(&source));
        assert_eq!(debug.sessions[0].frames[0].line, 5);
        assert!(
            debug.sessions[0]
                .breakpoints
                .iter()
                .any(|point| { point.path == source && point.line == 5 && point.verified })
        );
        assert_eq!(
            debug.sessions[0].stopped_reason.as_deref(),
            Some("breakpoint")
        );
        debug
            .execute(DebugCommand::Evaluate {
                session: id,
                expression: "result".into(),
            })
            .unwrap();
        wait(&mut browser, &mut debug, Some(&bridge), |_, d| {
            d.sessions[0]
                .evaluations
                .iter()
                .any(|e| e.expression == "result" && e.result == "42")
        });
        let locals = debug.sessions[0]
            .variables
            .values()
            .flatten()
            .filter(|v| ["left", "right", "result"].contains(&v.name.as_str()))
            .map(|v| json!({"name":v.name,"value":v.value}))
            .collect::<Vec<_>>();
        let epoch = debug.sessions[0].stop_epoch;
        debug.execute(DebugCommand::Next(id)).unwrap();
        wait(&mut browser, &mut debug, Some(&bridge), |_, d| {
            d.sessions[0].state == DebugState::Stopped
                && d.sessions[0].stop_epoch > epoch
                && !d.sessions[0].frames.is_empty()
        });
        assert_eq!(debug.sessions[0].frames[0].line, 6);
        debug.execute(DebugCommand::Continue(id)).unwrap();
        wait(&mut browser, &mut debug, Some(&bridge), |b, d| {
            d.sessions[0].state == DebugState::Running && b.tabs[&1].title == "answer=42:1"
        });
        site.trigger.store(2, Ordering::Release);
        wait(&mut browser, &mut debug, Some(&bridge), |_, d| {
            d.sessions[0].state == DebugState::Stopped && !d.sessions[0].frames.is_empty()
        });
        debug.execute(DebugCommand::Stop(id)).unwrap();
        wait(&mut browser, &mut debug, Some(&bridge), |_, d| {
            !d.has_active_sessions()
        });
        assert!(bridge.shutdown(Duration::from_secs(1)));
        wait(&mut browser, &mut debug, None, |b, _| {
            b.tabs[&1].title == "answer=42:2"
        });
        assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err());
        assert_eq!(browser.state, EngineState::Ready);
        assert_eq!(browser.tabs[&2].title, "Unrelated CEF page");
        assert_eq!(browser.engine_pid(), Some(engine_pid));
        site.trigger.store(3, Ordering::Release);
        wait(&mut browser, &mut debug, None, |b, _| {
            b.tabs[&1].title == "answer=42:3"
        });
        let versions = browser.versions.clone();
        browser.shutdown();
        wait(&mut browser, &mut debug, None, |b, _| {
            b.state == EngineState::Stopped && b.engine_pid().is_none()
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while owned
            .iter()
            .any(|pid| PathBuf::from(format!("/proc/{pid}")).exists())
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            owned
                .iter()
                .all(|pid| !PathBuf::from(format!("/proc/{pid}")).exists()),
            "owned CEF processes survived: {owned:?}"
        );
        reports.push(json!({"language":if typescript{"TypeScript"}else{"JavaScript"},"versions":versions,"breakpoint_source":source.file_name().unwrap().to_string_lossy(),"breakpoint_line":5,"breakpoint_verified_after_hit":true,"locals":locals,"evaluate":"42","step_line":6,"continue":true,"stop_resumed_page":true,"page_and_other_tab_survive_stop":true,"port_reclaimed":true,"engine_and_descendants_reaped":true}));
    }
    if let Some(directory) = std::env::var_os("CODISSEU_BROWSER_DEBUG_RESULTS") {
        let directory = PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("real-cef-debug-results.json"),
            serde_json::to_vec_pretty(&json!({"result":"passed","scenarios":reports})).unwrap(),
        )
        .unwrap();
    }
}
