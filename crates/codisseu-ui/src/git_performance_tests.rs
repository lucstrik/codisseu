//! Own bounded presentation fixture. Timing tests do not claim real backend coverage.
use super::*;
use codisseu_core::git::{BranchInfo, CommitEntry, HistoryPage, TagInfo};
use egui_kittest::{Harness, kittest::Queryable};
use std::time::Instant;
fn fixture(refs: usize, lanes: usize) -> (tempfile::TempDir, CodisseuApp) {
    let dir = tempfile::tempdir().unwrap();
    let mut app = CodisseuApp::new(AppService::new(dir.path().join("session.json")));
    app.tool_tab = ToolTab::Git;
    app.show_activity = true;
    app.service.git.branches = (0..refs)
        .map(|i| BranchInfo {
            name: format!("release/topic-{i:04}"),
            oid: format!("{i:040x}"),
            current: i == 0,
            remote: false,
            upstream: None,
            tracking: String::new(),
            worktree: None,
        })
        .collect();
    app.service.git.tags = (0..refs)
        .map(|i| TagInfo {
            name: format!("v1/release-{i:04}"),
            oid: format!("{i:040x}"),
            target: format!("{i:040x}"),
            annotated: false,
            message: String::new(),
        })
        .collect();
    app.service.git.history = Some(HistoryPage {
        resolved_reference: None,
        skip: 0,
        has_more: false,
        next_lanes: vec![],
        commits: (0..200)
            .map(|i| CommitEntry {
                oid: format!("{i:040x}"),
                parents: vec![format!("{:040x}", i + 1)],
                author: format!("Fixture Author {}", i % 30),
                email: "fixture@example.invalid".into(),
                timestamp: 1_791_100_000 - i * 60,
                subject: format!(
                    "Own history fixture {i:04}: preserve scroll and commit selection"
                ),
                refs: if i % 10 == 0 {
                    vec![format!("origin/release/{i:04}")]
                } else {
                    vec![]
                },
                lane: i as usize % lanes,
                edges: (0..lanes).map(|j| (j, j)).collect(),
            })
            .collect(),
    });
    (dir, app)
}
fn wheel(h: &mut Harness<'_, CodisseuApp>, delta: f32) {
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        delta: egui::vec2(0., delta),
        modifiers: Modifiers::NONE,
    });
}
#[test]
#[ignore = "Explicit CPU benchmark, deterministic own Git presentation fixture"]
fn git_scroll_frame_probe() {
    for refs in [200, 2048] {
        let (_dir, app) = fixture(refs, 64);
        let timing = std::rc::Rc::new(std::cell::Cell::new(0u128));
        let elapsed = timing.clone();
        let mut h = Harness::builder().with_size([1400., 800.]).build_ui_state(
            move |ui, a: &mut CodisseuApp| {
                if !a.styled {
                    a.tokens.apply(ui.ctx());
                    a.styled = true;
                }
                let start = Instant::now();
                a.tool_panel(ui);
                elapsed.set(start.elapsed().as_micros());
            },
            app,
        );
        for _ in 0..20 {
            h.step();
        }
        let point = h
            .get_by_label(
                "Commit 00000000 Own history fixture 0000: preserve scroll and commit selection",
            )
            .rect()
            .center();
        h.hover_at(point);
        let mut frames = vec![];
        let mut body = vec![];
        for n in 0..120 {
            wheel(&mut h, if n < 60 { -26. } else { 26. });
            let start = Instant::now();
            h.step();
            frames.push(start.elapsed().as_micros());
            body.push(timing.get());
        }
        frames.sort();
        body.sort();
        println!(
            "{}",
            serde_json::json!({"fixture":"200 commits, 64 lanes, own branch and tag lists", "refs_per_kind":refs,"samples":120,
            "whole_frame_us":{"p50":frames[60],"p95":frames[114],"max":frames[119]},
            "git_body_us":{"p50":body[60],"p95":body[114],"max":body[119]}})
        );
    }
}

