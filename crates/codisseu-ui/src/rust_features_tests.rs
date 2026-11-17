//! Real widget acceptance: analyzer actions, preview/apply/undo and visual-only hints.
//! Each run owns its project/profile; the fixture build script must never run.
use super::*;
use codisseu_core::{Language, ProviderConfig, ProviderState, Selection};
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{fs, thread, time::Instant};

const SOURCE: &str = "// ação 😀\r\npub fn combine(first: i32, second: i32) -> i32 { first + second }\r\npub fn run() -> i32 {\r\n    let mut values = HashMap::new();\r\n    values.insert(\"answer\", 42);\r\n    let _emoji = \"😀\"; let result = combine(40, 2);\r\n    result + values[\"answer\"]\r\n}\r\n";
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
            "Rust UI timeout: status={} error={:?} actions={:?} hints={:?}",
            h.state().service.status,
            h.state().service.last_error,
            h.state().service.code_actions.error,
            h.state().service.inlay_hints.error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn idle(h: &mut Harness<'_, CodisseuApp>) {
    wait(h, |a| !a.service.is_busy());
}
fn select(h: &mut Harness<'_, CodisseuApp>, selection: Selection) {
    let doc = h.state().service.active_document().unwrap();
    let command = Command::SetSelection {
        id: doc.id,
        expected_version: doc.version,
        selection,
    };
    h.state_mut().service.dispatch(command).unwrap();
    h.get_by_role(Role::MultilineTextInput).focus();
    h.step();
    h.step();
}
fn capture(h: &mut Harness<'_, CodisseuApp>, scale: f32, name: &str) {
    if std::env::var_os("CODISSEU_RUST_FEATURE_CAPTURES").is_none() {
        return;
    }
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/rust-professional-20261030/ui-features")
        .join(if scale == 1. { "100" } else { "125" });
    fs::create_dir_all(&dir).unwrap();
    h.render()
        .unwrap()
        .save(dir.join(format!("{name}.png")))
        .unwrap();
}
fn setup(scale: f32) -> (tempfile::TempDir, Harness<'static, CodisseuApp>, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Rust correções ação");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname=\"codisseu_rust_features_ui\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
    )
    .unwrap();
    let path = root.join("src/lib.rs");
    fs::write(&path, SOURCE).unwrap();
    fs::write(
        root.join("build.rs"),
        "fn main(){std::fs::write(\"BUILD_EXECUTED\",\"unexpected\").unwrap();}\n",
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
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(path.clone()))
        .unwrap();
    idle(&mut h);
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
    wait(&mut h, |a| {
        !a.service.is_busy()
            && a.service
                .language
                .statuses
                .iter()
                .any(|p| p.language == Language::Rust && p.state == ProviderState::Ready)
            && a.service.language.analyzed_versions.contains_key(&path)
    });
    (dir, h, root)
}
fn assert_source(h: &Harness<'_, CodisseuApp>, expected: &str) {
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        expected
    );
}
fn import_action(h: &mut Harness<'_, CodisseuApp>) -> String {
    let at = SOURCE[..SOURCE.find("HashMap").unwrap()].chars().count() + 2;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        select(h, Selection::caret(at));
        h.key_press_modifiers(Modifiers::ALT, Key::Enter);
        wait(h, |a| !a.service.code_actions.is_pending());
        if let Some(item) = h
            .state()
            .service
            .code_actions
            .items
            .iter()
            .find(|i| i.title.contains("HashMap") && i.disabled_reason.is_none())
        {
            return if item.preferred {
                format!("{}  · recomendada", item.title)
            } else {
                item.title.clone()
            };
        }
        assert!(
            Instant::now() < deadline,
            "Missing real import action: {:?} {:?}",
            h.state().service.code_actions.items,
            h.state().service.code_actions.error
        );
        if h.state().service.code_actions.is_open() {
            h.get_by_label("Cancelar").click();
            h.step();
            h.step();
        }
        thread::sleep(Duration::from_millis(100));
    }
}
#[test]
#[ignore = "real rust-analyzer/rust-src and WGPU screenshots at 100%/125%"]
fn real_rust_quick_fix_widgets_preview_apply_undo_and_visual_hint_copy_hit_testing() {
    for scale in [1., 1.25] {
        let (_dir, mut h, root) = setup(scale);
        let path = root.join("src/lib.rs");
        let label = import_action(&mut h);
        eprintln!("scale={scale} real action={label}");
        assert_source(&h, SOURCE);
        assert!(h.query_by_label("Correções rápidas · Rust").is_some());
        capture(&mut h, scale, "quick-fix-list");
        h.get_by_label(&label).click();
        wait(&mut h, |a| {
            a.service.code_actions.has_preview() && !a.service.code_actions.is_pending()
        });
        assert_source(&h, SOURCE);
        assert_eq!(fs::read_to_string(&path).unwrap(), SOURCE);
        let files = &h.state().service.code_actions.files;
        assert_eq!(files.len(), 1);
        assert!(files[0].after.contains("use std::collections::HashMap;"));
        assert!(h.query_by_label("Preview da correção Rust").is_some());
        assert!(h.query_by_label("Antes").is_some());
        assert!(h.query_by_label("Depois").is_some());
        // The footer must stay clickable as soon as the preview appears. Hover
        // one normal frame, then verify its geometry before the first click.
        let before = h.get_by_label("Aplicar correção").rect();
        h.get_by_label("Aplicar correção").hover();
        h.step();
        assert_eq!(
            before,
            h.get_by_label("Aplicar correção").rect(),
            "preview footer moved under pointer"
        );
        capture(&mut h, scale, "quick-fix-preview");
        h.get_by_label("Aplicar correção").click();
        wait(&mut h, |a| !a.service.code_actions.is_open());
        let fixed = h
            .state()
            .service
            .active_document()
            .unwrap()
            .text()
            .to_owned();
        assert!(fixed.contains("use std::collections::HashMap;"));
        assert!(fixed.contains("// ação 😀\r\n"));
        assert_eq!(fs::read_to_string(&path).unwrap(), SOURCE);
        assert!(h.state().service.active_document().unwrap().is_dirty());
        wait(&mut h, |a| {
            a.editor_metrics().inlay_hints > 0
                && a.service
                    .inlay_hints
                    .items
                    .iter()
                    .any(|i| i.kind == Some(1))
                && a.service
                    .inlay_hints
                    .items
                    .iter()
                    .any(|i| i.label.contains("first:") || i.label.contains("second:"))
        });
        eprintln!(
            "scale={scale} visual hints={} real={:?}",
            h.state().editor_metrics().inlay_hints,
            h.state().service.inlay_hints.items
        );
        assert_source(&h, &fixed);
        capture(&mut h, scale, "real-type-parameter-inlay-hints");
        // The painted caret lies after several real type/parameter hints and an emoji.
        // Relocate it, then click the earlier screen coordinate: mapping must recover
        // the original source index rather than a display offset containing labels.
        let byte = fixed.find("combine(40, 2)").unwrap() + "combine(40, ".len();
        let at = fixed[..byte].chars().count();
        select(&mut h, Selection::caret(at));
        let caret = h.state().caret_screen.unwrap();
        let point = caret - egui::vec2(0., h.state().tokens.editor_line_height / 2.);
        select(&mut h, Selection::caret(at - 1));
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
        assert_eq!(
            h.state().service.active_document().unwrap().selection(),
            Selection::caret(at),
            "pointer after real hints must preserve source caret at scale {scale}"
        );
        assert_source(&h, &fixed);
        let len = fixed.chars().count();
        select(
            &mut h,
            Selection {
                anchor: 0,
                head: len,
            },
        );
        h.event(egui::Event::Copy);
        h.step();
        let copied = h
            .output()
            .platform_output
            .commands
            .iter()
            .find_map(|c| {
                if let egui::OutputCommand::CopyText(t) = c {
                    Some(t)
                } else {
                    None
                }
            })
            .expect("real editor Copy output");
        assert_eq!(
            copied, &fixed,
            "copy must exclude visual hints and preserve CRLF/emoji"
        );
        select(&mut h, Selection::caret(at));
        capture(&mut h, scale, "hint-pointer-maps-to-source");
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.step();
        h.step();
        assert_source(&h, SOURCE);
        assert_eq!(fs::read_to_string(&path).unwrap(), SOURCE);
        capture(&mut h, scale, "quick-fix-undone");
        assert!(!root.join("BUILD_EXECUTED").exists());
        assert!(!root.join("target").exists());
        assert!(!h.state().terminal.has_sessions());
        h.state_mut()
            .service
            .dispatch(Command::StopProviders)
            .unwrap();
        idle(&mut h);
    }
}
