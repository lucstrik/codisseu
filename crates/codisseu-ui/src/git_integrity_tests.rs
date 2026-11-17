//! Regression flows driven through the real Git worker and native egui widgets.
//! Every Git write belongs to a disposable fixture; no network transport is used.
use super::*;
use crate::git::{GitActionKind, GitDialog};
use codisseu_core::git::{DiffSide, GitCommand, workflows::WorkflowAction};
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::{
    fs,
    path::Path,
    process::Command as Process,
    thread,
    time::{Duration, Instant},
};

struct State {
    app: CodisseuApp,
    pause_delivery: bool,
    show_branch: bool,
}
type GitHarness = Harness<'static, State>;

fn cli(path: &Path, arguments: &[&str]) -> String {
    let output = Process::new("git")
        .arg("-C")
        .arg(path)
        .args(arguments)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_EDITOR", "true")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn fixture() -> (tempfile::TempDir, PathBuf, GitHarness) {
    fixture_at_scale(1.)
}

#[test]
fn git_direct_actions_stage_unstage_commit_push_pull_without_review() {
    let (dir, root, mut h) = fixture();
    h.state_mut().app.send(Command::SetGitConfirmActions(false));
    change_file(&root, &mut h);
    h.state_mut()
        .app
        .git_prepare(codisseu_core::git::LocalAction::Stage {
            paths: vec!["app.js".into()],
        });
    settle(&mut h);
    assert!(h.state().app.git_ui.dialog.is_none());
    assert!(cli(&root, &["diff", "--cached", "--name-only"]).contains("app.js"));
    h.state_mut()
        .app
        .git_prepare(codisseu_core::git::LocalAction::Unstage {
            paths: vec!["app.js".into()],
        });
    settle(&mut h);
    assert!(cli(&root, &["diff", "--cached", "--name-only"]).is_empty());
    h.state_mut().app.git_ui.message = "Direct UI commit".into();
    h.state_mut().app.git_commit(false);
    settle(&mut h);
    assert_eq!(
        cli(&root, &["log", "-1", "--format=%s"]).trim(),
        "Direct UI commit"
    );
    assert!(h.state().app.git_ui.message.is_empty());
    cli(&root, &["config", "protocol.file.allow", "always"]);
    let remote = dir.path().join("remote.git");
    cli(&root, &["init", "--bare", remote.to_str().unwrap()]);
    cli(
        &root,
        &["remote", "add", "upstream", remote.to_str().unwrap()],
    );
    h.state_mut().app.git_send(GitCommand::Remotes);
    settle(&mut h);
    h.state_mut()
        .app
        .git_action(GitActionKind::Push, String::new());
    settle(&mut h);
    assert!(h.state().app.git_ui.dialog.is_none());
    assert!(
        h.state().app.service.git.error.is_none(),
        "{:?}",
        h.state().app.service.git.error
    );
    assert_eq!(
        cli(&root, &["rev-parse", "HEAD"]),
        cli(&remote, &["rev-parse", "refs/heads/main"])
    );
    h.state_mut()
        .app
        .git_action(GitActionKind::Fetch, String::new());
    settle(&mut h);
    assert!(h.state().app.git_ui.dialog.is_none());
    cli(&root, &["branch", "--set-upstream-to=upstream/main"]);
    let other = dir.path().join("other");
    cli(
        &root,
        &[
            "clone",
            "-b",
            "main",
            remote.to_str().unwrap(),
            other.to_str().unwrap(),
        ],
    );
    cli(&other, &["config", "user.name", "Local Fixture"]);
    cli(&other, &["config", "user.email", "fixture@example.invalid"]);
    cli(&other, &["config", "commit.gpgsign", "false"]);
    cli(&other, &["config", "core.hooksPath", "/dev/null"]);
    fs::write(other.join("pulled.js"), "export const pulled = true;\n").unwrap();
    cli(&other, &["add", "."]);
    cli(&other, &["commit", "-m", "Incoming"]);
    cli(&other, &["push", "origin", "main"]);
    h.state_mut().app.git_send(GitCommand::Refresh);
    settle(&mut h);
    h.state_mut()
        .app
        .git_action(GitActionKind::Pull, String::new());
    settle(&mut h);
    assert!(h.state().app.git_ui.dialog.is_none());
    assert!(root.join("pulled.js").exists());
    assert!(
        h.state().app.service.git.error.is_none(),
        "{:?}",
        h.state().app.service.git.error
    );
    let published = cli(&remote, &["rev-parse", "refs/heads/main"]);
    h.state_mut()
        .app
        .git_action(GitActionKind::Push, "explicit-other-branch".into());
    assert!(
        matches!(&h.state().app.git_ui.dialog,Some(GitDialog::Push{source,..}) if source=="explicit-other-branch")
    );
    assert_eq!(cli(&remote, &["rev-parse", "refs/heads/main"]), published);
}

