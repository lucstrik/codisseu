//! Real filesystem updates while the native editor is only being observed.
use codisseu_core::{AppService, Command};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

type H = Harness<'static, CodisseuApp>;

fn wait(h: &mut H, ready: impl Fn(&CodisseuApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        h.step();
        if ready(h.state()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{} {:?}",
            h.state().service.status,
            h.state().service.last_error
        );
        thread::sleep(Duration::from_millis(3));
    }
}

fn fixture(source: &str, scale: f32) -> (tempfile::TempDir, PathBuf, H) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    fs::create_dir(&root).unwrap();
    let path = root.join("main.ts");
    fs::write(&path, source).unwrap();
    let mut service = AppService::new(dir.path().join("profile/session.json"));
    service.dispatch(Command::OpenWorkspace(root)).unwrap();
    service.dispatch(Command::OpenFile(path.clone())).unwrap();
    let mut h = Harness::builder()
        .with_size([1100., 760.])
        .with_pixels_per_point(scale)
        .with_max_steps(80)
        .build_ui_state(
            |ui, app: &mut CodisseuApp| app.show(ui),
            CodisseuApp::new(service),
        );
    wait(&mut h, |a| !a.service.is_busy());
    h.get_by_role(Role::MultilineTextInput).focus();
    h.step();
    (dir, path, h)
}

fn assert_clean(h: &H, path: &Path, expected: &str) {
    let doc = h
        .state()
        .service
        .documents
        .iter()
        .find(|d| d.path == path)
        .unwrap();
    assert_eq!(doc.text(), expected);
    assert!(
        !doc.is_dirty(),
        "a passive reload must not become a local edit"
    );
    assert_eq!(fs::read(path).unwrap(), expected.as_bytes());
    assert!(
        h.state().service.last_error.is_none(),
        "{:?}",
        h.state().service.last_error
    );
}

#[test]
fn external_reload_active_editor_shrinks_unicode_without_creating_local_changes() {
    for scale in [1.0, 1.25] {
        let initial = "// original 😀 e\u{301}\r\n".repeat(140);
        let (_dir, path, mut h) = fixture(&initial, scale);
        h.key_press_modifiers(Modifiers::CTRL, Key::End);
        h.step();
        let original = h.state().service.active_document().unwrap();
        let (id, version) = (original.id, original.version);
        assert!(original.selection().head > 1000);
        let replacement = "export const ação = '雨😀e\u{301}';\r\n";
        fs::write(&path, replacement).unwrap();
        wait(&mut h, |a| {
            a.service.active_document().unwrap().text() == replacement
        });
        for _ in 0..8 {
            h.step();
        }
        assert_clean(&h, &path, replacement);
        let doc = h.state().service.active_document().unwrap();
        assert_eq!(doc.id, id);
        assert_eq!(doc.version, version + 1);
        assert!(doc.selection().head <= replacement.chars().count());
        assert_eq!(h.state().editor_metrics().lines, 2);
        assert!(h.state().editor_metrics().layout_error.is_none());
        // Closing through the real shortcut must not ask to discard anything.
        h.key_press_modifiers(Modifiers::COMMAND, Key::W);
        wait(&mut h, |a| a.service.documents.is_empty());
        assert!(h.query_by_label("Alterações não salvas").is_none());
        assert_eq!(fs::read_to_string(path).unwrap(), replacement);
    }
}

#[test]
fn external_reload_inactive_tab_and_split_share_current_content_without_save() {
    let (dir, path, mut h) = fixture("export const before = 1;\n", 1.0);
    let id = h.state().service.active.unwrap();
    let left = h.state().service.active_view().unwrap();
    h.state_mut().service.dispatch(Command::CreateView).unwrap();
    h.step();
    let right = h.state().service.active_view().unwrap();
    assert_ne!(left, right);
    assert_eq!(h.get_all_by_role(Role::MultilineTextInput).count(), 2);
    let other = path.parent().unwrap().join("other.ts");
    fs::write(&other, "export const untouched = true;\n").unwrap();
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(other.clone()))
        .unwrap();
    wait(&mut h, |a| !a.service.is_busy());
    let replacement = "// external atomic replacement 😀\nexport const after = 2;\n";
    let temporary = dir.path().join("replacement.tmp");
    fs::write(&temporary, replacement).unwrap();
    fs::rename(&temporary, &path).unwrap();
    wait(&mut h, |a| {
        a.service
            .documents
            .iter()
            .find(|d| d.id == id)
            .unwrap()
            .text()
            == replacement
    });
    assert_clean(&h, &path, replacement);
    assert_clean(&h, &other, "export const untouched = true;\n");
    h.state_mut()
        .service
        .dispatch(Command::ShowDocumentInView { view: right, id })
        .unwrap();
    h.step();
    h.step();
    assert_clean(&h, &path, replacement);
    assert_eq!(
        h.state().service.view_document(left).unwrap().text(),
        replacement
    );
    assert_eq!(
        h.state().service.view_document(right).unwrap().text(),
        replacement
    );
    assert_eq!(h.state().editor_view_metrics()[&left].lines, 3);
    assert_eq!(h.state().editor_view_metrics()[&right].lines, 3);
}

#[test]
fn external_reload_invalidates_preedit_before_late_ime_commit() {
    let (_dir, path, mut h) = fixture("const original = 1;\n", 1.0);
    h.event(egui::Event::Ime(egui::ImeEvent::Preedit {
        text: "雨".into(),
        active_range_chars: None,
    }));
    h.step();
    assert!(h.state().editor_metrics().composing);
    let replacement = "const changed = '😀';\r\n";
    fs::write(&path, replacement).unwrap();
    wait(&mut h, |a| {
        a.service.active_document().unwrap().text() == replacement
    });
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("雨".into())));
    h.step();
    h.step();
    assert_clean(&h, &path, replacement);
    assert!(!h.state().editor_metrics().composing);
}
