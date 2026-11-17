//! Explorer selection is presentation state; deletion still uses the real worker.
use codisseu_core::{AppService, Command};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness, Node,
    kittest::{NodeT, Queryable},
};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
type H = Harness<'static, CodisseuApp>;

fn settle(h: &mut H) {
    h.step();
    let until = Instant::now() + Duration::from_secs(15);
    while h.state().service.is_busy() {
        assert!(Instant::now() < until, "Explorer worker did not settle");
        thread::sleep(Duration::from_millis(2));
        h.step();
    }
    h.run_steps(6);
}
fn files(root: &Path) {
    fs::create_dir_all(root.join("FolderA")).unwrap();
    fs::create_dir_all(root.join("FolderB")).unwrap();
    fs::write(root.join("FolderA/child.txt"), "folder child 🦀\n").unwrap();
    fs::write(root.join("alpha.txt"), "alpha saved\n").unwrap();
    fs::write(root.join("bravo.txt"), "bravo saved\n").unwrap();
    fs::write(root.join("charlie.txt"), "charlie saved\n").unwrap();
    fs::write(root.join("keep.txt"), "keep saved\n").unwrap();
}
fn open(root: &Path, session: PathBuf, scale: f32) -> H {
    let mut service = AppService::new(session);
    service
        .dispatch(Command::SetAutoSave {
            enabled: false,
            delay_ms: 1000,
        })
        .unwrap();
    service
        .dispatch(Command::OpenWorkspace(root.to_path_buf()))
        .unwrap();
    service
        .dispatch(Command::OpenFile(root.join("keep.txt")))
        .unwrap();
    let mut h = Harness::builder()
        .with_size([1280., 900.])
        .with_pixels_per_point(scale)
        .with_max_steps(60)
        .with_step_dt(1. / 60.)
        .build_eframe(|_| CodisseuApp::new(service));
    settle(&mut h);
    h
}
fn fixture(scale: f32) -> (tempfile::TempDir, PathBuf, H) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    files(&root);
    let h = open(&root, dir.path().join("profile/session.json"), scale);
    (dir, root, h)
}
fn row<'a>(h: &'a H, label: &'a str) -> Node<'a> {
    h.get_all_by_label(label)
        .into_iter()
        .find(|n| n.rect().left() < 285. && n.rect().top() > 65.)
        .unwrap_or_else(|| panic!("missing Explorer row {label}"))
}
fn select(h: &mut H, name: &str, mut modifiers: Modifiers) {
    modifiers.command = modifiers.ctrl;
    row(h, name).click_modifiers(modifiers);
    settle(h);
}
fn selected(h: &H, name: &str) -> bool {
    row(h, name).accesskit_node().is_selected().unwrap_or(false)
}
fn expect_selection(h: &H, names: &[&str]) {
    for name in [
        "FolderA",
        "FolderB",
        "alpha.txt",
        "bravo.txt",
        "charlie.txt",
        "keep.txt",
    ] {
        assert_eq!(selected(h, name), names.contains(&name), "row {name}");
    }
}
fn key(h: &mut H, mut modifiers: Modifiers, key: Key) {
    modifiers.command = modifiers.ctrl;
    h.key_press_modifiers(modifiers, key);
    settle(h);
}
fn capture(h: &mut H, name: &str, scale: f32) {
    if let Some(out) = std::env::var_os("CODISSEU_EXPLORER_MULTI_CAPTURES") {
        let out = PathBuf::from(out);
        fs::create_dir_all(&out).unwrap();
        h.render()
            .unwrap()
            .save(out.join(format!("{name}-{}.png", (scale * 100.) as u32)))
            .unwrap();
    }
}

