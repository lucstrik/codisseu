//! Repeatable whole-frame CPU probe; no renderer or business source code.
use codisseu_core::{AppService, Command, TreeEntry};
use codisseu_ui::CodisseuApp;
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

#[test]
#[ignore = "CPU benchmark: run alone with --nocapture"]
fn large_project_scroll_frames() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let path = root.join("controller.php");
    fs::write(
        &path,
        format!(
            "<?php\n{}",
            "// Controller line with Unicode ação 🌧\n".repeat(4000)
        ),
    )
    .unwrap();
    let mut service = AppService::new(root.join("profile.json"));
    service.dispatch(Command::OpenFile(path)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    service.workspace = Some(root.clone());
    service.entries = (0..5000)
        .map(|i| TreeEntry {
            path: root.join(format!("Controller{i:04}.php")),
            is_dir: false,
            depth: 0,
        })
        .collect();
    let timing = std::rc::Rc::new(std::cell::Cell::new(0u128));
    let measure = timing.clone();
    let mut h = Harness::builder().with_size([1280., 800.]).build_ui_state(
        move |ui, a: &mut CodisseuApp| {
            let start = Instant::now();
            a.show(ui);
            measure.set(start.elapsed().as_micros());
        },
        CodisseuApp::new(service),
    );
    for _ in 0..30 {
        h.step();
    }
    let editor = h
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .rect()
        .center();
    h.hover_at(editor);
    let mut frames = vec![];
    let mut editors = vec![];
    let mut ui_frames = vec![];
    for _ in 0..120 {
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            delta: egui::vec2(0., -50.),
            modifiers: egui::Modifiers::NONE,
        });
        let start = Instant::now();
        h.step();
        frames.push(start.elapsed().as_micros());
        ui_frames.push(timing.get());
        editors.push(h.state().editor_metrics().frame_micros);
    }
    ui_frames.sort();
    frames.sort();
    editors.sort();
    println!(
        "{}",
        serde_json::json!({"fixture":"5000 root entries, 4000 PHP lines", "samples":frames.len(),"ui_show_us":{"p50":ui_frames[60],"p95":ui_frames[114],"max":ui_frames[119]},"whole_frame_us":{"p50":frames[60],"p95":frames[114],"max":frames[119]},"editor_us":{"p50":editors[60],"p95":editors[114],"max":editors[119]},"visible_start":h.state().editor_metrics().visible_start})
    );
    assert!(h.state().editor_metrics().visible_start > 100);
}

#[test]
fn project_tree_only_exposes_viewport_rows_and_opens_last_file_after_scroll() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir(&root).unwrap();
    for i in 0..1000 {
        fs::write(root.join(format!("File{i:04}.php")), "<?php // own fixture").unwrap();
    }
    let mut s = AppService::new(temp.path().join("session.json"));
    s.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while s.is_busy() {
        s.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), CodisseuApp::new(s));
    h.step();
    h.step();
    let point = h.get_by_label("File0000.php").rect().center();
    assert!(h.query_by_label("File0999.php").is_none());
    h.hover_at(point);
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        delta: egui::vec2(0., -30000.),
        modifiers: egui::Modifiers::NONE,
    });
    for _ in 0..60 {
        h.step();
    }
    assert!(h.query_by_label("File0000.php").is_none());
    h.get_by_label("File0999.php").click();
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().service.active_document().is_none() {
        h.step();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        h.state().service.active_document().unwrap().path,
        root.join("File0999.php")
    );
    assert!(!h.state().service.active_document().unwrap().is_dirty());
}

#[test]
#[ignore = "explicit read-only project probe; CODISSEU_SCROLL_PROJECT and CODISSEU_SCROLL_FILE"]
fn readonly_project_scroll_frames() {
    let root =
        std::path::PathBuf::from(std::env::var_os("CODISSEU_SCROLL_PROJECT").expect("project"))
            .canonicalize()
            .unwrap();
    let path = std::path::PathBuf::from(std::env::var_os("CODISSEU_SCROLL_FILE").expect("file"))
        .canonicalize()
        .unwrap();
    assert!(path.starts_with(&root));
    let original = fs::read(&path).unwrap();
    let profile = tempfile::tempdir().unwrap();
    let mut s = AppService::new(profile.path().join("session.json"));
    for command in [
        Command::OpenWorkspace(root),
        Command::OpenFile(path.clone()),
    ] {
        s.dispatch(command).unwrap();
        let end = Instant::now() + Duration::from_secs(20);
        while s.is_busy() {
            s.poll();
            assert!(Instant::now() < end);
            thread::sleep(Duration::from_millis(1));
        }
    }
    let entries = s.entries.len();
    let lines = s.active_document().unwrap().line_count();
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), CodisseuApp::new(s));
    for _ in 0..30 {
        h.step();
    }
    let center = h
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .rect()
        .center();
    h.hover_at(center);
    let mut frames = vec![];
    let mut starts = std::collections::BTreeSet::new();
    let mut editor_times = vec![];
    for i in 0..120 {
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            delta: egui::vec2(0., if (i / 30) % 2 == 0 { -50. } else { 50. }),
            modifiers: egui::Modifiers::NONE,
        });
        let start = Instant::now();
        h.step();
        frames.push(start.elapsed().as_micros());
        starts.insert(h.state().editor_metrics().visible_start);
        editor_times.push(h.state().editor_metrics().frame_micros);
    }
    frames.sort();
    editor_times.sort();
    assert!(starts.len() > 10);
    assert!(!h.state().service.active_document().unwrap().is_dirty());
    assert_eq!(fs::read(&path).unwrap(), original);
    println!(
        "{}",
        serde_json::json!({"scope":"explicit file read-only, no providers, no source output","entries":entries,"lines":lines,"bytes":original.len(),"frames":120,"whole_frame_us":{"p50":frames[60],"p95":frames[114],"max":frames[119]},"editor_us_p95":editor_times[114],"distinct_viewports":starts.len(),"disk_preserved":true})
    );
}

#[test]
fn collapsed_subtrees_do_not_hide_siblings_or_leak_expanded_descendants() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("fixture");
    fs::create_dir_all(root.join("app/Http/Controllers")).unwrap();
    fs::create_dir_all(root.join("routes")).unwrap();
    for name in [
        "app/Http/Controllers/Own.php",
        "routes/web.php",
        "index.php",
    ] {
        fs::write(root.join(name), "<?php // fixture").unwrap();
    }
    let mut service = AppService::new(temp.path().join("session.json"));
    service.dispatch(Command::OpenWorkspace(root)).unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
    }
    let mut h = Harness::builder().with_size([1280., 800.]).build_ui_state(
        |ui, a: &mut CodisseuApp| a.show(ui),
        CodisseuApp::new(service),
    );
    h.step();
    h.step();
    assert!(h.query_by_label("Http").is_none());
    for label in ["app", "Http", "Controllers"] {
        h.get_by_label(label).click();
        h.step();
        h.step();
    }
    assert!(h.query_by_label("Own.php").is_some());
    h.get_by_label("app").click();
    h.step();
    h.step();
    assert!(h.query_by_label("Http").is_none());
    assert!(h.query_by_label("Own.php").is_none());
    assert!(h.query_by_label("index.php").is_some());
    h.get_by_label("routes").click();
    h.step();
    h.step();
    assert!(h.query_by_label("web.php").is_some());
    h.get_by_label("app").click();
    h.step();
    h.step();
    assert!(h.query_by_label("Own.php").is_some());
    assert!(h.query_by_label("web.php").is_some());
}