#[test]
fn reference_widgets_are_limited_to_viewport_and_update_after_filter_or_refresh() {
    use egui::accesskit::Role;
    let (_dir, app) = fixture(1500, 8);
    let mut h = Harness::builder()
        .with_size([1300., 380.])
        .with_max_steps(40)
        .build_ui_state(
            |ui, a: &mut CodisseuApp| {
                if !a.styled {
                    a.tokens.apply(ui.ctx());
                    a.styled = true;
                }
                a.git_log_panel(ui);
            },
            app,
        );
    h.run();
    assert!(h.get_all_by_role(Role::TreeItem).count() < 35);
    assert!(h.query_by_label("Tag v1/release-1499").is_none());
    let point = h
        .get_by_label("Local branch release/topic-0000")
        .rect()
        .center();
    h.hover_at(point);
    wheel(&mut h, -150_000.);
    for _ in 0..60 {
        h.step();
    }
    assert!(h.get_all_by_role(Role::TreeItem).count() < 35);
    h.get_by_label("Tag v1/release-1499").click();
    h.run();
    assert_eq!(
        h.state().git_ui.commit.as_deref(),
        Some(format!("{:040x}", 1499).as_str())
    );
    h.get_by_label("Search branches or tags").click();
    h.run();
    h.event(egui::Event::Text("topic-0000".into()));
    h.run();
    assert!(
        h.query_by_label("Local branch release/topic-0000")
            .is_some()
    );
    assert!(
        h.query_by_label("Local branch release/topic-0001")
            .is_none()
    );
    h.state_mut().service.git.branches[0].name = "release/replaced-0000".into();
    h.state_mut().service.git.revision += 1;
    h.run();
    assert!(
        h.query_by_label("Local branch release/topic-0000")
            .is_none()
    );
}

#[test]
fn history_filter_revision_and_empty_page_do_not_reuse_stale_rows() {
    let (_dir, app) = fixture(5, 8);
    let mut h = Harness::builder()
        .with_size([1300., 380.])
        .with_max_steps(40)
        .build_ui_state(
            |ui, a: &mut CodisseuApp| {
                if !a.styled {
                    a.tokens.apply(ui.ctx());
                    a.styled = true;
                }
                a.git_log_panel(ui);
            },
            app,
        );
    h.run();
    h.get_by_label("Text/hash/author in this page").click();
    h.run();
    h.event(egui::Event::Text("fixture 0199".into()));
    h.run();
    let label = "Commit 00000000 Own history fixture 0199: preserve scroll and commit selection";
    h.get_by_label(label).click();
    h.run();
    assert_eq!(
        h.state().git_ui.commit.as_deref(),
        Some(format!("{:040x}", 199).as_str())
    );
    h.state_mut().service.git.history.as_mut().unwrap().commits[199].subject =
        "changed page".into();
    h.state_mut().service.git.revision += 1;
    h.run();
    assert!(h.query_by_label(label).is_none());
    assert!(
        h.query_by_label("No commits matching the current filter")
            .is_some()
    );
    h.state_mut().service.git.history = None;
    h.run();
    assert!(h.query_by_label(label).is_none());
}

#[test]
fn large_lazy_tree_keeps_all_entries_without_permanent_warning_widgets() {
    let dir = tempfile::tempdir().unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    let workspace = dir.path().join("tree");
    std::fs::create_dir(&workspace).unwrap();
    for i in 0..5001 {
        std::fs::write(workspace.join(format!("File{i:04}.txt")), "").unwrap();
    }
    service.dispatch(Command::OpenWorkspace(workspace)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    // M3.33 removed the global 5000-entry tree cutoff. Keep this regression
    // aligned with the real directory-at-a-time contract instead of requiring it.
    assert!(!service.tree_truncated);
    assert_eq!(service.entries.len(), 5001);
    assert!(!service.status.contains("limitada"));
    let mut h = Harness::builder().with_size([1280., 800.]).build_ui_state(
        |ui, a: &mut CodisseuApp| a.show(ui),
        CodisseuApp::new(service),
    );
    h.run();
    assert!(!h.state().service.tree_truncated);
    assert_eq!(h.state().service.entries.len(), 5001);
    assert!(
        h.query_by_label("Árvore parcial: limite de entradas/profundidade")
            .is_none()
    );
}
