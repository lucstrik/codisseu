//! Real pointer gestures on the editor tab strip; no direct reorder in UI tests.
use codisseu_core::{AppService, Command, Selection};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, Pos2, Rect, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs,
    time::{Duration, Instant},
};

type H = Harness<'static, CodisseuApp>;
fn settle(h: &mut H) {
    let deadline = Instant::now() + Duration::from_secs(15);
    for _ in 0..5 {
        h.step();
    }
    while h.state().service.is_busy() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
        h.step();
    }
    for _ in 0..3 {
        h.step();
    }
    assert!(
        h.state().service.last_error.is_none(),
        "{:?}",
        h.state().service.last_error
    );
}
fn fixture(count: usize, scale: f32) -> (tempfile::TempDir, H, Vec<u64>) {
    let dir = tempfile::tempdir().unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    let mut ids = Vec::new();
    for i in 0..count {
        let path = dir.path().join(format!("file{i:02}.txt"));
        fs::write(&path, "α😀\r\nabcdef\n".repeat(250)).unwrap();
        service.dispatch(Command::OpenFile(path)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while service.is_busy() {
            service.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        ids.push(service.active.unwrap());
    }
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .with_pixels_per_point(scale)
        .with_step_dt(1. / 60.)
        .build_ui_state(
            |ui, a: &mut CodisseuApp| a.show(ui),
            CodisseuApp::new(service),
        );
    settle(&mut h);
    (dir, h, ids)
}
fn rect(h: &H, name: &str) -> Rect {
    h.get_by_label(name).rect()
}
fn order(h: &H, view: u64) -> Vec<u64> {
    h.state().service.view(view).unwrap().tabs().to_vec()
}
fn start(h: &mut H, from: Pos2, to: Pos2) {
    h.hover_at(from);
    h.step();
    h.drag_at(from);
    h.step();
    h.hover_at(to);
    h.step();
    h.step();
}
fn drop(h: &mut H, to: Pos2) {
    h.drop_at(to);
    settle(h);
}
fn capture(h: &mut H, scale: f32, name: &str) {
    if let Some(dir) = std::env::var_os("CODISSEU_TAB_REORDER_CAPTURES") {
        let dir = std::path::PathBuf::from(dir);
        fs::create_dir_all(&dir).unwrap();
        h.render()
            .unwrap()
            .save(dir.join(format!("{scale}-{name}.png")))
            .unwrap();
    }
}

#[test]
fn drag_extremes_middle_cancel_click_close_and_keyboard_preserve_dirty_editor() {
    for scale in [1., 1.25] {
        let (dir, mut h, ids) = fixture(4, scale);
        let view = h.state().service.active_view().unwrap();
        h.get_by_role(Role::MultilineTextInput).click();
        h.step();
        h.key_press_modifiers(Modifiers::CTRL, Key::Home);
        h.step();
        h.event(egui::Event::Text("// unsaved\n".into()));
        h.step();
        h.step();
        let doc = h.state().service.active_document().unwrap();
        let text = doc.text().to_owned();
        let version = doc.version;
        let selection = doc.selection();
        let scroll = h.state().service.view(view).unwrap().scroll();
        let metrics = h.state().editor_metrics().clone();
        let from = rect(&h, "file03.txt •").center();
        let first = rect(&h, "file00.txt");
        let to = Pos2::new(first.left() + 2., first.center().y);
        capture(&mut h, scale, "before");
        start(&mut h, from, to);
        assert_eq!(order(&h, view), ids, "only drop commits");
        capture(&mut h, scale, "dragging");
        drop(&mut h, to);
        assert_eq!(order(&h, view), [ids[3], ids[0], ids[1], ids[2]]);
        let doc = h.state().service.active_document().unwrap();
        assert_eq!(
            (doc.id, doc.version, doc.selection()),
            (ids[3], version, selection)
        );
        assert_eq!(doc.text(), text);
        assert_eq!(h.state().service.view(view).unwrap().scroll(), scroll);
        assert_eq!(
            h.state().editor_metrics().visible_start,
            metrics.visible_start
        );
        capture(&mut h, scale, "after");
        // Reverse direction to the last position, including the close area as
        // a drop target (it must not fire a close on pointer release).
        let from = rect(&h, "file03.txt •").center();
        let last = rect(&h, "Fechar file02.txt");
        let to = last.right_center();
        start(&mut h, from, to);
        drop(&mut h, to);
        assert_eq!(order(&h, view), ids);
        // Passive tab to the middle: the dirty document remains selected.
        let from = rect(&h, "file00.txt").center();
        let target = rect(&h, "file02.txt");
        let to = target.right_center();
        start(&mut h, from, to);
        drop(&mut h, to);
        assert_eq!(order(&h, view), [ids[1], ids[2], ids[0], ids[3]]);
        assert_eq!(h.state().service.active, Some(ids[3]));
        let before = order(&h, view);
        let from = rect(&h, "file03.txt •").center();
        let to = rect(&h, "file01.txt").left_center();
        start(&mut h, from, to);
        h.key_press(Key::Escape);
        h.step();
        drop(&mut h, to);
        assert_eq!(order(&h, view), before);
        start(&mut h, from, to);
        let outside = Pos2::new(to.x, to.y + 90.);
        h.hover_at(outside);
        h.step();
        drop(&mut h, outside);
        assert_eq!(order(&h, view), before);
        start(&mut h, from, to);
        h.event(egui::Event::PointerGone);
        h.step();
        drop(&mut h, outside);
        assert_eq!(order(&h, view), before);
        // The close target has its own gesture; dragging it is not tab motion.
        let close = rect(&h, "Fechar file01.txt").center();
        start(&mut h, close, from);
        drop(&mut h, from);
        assert_eq!(order(&h, view), before);
        assert_eq!(h.state().service.documents.len(), 4);
        // Context menu keyboard action is available without a mouse drag.
        h.get_by_label("file00.txt").focus();
        h.step();
        h.key_press_modifiers(Modifiers::SHIFT, Key::F10);
        h.step();
        h.step();
        h.get_by_label("Mover aba à esquerda").click();
        settle(&mut h);
        assert_eq!(order(&h, view), [ids[1], ids[0], ids[2], ids[3]]);
        h.get_by_label("file03.txt •").click();
        settle(&mut h);
        h.get_by_role(Role::MultilineTextInput).focus();
        h.step();
        h.key_press_modifiers(Modifiers::CTRL, Key::Z);
        h.step();
        h.step();
        assert!(!h.state().service.active_document().unwrap().is_dirty());
        h.key_press_modifiers(Modifiers::CTRL | Modifiers::SHIFT, Key::Z);
        h.step();
        h.step();
        assert_eq!(h.state().service.active_document().unwrap().text(), text);
        h.key_press_modifiers(Modifiers::CTRL, Key::S);
        settle(&mut h);
        assert_eq!(
            fs::read_to_string(dir.path().join("file03.txt")).unwrap(),
            text
        );
        h.get_by_label("Fechar file00.txt").click();
        settle(&mut h);
        assert_eq!(h.state().service.documents.len(), 3);
    }
}

#[test]
fn drag_overflow_autoscrolls_both_directions_and_does_not_scroll_editor() {
    for scale in [1., 1.25] {
        let (_dir, mut h, ids) = fixture(20, scale);
        let view = h.state().service.active_view().unwrap();
        let first = rect(&h, "file00.txt");
        let right = h.get_by_label("Mais ações do grupo").rect().left() - 4.;
        let edge = Pos2::new(right, first.center().y);
        let selection = h.state().service.active_document().unwrap().selection();
        let scroll = h.state().service.view(view).unwrap().scroll();
        start(&mut h, first.center(), edge);
        for _ in 0..360 {
            h.step();
        }
        capture(&mut h, scale, "overflow-right");
        assert!(
            rect(&h, "file19.txt").left() < edge.x,
            "hidden last tab must become visible"
        );
        drop(&mut h, edge);
        assert_eq!(order(&h, view).last(), Some(&ids[0]));
        let from = rect(&h, "file00.txt").center();
        let left = Pos2::new(first.left() + 2., first.center().y);
        start(&mut h, from, left);
        for _ in 0..360 {
            h.step();
        }
        capture(&mut h, scale, "overflow-left");
        drop(&mut h, left);
        assert_eq!(order(&h, view), ids);
        assert_eq!(h.state().service.view(view).unwrap().scroll(), scroll);
        assert_eq!(
            h.state().service.active_document().unwrap().selection(),
            selection
        );
    }
}

#[test]
fn passive_group_reorder_is_independent_and_cross_group_drop_cancels() {
    let (_dir, mut h, ids) = fixture(3, 1.);
    h.set_size(egui::vec2(1800., 900.));
    settle(&mut h);
    let left = h.state().service.active_view().unwrap();
    h.state_mut().service.dispatch(Command::CreateView).unwrap();
    settle(&mut h);
    let right = h.state().service.active_view().unwrap();
    let from = h
        .get_all_by_label("file02.txt")
        .next()
        .unwrap()
        .rect()
        .center();
    let to = rect(&h, "file00.txt").left_center();
    start(&mut h, from, to);
    drop(&mut h, to);
    assert_eq!(order(&h, left), [ids[2], ids[0], ids[1]]);
    assert_eq!(order(&h, right), [ids[2]]);
    assert_eq!(h.state().service.active_view(), Some(right));
    let from = rect(&h, "file01.txt").center();
    let to = h
        .get_all_by_label("file02.txt")
        .last()
        .unwrap()
        .rect()
        .center();
    start(&mut h, from, to);
    drop(&mut h, to);
    assert_eq!(order(&h, left), [ids[2], ids[0], ids[1]]);
    assert_eq!(order(&h, right), [ids[2]]);
    assert_eq!(h.state().service.active_view(), Some(right));
    assert_eq!(
        h.state().service.view(right).unwrap().selection(),
        Selection::default()
    );
}

#[test]
fn losing_window_focus_or_closing_drag_source_cancels_stale_gesture() {
    let (_dir, mut h, ids) = fixture(3, 1.);
    let view = h.state().service.active_view().unwrap();
    let from = rect(&h, "file01.txt").center();
    let to = rect(&h, "file00.txt").left_center();
    start(&mut h, from, to);
    h.input_mut().focused = false;
    h.event(egui::Event::WindowFocused(false));
    h.step();
    h.input_mut().focused = true;
    h.event(egui::Event::WindowFocused(true));
    h.step();
    drop(&mut h, to);
    assert_eq!(order(&h, view), ids);
    start(&mut h, from, to);
    h.state_mut()
        .service
        .dispatch(Command::CloseTab {
            view,
            id: ids[1],
            discard: false,
        })
        .unwrap();
    h.step();
    drop(&mut h, to);
    assert_eq!(order(&h, view), [ids[0], ids[2]]);
    assert_eq!(h.state().service.active, Some(ids[2]));
}
