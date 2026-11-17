//! Actual status widgets and source selections, with no LSP or injected symbols.
use codisseu_core::{AppService, Command, Selection};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
type H = Harness<'static, CodisseuApp>;
fn wait(h: &mut H, f: impl Fn(&H) -> bool) {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        h.step();
        if f(h) {
            h.step();
            return;
        }
        assert!(
            Instant::now() < until,
            "{} {:?}",
            h.state().service.status,
            h.state().service.last_error
        );
        thread::sleep(Duration::from_millis(2));
    }
}
fn fixture(ext: &str, width: f32, scale: f32) -> (tempfile::TempDir, PathBuf, String, H) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    fs::create_dir_all(root.join("src/deep")).unwrap();
    let text=if ext=="php" {"<?php\r\nnamespace EditorAudit;\r\nclass Main {\r\n public function run(Greeting $greeter): string {\r\n  // 😀 é\r\n  return $greeter->greet(\"Ada\", 2);\r\n }\r\n}\r\n// outside\r\n"} else {"// 😀 é\nexport class Main {\n run(value: string) { return this.greet(\"Ada\", 2); }\n greet(name: string, repeat: number) { return name; }\n}\n// outside\n"}.to_owned();
    let path = root.join(format!("src/deep/Main.{ext}"));
    fs::write(&path, &text).unwrap();
    let mut s = AppService::new(dir.path().join("profile/session.json"));
    s.dispatch(Command::SetAutoSave {
        enabled: false,
        delay_ms: 1000,
    })
    .unwrap();
    s.dispatch(Command::OpenWorkspace(root)).unwrap();
    s.dispatch(Command::OpenFile(path.clone())).unwrap();
    let mut h = Harness::builder()
        .with_size([width, 760.])
        .with_pixels_per_point(scale)
        .with_max_steps(80)
        .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), CodisseuApp::new(s));
    wait(&mut h, |h| !h.state().service.is_busy());
    (dir, path, text, h)
}
fn select(h: &mut H, at: usize) {
    let d = h.state().service.active_document().unwrap();
    let (id, expected_version) = (d.id, d.version);
    h.state_mut()
        .service
        .dispatch(Command::SetSelection {
            id,
            expected_version,
            selection: Selection::caret(at),
        })
        .unwrap();
    h.step();
    h.step();
}
fn at(text: &str, needle: &str) -> usize {
    text[..text.find(needle).unwrap() + 1].chars().count()
}
fn shown(h: &H, label: &str) -> bool {
    h.query_by_label(label).is_some()
}
#[test]
fn php_js_ts_follow_cursor_arguments_and_click_with_back_without_changing_source() {
    for ext in ["php", "js", "ts"] {
        for scale in [1., 1.25] {
            let (_dir, path, original, mut h) = fixture(ext, 1280., scale);
            // The JS fixture uses the same grammar without TypeScript annotations.
            let text = if ext == "js" {
                original.replace(": string", "").replace(": number", "")
            } else {
                original.clone()
            };
            if text != original {
                let d = h.state().service.active_document().unwrap();
                let (id, expected_version) = (d.id, d.version);
                h.state_mut()
                    .service
                    .dispatch(Command::Edit {
                        id,
                        expected_version,
                        text: text.clone(),
                    })
                    .unwrap();
            }
            let origin = at(&text, "Ada");
            select(&mut h, origin);
            wait(&mut h, |h| shown(h, "Trilha: Argumento argumento 1"));
            assert!(shown(&h, "Trilha: Classe Main"));
            assert!(shown(&h, "Trilha: Método run"));
            assert!(shown(&h, &format!("Trilha: arquivo Main.{ext}")));
            h.get_by_label("Trilha: Método run").click();
            let target = text[..text.find("run(").unwrap()].chars().count();
            wait(&mut h, |h| {
                h.state()
                    .service
                    .active_document()
                    .unwrap()
                    .selection()
                    .head
                    == target
            });
            assert!(h.state().service.navigation.can_back());
            let d = h.state().service.active_document().unwrap();
            let (id, expected_version) = (d.id, d.version);
            h.state_mut()
                .service
                .dispatch(Command::NavigateBack {
                    id,
                    expected_version,
                    character_index: target,
                })
                .unwrap();
            wait(&mut h, |h| {
                h.state()
                    .service
                    .active_document()
                    .unwrap()
                    .selection()
                    .head
                    == origin
            });
            assert_eq!(h.state().service.active_document().unwrap().text(), text);
            assert_eq!(fs::read_to_string(path).unwrap(), original);
            if ext == "js" {
                h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
                wait(&mut h, |h| {
                    h.state().service.active_document().unwrap().text() == original
                });
            }
        }
    }
}
#[test]
fn contextual_trail_precedes_messages_and_outside_scope_returns_friendly_git_state() {
    let (_dir, _path, text, mut h) = fixture("php", 1280., 1.);
    let raw = "fatal: not a git repository (or any parent up to mount point /) Stopping at filesystem boundary (GIT_DISCOVERY_ACROSS_FILESYSTEM not set).";
    h.state_mut().service.git.error = Some(raw.into());
    select(&mut h, at(&text, "Ada"));
    wait(&mut h, |h| shown(h, "Trilha: Método run"));
    assert!(!shown(&h, "Projeto sem repositório Git"));
    assert!(shown(&h, "Mensagens da operação"));
    h.get_by_label("Mensagens da operação").click();
    h.run_steps(3);
    assert!(shown(&h, "Projeto sem repositório Git"));
    assert!(shown(&h, "Copiar detalhes técnicos"));
    h.key_press(Key::Escape);
    h.run_steps(3);
    select(&mut h, at(&text, "outside"));
    wait(&mut h, |h| shown(h, "Projeto sem repositório Git"));
    assert!(!shown(&h, "Trilha: Método run"));
    assert!(!shown(&h, raw));
    h.state_mut().service.status = "Arquivo salvo".into();
    h.run_steps(3);
    assert!(
        shown(&h, "Arquivo salvo"),
        "a new operation replaces the older no-Git notice"
    );
}
#[test]
fn changed_content_invalidates_trail_and_active_split_uses_its_own_caret() {
    let (_dir, _path, text, mut h) = fixture("ts", 1280., 1.25);
    select(&mut h, at(&text, "Ada"));
    wait(&mut h, |h| shown(h, "Trilha: Método run"));
    let d = h.state().service.active_document().unwrap();
    let (id, expected_version) = (d.id, d.version);
    h.state_mut()
        .service
        .dispatch(Command::Edit {
            id,
            expected_version,
            text: text.replace("run(", "execute("),
        })
        .unwrap();
    h.step();
    assert!(!shown(&h, "Trilha: Método run"));
    select(&mut h, at(&text.replace("run(", "execute("), "Ada"));
    wait(&mut h, |h| shown(h, "Trilha: Método execute"));
    if !shown(&h, "Exibir") {
        h.get_by_label("Menu principal").click();
        h.run_steps(3);
    }
    h.get_by_label("Exibir").click();
    h.run_steps(3);
    h.get_by_label("Dividir à direita").click();
    h.run_steps(4);
    let right = h.state().service.active_view().unwrap();
    select(&mut h, text.chars().count() + 4);
    h.run_steps(3);
    assert!(!shown(&h, "Trilha: Método execute"));
    let left = h
        .state()
        .service
        .document_views()
        .iter()
        .find(|v| v.id != right)
        .unwrap()
        .id;
    h.state_mut()
        .service
        .dispatch(Command::ActivateView { view: left })
        .unwrap();
    select(&mut h, at(&text.replace("run(", "execute("), "Ada"));
    wait(&mut h, |h| shown(h, "Trilha: Método execute"));
}
#[test]
fn narrow_status_elides_into_real_menu_and_keyboard_returns_to_editor() {
    for scale in [1., 1.25] {
        let (_dir, _path, text, mut h) = fixture("php", 800., scale);
        select(&mut h, at(&text, "Ada"));
        wait(&mut h, |h| shown(h, "Trilha: Argumento argumento 1"));
        let copilot = h.get_by_label("Copilot").rect();
        for n in h.root().children_recursive().filter(|n| {
            n.accesskit_node()
                .label()
                .is_some_and(|l| l.starts_with("Trilha:"))
        }) {
            let r = n.rect();
            assert!(
                r.left() >= 0. && r.right() <= copilot.left() + 1. && r.bottom() <= 760.,
                "{r:?} {copilot:?}"
            );
        }
        assert!(shown(&h, "Trilha: caminho completo"));
        h.get_by_label("Trilha: caminho completo").click();
        h.run_steps(3);
        assert!(shown(&h, "Trilha: pasta project"));
        h.key_press(Key::Escape);
        h.run_steps(3);
        h.key_press_modifiers(Modifiers::ALT, Key::Home);
        h.run_steps(3);
        assert!(h.get_by_label("Trilha: Argumento argumento 1").is_focused());
        h.key_press(Key::Escape);
        h.run_steps(3);
        assert!(h.get_by_role(Role::MultilineTextInput).is_focused());
        h.get_by_label("Configurações").click();
        h.run_steps(4);
        let focused = h.ctx.memory(|m| m.focused());
        h.key_press_modifiers(Modifiers::ALT, Key::Home);
        h.run_steps(3);
        assert_eq!(
            h.ctx.memory(|m| m.focused()),
            focused,
            "navigation shortcut must not steal modal input"
        );
    }
}
