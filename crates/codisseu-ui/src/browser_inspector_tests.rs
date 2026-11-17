//! Actual native inspector widget acceptance against an owned HTTP/CEF fixture.
//! Run with artifacts/browser-inspector-20261102/ui/run.py on its private Xvfb.
use super::*;
use codisseu_core::browser::{EngineConfig, EngineState, inspector::Lifecycle};
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{By, NodeT, Queryable},
};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    path::Path,
    process::{Child, Command as ProcessCommand, Stdio},
    sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
    thread,
    time::Instant,
};

type H = Harness<'static, CodisseuApp>;
const SAVED: &str = "// original source\nlet untouched = 0;\n";
const DIRTY: &str = "// fonte dirty preservada · ação😀\nlet untouched = 42;\n";
const CASES: [(f32, bool); 4] = [(1., false), (1.25, false), (1., true), (1.25, true)];
fn cases() -> Vec<(f32, bool)> {
    let requested = std::env::var("CODISSEU_INSPECTOR_GEOMETRY").ok();
    let cases = CASES
        .into_iter()
        .filter(|(scale, compact)| {
            requested.as_ref().is_none_or(|value| {
                value
                    == &format!(
                        "{}-{}",
                        if *scale == 1. { "100" } else { "125" },
                        if *compact { "compact" } else { "normal" }
                    )
            })
        })
        .collect::<Vec<_>>();
    assert!(
        !cases.is_empty(),
        "unknown inspector geometry: {requested:?}"
    );
    cases
}

