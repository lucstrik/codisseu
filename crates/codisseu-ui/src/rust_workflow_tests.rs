//! Real egui controls -> shared service -> analyzer. Fixtures/profiles are owned
//! temporary directories; opening or reviewing Build never executes their code.
use super::*;
use codisseu_core::{Language, ProviderConfig, ProviderState, Selection};
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::{fs, path::Path, thread, time::Instant};

const BUILD: &str = "Build Rust · Cargo build · Ctrl+F9";
fn wait(h: &mut Harness<'_, CodisseuApp>, predicate: impl Fn(&CodisseuApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        h.step();
        if predicate(h.state()) {
            h.step();
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{} {:?} rename={:?}",
            h.state().service.status,
            h.state().service.last_error,
            h.state().service.rename.error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn idle(h: &mut Harness<'_, CodisseuApp>) {
    wait(h, |a| !a.service.is_busy());
}
fn open(h: &mut Harness<'_, CodisseuApp>, path: &Path) {
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(path.into()))
        .unwrap();
    idle(h);
    h.run();
}
fn capture(h: &mut Harness<'_, CodisseuApp>, scale: f32, name: &str) {
    if std::env::var_os("CODISSEU_RUST_WORKFLOW_CAPTURES").is_none() {
        return;
    }
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/rust-professional-20261030")
        .join(if scale == 1. {
            "ui-workflows100"
        } else {
            "ui-workflows125"
        });
    fs::create_dir_all(&directory).unwrap();
    h.run();
    h.render()
        .unwrap()
        .save(directory.join(format!("{name}.png")))
        .unwrap();
}
fn setup(scale: f32) -> (tempfile::TempDir, Harness<'static, CodisseuApp>, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Rust GUI ação");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname=\"codisseu_rust_gui_workflow\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"),"// ação 😀\r\npub mod other;\r\npub fn rainfall(value: i32) -> i32 { let amount = value + 1; amount }\r\npub fn run() -> i32 { rainfall(41) }\r\n// rainfall remains a comment\r\npub const LABEL: &str = \"rainfall\";\r\n").unwrap();
    fs::write(
        root.join("src/other.rs"),
        "pub fn call() -> i32 { crate::rainfall(1) }\r\n",
    )
    .unwrap();
    fs::write(
        root.join("build.rs"),
        "fn main() { std::fs::write(\"BUILD_EXECUTED\", \"unexpected\").unwrap(); }\n",
    )
    .unwrap();
    let mut service = AppService::new(dir.path().join("profile/session.json"));
    service
        .dispatch(Command::OpenWorkspace(root.clone()))
        .unwrap();
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .with_pixels_per_point(scale)
        .with_max_steps(150)
        .build_ui_state(
            |ui, a: &mut CodisseuApp| a.show(ui),
            CodisseuApp::new(service),
        );
    idle(&mut h);
    open(&mut h, &root.join("src/lib.rs"));
    (dir, h, root)
}
fn click_disabled(h: &mut Harness<'_, CodisseuApp>, label: &str) {
    let point = h.get_by_label(label).rect().center();
    h.hover_at(point);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    h.step();
    h.step();
}
#[test]
fn rust_build_toolbar_tracks_cargo_context_and_disabled_languages_never_open_or_execute() {
    for scale in [1., 1.25] {
        let (dir, mut h, root) = setup(scale);
        wait(&mut h, |a| a.rust_build_enabled());
        h.run();
        assert!(!h.get_by_label(BUILD).accesskit_node().is_disabled());
        let build = h.get_by_label(BUILD).rect();
        let debug = h.get_by_label("Debug…  Shift+F9").rect();
        assert!((build.center().y - debug.center().y).abs() < 2.);
        assert!(
            build.right() <= debug.left() + 1. && debug.left() - build.right() < 80.,
            "Build must remain beside run/debug: {build:?} {debug:?}"
        );
        capture(&mut h, scale, "build-enabled-rust");
        h.get_by_label(BUILD).click();
        wait(&mut h, |a| a.rust_tools.open);
        h.run();
        assert!(h.query_by_label("Rust · Cargo").is_some());
        assert!(
            h.get_by_label("Executar tarefa Cargo")
                .accesskit_node()
                .is_disabled()
        );
        assert!(!h.state().terminal.has_sessions());
        capture(&mut h, scale, "build-review-requires-trust");
        h.get_by_label("Cancelar").click();
        h.run();
        assert!(!h.state().rust_tools.open);
        open(&mut h, &root.join("Cargo.toml"));
        wait(&mut h, |a| a.rust_build_enabled());
        assert!(!h.get_by_label(BUILD).accesskit_node().is_disabled());
        capture(&mut h, scale, "build-enabled-cargo-manifest");
        open(&mut h, &root.join("src/lib.rs"));
        wait(&mut h, |a| a.rust_build_enabled());
        for (name, text) in [
            ("example.php", "<?php echo 42;\n"),
            ("example.js", "const value = 42;\n"),
            ("example.ts", "const value: number = 42;\n"),
            ("example.sql", "SELECT 42;\n"),
        ] {
            let path = root.join(name);
            fs::write(&path, text).unwrap();
            open(&mut h, &path);
            assert!(!h.state().rust_build_enabled());
            assert!(h.get_by_label(BUILD).accesskit_node().is_disabled());
            capture(&mut h, scale, &format!("build-disabled-{name}"));
            click_disabled(&mut h, BUILD);
            assert!(!h.state().rust_tools.open);
            assert!(!h.state().terminal.has_sessions());
            assert_eq!(fs::read_to_string(&path).unwrap(), text);
        }
        let bare = dir.path().join("no Cargo");
        fs::create_dir(&bare).unwrap();
        let source = bare.join("plain.rs");
        fs::write(&source, "fn main() {}\n").unwrap();
        h.state_mut()
            .service
            .dispatch(Command::OpenWorkspace(bare.clone()))
            .unwrap();
        idle(&mut h);
        open(&mut h, &source);
        assert!(!h.state().rust_build_enabled());
        assert!(h.get_by_label(BUILD).accesskit_node().is_disabled());
        click_disabled(&mut h, BUILD);
        assert!(!h.state().rust_tools.open);
        capture(&mut h, scale, "build-disabled-rust-no-cargo");
        h.state_mut()
            .service
            .dispatch(Command::OpenWorkspace(root.clone()))
            .unwrap();
        idle(&mut h);
        open(&mut h, &root.join("src/lib.rs"));
        wait(&mut h, |a| a.rust_build_enabled());
        assert!(!h.get_by_label(BUILD).accesskit_node().is_disabled());
        assert!(h.state().service.language.statuses.is_empty());
        assert!(!h.state().terminal.has_sessions());
        assert!(h.state().service.debugger.sessions.is_empty());
        assert!(!root.join("BUILD_EXECUTED").exists());
        assert!(!root.join("target").exists());
    }
}
fn start_provider(h: &mut Harness<'_, CodisseuApp>, path: &Path) {
    h.state_mut()
        .service
        .dispatch(Command::ConfigureProviders(vec![ProviderConfig {
            language: Language::Rust,
            program: std::env::var_os("CODISSEU_RUST_ANALYZER")
                .unwrap_or_else(|| "rust-analyzer".into())
                .into(),
            args: vec![],
        }]))
        .unwrap();
    h.state_mut()
        .service
        .dispatch(Command::StartProviders)
        .unwrap();
    wait(h, |a| {
        !a.service.is_busy()
            && a.service
                .language
                .statuses
                .iter()
                .any(|p| p.language == Language::Rust && p.state == ProviderState::Ready)
            && a.service.language.analyzed_versions.contains_key(path)
    });
}
fn rename_using_widgets_attempt(
    h: &mut Harness<'_, CodisseuApp>,
    path: &Path,
    needle: &str,
    new_name: &str,
    scale: f32,
    tag: &str,
) -> bool {
    open(h, path);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let doc = h.state().service.active_document().unwrap();
        let at = doc.text()[..doc.text().find(needle).unwrap()]
            .chars()
            .count()
            + 1;
        let cmd = Command::SetSelection {
            id: doc.id,
            expected_version: doc.version,
            selection: Selection::caret(at),
        };
        h.state_mut().service.dispatch(cmd).unwrap();
        h.get_by_role(Role::MultilineTextInput).focus();
        h.step();
        h.key_press_modifiers(Modifiers::SHIFT, Key::F6);
        wait(h, |a| !a.service.rename.is_pending());
        if h.state().service.rename.is_prepared() {
            break;
        }
        h.key_press(Key::Escape);
        h.step();
        assert!(
            Instant::now() < deadline,
            "{:?}",
            h.state().service.rename.error
        );
        thread::sleep(Duration::from_millis(50));
    }
    wait(h, |a| a.service.rename.is_prepared());
    for _ in 0..12 {
        h.step();
        if h.query_by_label("Renomear símbolo e seus usos para:")
            .is_some_and(|n| n.is_focused())
        {
            break;
        }
    }
    h.key_press_modifiers(Modifiers::SHIFT, Key::F6);
    h.run();
    assert!(
        h.query_by_label("Renomear símbolo e seus usos para:")
            .is_some()
    );
    h.get_by_label("Renomear símbolo e seus usos para:").focus();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(egui::Event::Text(new_name.into()));
    h.step();
    h.step();
    capture(h, scale, &format!("rename-{tag}-dialog"));
    h.get_by_label("Preview").click();
    wait(h, |a| {
        !a.service.rename.is_pending()
            && (!a.service.rename.files.is_empty() || a.service.rename.error.is_some())
    });
    if h.state()
        .service
        .rename
        .error
        .as_deref()
        .is_some_and(|e| e.to_lowercase().contains("content modified"))
    {
        // The analyzer may replace its startup workspace between prepare and rename.
        // Repeat the complete user gesture; validation/conflict errors never retry.
        eprintln!("Rust GUI analyzer workspace changed during {tag}; retrying full user gesture");
        h.key_press(Key::Escape);
        h.step();
        return false;
    }
    h.run();
    assert!(
        h.state().service.rename.can_apply(),
        "{:?} {:?}",
        h.state().service.rename.error,
        h.state().service.rename.validation
    );
    assert!(
        h.query_by_label("Rust-analyzer e validação estrutural Rust dos arquivos locais.")
            .is_some()
    );
    assert!(!h.get_by_label("Refatorar").accesskit_node().is_disabled());
    capture(h, scale, &format!("rename-{tag}-preview"));
    h.get_by_label("Refatorar").click();
    idle(h);
    assert!(
        h.state().service.last_error.is_none(),
        "{:?}",
        h.state().service.last_error
    );
    true
}
fn rename_using_widgets(
    h: &mut Harness<'_, CodisseuApp>,
    path: &Path,
    needle: &str,
    new_name: &str,
    scale: f32,
    tag: &str,
) {
    for _ in 0..3 {
        if rename_using_widgets_attempt(h, path, needle, new_name, scale, tag) {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("analyzer workspace did not stabilize for {tag}");
}
fn current(h: &Harness<'_, CodisseuApp>, path: &Path) -> String {
    h.state()
        .service
        .documents
        .iter()
        .find(|d| d.path == path)
        .unwrap()
        .text()
        .to_owned()
}
#[test]
#[ignore = "real Rust GUI rename: requires rust-analyzer/rust-src"]
fn rust_real_rename_widgets_preview_scope_group_undo_save_and_raw_identifier() {
    for scale in [1., 1.25] {
        let (_dir, mut h, root) = setup(scale);
        let lib = root.join("src/lib.rs");
        let other = root.join("src/other.rs");
        let disk_lib = fs::read_to_string(&lib).unwrap();
        let disk_other = fs::read_to_string(&other).unwrap();
        open(&mut h, &other);
        let d = h.state().service.active_document().unwrap();
        let dirty_other = format!("// dirty 😀\r\n{}", d.text());
        let cmd = Command::Edit {
            id: d.id,
            expected_version: d.version,
            text: dirty_other.clone(),
        };
        h.state_mut().service.dispatch(cmd).unwrap();
        idle(&mut h);
        open(&mut h, &lib);
        start_provider(&mut h, &lib);
        rename_using_widgets(
            &mut h,
            &lib,
            "rainfall(value",
            "precipitação",
            scale,
            "function",
        );
        let after_lib = current(&h, &lib);
        let after_other = current(&h, &other);
        assert!(after_lib.contains("pub fn precipitação("));
        assert!(after_other.contains("crate::precipitação(1)"));
        assert!(after_lib.contains("// rainfall remains a comment"));
        assert!(after_lib.contains("\"rainfall\""));
        assert_eq!(fs::read_to_string(&lib).unwrap(), disk_lib);
        assert_eq!(fs::read_to_string(&other).unwrap(), disk_other);
        open(&mut h, &lib);
        h.get_by_role(Role::MultilineTextInput).focus();
        h.step();
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        idle(&mut h);
        assert_eq!(current(&h, &lib), disk_lib);
        assert_eq!(current(&h, &other), dirty_other);
        h.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
        idle(&mut h);
        assert_eq!(current(&h, &lib), after_lib);
        assert_eq!(current(&h, &other), after_other);
        h.key_press_modifiers(Modifiers::COMMAND, Key::S);
        idle(&mut h);
        assert_eq!(fs::read_to_string(&lib).unwrap(), after_lib);
        assert_eq!(fs::read_to_string(&other).unwrap(), disk_other);
        open(&mut h, &other);
        h.get_by_role(Role::MultilineTextInput).focus();
        h.step();
        h.key_press_modifiers(Modifiers::COMMAND, Key::S);
        idle(&mut h);
        assert_eq!(fs::read_to_string(&other).unwrap(), after_other);
        rename_using_widgets(&mut h, &lib, "amount =", "r#type", scale, "raw-local");
        let raw = current(&h, &lib);
        assert!(raw.contains("let r#type = value + 1; r#type"), "{raw}");
        assert_eq!(fs::read_to_string(&lib).unwrap(), after_lib);
        assert!(!root.join("BUILD_EXECUTED").exists());
        assert!(!h.state().terminal.has_sessions());
        assert!(h.state().service.debugger.sessions.is_empty());
        h.state_mut()
            .service
            .dispatch(Command::StopProviders)
            .unwrap();
        idle(&mut h);
        assert!(!h.state().service.providers_running());
    }
}