#[test]
fn ctrl_click_toggles_mixed_rows_without_opening_or_expanding_them() {
    for scale in [1., 1.25] {
        let (_dir, root, mut h) = fixture(scale);
        select(&mut h, "alpha.txt", Modifiers::NONE);
        let id = h.state().service.active_document().unwrap().id;
        select(&mut h, "FolderA", Modifiers::CTRL);
        select(&mut h, "bravo.txt", Modifiers::CTRL);
        expect_selection(&h, &["alpha.txt", "FolderA", "bravo.txt"]);
        assert_eq!(h.state().service.active_document().unwrap().id, id);
        assert_eq!(h.state().service.documents.len(), 2);
        assert!(h.query_by_label("child.txt").is_none());
        capture(&mut h, "mixed-selection", scale);
        select(&mut h, "alpha.txt", Modifiers::CTRL);
        expect_selection(&h, &["FolderA", "bravo.txt"]);
        assert_eq!(h.state().service.active_document().unwrap().id, id);
        select(&mut h, "bravo.txt", Modifiers::NONE);
        expect_selection(&h, &["bravo.txt"]);
        assert_eq!(
            h.state().service.active_document().unwrap().path,
            root.join("bravo.txt")
        );
    }
}

#[test]
fn shift_range_and_ctrl_a_follow_visible_rows_and_exclude_workspace_root() {
    for scale in [1., 1.25] {
        let (_dir, _root, mut h) = fixture(scale);
        select(&mut h, "alpha.txt", Modifiers::NONE);
        select(&mut h, "charlie.txt", Modifiers::SHIFT);
        expect_selection(&h, &["alpha.txt", "bravo.txt", "charlie.txt"]);
        select(&mut h, "FolderA", Modifiers::CTRL);
        select(&mut h, "FolderB", Modifiers::CTRL.plus(Modifiers::SHIFT));
        expect_selection(
            &h,
            &[
                "FolderA",
                "FolderB",
                "alpha.txt",
                "bravo.txt",
                "charlie.txt",
            ],
        );
        select(&mut h, "FolderA", Modifiers::NONE);
        assert!(h.query_by_label("child.txt").is_some());
        key(&mut h, Modifiers::CTRL, Key::A);
        expect_selection(
            &h,
            &[
                "FolderA",
                "FolderB",
                "alpha.txt",
                "bravo.txt",
                "charlie.txt",
                "keep.txt",
            ],
        );
        assert!(selected(&h, "child.txt"));
        assert!(!selected(&h, "workspace"));
        capture(&mut h, "visible-select-all", scale);
        key(&mut h, Modifiers::NONE, Key::Delete);
        // FolderA already covers child.txt; review includes it only once.
        h.get_by_label("Move 6 items to Trash?");
        capture(&mut h, "parent-child-review", scale);
        h.get_by_label("Cancel").click();
        settle(&mut h);
        select(&mut h, "alpha.txt", Modifiers::NONE);
        select(&mut h, "charlie.txt", Modifiers::CTRL);
        select(&mut h, "charlie.txt", Modifiers::CTRL);
        expect_selection(&h, &["alpha.txt"]);
        select(&mut h, "keep.txt", Modifiers::SHIFT);
        expect_selection(&h, &["alpha.txt", "charlie.txt", "keep.txt"]);
        select(&mut h, "charlie.txt", Modifiers::SHIFT);
        expect_selection(&h, &["alpha.txt", "charlie.txt"]);
    }
}

#[test]
fn batch_delete_cancel_and_escape_preserve_sources_and_selection() {
    let (_dir, root, mut h) = fixture(1.25);
    select(&mut h, "alpha.txt", Modifiers::NONE);
    select(&mut h, "FolderA", Modifiers::CTRL);
    select(&mut h, "bravo.txt", Modifiers::CTRL);
    key(&mut h, Modifiers::NONE, Key::Delete);
    h.get_by_label("Move 3 items to Trash?");
    assert!(
        !h.get_by_label("Move to Trash")
            .accesskit_node()
            .is_disabled()
    );
    capture(&mut h, "batch-confirm", 1.25);
    h.get_by_label("Cancel").click();
    settle(&mut h);
    expect_selection(&h, &["alpha.txt", "FolderA", "bravo.txt"]);
    key(&mut h, Modifiers::NONE, Key::Delete);
    h.get_by_label("Move 3 items to Trash?");
    key(&mut h, Modifiers::NONE, Key::Escape);
    assert!(h.query_by_label("Move to Trash").is_none());
    assert_eq!(
        fs::read_to_string(root.join("alpha.txt")).unwrap(),
        "alpha saved\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("bravo.txt")).unwrap(),
        "bravo saved\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("FolderA/child.txt")).unwrap(),
        "folder child 🦀\n"
    );
}

