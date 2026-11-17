use codisseu_core::{AppService, Command};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, Pos2};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
type H = Harness<'static, CodisseuApp>;

fn settle(h: &mut H) {
    let until = Instant::now() + Duration::from_secs(15);
    while h.state().service.is_busy() {
        h.step();
        thread::sleep(Duration::from_millis(2));
        assert!(Instant::now() < until);
    }
    for _ in 0..4 {
        h.step();
    }
}
fn setup(scale: f32) -> (tempfile::TempDir, PathBuf, H) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    fs::create_dir_all(root.join("folder/sub")).unwrap();
    fs::create_dir(root.join("destination")).unwrap();
    fs::write(root.join("alpha.txt"), "ação 🦀\r\noriginal").unwrap();
    fs::write(root.join("folder/sub/nested.txt"), "nested").unwrap();
    fs::write(root.join("destination/target.txt"), "target").unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service
        .dispatch(Command::SetAutoSave {
            enabled: false,
            delay_ms: 1000,
        })
        .unwrap();
    service
        .dispatch(Command::OpenWorkspace(root.clone()))
        .unwrap();
    let mut h = Harness::builder()
        .with_size([1280., 900.])
        .with_pixels_per_point(scale)
        .build_ui_state(
            |ui, app: &mut CodisseuApp| app.show(ui),
            CodisseuApp::new(service),
        );
    settle(&mut h);
    (dir, root, h)
}
fn point(h: &H, label: &str) -> Pos2 {
    h.get_all_by_label(label)
        .into_iter()
        .find(|n| n.rect().left() < 300. && n.rect().top() > 60.)
        .unwrap_or_else(|| panic!("missing Explorer row {label}"))
        .rect()
        .center()
}
fn start(h: &mut H, source: &str, target: &str) -> Pos2 {
    let a = point(h, source);
    let b = point(h, target);
    h.hover_at(a);
    h.step();
    h.drag_at(a);
    h.step();
    h.hover_at(a + egui::vec2(12., 0.));
    h.step();
    h.hover_at(b);
    h.step();
    h.step();
    b
}
fn drag(h: &mut H, source: &str, target: &str) {
    let b = start(h, source, target);
    h.drop_at(b);
    h.step();
    settle(h);
}
fn capture(h: &mut H, name: &str, scale: f32) {
    if let Some(out) = std::env::var_os("CODISSEU_DRAG_CAPTURES") {
        let out = PathBuf::from(out);
        fs::create_dir_all(&out).unwrap();
        h.render()
            .unwrap()
            .save(out.join(format!("{name}-{}.png", (scale * 100.) as u32)))
            .unwrap();
    }
}
#[test]
fn real_pointer_moves_file_into_folder_and_back_to_root_at_two_scales() {
    for scale in [1., 1.25] {
        let (_dir, root, mut h) = setup(scale);
        let original = fs::read(root.join("alpha.txt")).unwrap();
        start(&mut h, "alpha.txt", "destination");
        capture(&mut h, "valid-target", scale);
        let b = point(&h, "destination");
        h.drop_at(b);
        h.step();
        settle(&mut h);
        assert!(!root.join("alpha.txt").exists());
        assert_eq!(
            fs::read(root.join("destination/alpha.txt")).unwrap(),
            original
        );
        assert!(h.state().service.project_files.error.is_none());
        // Successful ACK expands the destination and exposes the moved row.
        drag(&mut h, "alpha.txt", "workspace");
        assert_eq!(fs::read(root.join("alpha.txt")).unwrap(), original);
        assert!(!root.join("destination/alpha.txt").exists());
    }
}
#[test]
fn folder_drop_on_file_uses_containing_directory_and_preserves_dirty_document_history() {
    let (_dir, root, mut h) = setup(1.);
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(root.join("folder/sub/nested.txt")))
        .unwrap();
    settle(&mut h);
    h.get_by_role(egui::accesskit::Role::MultilineTextInput)
        .focus();
    h.step();
    h.event(egui::Event::Text("dirty ".into()));
    h.step();
    let doc = h.state().service.active_document().unwrap();
    let id = doc.id;
    let text = doc.text().to_owned();
    // Reveal loads the parent chain; expand the target to expose its file.
    h.get_by_label("destination").click();
    h.step();
    settle(&mut h);
    drag(&mut h, "folder", "target.txt");
    assert!(!root.join("folder").exists());
    assert_eq!(
        fs::read_to_string(root.join("destination/folder/sub/nested.txt")).unwrap(),
        "nested"
    );
    let doc = h.state().service.active_document().unwrap();
    assert_eq!(doc.id, id);
    assert_eq!(doc.path, root.join("destination/folder/sub/nested.txt"));
    assert_eq!(doc.text(), text);
    assert!(doc.is_dirty());
    assert!(doc.can_undo());
    assert_eq!(
        fs::read_to_string(root.join("destination/target.txt")).unwrap(),
        "target"
    );
}
#[test]
fn collision_descendant_same_parent_escape_and_outside_drop_preserve_disk() {
    let (_dir, root, mut h) = setup(1.);
    fs::write(root.join("destination/alpha.txt"), "collision").unwrap();
    drag(&mut h, "alpha.txt", "destination");
    assert!(h.state().service.project_files.error.is_some());
    assert_eq!(
        fs::read_to_string(root.join("destination/alpha.txt")).unwrap(),
        "collision"
    );
    assert!(root.join("alpha.txt").exists());
    // Expand the ancestor without dragging first, then reject a recursive move.
    h.get_by_label("folder").click();
    h.step();
    settle(&mut h);
    start(&mut h, "folder", "sub");
    capture(&mut h, "invalid-descendant", 1.);
    let b = point(&h, "sub");
    h.drop_at(b);
    h.step();
    settle(&mut h);
    assert!(root.join("folder/sub/nested.txt").exists());
    let b = start(&mut h, "alpha.txt", "destination");
    h.key_press(Key::Escape);
    h.step();
    h.drop_at(b);
    h.step();
    settle(&mut h);
    assert!(root.join("alpha.txt").exists());
    let b = start(&mut h, "alpha.txt", "workspace");
    h.drop_at(b);
    h.step();
    settle(&mut h);
    assert!(root.join("alpha.txt").exists());
    start(&mut h, "alpha.txt", "destination");
    h.drop_at(egui::pos2(700., 700.));
    h.step();
    settle(&mut h);
    assert!(root.join("alpha.txt").exists());
}
#[test]
fn hovering_closed_folder_opens_it_and_click_still_opens_a_file() {
    let (_dir, root, mut h) = setup(1.25);
    start(&mut h, "alpha.txt", "destination");
    for _ in 0..60 {
        h.step();
        thread::sleep(Duration::from_millis(2));
    }
    settle(&mut h);
    point(&h, "target.txt");
    let b = point(&h, "destination");
    h.drop_at(b);
    h.step();
    settle(&mut h);
    h.get_all_by_label("alpha.txt")
        .into_iter()
        .find(|n| n.rect().left() < 300. && n.rect().top() > 60.)
        .unwrap()
        .click();
    h.step();
    settle(&mut h);
    assert_eq!(
        h.state().service.active_document().unwrap().path,
        root.join("destination/alpha.txt")
    );
    // A normal editor shortcut continues to work after the drag.
    h.key_press_modifiers(Modifiers::ALT, Key::Z);
    h.step();
}