#[test]
fn git_direct_action_rejects_external_change_after_preparation_without_staging() {
    let (_dir, root, mut h) = fixture();
    change_file(&root, &mut h);
    h.state_mut().app.send(Command::SetGitConfirmActions(false));
    let index = fs::read(root.join(".git/index")).unwrap();
    h.state_mut()
        .app
        .git_prepare(codisseu_core::git::LocalAction::Stage {
            paths: vec!["app.js".into()],
        });
    let until = Instant::now() + Duration::from_secs(10);
    while h.state().app.service.git.preview.is_none() {
        h.state_mut().app.service.poll();
        if h.state().app.service.git.preview.is_some() {
            break;
        }
        h.state_mut().app.poll_git_ui();
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(2));
    }
    fs::write(root.join("app.js"), "// external after preparation\n").unwrap();
    settle(&mut h);
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    assert!(h.state().app.service.git.error.is_some());
    assert_eq!(
        fs::read_to_string(root.join("app.js")).unwrap(),
        "// external after preparation\n"
    );
}
fn fixture_at_scale(scale: f32) -> (tempfile::TempDir, PathBuf, GitHarness) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repository");
    fs::create_dir(&root).unwrap();
    cli(&root, &["init", "-b", "main"]);
    let hooks = root.join(".git/fixture-hooks");
    fs::create_dir(&hooks).unwrap();
    for (key, value) in [
        ("user.name", "Codisseu UI Fixture"),
        ("user.email", "fixture@example.invalid"),
        ("commit.gpgsign", "false"),
        ("protocol.allow", "never"),
        ("core.hooksPath", hooks.to_str().unwrap()),
    ] {
        cli(&root, &["config", key, value]);
    }
    fs::write(root.join("app.js"), "-- original 😀\n").unwrap();
    cli(&root, &["add", "app.js"]);
    cli(&root, &["commit", "-m", "Initial fixture message"]);
    let mut service = AppService::new(dir.path().join("session.json"));
    service
        .dispatch(Command::OpenWorkspace(root.clone()))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while service.is_busy() || service.workspace.is_none() || service.git.repo.is_none() {
        service.poll();
        assert!(
            Instant::now() < deadline,
            "opening Git fixture: {:?}",
            service.last_error
        );
        thread::sleep(Duration::from_millis(2));
    }
    service
        .dispatch(Command::SetCommitToolWindowVisible(true))
        .unwrap();
    // Legacy preview scenarios explicitly opt into confirmation. Product default is direct.
    service
        .dispatch(Command::SetGitConfirmActions(true))
        .unwrap();
    let app = CodisseuApp::new(service);
    let mut harness = Harness::builder()
        .with_size([1280., 900.])
        .with_pixels_per_point(scale)
        .with_max_steps(30)
        .build_ui_state(
            |ui, state: &mut State| {
                if !state.app.styled {
                    state.app.tokens.apply(ui.ctx());
                    state.app.styled = true;
                }
                if !state.pause_delivery {
                    state.app.service.poll();
                    state.app.poll_git_ui();
                }
                if state.show_branch {
                    ui.horizontal(|ui| state.app.git_branch_menu(ui));
                }
                if state.app.git_ui.commit_open {
                    state.app.git_commit_panel(ui);
                }
                let ctx = ui.ctx().clone();
                state.app.git_windows(&ctx);
            },
            State {
                app,
                pause_delivery: false,
                show_branch: false,
            },
        );
    settle(&mut harness);
    (dir, root, harness)
}