struct Http {
    origin: String,
    child: Child,
    _profile: tempfile::TempDir,
}
impl Http {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let profile = tempfile::tempdir().unwrap();
        let stderr = fs::File::create(profile.path().join("server.log")).unwrap();
        let mut child = ProcessCommand::new(root.join(".tools/python/cpython/bin/python3"))
            .arg("-I")
            .arg(root.join("fixtures/browser-inspector/server.py"))
            .arg("--journal")
            .arg(profile.path().join("requests.jsonl"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        Self {
            origin: value["origin"].as_str().unwrap().into(),
            child,
            _profile: profile,
        }
    }
    fn state(&self) -> serde_json::Value {
        let mut socket = TcpStream::connect(self.origin.trim_start_matches("http://")).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(
            socket,
            "GET /state HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut bytes = Vec::new();
        socket.read_to_end(&mut bytes).unwrap();
        let at = bytes.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        serde_json::from_slice(&bytes[at..]).unwrap()
    }
}
impl Drop for Http {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait(h: &mut H, predicate: impl Fn(&CodisseuApp) -> bool) {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        h.step();
        assert_ne!(
            h.state().service.browser.state,
            EngineState::Failed,
            "{:?}",
            h.state().service.browser.last_error
        );
        if predicate(h.state()) {
            h.step();
            return;
        }
        if Instant::now() >= end {
            let base = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../artifacts/browser-inspector-20261102/ui");
            fs::create_dir_all(&base).unwrap();
            h.render()
                .unwrap()
                .save(base.join("failure-current.png"))
                .unwrap();
            let snapshot = &h.state().service.browser_inspector.state;
            fs::write(base.join("failure-state.json"),serde_json::to_vec_pretty(&serde_json::json!({
                "revision":snapshot.revision,"selected_node":snapshot.selected_node,
                "dom":snapshot.dom.iter().map(|n|serde_json::json!({"id":n.id,"name":n.name,"local_name":n.local_name,"custom":n.custom_element,"shadow":n.shadow_root,"parent":n.parent})).collect::<Vec<_>>(),
                "network":snapshot.network.iter().map(|r|serde_json::json!({"id":r.id,"method":r.method,"finished":r.finished,"status":r.status})).collect::<Vec<_>>(),
                "paused":snapshot.paused.is_some(),"watch_expressions":snapshot.watches.iter().map(|w|&w.expression).collect::<Vec<_>>(),
                "sdk":h.state().service.browser_devtools_diagnostics(),"browser":h.state().browser_capture_state(),
                "frames":h.state().service.browser.frames.iter().map(|((id,surface),f)|serde_json::json!({"tab":id,"surface":format!("{surface:?}"),"width":f.width,"height":f.height,"nonuniform":f.bgra.chunks_exact(4).any(|p|p[..3]!=f.bgra[..3])})).collect::<Vec<_>>()
            })).unwrap()).unwrap();
            panic!(
                "inspector timeout: {:?}, browser={:?}, core={:?}, inspector={:?}",
                h.state().service.browser_inspector.state,
                h.state().service.browser.last_error,
                h.state().service.last_error,
                h.state().service.browser_inspector.state.error
            );
        }
        thread::sleep(Duration::from_millis(8));
    }
}
fn settle(h: &mut H, rectangle: impl Fn(&H) -> egui::Rect) {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut previous = rectangle(h);
    let mut stable = 0;
    while stable < 3 {
        h.step();
        let current = rectangle(h);
        if (current.min - previous.min).length() < 0.25
            && (current.max - previous.max).length() < 0.25
        {
            stable += 1;
        } else {
            stable = 0;
        }
        previous = current;
        assert!(
            Instant::now() < deadline,
            "scroll target kept moving: {current:?}"
        );
        thread::sleep(Duration::from_millis(8));
    }
}
fn scroll_native(h: &mut H, inner: bool, delta: f32) {
    let page = h.get_by_label("Página do navegador").rect();
    let window = h.ctx.input(|input| input.viewport_rect());
    let point = if inner {
        let clip = row_clip(h, RowKind::Dom).shrink2(egui::vec2(12., 12.));
        // When both semantic boundaries are visible, target the middle of the
        // actual tree viewport. Row fragments near its bottom can overlap the
        // floating horizontal scrollbar and consume the wheel without moving.
        let bounded_tree = h
            .query_by_label_contains("A árvore mostra elementos")
            .zip(h.query_by_label("Atributos"))
            .is_some_and(|(before, after)| {
                visible(before.rect(), body_clip(h)) && visible(after.rect(), body_clip(h))
            });
        if bounded_tree && clip.height() > 24. {
            clip.center()
        } else {
            let mut rows = ["#document", "html", "head", "body", "main", "h1#marker"]
                .into_iter()
                .flat_map(|label| h.query_all_by_label(label).map(|n| n.rect()))
                .collect::<Vec<_>>();
            rows.extend(h.query_all_by_label_contains("#text").map(|n| n.rect()));
            let row = rows
                .into_iter()
                .map(|r| r.intersect(clip))
                .find(|r| r.width() > 8. && r.height() > 8.);
            let Some(row) = row else {
                scroll_native(h, false, -32.);
                return;
            };
            row.center()
        }
    } else {
        // Frame inset8 + scrollbar4: outside the nested DOM/storage list.
        egui::pos2(page.right() - 12., window.bottom() - 80.)
    };
    h.hover_at(point);
    h.step();
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        delta: egui::vec2(0., delta),
        modifiers: Modifiers::NONE,
    });
    for _ in 0..12 {
        h.step();
        thread::sleep(Duration::from_millis(5));
    }
}
fn ensure_widget(h: &mut H, present: impl Fn(&H) -> bool, inner: bool, target: &str) {
    if present(h) {
        return;
    }
    // Scroll the actual native outer pane to its start, then scan the selected
    // pane/tree. This supports AccessKit culling without changing model state.
    scroll_native(h, false, 10000.);
    if inner {
        scroll_native(h, true, 10000.);
    }
    for _ in 0..200 {
        if present(h) {
            return;
        }
        // Compact body viewports can be shorter than100px. A32px scan
        // overlaps successive visible intervals instead of skipping labels.
        scroll_native(h, inner, -32.);
    }
    let base =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/browser-inspector-20261102/ui");
    h.render()
        .unwrap()
        .save(base.join("scroll-failure-current.png"))
        .unwrap();
    fs::write(base.join("scroll-failure-current.json"),serde_json::to_vec_pretty(&serde_json::json!({"target":target,"inner":inner,"viewport":format!("{:?}",h.ctx.input(|i|i.viewport_rect())),"page":format!("{:?}",h.get_by_label("Página do navegador").rect()),"pointer":format!("{:?}",h.ctx.input(|i|i.pointer.hover_pos())),"modifiers":format!("{:?}",h.ctx.input(|i|i.modifiers)),"browser":h.state().browser_capture_state()})).unwrap()).unwrap();
    panic!("native inspector target absent after real viewport scrolling: {target}");
}
fn body_clip(h: &H) -> egui::Rect {
    let page = h.get_by_label("Página do navegador").rect();
    let top = [
        "Rede",
        "Componentes / DOM",
        "Armazenamento",
        "Variáveis",
        "Tokens",
    ]
    .into_iter()
    .filter_map(|label| h.query_by_label(label).map(|n| n.rect().bottom()))
    .fold(page.bottom(), f32::max)
        + 8.;
    egui::Rect::from_min_max(
        egui::pos2(page.left() + 8., top),
        egui::pos2(
            page.right() - 8.,
            h.ctx.input(|i| i.viewport_rect().bottom()) - 55.,
        ),
    )
}
fn target_clip(h: &H, label: &str) -> egui::Rect {
    if matches!(
        label,
        "Inspecionar página"
            | "Fechar inspetor"
            | "Rede"
            | "Componentes / DOM"
            | "Armazenamento"
            | "Variáveis"
            | "Tokens"
            | "Ferramentas do desenvolvedor (F12)"
    ) {
        h.ctx.input(|i| i.viewport_rect())
    } else {
        body_clip(h)
    }
}
fn visible(rect: egui::Rect, clip: egui::Rect) -> bool {
    let r = rect.intersect(clip);
    r.width() > 8. && r.height() > 8.
}
fn pointer_click(h: &mut H, point: egui::Pos2) {
    h.hover_at(point);
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
}
fn show_label(h: &mut H, label: &str) {
    ensure_widget(
        h,
        |h| {
            h.query_by_label(label)
                .is_some_and(|n| visible(n.rect(), target_clip(h, label)))
        },
        false,
        label,
    );
    settle(h, |h| h.get_by_label(label).rect());
}
fn show_contains(h: &mut H, text: &str) {
    ensure_widget(
        h,
        |h| {
            h.query_all_by_label_contains(text)
                .any(|n| visible(n.rect(), body_clip(h)))
        },
        false,
        text,
    );
}
fn click(h: &mut H, label: &str) -> Vec<String> {
    ensure_widget(
        h,
        |h| {
            h.query_by_label(label)
                .is_some_and(|n| visible(n.rect(), target_clip(h, label)))
        },
        false,
        label,
    );
    settle(h, |h| h.get_by_label(label).rect());
    let point = h
        .get_by_label(label)
        .rect()
        .intersect(target_clip(h, label))
        .center();
    pointer_click(h, point);
    h.output()
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::CopyText(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}
fn click_contains(h: &mut H, label: &str) {
    ensure_widget(
        h,
        |h| {
            h.query_by_label_contains(label)
                .is_some_and(|n| visible(n.rect(), body_clip(h)))
        },
        false,
        label,
    );
    settle(h, |h| h.get_by_label_contains(label).rect());
    let point = h
        .get_by_label_contains(label)
        .rect()
        .intersect(body_clip(h))
        .center();
    pointer_click(h, point);
}
#[derive(Clone, Copy)]
enum RowKind {
    Request,
    Dom,
    Storage,
}
fn row_clip(h: &H, kind: RowKind) -> egui::Rect {
    let page = h.get_by_label("Página do navegador").rect();
    let bottom = h.ctx.input(|i| i.viewport_rect().bottom()) - 55.;
    let top = [
        "Rede",
        "Componentes / DOM",
        "Armazenamento",
        "Variáveis",
        "Tokens",
    ]
    .into_iter()
    .filter_map(|label| h.query_by_label(label).map(|n| n.rect().bottom()))
    .fold(page.bottom(), f32::max)
        + 8.;
    let mut clip = egui::Rect::from_min_max(
        egui::pos2(page.left() + 8., top),
        egui::pos2(page.right() - 8., bottom),
    );
    let before = match kind {
        RowKind::Request => h
            .query_by_label_contains("Até 200 requests")
            .map(|n| n.rect()),
        RowKind::Dom => h
            .query_by_label_contains("A árvore mostra elementos")
            .map(|n| n.rect()),
        RowKind::Storage => ["Cookies", "localStorage", "sessionStorage", "Nova entrada"]
            .into_iter()
            .filter_map(|label| h.query_by_label(label).map(|n| n.rect()))
            .max_by(|a, b| a.bottom().total_cmp(&b.bottom())),
    };
    if let Some(before) = before {
        clip.min.y = clip.min.y.max(before.bottom() + 3.);
    }
    let after = match kind {
        RowKind::Request => h
            .query_by_label_contains("Selecione uma request")
            .map(|n| n.rect()),
        RowKind::Dom => h.query_by_label("Atributos").map(|n| n.rect()),
        RowKind::Storage => h
            .query_by_role_and_label(Role::Label, "Nome / chave")
            .map(|n| n.rect()),
    };
    if let Some(after) = after {
        clip.max.y = clip.max.y.min(after.top() - 3.);
    }
    if matches!(kind, RowKind::Dom) {
        // Exclude the floating horizontal scrollbar below the final visible
        // row; its rectangle can overlap an AccessKit row fragment.
        clip.min.y += 4.;
        clip.max.y -= 15.;
    }
    clip
}
fn click_row(h: &mut H, needle: &str, kind: RowKind) {
    wait(h, |a| !a.service.browser_inspector.state.busy);
    ensure_widget(
        h,
        |h| {
            h.query(By::new().role(Role::Button).label_contains(needle))
                .is_some_and(|n| {
                    let rect = n.rect();
                    let visible = rect.intersect(row_clip(h, kind));
                    visible.width() > 8.
                        && visible.height() > 8.
                        && (!matches!(kind, RowKind::Dom) || visible.height() >= rect.height() - 1.)
                })
        },
        matches!(kind, RowKind::Dom),
        needle,
    );
    settle(h, |h| {
        h.get(By::new().role(Role::Button).label_contains(needle))
            .rect()
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    let point = loop {
        let point = h
            .get(By::new().role(Role::Button).label_contains(needle))
            .rect()
            .intersect(row_clip(h, kind))
            .center();
        h.hover_at(point);
        h.step();
        settle(h, |h| {
            h.get(By::new().role(Role::Button).label_contains(needle))
                .rect()
        });
        let current = h
            .get(By::new().role(Role::Button).label_contains(needle))
            .rect()
            .intersect(row_clip(h, kind))
            .center();
        if current.distance(point) < 0.25 {
            break point;
        }
        assert!(Instant::now() < deadline, "row moved after hover: {needle}");
    };
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        h.step();
        if h.query(By::new().role(Role::Button).label_contains(needle))
            .is_some_and(|n| n.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True))
        {
            break;
        }
        if Instant::now() >= deadline {
            let base = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../artifacts/browser-inspector-20261102/ui");
            h.render()
                .unwrap()
                .save(base.join("row-selection-failure-current.png"))
                .unwrap();
            panic!(
                "native row was not selected after its first visible click: {needle}; point={point:?}; rect={:?}; clip={:?}",
                h.query(By::new().role(Role::Button).label_contains(needle))
                    .map(|n| n.rect()),
                row_clip(h, kind)
            );
        }
        thread::sleep(Duration::from_millis(8));
    }
}
fn field(h: &mut H, label: &str, text: &str) {
    ensure_widget(
        h,
        |h| {
            h.query_by_role_and_label(Role::TextInput, label)
                .is_some_and(|n| visible(n.rect(), body_clip(h)))
        },
        false,
        label,
    );
    h.get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    h.step();
    settle(h, |h| {
        h.get_by_role_and_label(Role::TextInput, label).rect()
    });
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.step();
    h.key_press_modifiers(Modifiers::CTRL | Modifiers::COMMAND, Key::A);
    h.step();
    h.event(egui::Event::Text(text.into()));
    h.step();
    if !matches!(label, "Valor armazenado" | "Token JWT") {
        assert_eq!(
            h.get_by_role_and_label(Role::TextInput, label)
                .value()
                .as_deref(),
            Some(text),
            "native field did not receive input: {label}"
        );
    }
}
fn capture(h: &mut H, scale: f32, compact: bool, name: &str) {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/browser-inspector-20261102/ui");
    let root = root.join(format!(
        "{}-{}",
        if scale == 1. { "100" } else { "125" },
        if compact { "compact" } else { "normal" }
    ));
    fs::create_dir_all(&root).unwrap();
    h.render()
        .unwrap()
        .save(root.join(format!("{name}.png")))
        .unwrap();
    let s = &h.state().service.browser_inspector.state;
    fs::write(root.join(format!("{name}.json")),serde_json::to_vec_pretty(&serde_json::json!({
        "lifecycle":format!("{:?}",s.lifecycle),"revision":s.revision,"dom_nodes":s.dom.len(),"requests":s.network.len(),
        "local_keys":s.local_storage.iter().map(|(k,_)|k).collect::<Vec<_>>(),"session_keys":s.session_storage.iter().map(|(k,_)|k).collect::<Vec<_>>(),
        "cookie_names":s.cookies.iter().map(|c|&c.name).collect::<Vec<_>>(),"paused":s.paused.is_some(),"watch_count":s.watches.len(),
        "source_dirty":h.state().service.active_document().is_some_and(|d|d.is_dirty())
    })).unwrap()).unwrap();
}
fn dirty(h: &H, file: &Path) {
    let d = h.state().service.active_document().unwrap();
    assert_eq!(d.path, file);
    assert_eq!(d.text(), DIRTY);
    assert!(d.is_dirty());
    assert_eq!(fs::read_to_string(file).unwrap(), SAVED);
}
fn expand_tools(h: &mut H) {
    let page = h.get_by_label("Página do navegador").rect();
    let start = egui::pos2(page.center().x, page.bottom() + 3.);
    let target = egui::pos2(start.x, page.top() + page.height() * 0.3);
    h.hover_at(start);
    h.step();
    h.event(egui::Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    h.event(egui::Event::PointerMoved(target));
    h.step();
    h.event(egui::Event::PointerButton {
        pos: target,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    assert!(
        h.get_by_label("Página do navegador").rect().height() < page.height() * 0.7,
        "real inspector divider drag did not expand tools"
    );
}
fn setup(scale: f32, compact: bool, http: &Http) -> (tempfile::TempDir, H, PathBuf) {
    assert_eq!(
        std::env::var("CODISSEU_BROWSER_UI").as_deref(),
        Ok("1"),
        "run the private Xvfb launcher"
    );
    let profile = tempfile::tempdir().unwrap();
    let root = profile.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let file = root.join("main.js");
    fs::write(&file, SAVED).unwrap();
    let mut service = AppService::new(profile.path().join("profile/session.json"));
    service.dispatch(Command::OpenWorkspace(root)).unwrap();
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
    wait(&mut h, |a| {
        !a.service.is_busy() && a.service.active_document().is_some_and(|d| d.path == file)
    });
    let d = h.state().service.active_document().unwrap();
    let command = Command::Edit {
        id: d.id,
        expected_version: d.version,
        text: DIRTY.into(),
    };
    h.state_mut().service.dispatch(command).unwrap();
    h.step();
    h.state_mut().open_browser_url(&http.origin);
    wait(&mut h, |a| {
        a.browser_capture_ready()
            && a.browser_active()
                .is_some_and(|id| a.service.browser.tabs[&id].title == "Inspector fixture · pronta")
    });
    click(&mut h, "Inspecionar página");
    wait(&mut h, |a| {
        a.service.browser_inspector.state.lifecycle == Lifecycle::Ready
    });
    let name = thread::current().name().unwrap_or("scenario").to_owned();
    let kind = if name.contains("network_body") {
        "network"
    } else if name.contains("storage_cookie") {
        "storage"
    } else {
        "variables"
    };
    capture(&mut h, scale, compact, &format!("00-default-{kind}"));
    expand_tools(&mut h);
    capture(&mut h, scale, compact, &format!("00b-expanded-{kind}"));
    dirty(&h, &file);
    (profile, h, file)
}
fn finish(h: &mut H, file: &Path) {
    if h.state().service.browser_inspector.state.lifecycle != Lifecycle::Stopped {
        click(h, "Fechar inspetor");
    }
    wait(h, |a| {
        a.service.browser_inspector.state.lifecycle == Lifecycle::Stopped
    });
    dirty(h, file);
    h.state_mut().service.browser.shutdown();
    wait(h, |a| {
        a.service.browser.state == EngineState::Stopped && a.service.browser.engine_pid().is_none()
    });
    dirty(h, file);
}
fn evaluate(h: &mut H, expression: &str) -> String {
    static SERIAL: AtomicU64 = AtomicU64::new(1);
    // A harmless unique comment distinguishes this response from an older
    // displayed result when the same expression is evaluated again.
    let request = format!(
        "({expression}) /* inspector-qa:{} */",
        SERIAL.fetch_add(1, AtomicOrdering::Relaxed)
    );
    click(h, "Variáveis");
    field(h, "Expressão JavaScript", &request);
    click(h, "Avaliar expressão");
    wait(h, |a| {
        a.service
            .browser_inspector
            .state
            .evaluation
            .as_ref()
            .is_some_and(|e| e.expression == request && (e.value.is_some() || e.error.is_some()))
    });
    let e = h
        .state()
        .service
        .browser_inspector
        .state
        .evaluation
        .as_ref()
        .unwrap();
    assert!(e.error.is_none(), "{:?}", e.error);
    e.value.as_ref().unwrap().description.clone()
}

#[test]
#[ignore = "actual CEF/private Xvfb; native inspector network and DOM at four scale/size cases"]
fn cef_native_inspector_network_body_dom_shadow_and_dirty_source() {
    for (scale, compact) in cases() {
        let http = Http::new();
        let (_profile, mut h, file) = setup(scale, compact, &http);
        evaluate(&mut h, "setTimeout(()=>sendProbe(),0)");
        click(&mut h, "Rede");
        wait(&mut h, |a| {
            a.service
                .browser_inspector
                .state
                .network
                .iter()
                .any(|r| r.url.ends_with("/api/echo") && r.finished && r.status == Some(201.))
        });
        let data = http.state();
        let requests = data["requests"].as_array().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["method"], "POST");
        let body: serde_json::Value =
            serde_json::from_str(requests[0]["body"].as_str().unwrap()).unwrap();
        assert_eq!(body["left"], 19);
        assert_eq!(body["right"], 23);
        let request = h
            .state()
            .service
            .browser_inspector
            .state
            .network
            .iter()
            .find(|r| r.url.ends_with("/api/echo"))
            .unwrap();
        assert_eq!(request.method, "POST");
        assert!(request.duration_ms.is_some_and(|x| x > 0.));
        assert!(request.request_headers.iter().any(
            |(k, v)| k.eq_ignore_ascii_case("authorization") && v == "Bearer fixture-bearer-42"
        ));
        assert!(
            request
                .response_headers
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("x-fixture-response") && v == "response42")
        );
        let request_id = request.id.clone();
        field(&mut h, "Filtrar requests", "/api/echo");
        click_row(&mut h, "201  POST", RowKind::Request);
        assert_eq!(
            h.query_all_by_label_contains("Bearer fixture-bearer-42")
                .count(),
            0
        );
        capture(&mut h, scale, compact, "01-network-masked");
        click(&mut h, "Revelar valores");
        show_label(&mut h, "Request headers");
        show_contains(&mut h, "Bearer fixture-bearer-42");
        assert!(
            h.query_all_by_label_contains("Bearer fixture-bearer-42")
                .count()
                > 0
        );
        capture(&mut h, scale, compact, "02-network-revealed");
        click(&mut h, "Payload");
        capture(&mut h, scale, compact, "03-request-payload-readonly");
        click(&mut h, "Response");
        click(&mut h, "Capturar resposta");
        wait(&mut h, |a| {
            a.service
                .browser_inspector
                .state
                .bodies
                .contains_key(&request_id)
        });
        let response = &h.state().service.browser_inspector.state.bodies[&request_id];
        assert!(response.error.is_none());
        assert!(!response.base64);
        let json: serde_json::Value = serde_json::from_str(&response.text).unwrap();
        assert_eq!(json["answer"], 42);
        assert_eq!(json["authorization_received"], true);
        let copied = click(&mut h, "Copiar conteúdo");
        assert!(copied.iter().any(|text| {
            serde_json::from_str::<serde_json::Value>(text)
                .ok()
                .is_some_and(|v| v["answer"] == 42)
        }));
        // A native button can own keyboard focus too. Ctrl+S here must not
        // save the dirty source document behind the browser.
        h.key_press_modifiers(Modifiers::CTRL | Modifiers::COMMAND, Key::S);
        h.step();
        wait(&mut h, |a| !a.service.is_busy());
        dirty(&h, &file);
        capture(&mut h, scale, compact, "04-response-readonly");
        dirty(&h, &file);
        click(&mut h, "Componentes / DOM");
        wait(&mut h, |a| {
            a.service
                .browser_inspector
                .state
                .dom
                .iter()
                .any(|n| n.custom_element && n.local_name == "fixture-counter")
                && a.service
                    .browser_inspector
                    .state
                    .dom
                    .iter()
                    .any(|n| n.shadow_root)
        });
        click_row(&mut h, "h1#marker", RowKind::Dom);
        wait(&mut h, |a| {
            a.service
                .browser_inspector
                .state
                .attributes
                .iter()
                .any(|(k, v)| k == "id" && v == "marker")
                && !a.service.browser_inspector.state.styles.is_empty()
        });
        capture(&mut h, scale, compact, "05-dom-attributes-styles");
        click_row(&mut h, "[componente]", RowKind::Dom);
        capture(&mut h, scale, compact, "06-web-component");
        click_row(&mut h, "[shadow root]", RowKind::Dom);
        capture(&mut h, scale, compact, "07-shadow-root");
        finish(&mut h, &file);
    }
}

#[test]
#[ignore = "actual CEF/private Xvfb; storage/cookies CRUD and token widgets at four scale/size cases"]
fn cef_native_inspector_storage_cookie_crud_mask_copy_and_jwt() {
    for (scale, compact) in cases() {
        let http = Http::new();
        let (_profile, mut h, file) = setup(scale, compact, &http);
        click(&mut h, "Armazenamento");
        wait(&mut h, |a| {
            a.service
                .browser_inspector
                .state
                .cookies
                .iter()
                .any(|c| c.name == "fixture_http")
        });
        let cookie = h
            .state()
            .service
            .browser_inspector
            .state
            .cookies
            .iter()
            .find(|c| c.name == "fixture_http")
            .unwrap();
        assert!(cookie.http_only);
        assert_eq!(cookie.same_site.as_deref(), Some("Strict"));
        assert!(!evaluate(&mut h, "document.cookie").contains("fixture_http"));
        click(&mut h, "Armazenamento");
        capture(&mut h, scale, compact, "10-http-only-cookie-masked");
        click(&mut h, "Nova entrada");
        field(&mut h, "Nome / chave", "qa_cookie");
        field(&mut h, "Valor armazenado", "cookie-created-42");
        click(&mut h, "Salvar valor");
        wait(&mut h, |a| {
            a.service
                .browser_inspector
                .state
                .cookies
                .iter()
                .any(|c| c.name == "qa_cookie" && c.value == "cookie-created-42")
        });
        assert!(evaluate(&mut h, "document.cookie").contains("qa_cookie=cookie-created-42"));
        click(&mut h, "Armazenamento");
        click_row(&mut h, "qa_cookie", RowKind::Storage);
        field(&mut h, "Valor armazenado", "cookie-edited-42");
        click(&mut h, "Salvar valor");
        wait(&mut h, |a| {
            a.service
                .browser_inspector
                .state
                .cookies
                .iter()
                .any(|c| c.name == "qa_cookie" && c.value == "cookie-edited-42")
        });
        capture(&mut h, scale, compact, "11-cookie-edited");
        click(&mut h, "Excluir valor");
        wait(&mut h, |a| {
            !a.service
                .browser_inspector
                .state
                .cookies
                .iter()
                .any(|c| c.name == "qa_cookie")
        });
        assert!(!evaluate(&mut h, "document.cookie").contains("qa_cookie="));
        for (kind, property, key) in [
            ("localStorage", "localStorage", "qa-local"),
            ("sessionStorage", "sessionStorage", "qa-session"),
        ] {
            click(&mut h, "Armazenamento");
            click(&mut h, kind);
            click(&mut h, "Nova entrada");
            field(&mut h, "Nome / chave", key);
            field(&mut h, "Valor armazenado", "created42");
            click(&mut h, "Salvar valor");
            assert!(
                evaluate(&mut h, &format!("{property}.getItem('{key}')")).contains("created42")
            );
            click(&mut h, "Armazenamento");
            click(&mut h, kind);
            click_row(&mut h, key, RowKind::Storage);
            field(&mut h, "Valor armazenado", "edited42");
            click(&mut h, "Salvar valor");
            assert!(evaluate(&mut h, &format!("{property}.getItem('{key}')")).contains("edited42"));
            click(&mut h, "Armazenamento");
            click(&mut h, kind);
            click_row(&mut h, key, RowKind::Storage);
            capture(&mut h, scale, compact, &format!("12-{kind}-edited"));
            click(&mut h, "Excluir valor");
            assert_eq!(
                evaluate(&mut h, &format!("{property}.getItem('{key}')===null")),
                "true"
            );
        }
        click(&mut h, "Armazenamento");
        click(&mut h, "localStorage");
        click_row(&mut h, "access_token", RowKind::Storage);
        assert_eq!(
            h.query_all_by_label_contains("fixture-local-token-42")
                .count(),
            0
        );
        capture(&mut h, scale, compact, "13-token-masked");
        let copied = click(&mut h, "Copiar valor");
        assert!(copied.iter().any(|v| v == "fixture-local-token-42"));
        click(&mut h, "Revelar valores");
        assert!(
            h.query_all_by_label_contains("fixture-local-token-42")
                .count()
                > 0
        );
        capture(&mut h, scale, compact, "14-token-revealed");
        click_row(&mut h, "jwt_token", RowKind::Storage);
        click(&mut h, "Examinar como JWT");
        let before = http.state()["requests"].as_array().unwrap().len();
        click(&mut h, "Decodificar JWT");
        show_contains(&mut h, "fixture-user");
        assert!(h.query_all_by_label_contains("fixture-user").count() > 0);
        assert!(
            h.query_by_label(
                "Decodificação não verifica a assinatura nem a autenticidade do token."
            )
            .is_some()
        );
        assert_eq!(http.state()["requests"].as_array().unwrap().len(), before);
        capture(&mut h, scale, compact, "15-jwt-decoded-locally");
        finish(&mut h, &file);
    }
}

#[test]
#[ignore = "actual CEF/private Xvfb; native pause, frame locals/watch42, step and resume"]
fn cef_native_inspector_pause_locals_watch_step_resume_and_dirty_source() {
    for (scale, compact) in cases() {
        let http = Http::new();
        let (_profile, mut h, file) = setup(scale, compact, &http);
        click(&mut h, "Variáveis");
        click(&mut h, "Pausar JavaScript");
        wait(&mut h, |a| {
            a.service.browser_inspector.state.paused.is_some()
        });
        capture(&mut h, scale, compact, "20-native-pause");
        click(&mut h, "Continuar JavaScript");
        wait(&mut h, |a| {
            a.service.browser_inspector.state.paused.is_none()
        });
        evaluate(&mut h, "setTimeout(()=>computeProbe(),0)");
        wait(&mut h, |a| {
            a.service
                .browser_inspector
                .state
                .paused
                .as_ref()
                .is_some_and(|p| {
                    p.frames
                        .first()
                        .is_some_and(|f| f.function_name.contains("computeProbe"))
                })
        });
        field(&mut h, "Expressão de watch", "result");
        click(&mut h, "Adicionar watch");
        wait(&mut h, |a| {
            a.service.browser_inspector.state.watches.iter().any(|w| {
                w.expression == "result" && w.value.as_ref().is_some_and(|v| v.description == "42")
            })
        });
        click_contains(&mut h, "local ·");
        click(&mut h, "Ler variáveis do escopo");
        wait(&mut h, |a| {
            a.service
                .browser_inspector
                .state
                .scope_values
                .values()
                .flatten()
                .any(|p| p.name == "result" && p.value.description == "42")
        });
        click(&mut h, "Revelar valores");
        capture(&mut h, scale, compact, "21-real-locals-watch42");
        let line = h
            .state()
            .service
            .browser_inspector
            .state
            .paused
            .as_ref()
            .unwrap()
            .frames[0]
            .line;
        click(&mut h, "Step over (F8)");
        wait(&mut h, |a| {
            a.service
                .browser_inspector
                .state
                .paused
                .as_ref()
                .is_some_and(|p| p.frames.first().is_some_and(|f| f.line > line))
        });
        capture(&mut h, scale, compact, "22-step-over");
        click(&mut h, "Continuar JavaScript");
        wait(&mut h, |a| {
            a.service.browser_inspector.state.paused.is_none()
                && a.browser_active()
                    .is_some_and(|id| a.service.browser.tabs[&id].title == "InspectorCompute:42")
        });
        assert_eq!(evaluate(&mut h, "window.lastResult"), "42");
        capture(&mut h, scale, compact, "23-resumed-page42");
        // Transfer the real paused page to SDK F12, then back to a new native
        // epoch. Cleanup from the old owner must not disable the new debugger.
        click(&mut h, "Pausar JavaScript");
        wait(&mut h, |a| {
            a.service.browser_inspector.state.paused.is_some()
        });
        let tab = h.state().browser_active().unwrap();
        click(&mut h, "Ferramentas do desenvolvedor (F12)");
        wait(&mut h, |a| {
            a.service.browser_inspector.state.lifecycle == Lifecycle::Stopped
                && a.service.browser.tabs[&tab].devtools_open
                && a.browser_capture_state()["surfaces"]
                    .as_array()
                    .is_some_and(|rows| {
                        rows.iter().any(|r| {
                            r["id"] == tab
                                && r["surface"] == "DevTools"
                                && r["seq"].as_u64().is_some_and(|s| s > 0)
                        })
                    })
                && a.service.browser_devtools_diagnostics()["gateways"]
                    .as_array()
                    .is_some_and(|rows| {
                        rows.iter()
                            .any(|r| r["id"] == tab && r["connected"] == true)
                    })
        });
        capture(&mut h, scale, compact, "24-native-paused-to-sdk");
        click(&mut h, "Inspecionar página");
        wait(&mut h, |a| {
            a.service.browser_inspector.state.lifecycle == Lifecycle::Ready
                && !a.service.browser.tabs[&tab].devtools_open
        });
        assert_eq!(evaluate(&mut h, "6*7"), "42");
        let ticks = evaluate(&mut h, "window.fixtureTicks")
            .parse::<u64>()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while evaluate(&mut h, "window.fixtureTicks")
            .parse::<u64>()
            .unwrap()
            <= ticks
        {
            assert!(
                Instant::now() < deadline,
                "old inspector cleanup left the page paused"
            );
        }
        click(&mut h, "Pausar JavaScript");
        wait(&mut h, |a| {
            a.service.browser_inspector.state.paused.is_some()
        });
        click(&mut h, "Continuar JavaScript");
        wait(&mut h, |a| {
            a.service.browser_inspector.state.paused.is_none()
        });
        capture(&mut h, scale, compact, "25-sdk-back-to-native42");
        finish(&mut h, &file);
    }
}