#[test]
fn collapse_chevron_focuses_parent_but_preserves_hidden_child_batch_for_delete() {
    for scale in [1., 1.25] {
        let (_dir, root, mut h) = fixture(scale);
        select(&mut h, "FolderA", Modifiers::NONE);
        select(&mut h, "child.txt", Modifiers::NONE);
        select(&mut h, "bravo.txt", Modifiers::CTRL);
        assert!(selected(&h, "child.txt") && selected(&h, "bravo.txt"));
        assert!(!selected(&h, "FolderA"));
        let doc = h.state().service.active_document().unwrap();
        let (id, version) = (doc.id, doc.version);
        let rect = row(&h, "FolderA").rect();
        let chevron = egui::pos2(rect.left() + 24. + 16. + 4., rect.center().y);
        h.event(egui::Event::PointerMoved(chevron));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: chevron,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        settle(&mut h);
        assert!(
            !h.get_all_by_label("child.txt")
                .into_iter()
                .any(|n| { n.rect().left() < 285. && n.rect().top() > 65. })
        );
        assert!(!selected(&h, "FolderA"));
        assert!(selected(&h, "bravo.txt"));
        assert!(row(&h, "FolderA").is_focused());
        let doc = h.state().service.active_document().unwrap();
        assert_eq!((doc.id, doc.version), (id, version));
        key(&mut h, Modifiers::NONE, Key::Delete);
        h.get_by_label("Move 2 items to Trash?");
        capture(&mut h, "hidden-child-review", scale);
        h.get_by_label("Cancel").click();
        settle(&mut h);
        h.event(egui::Event::PointerMoved(chevron));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: chevron,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        settle(&mut h);
        assert!(selected(&h, "child.txt") && selected(&h, "bravo.txt"));
        assert!(!selected(&h, "FolderA"));
        assert_eq!(
            fs::read_to_string(root.join("FolderA/child.txt")).unwrap(),
            "folder child 🦀\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("bravo.txt")).unwrap(),
            "bravo saved\n"
        );
    }
}

#[test]
fn context_on_selected_row_keeps_batch_and_unselected_row_replaces_it() {
    let (_dir, _root, mut h) = fixture(1.);
    select(&mut h, "alpha.txt", Modifiers::NONE);
    select(&mut h, "bravo.txt", Modifiers::CTRL);
    row(&h, "alpha.txt").click_secondary();
    settle(&mut h);
    expect_selection(&h, &["alpha.txt", "bravo.txt"]);
    h.get_by_label("Delete").click();
    settle(&mut h);
    h.get_by_label("Move 2 items to Trash?");
    h.get_by_label("Cancel").click();
    settle(&mut h);
    row(&h, "charlie.txt").click_secondary();
    settle(&mut h);
    expect_selection(&h, &["charlie.txt"]);
    h.get_by_label("Delete").click();
    settle(&mut h);
    h.get_by_label("Move ‘charlie.txt’ to Trash?");
    h.get_by_label("Cancel").click();
    settle(&mut h);
}

#[test]
fn one_dirty_descendant_disables_the_entire_batch_without_losing_buffer_or_undo() {
    let (_dir, root, mut h) = fixture(1.);
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(root.join("FolderA/child.txt")))
        .unwrap();
    settle(&mut h);
    let doc = h.state().service.active_document().unwrap();
    let (id, expected_version) = (doc.id, doc.version);
    h.state_mut()
        .service
        .dispatch(Command::Edit {
            id,
            expected_version,
            text: "dirty descendant 🦀\n".into(),
        })
        .unwrap();
    settle(&mut h);
    select(&mut h, "alpha.txt", Modifiers::NONE);
    select(&mut h, "FolderA", Modifiers::CTRL);
    key(&mut h, Modifiers::NONE, Key::Delete);
    h.get_by_label("Move 2 items to Trash?");
    assert!(
        h.get_by_label("Move to Trash")
            .accesskit_node()
            .is_disabled()
    );
    h.get_by_label("Move to Trash").click();
    settle(&mut h);
    assert!(root.join("alpha.txt").is_file());
    let doc = h
        .state()
        .service
        .documents
        .iter()
        .find(|d| d.id == id)
        .unwrap();
    assert_eq!(doc.text(), "dirty descendant 🦀\n");
    assert!(doc.is_dirty() && doc.can_undo());
    assert_eq!(
        fs::read_to_string(root.join("FolderA/child.txt")).unwrap(),
        "folder child 🦀\n"
    );
    h.get_by_label("Cancel").click();
    settle(&mut h);
}

