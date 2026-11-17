use codisseu_core::{AppService, Command};
use codisseu_ui::CodisseuApp;
use egui::{Key, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
type H = Harness<'static, CodisseuApp>;
fn settle(h: &mut H) {
    let end = Instant::now() + Duration::from_secs(15);
    for _ in 0..8 {
        h.step();
    }
    while h.state().service.is_busy() {
        h.step();
        thread::sleep(Duration::from_millis(2));
        assert!(Instant::now() < end);
    }
    for _ in 0..5 {
        h.step();
    }
}
fn text_x(h: &H, name: &str) -> f32 {
    fn find(s: &egui::Shape, n: &str) -> Option<f32> {
        match s {
            egui::Shape::Text(t) if t.galley.job.text == n => Some(t.pos.x),
            egui::Shape::Vec(v) => v.iter().find_map(|s| find(s, n)),
            _ => None,
        }
    }
    h.output()
        .shapes
        .iter()
        .find_map(|s| find(&s.shape, name))
        .unwrap_or_else(|| panic!("no painted row {name}"))
}
fn capture(h: &mut H, name: &str, scale: f32) {
    if let Some(out) = std::env::var_os("CODISSEU_TREE_ALIGN_CAPTURES") {
        let out = PathBuf::from(out);
        fs::create_dir_all(&out).unwrap();
        h.render()
            .unwrap()
            .save(out.join(format!("{name}-{}.png", (scale * 100.) as u32)))
            .unwrap();
    }
}
#[test]
fn nested_creation_aligns_immediately_and_is_stable_after_collapse() {
    for (scale, width) in [(1., 150.), (1.25, 150.), (1., 240.), (1.25, 240.)] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("empty");
        fs::create_dir(&root).unwrap();
        let mut s = AppService::new(dir.path().join("session.json"));
        s.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
        let mut layout = s.shell_layout();
        layout.left_width = Some(width);
        s.dispatch(Command::SetShellLayout(layout)).unwrap();
        let mut h = Harness::builder()
            .with_size([1280., 900.])
            .with_pixels_per_point(scale)
            .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), CodisseuApp::new(s));
        settle(&mut h);
        let mut path = root.clone();
        let mut before = Vec::<f32>::new();
        for (depth, name) in ["RootAlpha", "NestedBeta", "ThirdGamma"].iter().enumerate() {
            h.get_by_label("New Folder…").click();
            h.step();
            h.step();
            h.get_by_role_and_label(Role::TextInput, "Folder name")
                .focus();
            h.step();
            h.event(egui::Event::Text((*name).into()));
            h.step();
            capture(&mut h, &format!("typing-{width}-{depth}"), scale);
            for (index, prior) in ["RootAlpha", "NestedBeta", "ThirdGamma"]
                .iter()
                .take(depth)
                .enumerate()
            {
                assert!(
                    (text_x(&h, prior) - before[index]).abs() < 0.5,
                    "{prior} shifted while typing child"
                );
            }
            h.key_press(Key::Enter);
            h.step();
            settle(&mut h);
            path = path.join(name);
            assert!(path.is_dir());
            assert_eq!(
                h.state()
                    .service
                    .entries
                    .iter()
                    .find(|e| e.path == path)
                    .unwrap()
                    .depth,
                depth
            );
            let x = text_x(&h, name);
            if let Some(prev) = before.last() {
                assert!(
                    (x - prev - 16.).abs() < 0.5,
                    "depth{depth}: x{x} previous{prev}"
                );
            }
            before.push(x);
            capture(&mut h, &format!("created-{width}-{depth}"), scale);
        }
        h.get_by_label("RootAlpha").click();
        h.step();
        h.step();
        h.get_by_label("RootAlpha").click();
        h.step();
        settle(&mut h);
        for (name, x) in ["RootAlpha", "NestedBeta", "ThirdGamma"].iter().zip(before) {
            assert!(
                (text_x(&h, name) - x).abs() < 0.5,
                "{name} changed after collapse"
            );
        }
        capture(&mut h, "reopened", scale);
    }
}

#[test]
fn file_creation_and_rename_keep_the_parent_column_in_a_narrow_panel() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("empty");
    fs::create_dir(&root).unwrap();
    let mut s = AppService::new(dir.path().join("session.json"));
    s.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    let mut layout = s.shell_layout();
    layout.left_width = Some(150.);
    s.dispatch(Command::SetShellLayout(layout)).unwrap();
    let mut h = Harness::builder()
        .with_size([1280., 900.])
        .with_pixels_per_point(1.25)
        .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), CodisseuApp::new(s));
    settle(&mut h);
    h.get_by_label("New Folder…").click();
    h.step();
    h.step();
    h.event(egui::Event::Text("RootAlpha".into()));
    h.step();
    h.key_press(Key::Enter);
    h.step();
    settle(&mut h);
    let parent = text_x(&h, "RootAlpha");
    h.get_by_label("New File…").click();
    h.step();
    h.step();
    h.event(egui::Event::Text("small.txt".into()));
    h.step();
    h.key_press(Key::Enter);
    h.step();
    settle(&mut h);
    capture(&mut h, "created-file", 1.25);
    assert!(
        (text_x(&h, "RootAlpha") - parent).abs() < 0.5,
        "parent shifted after creating file"
    );
    assert!(root.join("RootAlpha/small.txt").is_file());
    h.get_all_by_label("small.txt")
        .into_iter()
        .find(|n| n.rect().left() < 300.)
        .unwrap()
        .click();
    h.step();
    h.step();
    h.key_press(Key::F2);
    h.step();
    h.step();
    h.key_press_modifiers(
        egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        },
        Key::A,
    );
    h.step();
    h.event(egui::Event::Text("renamed.md".into()));
    h.step();
    assert!(
        (text_x(&h, "RootAlpha") - parent).abs() < 0.5,
        "parent shifted during rename"
    );
    h.key_press(Key::Enter);
    h.step();
    settle(&mut h);
    assert!(root.join("RootAlpha/renamed.md").is_file());
    assert!(
        (text_x(&h, "RootAlpha") - parent).abs() < 0.5,
        "parent shifted after rename"
    );
}

#[test]
fn wide_sibling_does_not_pan_the_tree_when_creating_a_short_child() {
    for scale in [1., 1.25] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("empty");
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("Host")).unwrap();
        fs::create_dir(
            root.join(
                "Z sibling with a deliberately very long folder name to widen the cached tree",
            ),
        )
        .unwrap();
        let mut s = AppService::new(dir.path().join("session.json"));
        s.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
        let mut h = Harness::builder()
            .with_size([1280., 900.])
            .with_pixels_per_point(scale)
            .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), CodisseuApp::new(s));
        settle(&mut h);
        let rect = h.get_by_label("Host").rect();
        let hit = egui::pos2(rect.left() + 70., rect.center().y);
        h.hover_at(hit);
        h.step();
        h.drag_at(hit);
        h.step();
        h.drop_at(hit);
        h.step();
        settle(&mut h);
        let x = text_x(&h, "Host");
        h.get_by_label("New Folder…").click();
        h.step();
        h.step();
        h.event(egui::Event::Text("Child".into()));
        h.step();
        assert!(
            (text_x(&h, "Host") - x).abs() < 0.5,
            "wide sibling moved parent while typing"
        );
        h.key_press(Key::Enter);
        h.step();
        settle(&mut h);
        capture(&mut h, "wide-sibling", scale);
        assert!(root.join("Host/Child").is_dir());
        assert!((text_x(&h, "Host") - x).abs() < 0.5);
        assert!((text_x(&h, "Child") - x - 16.).abs() < 0.5);
    }
}
