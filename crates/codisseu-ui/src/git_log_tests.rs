use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

fn branch(name: &str, remote: bool, current: bool) -> BranchInfo {
    BranchInfo {
        name: name.into(),
        oid: "a".repeat(40),
        remote,
        current,
        upstream: None,
        tracking: String::new(),
        worktree: None,
    }
}
fn harness() -> (tempfile::TempDir, Harness<'static, CodisseuApp>) {
    let dir = tempfile::tempdir().unwrap();
    let mut app = CodisseuApp::new(AppService::new(dir.path().join("session.json")));
    app.git_ui.branches = true;
    app.service.git.branches = vec![
        branch("main", false, true),
        branch("feature/greeting", false, false),
        branch("feature/encoding", false, false),
        branch("origin/feature/greeting", true, false),
    ];
    app.service.git.tags = vec![TagInfo {
        name: "release/v1.0.0".into(),
        oid: "b".repeat(40),
        target: "a".repeat(40),
        annotated: true,
        message: "Release".into(),
    }];
    let mut harness = Harness::builder().with_size([1300., 380.]).build_ui_state(
        |ui, app: &mut CodisseuApp| {
            if !app.styled {
                app.tokens.apply(ui.ctx());
                app.styled = true;
            }
            app.git_log_panel(ui);
        },
        app,
    );
    harness.run();
    (dir, harness)
}

#[test]
fn reference_tree_keeps_exact_leaf_refs_and_real_hierarchy() {
    let (_dir, mut h) = harness();
    assert!(h.query_by_label("Local/feature").is_some());
    assert!(h.query_by_label("Remote/origin/feature").is_some());
    assert!(h.query_by_label("Tags/release").is_some());
    h.get_by_label("Local branch feature/greeting").click();
    h.run();
    assert_eq!(h.state().git_ui.log_branch, "refs/heads/feature/greeting");
    assert!(
        h.get_by_label("Local branch feature/greeting")
            .accesskit_node()
            .is_selected()
            .unwrap_or(false)
    );
    assert!(h.state().git_ui.queue.iter().any(|command|matches!(&command.command,GitCommand::HistoryReference{reference:Some(reference),skip:0,..}if reference=="refs/heads/feature/greeting")));
    assert!(
        !h.state()
            .git_ui
            .queue
            .iter()
            .any(|command| matches!(&command.command, GitCommand::Prepare(_)))
    );
}
#[test]
fn branch_search_reveals_matching_closed_folder_without_changing_commit_filter() {
    let (_dir, mut h) = harness();
    h.get_by_label("Local/feature").click();
    h.run();
    assert!(h.query_by_label("Local branch feature/greeting").is_none());
    h.get_by_label("Search branches or tags").click();
    h.step();
    h.event(egui::Event::Text("greeting".into()));
    h.run();
    assert!(h.query_by_label("Local branch feature/greeting").is_some());
    assert!(
        h.query_by_label("Remote branch origin/feature/greeting")
            .is_some()
    );
    assert!(h.query_by_label("Local branch feature/encoding").is_none());
    assert!(h.state().git_ui.filter.is_empty());
    assert!(h.state().git_ui.queue.is_empty());
}
#[test]
fn collapse_and_expand_controls_operate_only_log_groups() {
    let (_dir, mut h) = harness();
    h.state_mut().git_ui.groups.insert("Changes".into(), false);
    h.get_by_label("Collapse all Git branches").click();
    h.run();
    assert!(h.query_by_label("main").is_none());
    assert!(h.query_by_label("Local branch feature/greeting").is_none());
    h.get_by_label("Expand all Git branches").click();
    h.run();
    assert!(h.query_by_label("Local branch feature/greeting").is_some());
    assert_eq!(h.state().git_ui.groups.get("Changes"), Some(&false));
}
#[test]
fn tree_model_does_not_confuse_common_prefix_with_a_ref() {
    let mut tree = RefTree::default();
    tree.insert(RefLeaf::Branch(branch("feature", false, false)));
    tree.insert(RefLeaf::Branch(branch(
        "feature/nested/with-ação",
        false,
        false,
    )));
    assert_eq!(tree.leaves[0].name(), "feature");
    assert_eq!(
        tree.children["feature"].children["nested"].leaves[0].name(),
        "feature/nested/with-ação"
    );
}