#[test]
fn delete_in_editor_does_not_trash_the_explorer_selection() {
    let (_dir, root, mut h) = fixture(1.);
    select(&mut h, "alpha.txt", Modifiers::NONE);
    select(&mut h, "bravo.txt", Modifiers::CTRL);
    h.get_by_role(Role::MultilineTextInput).focus();
    settle(&mut h);
    key(&mut h, Modifiers::NONE, Key::Delete);
    assert!(h.query_by_label("Move to Trash").is_none());
    assert_eq!(
        fs::read_to_string(root.join("alpha.txt")).unwrap(),
        "alpha saved\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("bravo.txt")).unwrap(),
        "bravo saved\n"
    );
}

#[test]
fn deselecting_the_last_row_keeps_delete_owned_by_the_empty_explorer_selection() {
    let (_dir, root, mut h) = fixture(1.);
    select(&mut h, "alpha.txt", Modifiers::NONE);
    let doc = h.state().service.active_document().unwrap();
    let (id, version, text) = (doc.id, doc.version, doc.text().to_owned());
    select(&mut h, "alpha.txt", Modifiers::CTRL);
    expect_selection(&h, &[]);
    key(&mut h, Modifiers::NONE, Key::Delete);
    assert!(h.query_by_label("Move to Trash").is_none());
    let doc = h
        .state()
        .service
        .documents
        .iter()
        .find(|d| d.id == id)
        .unwrap();
    assert_eq!((doc.version, doc.text()), (version, text.as_str()));
    assert_eq!(
        fs::read_to_string(root.join("alpha.txt")).unwrap(),
        "alpha saved\n"
    );
}

#[test]
fn active_document_ack_cannot_replace_modified_single_or_empty_explorer_selection() {
    let (_dir, root, mut h) = fixture(1.);
    let keep = h.state().service.active_document().unwrap().id;
    select(&mut h, "alpha.txt", Modifiers::NONE);
    let alpha = h.state().service.active_document().unwrap().id;
    select(&mut h, "bravo.txt", Modifiers::CTRL);
    select(&mut h, "alpha.txt", Modifiers::CTRL);
    expect_selection(&h, &["bravo.txt"]);
    // Model an already-issued OpenFile result activating another existing
    // document after the user has changed the Explorer selection.
    h.state_mut()
        .service
        .dispatch(Command::Activate { id: keep })
        .unwrap();
    settle(&mut h);
    assert_eq!(h.state().service.active_document().unwrap().id, keep);
    expect_selection(&h, &["bravo.txt"]);
    assert!(row(&h, "alpha.txt").is_focused());
    select(&mut h, "bravo.txt", Modifiers::CTRL);
    expect_selection(&h, &[]);
    h.state_mut()
        .service
        .dispatch(Command::Activate { id: alpha })
        .unwrap();
    settle(&mut h);
    expect_selection(&h, &[]);
    let doc = h.state().service.active_document().unwrap();
    let version = doc.version;
    assert_eq!(doc.id, alpha);
    key(&mut h, Modifiers::NONE, Key::Delete);
    assert!(h.query_by_label("Move to Trash").is_none());
    assert_eq!(
        h.state().service.active_document().unwrap().version,
        version
    );
    assert_eq!(
        fs::read_to_string(root.join("alpha.txt")).unwrap(),
        "alpha saved\n"
    );
}