fn settle(h: &mut GitHarness) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut idle_frames = 0;
    loop {
        h.step();
        let state = h.state();
        if !state.app.service.is_busy() && !state.app.git_repository_uis_pending() {
            idle_frames += 1;
            if idle_frames >= 3 {
                break;
            }
        } else {
            idle_frames = 0;
        }
        assert!(
            Instant::now() < deadline,
            "Git UI did not settle: {} {:?}",
            state.app.service.git.status,
            state.app.service.git.error
        );
        thread::sleep(Duration::from_millis(2));
    }
    h.run();
}
fn change_file(root: &Path, h: &mut GitHarness) {
    fs::write(root.join("app.js"), "++ edited 😀\n").unwrap();
    h.state_mut().app.git_send(GitCommand::Refresh);
    settle(h);
    h.state_mut().app.git_ui.selected.insert("app.js".into());
}
fn replace_input(h: &mut GitHarness, label: &str, text: &str) {
    h.get_by_label(label).click();
    h.step();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(egui::Event::Text(text.into()));
    h.step();
}

#[test]
fn cancelled_commit_then_real_stash_preserves_draft_and_does_not_push() {
    let (_dir, root, mut h) = fixture();
    change_file(&root, &mut h);
    h.state_mut().app.git_ui.message = "Draft belongs to a future commit".into();
    h.state_mut().app.git_commit(true);
    settle(&mut h);
    assert!(matches!(
        h.state().app.git_ui.dialog,
        Some(GitDialog::Preview)
    ));
    h.get_by_label("Cancel").click();
    settle(&mut h);
    h.state_mut()
        .app
        .git_prepare_workflow(WorkflowAction::StashCreate {
            message: "Own UI fixture stash".into(),
            include_untracked: false,
        });
    settle(&mut h);
    h.get_by_label("Apply").click();
    settle(&mut h);
    assert_eq!(cli(&root, &["status", "--porcelain"]), "");
    assert!(cli(&root, &["stash", "list"]).contains("Own UI fixture stash"));
    assert_eq!(
        cli(&root, &["log", "-1", "--format=%s"]).trim(),
        "Initial fixture message"
    );
    assert_eq!(
        h.state().app.git_ui.message,
        "Draft belongs to a future commit"
    );
    assert!(!matches!(
        h.state().app.git_ui.dialog,
        Some(GitDialog::Action {
            kind: GitActionKind::Push,
            ..
        })
    ));
}

#[cfg(unix)]
#[test]
fn editing_next_message_while_real_commit_hook_runs_preserves_new_draft() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, root, mut h) = fixture();
    change_file(&root, &mut h);
    let hook = root.join(".git/fixture-hooks/pre-commit");
    fs::write(&hook, "#!/bin/sh\n: > .git/fixture-hook-entered\ncount=0\nwhile [ ! -f .git/fixture-hook-release ]; do\n count=$((count+1))\n [ \"$count\" -lt 500 ] || exit 1\n sleep 0.02\ndone\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o700)).unwrap();
    struct Release(PathBuf);
    impl Drop for Release {
        fn drop(&mut self) {
            let _ = fs::write(&self.0, "release");
        }
    }
    let release = Release(root.join(".git/fixture-hook-release"));
    h.state_mut().app.git_ui.commit_open = true;
    h.state_mut().app.git_ui.message = "Approved actual commit".into();
    h.state_mut().app.git_commit(false);
    settle(&mut h);
    h.get_by_label("Apply").click();
    h.step();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !root.join(".git/fixture-hook-entered").exists() {
        h.step();
        thread::sleep(Duration::from_millis(2));
        assert!(
            Instant::now() < deadline,
            "real pre-commit hook did not start"
        );
    }
    assert!(h.state().app.service.git.is_busy());
    replace_input(
        &mut h,
        "Commit Message",
        "Next message typed during hook 😀",
    );
    assert_eq!(
        h.state().app.git_ui.message,
        "Next message typed during hook 😀"
    );
    drop(release);
    settle(&mut h);
    assert_eq!(
        cli(&root, &["log", "-1", "--format=%B"]).trim(),
        "Approved actual commit"
    );
    assert_eq!(cli(&root, &["show", "HEAD:app.js"]), "++ edited 😀\n");
    assert_eq!(
        h.state().app.git_ui.message,
        "Next message typed during hook 😀"
    );
    assert!(
        h.state().app.service.git.error.is_none(),
        "{:?}",
        h.state().app.service.git.error
    );
}