#[test]
fn empty_and_filtered_log_do_not_expand_the_allocated_panel() {
    struct State {
        app: CodisseuApp,
        budget: f32,
        actual: f32,
    }
    let dir = tempfile::tempdir().unwrap();
    let mut app = CodisseuApp::new(AppService::new(dir.path().join("session.json")));
    app.git_ui.branches = true;
    let mut h = Harness::builder().with_size([1317., 284.]).build_ui_state(
        |ui, state: &mut State| {
            if !state.app.styled {
                state.app.tokens.apply(ui.ctx());
                state.app.styled = true;
            }
            state.budget = ui.available_height();
            state.actual = ui
                .allocate_ui_with_layout(
                    ui.available_size(),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| state.app.git_log_panel(ui),
                )
                .response
                .rect
                .height();
        },
        State {
            app,
            budget: 0.,
            actual: 0.,
        },
    );
    h.run();
    assert!(
        h.state().actual <= h.state().budget + 1.,
        "empty log expanded {} > {}",
        h.state().actual,
        h.state().budget
    );
    h.state_mut().app.service.git.history = Some(codisseu_core::git::HistoryPage {
        resolved_reference: None,
        skip: 0,
        has_more: false,
        next_lanes: vec![],
        commits: vec![CommitEntry {
            oid: "a".repeat(40),
            parents: vec![],
            author: "Fixture".into(),
            email: "fixture@example.invalid".into(),
            timestamp: 0,
            subject: "Visible commit".into(),
            refs: vec!["HEAD -> main".into()],
            lane: 0,
            edges: vec![],
        }],
    });
    h.state_mut().app.git_ui.filter = "does not match".into();
    h.run();
    assert!(
        h.state().actual <= h.state().budget + 1.,
        "filtered log expanded {} > {}",
        h.state().actual,
        h.state().budget
    );
}

#[test]
fn same_named_local_and_remote_refs_remain_distinct() {
    let (_dir, mut h) = harness();
    h.state_mut()
        .service
        .git
        .branches
        .push(branch("origin/feature/greeting", false, false));
    h.run();
    h.get_by_label("Local branch origin/feature/greeting")
        .click();
    h.run();
    assert_eq!(
        h.state().git_ui.log_branch,
        "refs/heads/origin/feature/greeting"
    );
    assert!(
        matches!(h.state().git_ui.queue.back().map(|request|&request.command),Some(GitCommand::HistoryReference{reference:Some(reference),..})if reference=="refs/heads/origin/feature/greeting")
    );
    h.get_by_label("Remote branch origin/feature/greeting")
        .click();
    h.run();
    assert_eq!(
        h.state().git_ui.log_branch,
        "refs/remotes/origin/feature/greeting"
    );
    assert!(
        matches!(h.state().git_ui.queue.back().map(|request|&request.command),Some(GitCommand::HistoryReference{reference:Some(reference),..})if reference=="refs/remotes/origin/feature/greeting")
    );
}

#[test]
fn favorite_branch_precedes_folders_and_keeps_full_identity() {
    let (_dir, mut h) = harness();
    h.state_mut().service.git.branch_preferences.favorites = vec!["refs/heads/main".into()];
    h.run();
    assert!(
        h.get_by_label("Local branch main").rect().top()
            < h.get_by_label("Local/feature").rect().top()
    );
    h.get_by_label("Local branch main").click();
    h.run();
    assert_eq!(h.state().git_ui.log_branch, "refs/heads/main");
    assert_eq!(
        h.state().service.git.branch_preferences.favorites,
        ["refs/heads/main"]
    );
}

#[test]
fn branch_tree_visibility_toggle_keeps_log_alive_and_controls_inside_its_bounds() {
    let (_dir, mut h) = harness();
    for expected in [false, true, false, true] {
        h.get_by_label("Show or hide Git branches").click();
        h.run();
        assert_eq!(h.state().git_ui.branches, expected);
        assert_eq!(
            h.query_by_label("Search branches or tags").is_some(),
            expected
        );
        assert!(h.query_by_label("Filter history by branch").is_some());
        assert!(!h.state().allow_close);
        assert!(!h.state().exit_when_saved);
    }
}

#[test]
fn long_ref_filter_stays_bounded_and_reselect_does_not_reload_or_fill_queue() {
    let (_dir, mut h) = harness();
    let long = format!("origin/feature/{}", "very-long-branch-".repeat(12));
    h.state_mut()
        .service
        .git
        .branches
        .push(branch(&long, true, false));
    h.state_mut().git_ui.log_branch = format!("refs/remotes/{long}");
    h.run();
    let filter = h.get_by_label("Filter history by branch").rect();
    assert!(
        filter.width() <= 136.,
        "long ref expanded filter: {filter:?}"
    );
    assert!(h.get_by_label("Git Log actions").rect().right() < 1300.);
    h.set_size(egui::vec2(800., 380.));
    h.run();
    assert!(
        h.get_by_label("Git Log actions").rect().right()
            < h.get_by_label("Show selected commit diff").rect().left()
    );
    assert!(h.get_by_label("Filter history by branch").rect().width() <= 136.);
    h.set_size(egui::vec2(1300., 380.));
    h.run();
    h.get_by_label("Local branch main").click();
    h.run();
    let queued = h.state().git_ui.queue.len();
    assert_eq!(queued, 1);
    h.get_by_label("Local branch main").click();
    h.run();
    assert_eq!(h.state().git_ui.queue.len(), queued);
    for reference in [
        "refs/heads/feature/greeting",
        "refs/remotes/origin/feature/greeting",
        "HEAD",
        "refs/heads/main",
    ] {
        h.state_mut().git_ui.log_branch = reference.into();
        h.state_mut().git_request_history(true);
    }
    assert_eq!(h.state().git_ui.queue.len(), 1);
    assert!(
        matches!(h.state().git_ui.queue.back().map(|request|&request.command),Some(GitCommand::HistoryReference{reference:Some(r),..})if r=="refs/heads/main")
    );
}

#[path = "git_log_stability_tests.rs"]
mod stability;