#[test]
fn pointer_click_uses_its_modifier_snapshot_when_control_is_released_in_the_same_frame() {
    let (_dir, _root, mut h) = fixture(1.);
    select(&mut h, "alpha.txt", Modifiers::NONE);
    let active = h.state().service.active_document().unwrap().id;
    let coalesced_control_click = |h: &mut H, name: &str| {
        let pos = row(h, name).rect().center();
        let modifiers = Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        // Real winit input can deliver all of these before one paint. Unlike
        // the normal kittest event queue, RawInput exercises a single frame.
        h.input_mut().events.extend([
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers,
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers,
            },
            egui::Event::ModifiersChanged(Modifiers::NONE),
        ]);
        h.step();
        settle(h);
    };
    coalesced_control_click(&mut h, "bravo.txt");
    expect_selection(&h, &["alpha.txt", "bravo.txt"]);
    assert_eq!(h.state().service.active_document().unwrap().id, active);
    coalesced_control_click(&mut h, "FolderA");
    expect_selection(&h, &["alpha.txt", "bravo.txt", "FolderA"]);
    assert!(h.query_by_label("child.txt").is_none());
    coalesced_control_click(&mut h, "alpha.txt");
    expect_selection(&h, &["bravo.txt", "FolderA"]);
    assert_eq!(h.state().service.active_document().unwrap().id, active);
    assert_eq!(h.state().service.documents.len(), 2);
}

#[test]
fn switching_workspace_clears_old_selection_even_for_equal_names() {
    let (dir, root, mut h) = fixture(1.);
    select(&mut h, "alpha.txt", Modifiers::NONE);
    select(&mut h, "bravo.txt", Modifiers::CTRL);
    let other = dir.path().join("other");
    files(&other);
    h.state_mut()
        .service
        .dispatch(Command::OpenWorkspace(other.clone()))
        .unwrap();
    settle(&mut h);
    select(&mut h, "charlie.txt", Modifiers::NONE);
    expect_selection(&h, &["charlie.txt"]);
    key(&mut h, Modifiers::NONE, Key::Delete);
    h.get_by_label("Move ‘charlie.txt’ to Trash?");
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert!(root.join("alpha.txt").is_file() && root.join("bravo.txt").is_file());
}

#[cfg(target_os = "linux")]
#[test]
fn selected_files_and_folder_go_to_private_trash_and_close_only_their_clean_documents() {
    const CHILD: &str = "CODISSEU_EXPLORER_MULTI_TRASH_CHILD";
    let Some(base) = std::env::var_os(CHILD) else {
        let owned = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts");
        fs::create_dir_all(&owned).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("codisseu-explorer-multi-trash-")
            .tempdir_in(fs::canonicalize(owned).unwrap())
            .unwrap();
        files(&dir.path().join("workspace"));
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "selected_files_and_folder_go_to_private_trash_and_close_only_their_clean_documents",
                "--nocapture",
            ])
            .env(CHILD, dir.path())
            .env("XDG_DATA_HOME", dir.path().join("data"))
            .env("XDG_CONFIG_HOME", dir.path().join("config"))
            .env("XDG_CACHE_HOME", dir.path().join("cache"))
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            fs::read_to_string(dir.path().join("data/Trash/files/alpha.txt")).unwrap(),
            "alpha saved\n"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("data/Trash/files/FolderA/child.txt")).unwrap(),
            "folder child 🦀\n"
        );
        assert!(
            dir.path()
                .join("data/Trash/info/alpha.txt.trashinfo")
                .is_file()
        );
        return;
    };
    let base = PathBuf::from(base);
    let root = base.join("workspace");
    let mut h = open(&root, base.join("session.json"), 1.);
    for path in [
        root.join("alpha.txt"),
        root.join("FolderA/child.txt"),
        root.join("keep.txt"),
    ] {
        h.state_mut()
            .service
            .dispatch(Command::OpenFile(path))
            .unwrap();
        settle(&mut h);
    }
    select(&mut h, "alpha.txt", Modifiers::NONE);
    select(&mut h, "FolderA", Modifiers::CTRL);
    key(&mut h, Modifiers::NONE, Key::Delete);
    h.get_by_label("Move 2 items to Trash?");
    let confirm = h.get_by_label("Move to Trash");
    assert!(!confirm.accesskit_node().is_disabled());
    assert!(confirm.is_focused());
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert!(
        h.state().service.project_files.error.is_none(),
        "{:?}",
        h.state().service.project_files.error
    );
    assert!(!root.join("alpha.txt").exists() && !root.join("FolderA").exists());
    assert_eq!(h.state().service.documents.len(), 1);
    assert_eq!(h.state().service.documents[0].path, root.join("keep.txt"));
    assert_eq!(
        fs::read_to_string(root.join("bravo.txt")).unwrap(),
        "bravo saved\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("keep.txt")).unwrap(),
        "keep saved\n"
    );
}
