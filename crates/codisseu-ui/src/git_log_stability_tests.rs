//! Real-worker busy/ready transitions with delivery deliberately held at the UI boundary.
use super::*;
use egui_kittest::kittest::NodeT;
use std::{
    fs,
    path::Path,
    process::Command as Process,
    thread,
    time::{Duration, Instant},
};
fn cli(root: &Path, args: &[&str]) -> String {
    let o = Process::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap().trim().into()
}
fn idle(app: &mut CodisseuApp) {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        app.service.poll();
        app.poll_git_ui();
        if !app.service.is_busy() && app.git_ui.queue.is_empty() {
            break;
        }
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn busy_history_keeps_geometry_page_buffer_and_draft_and_routes_cancellation_locally() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    cli(&root, &["init", "-b", "main"]);
    for (k, v) in [
        ("user.name", "Fixture"),
        ("user.email", "fixture@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", "/dev/null"),
    ] {
        cli(&root, &["config", k, v]);
    }
    let path = root.join("source.js");
    fs::write(&path, "const value = 1;\n").unwrap();
    cli(&root, &["add", "source.js"]);
    cli(&root, &["commit", "-m", "base"]);
    let base = cli(&root, &["rev-parse", "HEAD"]);
    fs::write(&path, "const value = 2;\n").unwrap();
    cli(&root, &["commit", "-am", "newer"]);
    let head = cli(&root, &["rev-parse", "HEAD"]);
    cli(&root, &["update-ref", "refs/remotes/origin/base", &base]);
    cli(&root, &["update-ref", "refs/remotes/origin/newer", &head]);
    let index = fs::read(root.join(".git/index")).unwrap();
    let mut service = AppService::new(root.join("session.json"));
    service
        .dispatch(Command::OpenWorkspace(root.clone()))
        .unwrap();
    service.dispatch(Command::OpenFile(path.clone())).unwrap();
    let mut app = CodisseuApp::new(service);
    idle(&mut app);
    app.git_open_log();
    idle(&mut app);
    let doc = app.service.active_document().unwrap();
    let (id, version) = (doc.id, doc.version);
    app.service
        .dispatch(Command::Edit {
            id,
            expected_version: version,
            text: "// dirty 🌧\n".into(),
        })
        .unwrap();
    app.git_ui.message = "draft stays".into();
    let first = app.service.git.history.as_ref().unwrap().commits[0]
        .oid
        .clone();
    let mut h = Harness::builder()
        .with_size([1400., 800.])
        .with_max_steps(30)
        .build_ui_state(
            |ui, app: &mut CodisseuApp| {
                if !app.styled {
                    app.tokens.apply(ui.ctx());
                    app.styled = true;
                }
                app.main_toolbar(ui);
                app.tool_panel(ui);
            },
            app,
        );
    h.run();
    let controls = [
        "Show or hide Git branches",
        "Search branches or tags",
        "Text/hash/author in this page",
        "Filter history by branch",
        "Hide Git",
    ];
    let before: Vec<_> = controls
        .iter()
        .map(|label| h.get_by_label(label).rect())
        .collect();
    h.get_by_label("Remote branch origin/base").click();
    h.step();
    h.state_mut().poll_git_ui();
    h.step();
    assert!(h.state().service.git.history_pending());
    assert!(h.query_by_label("Cancelar operações").is_none());
    assert!(h.query_by_label("Cancel Git operation").is_some());
    assert_eq!(
        h.state().service.git.history.as_ref().unwrap().commits[0].oid,
        first
    );
    for (label, rect) in controls.iter().zip(&before) {
        assert_eq!(h.get_by_label(label).rect(), *rect, "busy moved {label}");
    }
    let old_commit = format!("Commit {} newer", &head[..8]);
    assert!(h.get_by_label(&old_commit).accesskit_node().is_disabled());
    h.get_by_label("Remote branch origin/newer").click();
    h.step();
    h.get_by_label("Remote branch origin/base").click();
    h.step();
    h.get_by_label("Remote branch origin/newer").click();
    h.step();
    assert_eq!(h.state().git_ui.queue.len(), 1);
    idle(h.state_mut());
    h.step();
    assert_eq!(h.state().git_ui.log_branch, "refs/remotes/origin/newer");
    assert_eq!(
        h.state().service.git.history.as_ref().unwrap().commits[0].oid,
        head
    );
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        "// dirty 🌧\n"
    );
    assert_eq!(h.state().git_ui.message, "draft stays");
    for (label, rect) in controls.iter().zip(&before) {
        assert_eq!(h.get_by_label(label).rect(), *rect, "ready moved {label}");
    }
    h.get_by_label("Remote branch origin/base").click();
    h.step();
    h.state_mut().poll_git_ui();
    h.step();
    h.get_by_label("Cancel Git operation").click();
    h.step();
    idle(h.state_mut());
    h.step();
    assert!(h.query_by_label("Cancel Git operation").is_none());
    assert!(h.state().service.git.error.is_some());
    for (label, rect) in controls.iter().zip(&before) {
        assert_eq!(h.get_by_label(label).rect(), *rect, "cancel moved {label}");
    }
    assert_eq!(cli(&root, &["rev-parse", "HEAD"]), head);
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read_to_string(&path).unwrap(), "const value = 2;\n");
    // Clicking the selected ref retries a cancelled/failed read, without forcing
    // the user to choose an unrelated branch first.
    h.get_by_label("Remote branch origin/base").click();
    h.step();
    assert_eq!(h.state().git_ui.queue.len(), 1);
    idle(h.state_mut());
    h.step();
    assert!(h.state().service.git.error.is_none());
    assert_eq!(
        h.state().service.git.history.as_ref().unwrap().commits[0].oid,
        base
    );
    // Other pending work still has the global cancellation action.
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(path))
        .unwrap();
    h.step();
    assert!(h.query_by_label("Cancelar operações").is_some());
    idle(h.state_mut());
    h.state_mut().show_activity = false;
    h.state_mut()
        .service
        .dispatch(Command::Git(GitCommand::Branches))
        .unwrap();
    h.step();
    assert!(
        h.query_by_label("Cancelar operações").is_some(),
        "Git jobs outside the Log keep their existing cancellation route"
    );
    assert!(!h.state().allow_close);
    assert!(!h.state().exit_when_saved);
}
#[test]
fn coalescing_history_never_drops_a_queued_mutation_review() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = CodisseuApp::new(AppService::new(dir.path().join("session.json")));
    app.git_send(GitCommand::History {
        skip: 0,
        limit: 200,
        path: None,
    });
    app.git_send(GitCommand::Prepare(
        codisseu_core::git::LocalAction::CheckoutBranch {
            name: "keep-review".into(),
        },
    ));
    app.git_send(GitCommand::Tags);
    app.git_ui.log_branch = "refs/heads/latest".into();
    app.git_request_history(true);
    assert_eq!(app.git_ui.queue.len(), 3);
    assert!(matches!(
        &app.git_ui.queue[0].command,
        GitCommand::Prepare(_)
    ));
    assert!(matches!(&app.git_ui.queue[1].command, GitCommand::Tags));
    assert!(
        matches!(&app.git_ui.queue[2].command,GitCommand::HistoryReference{reference:Some(r),..}if r=="refs/heads/latest")
    );
}
