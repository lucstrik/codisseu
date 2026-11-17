//! Explorer uses real asynchronous directory loading while widgets remain virtualized.
use codisseu_core::{AppService, Command};
use codisseu_ui::CodisseuApp;
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn settle(h: &mut Harness<'_, CodisseuApp>) {
    let until = Instant::now() + Duration::from_secs(15);
    loop {
        h.step();
        if !h.state().service.is_busy() {
            break;
        }
        assert!(Instant::now() < until, "{:?}", h.state().service.last_error);
        thread::sleep(Duration::from_millis(2));
    }
    h.run();
    assert!(
        h.state().service.last_error.is_none(),
        "{:?}",
        h.state().service.last_error
    );
}
fn capture(h: &mut Harness<'_, CodisseuApp>, name: &str) {
    if let Some(out) = std::env::var_os("CODISSEU_TREE_WIDGET_CAPTURES") {
        let out = std::path::PathBuf::from(out);
        fs::create_dir_all(&out).unwrap();
        h.render()
            .unwrap()
            .save(out.join(format!("{name}.png")))
            .unwrap();
    }
}
#[test]
fn expands_large_branch_opens_last_file_and_reveals_dependency_and_hidden_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    fs::create_dir(&root).unwrap();
    for folder in ["a-heavy", "vendor", "z-last"] {
        fs::create_dir(root.join(folder)).unwrap();
    }
    for n in 0..6001 {
        fs::write(
            root.join(format!("a-heavy/File{n:05}.php")),
            format!("<?php // own fixture {n}\n"),
        )
        .unwrap();
    }
    fs::write(
        root.join("vendor/autoload.php"),
        "<?php // dependency fixture\n",
    )
    .unwrap();
    fs::write(root.join(".env"), "OWN_FIXTURE=1\n").unwrap();
    fs::write(root.join("zz-last.php"), "<?php // last sibling\n").unwrap();
    let mut s = AppService::new(dir.path().join("session.json"));
    s.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    while s.is_busy() {
        s.poll();
        thread::sleep(Duration::from_millis(2));
    }
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .with_max_steps(40)
        .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), CodisseuApp::new(s));
    h.run();
    assert!(h.query_by_label("vendor").is_some());
    assert!(h.query_by_label(".env").is_some());
    assert!(h.query_by_label("zz-last.php").is_some());
    assert_eq!(h.state().service.entries.len(), 5);
    capture(&mut h, "all-root-children");
    h.get_by_label("a-heavy").click();
    settle(&mut h);
    assert!(
        h.state()
            .service
            .tree_directory_loaded(&root.join("a-heavy"))
    );
    assert!(h.query_by_label("File00000.php").is_some());
    assert!(h.query_by_label("File06000.php").is_none());
    let point = h.get_by_label("File00000.php").rect().center();
    h.hover_at(point);
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0., -1_000_000.),
        modifiers: egui::Modifiers::NONE,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
    });
    h.run();
    assert!(h.query_by_label("File06000.php").is_some());
    assert!(h.query_all_by_role(egui::accesskit::Role::TreeItem).count() < 40);
    h.get_by_label("File06000.php").click();
    settle(&mut h);
    assert_eq!(
        h.state().service.active_document().unwrap().path,
        root.join("a-heavy/File06000.php")
    );
    capture(&mut h, "last-file-after-scroll");
    h.hover_at(point);
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0., 1_000_000.),
        modifiers: egui::Modifiers::NONE,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
    });
    h.run();
    h.get_by_label("a-heavy").click();
    settle(&mut h);
    h.get_by_label("vendor").click();
    settle(&mut h);
    h.get_by_label("autoload.php").click();
    settle(&mut h);
    assert_eq!(
        h.state().service.active_document().unwrap().path,
        root.join("vendor/autoload.php")
    );
    capture(&mut h, "dependency-loaded");
    h.get_by_label(".env").click();
    settle(&mut h);
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        "OWN_FIXTURE=1\n"
    );
}