#[test]
fn root_is_not_a_drag_source_and_workspace_switch_invalidates_payload() {
    let (dir, root, mut h) = setup(1.);
    drag(&mut h, "workspace", "destination");
    assert!(root.is_dir());
    assert!(!root.join("destination/workspace").exists());
    // Ensure the root is expanded after the cancelled root gesture.
    if h.query_by_label("alpha.txt").is_none() {
        let p = point(&h, "workspace");
        h.hover_at(p);
        h.step();
        h.drag_at(p);
        h.step();
        h.drop_at(p);
        h.step();
        settle(&mut h);
    }
    start(&mut h, "alpha.txt", "destination");
    let other = dir.path().join("other");
    fs::create_dir(&other).unwrap();
    h.state_mut()
        .service
        .dispatch(Command::OpenWorkspace(other.clone()))
        .unwrap();
    settle(&mut h);
    let target = point(&h, "other");
    h.hover_at(target);
    h.step();
    h.drop_at(target);
    h.step();
    settle(&mut h);
    assert!(root.join("alpha.txt").exists());
    assert!(!other.join("alpha.txt").exists());
}

#[test]
fn binary_file_moves_without_opening_or_decoding_the_source() {
    let (_dir, root, mut h) = setup(1.);
    let bytes = [0, 255, 128, 10, 13, 42];
    fs::write(root.join("image.bin"), bytes).unwrap();
    h.state_mut()
        .service
        .dispatch(Command::ProjectFile(
            codisseu_core::ProjectFileCommand::Refresh,
        ))
        .unwrap();
    settle(&mut h);
    drag(&mut h, "image.bin", "destination");
    assert!(!root.join("image.bin").exists());
    assert_eq!(fs::read(root.join("destination/image.bin")).unwrap(), bytes);
    assert!(h.state().service.documents.is_empty());
    assert!(h.state().service.last_error.is_none());
}

#[test]
fn edge_scroll_reaches_a_virtualized_target_and_drop_on_file_moves_to_its_parent() {
    let (_dir, root, mut h) = setup(1.);
    fs::write(root.join("folder/from.txt"), "scroll source").unwrap();
    for index in 0..100 {
        fs::write(root.join(format!("row{index:03}.txt")), "target").unwrap();
    }
    h.state_mut()
        .service
        .dispatch(Command::ProjectFile(
            codisseu_core::ProjectFileCommand::Refresh,
        ))
        .unwrap();
    settle(&mut h);
    h.get_by_label("folder").click();
    h.step();
    settle(&mut h);
    start(&mut h, "from.txt", "row001.txt");
    h.hover_at(egui::pos2(150., 850.));
    h.step();
    // At maximum scroll the final row is flush with the bottom. Wait until it
    // reaches the held edge pointer, then verify the actual drop on disk.
    let mut reached = false;
    for _ in 0..350 {
        h.step();
        if h.query_by_label("row099.txt")
            .is_some_and(|n| n.rect().contains(egui::pos2(150., 850.)))
        {
            reached = true;
            break;
        }
    }
    capture(&mut h, "edge-scroll", 1.);
    let visible: Vec<_> = (0..100)
        .filter(|i| h.query_by_label(&format!("row{i:03}.txt")).is_some())
        .collect();
    assert!(
        reached,
        "edge scroll must construct the distant target row: {visible:?}"
    );
    let target = point(&h, "row099.txt");
    h.hover_at(target);
    h.step();
    h.drop_at(target);
    h.step();
    settle(&mut h);
    assert!(!root.join("folder/from.txt").exists());
    assert_eq!(
        fs::read_to_string(root.join("from.txt")).unwrap(),
        "scroll source"
    );
    assert_eq!(
        fs::read_to_string(root.join("row099.txt")).unwrap(),
        "target"
    );
}