#[test]
fn cancelling_real_conflict_preview_restores_unsaved_result() {
    let (_dir, root, mut h) = fixture();
    cli(&root, &["checkout", "-b", "topic"]);
    fs::write(root.join("app.js"), "topic side\n").unwrap();
    cli(&root, &["commit", "-am", "Topic"]);
    cli(&root, &["checkout", "main"]);
    fs::write(root.join("app.js"), "main side\n").unwrap();
    cli(&root, &["commit", "-am", "Main"]);
    let merge = Process::new("git")
        .arg("-C")
        .arg(&root)
        .args(["merge", "--no-edit", "topic"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(!merge.status.success());
    let disk_before = fs::read(root.join("app.js")).unwrap();
    let index_before = cli(&root, &["ls-files", "--unmerged"]);
    h.state_mut().app.git_send(GitCommand::Refresh);
    settle(&mut h);
    h.state_mut().app.git_open_conflict("app.js".into());
    settle(&mut h);
    h.get_by_role(Role::MultilineTextInput).click();
    h.step();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(egui::Event::Text(
        "Resolved draft not yet written 😀\n".into(),
    ));
    h.step();
    h.get_by_label("Marcar resultado como resolvido").click();
    h.run();
    h.get_by_label("Apply Changes").click();
    settle(&mut h);
    assert!(matches!(
        h.state().app.git_ui.dialog,
        Some(GitDialog::Preview)
    ));
    h.get_by_label("Cancel").click();
    settle(&mut h);
    match &h.state().app.git_ui.dialog {
        Some(GitDialog::Conflict { result, .. }) => {
            assert_eq!(result, "Resolved draft not yet written 😀\n")
        }
        _ => panic!("cancel did not restore conflict editor"),
    }
    assert_eq!(fs::read(root.join("app.js")).unwrap(), disk_before);
    assert_eq!(cli(&root, &["ls-files", "--unmerged"]), index_before);
}

#[test]
fn delayed_amend_details_do_not_replace_text_typed_after_request() {
    let (_dir, _root, mut h) = fixture();
    h.state_mut().app.git_ui.commit_open = true;
    h.state_mut().app.git_ui.message = "Existing draft before Amend".into();
    h.step();
    // Hold only event delivery, never substitute a mocked Git reply.
    h.state_mut().pause_delivery = true;
    h.get_by_label("Amend").click();
    h.step();
    assert!(h.state().app.git_ui.amend);
    h.state_mut().app.poll_git_ui(); // dispatch the actual CommitDetails request
    assert!(h.state().app.service.git.is_busy());
    replace_input(
        &mut h,
        "Commit Message",
        "Newer user draft while details wait",
    );
    h.state_mut().pause_delivery = false;
    settle(&mut h);
    assert!(
        h.state()
            .app
            .service
            .git
            .details
            .as_ref()
            .is_some_and(|d| d.message.trim() == "Initial fixture message")
    );
    assert_eq!(
        h.state().app.git_ui.message,
        "Newer user draft while details wait"
    );
    h.get_by_label("Amend").click();
    settle(&mut h);
    assert!(!h.state().app.git_ui.amend);
    assert_eq!(
        h.state().app.git_ui.message,
        "Newer user draft while details wait"
    );

    // Turning Amend off before the real reply arrives also revokes that load.
    h.state_mut().pause_delivery = true;
    h.get_by_label("Amend").click();
    h.step();
    h.state_mut().app.poll_git_ui();
    assert!(h.state().app.service.git.is_busy());
    h.get_by_label("Amend").click();
    h.step();
    assert!(!h.state().app.git_ui.amend);
    h.state_mut().pause_delivery = false;
    settle(&mut h);
    assert_eq!(
        h.state().app.git_ui.message,
        "Newer user draft while details wait"
    );

    // An unchanged loaded message can be toggled off and restore the draft.
    h.get_by_label("Amend").click();
    settle(&mut h);
    assert_eq!(
        h.state().app.git_ui.message.trim(),
        "Initial fixture message"
    );
    h.get_by_label("Amend").click();
    settle(&mut h);
    assert_eq!(
        h.state().app.git_ui.message,
        "Newer user draft while details wait"
    );
}

#[test]
fn ctrl_shift_k_opens_push_instead_of_commit() {
    let (_dir, _root, mut h) = fixture();
    let ctx = egui::Context::default();
    let modifiers = Modifiers::CTRL | Modifiers::SHIFT;
    let mut input = egui::RawInput::default();
    input.events.push(egui::Event::Key {
        key: Key::K,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    });
    let mut output = ctx.run_ui(input, |ui| {
        assert!(h.state_mut().app.git_shortcuts(ui.ctx()));
    });
    output.textures_delta.clear();
    assert!(matches!(
        h.state().app.git_ui.dialog,
        Some(GitDialog::Push { .. })
    ));
    assert!(!h.state().app.git_ui.commit_open);
}

#[test]
fn actual_git_patch_retains_content_starting_with_diff_header_prefixes() {
    let (_dir, root, mut h) = fixture();
    fs::write(root.join("app.js"), "++ replacement 😀\n").unwrap();
    h.state_mut().app.git_send(GitCommand::Diff {
        path: "app.js".into(),
        side: DiffSide::Working,
    });
    settle(&mut h);
    let patch = h.state().app.service.git.diff.as_ref().unwrap();
    let rows = crate::git_diff::diff_rows(std::str::from_utf8(&patch.bytes).unwrap());
    assert!(rows.iter().any(|r| {
        r.left
            .as_ref()
            .is_some_and(|(_, text, changed)| *changed && text == "-- original 😀")
    }));
    assert!(rows.iter().any(|r| {
        r.right
            .as_ref()
            .is_some_and(|(_, text, changed)| *changed && text == "++ replacement 😀")
    }));
}

#[test]
fn reviewed_push_uses_distinct_target_and_rejects_changed_head() {
    let (dir, root, mut h) = fixture();
    let remote = dir.path().join("remote.git");
    cli(&root, &["init", "--bare", remote.to_str().unwrap()]);
    cli(
        &root,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    cli(
        &root,
        &["-c", "protocol.file.allow=always", "push", "origin", "main"],
    );
    // The fixture forbids every transport globally, so explicitly enable only its local file transport.
    cli(&root, &["config", "protocol.file.allow", "always"]);
    fs::write(root.join("outgoing.txt"), "reviewed outgoing\n").unwrap();
    cli(&root, &["add", "outgoing.txt"]);
    cli(&root, &["commit", "-m", "Outgoing UI review"]);
    h.state_mut().app.git_send(GitCommand::Refresh);
    settle(&mut h);
    h.state_mut()
        .app
        .git_action(GitActionKind::Push, String::new());
    settle(&mut h);
    h.get_by_label("Push destination").click();
    h.run();
    replace_input(&mut h, "Target branch", "review/custom-target");
    h.run();
    h.get_by_label("Apply destination").click();
    settle(&mut h);
    let review = h.state().app.service.git.push_review.as_ref().unwrap();
    assert_eq!(review.commits[0].subject, "Outgoing UI review");
    assert!(
        review
            .changes
            .iter()
            .any(|c| c.path == Path::new("outgoing.txt"))
    );
    h.get_by_label("outgoing.txt").click();
    h.run();
    h.get_by_label("Show outgoing diff").click();
    settle(&mut h);
    assert!(h.state().app.git_ui.diff_return.is_some());
    assert!(
        h.state()
            .app
            .service
            .git
            .revision_diff
            .as_ref()
            .is_some_and(|d| d.path.as_deref() == Some(Path::new("outgoing.txt"))
                && String::from_utf8_lossy(&d.bytes).contains("reviewed outgoing"))
    );
    h.get_by_label("Close").click();
    h.run();
    assert!(
        matches!(&h.state().app.git_ui.dialog,Some(GitDialog::Push{target,..})if target=="review/custom-target")
    );
    h.get_by_label("Push").click();
    settle(&mut h);
    assert_eq!(
        cli(&remote, &["rev-parse", "refs/heads/review/custom-target"]),
        cli(&root, &["rev-parse", "HEAD"])
    );
    assert_eq!(cli(&root, &["branch", "--show-current"]), "main\n");
    assert!(!h.state().app.git_ui.applying);
    assert!(!h.state().app.git_ui.apply_dispatched);
    assert!(
        !h.state().app.git_ui.output.is_empty(),
        "UI must process the actual Apply acknowledgement"
    );
    h.state_mut()
        .app
        .git_action(GitActionKind::Push, String::new());
    settle(&mut h);
    let before = cli(&remote, &["rev-parse", "refs/heads/main"]);
    fs::write(root.join("after-review.txt"), "new state\n").unwrap();
    cli(&root, &["add", "after-review.txt"]);
    cli(&root, &["commit", "-m", "Changed after visible review"]);
    h.get_by_label("Push").click();
    settle(&mut h);
    assert_eq!(cli(&remote, &["rev-parse", "refs/heads/main"]), before);
    assert!(
        h.state()
            .app
            .service
            .git
            .error
            .as_deref()
            .is_some_and(|e| e.contains("mudou"))
    );
}

#[test]
fn native_shelf_widgets_preserve_index_layers_and_commit_draft() {
    let (_dir, root, mut h) = fixture();
    fs::write(root.join("app.js"), "staged own layer\n").unwrap();
    cli(&root, &["add", "app.js"]);
    fs::write(root.join("app.js"), "working own layer\n").unwrap();
    fs::write(root.join("excluded.txt"), "unselected staged\n").unwrap();
    cli(&root, &["add", "excluded.txt"]);
    h.state_mut().app.git_ui.message = "Keep this commit draft".into();
    h.state_mut().app.git_open_shelve(vec!["app.js".into()]);
    // The readonly preview now requests the actual diff from the Git worker.
    settle(&mut h);
    h.get_by_label("Shelve Changes").click();
    settle(&mut h);
    h.get_by_label("Apply").click();
    settle(&mut h);
    assert_eq!(
        fs::read_to_string(root.join("app.js")).unwrap(),
        "-- original 😀\n"
    );
    assert_eq!(
        cli(&root, &["show", ":excluded.txt"]),
        "unselected staged\n"
    );
    let id = h.state().app.service.git.shelves[0].id.clone();
    h.state_mut()
        .app
        .git_prepare_shelf(codisseu_core::git::shelf::ShelfAction::Unshelve { id });
    settle(&mut h);
    h.get_by_label("Apply").click();
    settle(&mut h);
    assert_eq!(
        fs::read_to_string(root.join("app.js")).unwrap(),
        "working own layer\n"
    );
    assert_eq!(cli(&root, &["show", ":app.js"]), "staged own layer\n");
    assert_eq!(
        cli(&root, &["show", ":excluded.txt"]),
        "unselected staged\n"
    );
    assert_eq!(h.state().app.git_ui.message, "Keep this commit draft");
    assert_eq!(h.state().app.service.git.shelves.len(), 1);
}

#[test]
fn interactive_rebase_inline_message_blocks_start_until_confirmed() {
    let (_dir, root, mut h) = fixture();
    let base = cli(&root, &["rev-parse", "HEAD"]).trim().to_owned();
    for name in ["Alpha", "Bravo"] {
        fs::write(root.join(format!("{name}.txt")), name).unwrap();
        cli(&root, &["add", "."]);
        cli(&root, &["commit", "-m", name]);
    }
    h.state_mut().app.git_send(GitCommand::Refresh);
    settle(&mut h);
    h.state_mut().app.git_open_interactive_rebase(base.clone());
    settle(&mut h);
    h.get_by_label("Reword").click();
    h.run();
    assert!(
        h.get_by_label("Start Rebasing")
            .accesskit_node()
            .is_disabled()
    );
    replace_input(
        &mut h,
        "Rebase commit message",
        "Complete reword body\n\nPreserved second paragraph",
    );
    h.get_by_label("Done").click();
    h.run();
    assert!(
        !h.get_by_label("Start Rebasing")
            .accesskit_node()
            .is_disabled()
    );
    let Some(GitDialog::InteractiveRebase { steps, editing, .. }) = &h.state().app.git_ui.dialog
    else {
        panic!("rebase dialog lost")
    };
    assert!(!editing);
    assert_eq!(
        steps[0].message.as_deref(),
        Some("Complete reword body\n\nPreserved second paragraph")
    );
    h.get_by_label("Show commit diff").click();
    settle(&mut h);
    assert!(h.state().app.git_ui.diff_return.is_some());
    h.get_by_label("Close").click();
    h.run();
    assert!(
        matches!(&h.state().app.git_ui.dialog,Some(GitDialog::InteractiveRebase{steps,..}) if steps[0].message.as_deref()==Some("Complete reword body\n\nPreserved second paragraph"))
    );
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert_eq!(cli(&root, &["log", "-1", "--format=%s"]), "Bravo\n");
    fs::write(root.join("Charlie.txt"), "Charlie").unwrap();
    cli(&root, &["add", "Charlie.txt"]);
    cli(&root, &["commit", "-m", "Charlie"]);
    h.state_mut().app.git_open_interactive_rebase(base);
    settle(&mut h);
    let Some(GitDialog::InteractiveRebase { steps, .. }) = &h.state().app.git_ui.dialog else {
        panic!("reopened series missing")
    };
    assert_eq!(
        steps.len(),
        3,
        "reopening must consume a fresh worker response"
    );
}

#[path = "git_review_regressions.rs"]
mod review_regressions;

#[test]
fn compact_branch_tag_reset_forms_execute_and_preserve_cancelled_inputs() {
    let (_dir, root, mut h) = fixture();
    h.state_mut()
        .app
        .git_action(GitActionKind::NewBranch, String::new());
    h.run();
    replace_input(&mut h, "Branch Name", "feature/compact-form");
    h.get_by_label("Create").click();
    settle(&mut h);
    h.get_by_label("Cancel").click();
    h.run();
    assert!(
        matches!(&h.state().app.git_ui.dialog,Some(GitDialog::Action{kind:GitActionKind::NewBranch,other,check:true,..}) if other=="feature/compact-form")
    );
    h.get_by_label("Create").click();
    settle(&mut h);
    h.get_by_label("Apply").click();
    settle(&mut h);
    assert_eq!(
        cli(&root, &["branch", "--show-current"]),
        "feature/compact-form\n"
    );
    h.state_mut()
        .app
        .git_action(GitActionKind::NewTag, "HEAD".into());
    h.run();
    replace_input(&mut h, "Tag Name", "v0.1-compact");
    replace_input(
        &mut h,
        "Tag message",
        "Full tag annotation\n\nSecond paragraph",
    );
    h.get_by_label("Create Tag").click();
    settle(&mut h);
    h.get_by_label("Apply").click();
    settle(&mut h);
    assert!(
        cli(&root, &["cat-file", "-p", "refs/tags/v0.1-compact"])
            .contains("Full tag annotation\n\nSecond paragraph")
    );
    h.state_mut()
        .app
        .git_action(GitActionKind::Reset, "HEAD".into());
    h.run();
    assert!(matches!(
        &h.state().app.git_ui.dialog,
        Some(GitDialog::Action {
            kind: GitActionKind::Reset,
            mode: 1,
            ..
        })
    ));
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert_eq!(cli(&root, &["status", "--porcelain"]), "");
}

#[path = "git_shelf_visual_tests.rs"]
mod shelf_visual_tests;

#[path = "git_scm_tests.rs"]
mod scm_tests;

#[path = "git_scm_routing_tests.rs"]
mod scm_routing_tests;

#[path = "git_merge_tests.rs"]
mod merge_tests;
