//! Source Control flows operate only on disposable real repositories.
use super::*;

fn open_scm(h: &mut GitHarness) {
    h.state_mut().app.git_open_source_control();
    settle(h);
}
fn options(h: &mut GitHarness, root: &Path) {
    h.get_by_label(&format!("Commit options {}", root.display()))
        .click();
    h.run();
}
fn second_repo(path: &Path) {
    fs::create_dir(path).unwrap();
    cli(path, &["init", "-b", "main"]);
    for (key, value) in [
        ("user.name", "Second Fixture"),
        ("user.email", "second@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", "/dev/null"),
    ] {
        cli(path, &["config", key, value]);
    }
    fs::write(path.join("other.txt"), "second initial\n").unwrap();
    cli(path, &["add", "other.txt"]);
    cli(path, &["commit", "-m", "Second initial"]);
}

#[test]
fn workspace_file_first_non_git_folder_shows_only_declared_external_repositories() {
    let (dir, first, mut h) = fixture();
    let second = dir.path().join("external");
    second_repo(&second);
    let config = dir.path().join("config");
    fs::create_dir(&config).unwrap();
    fs::create_dir(config.join("documents")).unwrap();
    let excluded = config.join("excluded");
    second_repo(&excluded);
    let file = config.join("team.code-workspace");
    fs::write(&file,r#"{"folders":[{"name":"Documentos","path":"documents"},{"name":"API","path":"../repository"},{"name":"Frontend","path":"../external"},{"path":"missing"}]}"#).unwrap();
    let bytes = fs::read(&file).unwrap();
    h.state_mut()
        .app
        .service
        .dispatch(Command::OpenWorkspace(file.clone()))
        .unwrap();
    settle(&mut h);
    open_scm(&mut h);
    assert!(
        h.state().app.service.git.error.is_none(),
        "{:?}",
        h.state().app.service.git.error
    );
    assert_eq!(h.state().app.service.git.repositories.len(), 2);
    assert!(h.query_by_label("Repositories (2)").is_some());
    for root in [&first, &second] {
        assert!(
            h.query_by_label(&format!("Repository {}", root.display()))
                .is_some()
        );
    }
    assert!(
        h.query_by_label(&format!("Repository {}", excluded.display()))
            .is_none()
    );
    h.get_by_label(&format!("Repository {}", second.display()))
        .click();
    settle(&mut h);
    assert_eq!(
        h.state().app.service.git_selected_repository(),
        Some(second.as_path())
    );
    assert!(!h.state().app.service.git.trusted);
    if let Some(out) = std::env::var_os("CODISSEU_WORKSPACE_GIT_CAPTURES") {
        fs::create_dir_all(&out).unwrap();
        h.render()
            .unwrap()
            .save(PathBuf::from(out).join("declared-repositories.png"))
            .unwrap();
    }
    assert_eq!(fs::read(file).unwrap(), bytes);
    assert_eq!(cli(&first, &["status", "--porcelain"]), "");
    assert_eq!(cli(&second, &["status", "--porcelain"]), "");
    assert_eq!(cli(&excluded, &["status", "--porcelain"]), "");
}

#[test]
fn scm_multirepo_selection_routes_and_restores_each_commit_draft() {
    let (dir, first, mut h) = fixture();
    let second = dir.path().join("second-repository");
    second_repo(&second);
    h.state_mut()
        .app
        .service
        .dispatch(Command::OpenWorkspace(dir.path().into()))
        .unwrap();
    settle(&mut h);
    open_scm(&mut h);
    assert_eq!(h.state().app.service.git.repositories.len(), 2);
    h.get_by_label(&format!("Repository {}", first.display()))
        .click();
    settle(&mut h);
    replace_input(
        &mut h,
        &format!("Commit message {}", first.display()),
        "First repository draft 😀",
    );
    settle(&mut h);
    h.get_by_label(&format!("Repository {}", second.display()))
        .click();
    settle(&mut h);
    assert_eq!(
        h.state().app.service.git.repo.as_ref().unwrap().root,
        second
    );
    replace_input(
        &mut h,
        &format!("Commit message {}", second.display()),
        "Second repository draft",
    );
    settle(&mut h);
    h.get_by_label(&format!("Repository {}", first.display()))
        .click();
    settle(&mut h);
    assert_eq!(h.state().app.service.git.repo.as_ref().unwrap().root, first);
    assert_eq!(h.state().app.git_ui.message, "First repository draft 😀");
    assert_eq!(
        h.state().app.service.git_draft(&second),
        Some("Second repository draft")
    );
    assert_eq!(
        cli(&first, &["log", "-1", "--format=%s"]),
        "Initial fixture message\n"
    );
    assert_eq!(
        cli(&second, &["log", "-1", "--format=%s"]),
        "Second initial\n"
    );
}

#[test]
fn scm_multi_project_keeps_healthy_sections_and_exposes_local_discovery_errors() {
    let (dir, first, mut h) = fixture();
    let group = dir.path().join("billing");
    fs::create_dir(&group).unwrap();
    let second = group.join("repository");
    second_repo(&second);
    fs::write(second.join("other.txt"), "modified sibling\n").unwrap();
    let broken = dir.path().join("broken-service");
    fs::create_dir(&broken).unwrap();
    fs::write(broken.join(".git"), "gitdir: missing-own-fixture\n").unwrap();
    h.state_mut()
        .app
        .service
        .dispatch(Command::OpenWorkspace(dir.path().into()))
        .unwrap();
    settle(&mut h);
    open_scm(&mut h);
    h.set_size(egui::vec2(640., 1100.));
    h.run();
    assert_eq!(h.state().app.service.git.repositories.len(), 2);
    assert_eq!(h.state().app.service.git.discovery_errors.len(), 1);
    assert!(
        h.query_by_label(&format!("Repository {}", first.display()))
            .is_some()
    );
    assert!(
        h.query_by_label(&format!("Repository {}", second.display()))
            .is_some()
    );
    assert!(h.query_by_label("Repositories (2)").is_some());
    let first_index = cli(&first, &["write-tree"]);
    let second_index = cli(&second, &["write-tree"]);
    if let Some(output) = std::env::var_os("CODISSEU_SCM_VISUAL_CAPTURE_DIR") {
        let output = PathBuf::from(output);
        fs::create_dir_all(&output).unwrap();
        h.render()
            .unwrap()
            .save(output.join("multi-project-healthy-sections.png"))
            .unwrap();
    }
    h.get_by_label("Source Control actions").click();
    h.run();
    h.get_by_label("Repository discovery errors (1)").click();
    h.run();
    assert!(h.query_by_label(&broken.display().to_string()).is_some());
    if let Some(output) = std::env::var_os("CODISSEU_SCM_VISUAL_CAPTURE_DIR") {
        h.render()
            .unwrap()
            .save(PathBuf::from(output).join("multi-project-local-error.png"))
            .unwrap();
    }
    assert_eq!(cli(&first, &["write-tree"]), first_index);
    assert_eq!(cli(&second, &["write-tree"]), second_index);
}

#[test]
fn scm_stage_unstage_require_review_and_cancel_never_changes_index() {
    let (_dir, root, mut h) = fixture();
    change_file(&root, &mut h);
    open_scm(&mut h);
    h.get_by_label("Stage app.js").click();
    settle(&mut h);
    assert!(cli(&root, &["diff", "--cached", "--name-only"]).is_empty());
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert!(cli(&root, &["diff", "--cached", "--name-only"]).is_empty());
    h.get_by_label("Stage app.js").click();
    settle(&mut h);
    h.get_by_label("Apply").click();
    settle(&mut h);
    assert_eq!(cli(&root, &["diff", "--cached", "--name-only"]), "app.js\n");
    h.get_by_label("Unstage app.js").click();
    settle(&mut h);
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert_eq!(cli(&root, &["diff", "--cached", "--name-only"]), "app.js\n");
    h.get_by_label("Unstage app.js").click();
    settle(&mut h);
    h.get_by_label("Apply").click();
    settle(&mut h);
    assert!(cli(&root, &["diff", "--cached", "--name-only"]).is_empty());
    assert_eq!(
        fs::read_to_string(root.join("app.js")).unwrap(),
        "++ edited 😀\n"
    );
}

#[test]
fn scm_commit_staged_with_coauthor_and_signoff_keeps_unstaged_file() {
    let (_dir, root, mut h) = fixture();
    fs::write(root.join("app.js"), "selected staged\n").unwrap();
    cli(&root, &["add", "app.js"]);
    fs::write(root.join("unselected.txt"), "keep unstaged\n").unwrap();
    h.state_mut().app.git_send(GitCommand::Refresh);
    settle(&mut h);
    open_scm(&mut h);
    replace_input(
        &mut h,
        &format!("Commit message {}", root.display()),
        "SCM reviewed commit",
    );
    options(&mut h, &root);
    h.get_by_label("Add Co-authors…").click();
    settle(&mut h);
    replace_input(
        &mut h,
        "Co-authors",
        "Alice Example <alice@example.invalid>",
    );
    h.get_by_label("Use Message").click();
    settle(&mut h);
    assert!(
        h.state()
            .app
            .git_ui
            .message
            .contains("Co-authored-by: Alice Example <alice@example.invalid>")
    );
    options(&mut h, &root);
    h.get_by_label("Add Signed-off-by").click();
    h.key_press(Key::Escape);
    settle(&mut h);
    let head = cli(&root, &["rev-parse", "HEAD"]);
    h.get_by_label("Commit Staged Changes").click();
    settle(&mut h);
    assert_eq!(cli(&root, &["rev-parse", "HEAD"]), head);
    h.get_by_label("Apply").click();
    settle(&mut h);
    let message = cli(&root, &["log", "-1", "--format=%B"]);
    assert!(message.contains("SCM reviewed commit"));
    assert!(message.contains("Co-authored-by: Alice Example <alice@example.invalid>"));
    assert!(message.contains("Signed-off-by: Codisseu UI Fixture <fixture@example.invalid>"));
    assert_eq!(cli(&root, &["show", "HEAD:app.js"]), "selected staged\n");
    assert_eq!(
        cli(&root, &["status", "--porcelain=v1"]),
        "?? unselected.txt\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("unselected.txt")).unwrap(),
        "keep unstaged\n"
    );
    assert!(h.state().app.git_ui.message.is_empty());
}

#[test]
fn scm_remote_form_cancel_and_apply_modify_only_local_reviewed_configuration() {
    let (_dir, root, mut h) = fixture();
    open_scm(&mut h);
    // Open the same real form exposed by the Remote submenu; the initial action
    // is selected explicitly to avoid mouse-hover submenu timing in this test.
    h.state_mut().app.git_ui.dialog = Some(GitDialog::Scm(crate::git_scm::ScmDialog {
        form: crate::git_scm::ScmForm::RemoteAdd,
        name: String::new(),
        value: String::new(),
        check: false,
    }));
    h.run();
    replace_input(&mut h, "Remote name", "review-local");
    replace_input(
        &mut h,
        "Remote URL",
        "/tmp/codisseu-fixture-never-contacted.git",
    );
    h.get_by_label("Review").click();
    settle(&mut h);
    assert!(cli(&root, &["remote"]).is_empty());
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert!(matches!(
        h.state().app.git_ui.dialog,
        Some(GitDialog::Scm(_))
    ));
    assert!(cli(&root, &["remote"]).is_empty());
    h.get_by_label("Review").click();
    settle(&mut h);
    h.get_by_label("Apply").click();
    settle(&mut h);
    assert_eq!(
        cli(&root, &["remote", "get-url", "review-local"]),
        "/tmp/codisseu-fixture-never-contacted.git\n"
    );
    assert_eq!(cli(&root, &["status", "--porcelain=v1"]), "");
    assert!(
        h.state()
            .app
            .service
            .git
            .remotes
            .iter()
            .any(|r| r.name == "review-local"),
        "Successful remote configuration must refresh the choices exposed by the next Remote form"
    );
    h.state_mut().app.git_ui.dialog = Some(GitDialog::Scm(crate::git_scm::ScmDialog {
        form: crate::git_scm::ScmForm::RemoteUrl,
        name: "review-local".into(),
        value: "/tmp/proposed-new-url.git".into(),
        check: false,
    }));
    h.run();
    h.get_by_label("Review").click();
    settle(&mut h);
    cli(
        &root,
        &[
            "remote",
            "set-url",
            "review-local",
            "/tmp/changed-after-review.git",
        ],
    );
    h.get_by_label("Apply").click();
    settle(&mut h);
    assert_eq!(
        cli(&root, &["remote", "get-url", "review-local"]),
        "/tmp/changed-after-review.git\n",
        "Changed remote configuration must invalidate an older approval"
    );
    assert!(h.state().app.service.git.error.is_some());
}

#[test]
fn scm_discard_untracked_cancel_preserves_bytes_and_apply_keeps_durable_backup() {
    let (_dir, root, mut h) = fixture();
    let selected = "--new ação.bin";
    let bytes = b"owned\0binary\xff\n";
    fs::write(root.join(selected), bytes).unwrap();
    fs::write(root.join("keep-untracked.txt"), "unselected new file\n").unwrap();
    fs::write(root.join("app.js"), "tracked working changes stay 😀\n").unwrap();
    let index_before = cli(&root, &["write-tree"]);
    h.state_mut().app.git_send(GitCommand::Refresh);
    settle(&mut h);
    open_scm(&mut h);
    let row = format!("SCM file working {selected}");
    h.get_by_label(&row).click_secondary();
    h.run();
    h.get_by_label("Discard Changes…").click();
    settle(&mut h);
    assert_eq!(fs::read(root.join(selected)).unwrap(), bytes);
    let recovery = root.join(".git/codisseu/untracked-recovery");
    assert!(
        !recovery.exists(),
        "A read-only preview must not create a backup or remove a file"
    );
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert_eq!(fs::read(root.join(selected)).unwrap(), bytes);
    assert_eq!(cli(&root, &["write-tree"]), index_before);
    h.get_by_label(&row).click_secondary();
    h.run();
    h.get_by_label("Discard Changes…").click();
    settle(&mut h);
    h.get_by_label("Apply").click();
    settle(&mut h);
    assert!(!root.join(selected).exists());
    assert_eq!(
        fs::read_to_string(root.join("app.js")).unwrap(),
        "tracked working changes stay 😀\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("keep-untracked.txt")).unwrap(),
        "unselected new file\n"
    );
    assert_eq!(cli(&root, &["write-tree"]), index_before);
    let batches = fs::read_dir(&recovery)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(batches.len(), 1);
    assert_eq!(fs::read(batches[0].join("0.bin")).unwrap(), bytes);
    assert_eq!(fs::read(batches[0].join("0.removed")).unwrap(), bytes);
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(batches[0].join("metadata.json")).unwrap()).unwrap();
    assert_eq!(metadata["entries"].as_array().unwrap().len(), 1);
    assert!(
        metadata["entries"][0]["sha256"]
            .as_str()
            .is_some_and(|hash| hash.len() == 64)
    );
    assert!(
        h.state().app.git_ui.output.contains("untracked-recovery"),
        "The user must receive the durable recovery location"
    );
}

#[test]
fn scm_sync_pulls_then_opens_fresh_push_review_without_publishing() {
    let (dir, root, mut h) = fixture();
    let remote = dir.path().join("sync-fixture.git");
    cli(&root, &["init", "--bare", remote.to_str().unwrap()]);
    cli(&root, &["config", "protocol.file.allow", "always"]);
    cli(
        &root,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    cli(&root, &["push", "-u", "origin", "main"]);
    let published = cli(&remote, &["rev-parse", "refs/heads/main"]);
    fs::write(root.join("app.js"), "const localOnly = true;\n").unwrap();
    cli(&root, &["add", "app.js"]);
    cli(&root, &["commit", "-m", "Local commit waiting for review"]);
    h.state_mut().app.git_send(GitCommand::Refresh);
    settle(&mut h);
    open_scm(&mut h);
    h.state_mut().app.git_ui.message = "Unrelated future commit draft".into();
    h.get_by_label("Sync Changes 0↓ 1↑").click();
    settle(&mut h);
    h.get_by_label("Review").click();
    settle(&mut h);
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert_eq!(cli(&remote, &["rev-parse", "refs/heads/main"]), published);
    assert!(matches!(
        h.state().app.git_ui.dialog,
        Some(GitDialog::Scm(_))
    ));
    h.get_by_label("Review").click();
    settle(&mut h);
    h.get_by_label("Apply").click();
    settle(&mut h);
    assert!(matches!(
        h.state().app.git_ui.dialog,
        Some(GitDialog::Push { .. })
    ));
    assert_eq!(
        h.state()
            .app
            .service
            .git
            .push_review
            .as_ref()
            .unwrap()
            .commits
            .len(),
        1
    );
    assert_eq!(cli(&remote, &["rev-parse", "refs/heads/main"]), published);
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert_eq!(cli(&remote, &["rev-parse", "refs/heads/main"]), published);
    assert_eq!(
        h.state().app.git_ui.message,
        "Unrelated future commit draft"
    );
    h.state_mut().app.send(Command::SetGitConfirmActions(false));
    h.get_by_label("Sync Changes 0↓ 1↑").click();
    settle(&mut h);
    assert!(h.state().app.git_ui.dialog.is_none());
    assert_eq!(
        cli(&remote, &["rev-parse", "refs/heads/main"]),
        cli(&root, &["rev-parse", "HEAD"])
    );
    assert_eq!(
        h.state().app.git_ui.message,
        "Unrelated future commit draft"
    );
}

#[test]
fn scm_compact_two_repository_lists_share_real_status_and_keep_row_actions_accessible() {
    let (dir, first, mut h) = fixture();
    let second = dir.path().join("vscode-secondary");
    second_repo(&second);
    for (root, extra) in [(&first, true), (&second, false)] {
        fs::create_dir(root.join("docs")).unwrap();
        fs::create_dir(root.join("src")).unwrap();
        for (path, text) in [
            ("docs/old-name.txt", "rename me\n"),
            ("docs/obsolete.txt", "delete me\n"),
            ("src/greeting.js", "export const greeting = 'old';\n"),
            ("src/Greeting.php", "<?php echo 'old';\n"),
        ] {
            fs::write(root.join(path), text).unwrap();
        }
        cli(root, &["add", "docs", "src"]);
        cli(root, &["commit", "-m", "SCM compact fixture"]);
        cli(root, &["mv", "docs/old-name.txt", "docs/greeting.txt"]);
        cli(root, &["rm", "docs/obsolete.txt"]);
        fs::write(
            root.join("src/greeting.js"),
            "export const greeting = 'new';\n",
        )
        .unwrap();
        fs::write(root.join("src/Greeting.php"), "<?php echo 'new';\n").unwrap();
        fs::write(root.join("notes.txt"), "new notes\n").unwrap();
        if extra {
            fs::write(root.join("src/Greeter.ts"), "export class Greeter {}\n").unwrap();
            fs::write(root.join("search-fixture.txt"), "search evidence\n").unwrap();
        }
    }
    h.state_mut()
        .app
        .service
        .dispatch(Command::OpenWorkspace(dir.path().into()))
        .unwrap();
    settle(&mut h);
    h.set_size(egui::vec2(298., 860.));
    open_scm(&mut h);
    h.get_by_label(&format!("Repository {}", first.display()))
        .click();
    settle(&mut h);
    let root_label = format!("Repository {}", first.display());
    let root_row = h.get_by_label(&root_label);
    assert!(root_row.accesskit_node().is_selected().unwrap_or(false));
    assert_eq!(root_row.rect().height(), 26.);
    assert_eq!(
        h.get_by_label(&format!("Commit message {}", first.display()))
            .rect()
            .height(),
        28.
    );
    let commits = h
        .get_all_by_label("Commit Staged Changes")
        .collect::<Vec<_>>();
    assert_eq!(commits.len(), 2);
    assert!(commits.iter().all(|button| button.rect().height() == 28.));
    assert_eq!(
        h.get_by_label("SCM file working src/greeting.js")
            .rect()
            .height(),
        22.
    );
    assert!(
        h.query_by_label(&format!(
            "SCM file working src/greeting.js in {}",
            second.display()
        ))
        .is_some(),
        "Inactive repositories show their real cached files too"
    );
    // The mouse is over the repo header: action widgets remain present even
    // when their vector is intentionally painted only on hover/focus.
    assert!(h.query_by_label("Stage src/greeting.js").is_some());
    assert!(
        h.query_by_label(&format!("Stage src/greeting.js in {}", second.display()))
            .is_some()
    );
    if let Some(output) = std::env::var_os("CODISSEU_SCM_VISUAL_CAPTURE_DIR") {
        let output = PathBuf::from(output);
        fs::create_dir_all(&output).unwrap();
        h.render()
            .unwrap()
            .save(output.join("source-control-compact-two-repos.png"))
            .unwrap();
    }
    assert!(h.query_by_label("Repositories (2)").is_some());
    h.get_by_label("Changes section").click();
    settle(&mut h);
    assert!(h.query_by_label(&root_label).is_none());
    h.get_by_label("Changes section").click();
    settle(&mut h);
    assert!(h.query_by_label(&root_label).is_some());
    assert!(
        h.query_by_label("SCM file working src/greeting.js")
            .is_some()
    );
    h.get_by_label("Collapse All Repositories").click();
    settle(&mut h);
    assert!(h.query_by_label(&root_label).is_some());
    assert!(
        h.query_by_label("SCM file working src/greeting.js")
            .is_none()
    );
    assert!(
        h.query_by_label(&format!("Commit message {}", second.display()))
            .is_none()
    );
    assert_eq!(
        cli(&first, &["diff", "--cached", "--name-status"]),
        "R100\tdocs/old-name.txt\tdocs/greeting.txt\nD\tdocs/obsolete.txt\n"
    );
    assert_eq!(
        cli(&second, &["diff", "--cached", "--name-status"]),
        "R100\tdocs/old-name.txt\tdocs/greeting.txt\nD\tdocs/obsolete.txt\n"
    );
}

#[test]
fn workspace_initialize_form_defaults_to_declared_directory() {
    let (dir, _, mut h) = fixture();
    let config = dir.path().join("workspace-config");
    let project = dir.path().join("new-project");
    fs::create_dir(&config).unwrap();
    fs::create_dir(&project).unwrap();
    let file = config.join("team.code-workspace");
    fs::write(&file, r#"{"folders":[{"path":"../new-project"}]}"#).unwrap();
    h.state_mut()
        .app
        .service
        .dispatch(Command::OpenWorkspace(file))
        .unwrap();
    settle(&mut h);
    open_scm(&mut h);
    h.get_by_label("Initialize Repository").click();
    h.run();
    let Some(GitDialog::Action {
        kind: GitActionKind::Init,
        name,
        ..
    }) = &h.state().app.git_ui.dialog
    else {
        panic!("Initialize form missing");
    };
    assert_eq!(name, &project.display().to_string());
    assert!(!project.join(".git").exists());
    assert!(!config.join(".git").exists());
}

#[test]
fn scm_progress_line_stays_under_active_repository_without_moving_repositories_or_toolbar() {
    for scale in [1., 1.25] {
        for width in [230., 298.] {
            let (_dir, root, mut h) = fixture_at_scale(scale);
            h.state_mut().show_branch = true;
            h.set_size(egui::vec2(width, 640.));
            open_scm(&mut h);
            let labels = [
                "Changes section".to_owned(),
                "Repositories (1)".into(),
                "Source Control actions".into(),
                "Collapse All Repositories".into(),
                format!("Repository {}", root.display()),
            ];
            let before = labels
                .iter()
                .map(|label| h.get_by_label(label).rect())
                .collect::<Vec<_>>();
            assert!(h.query_by_role(Role::ProgressIndicator).is_none());
            let head = cli(&root, &["rev-parse", "HEAD"]);
            let index = fs::read(root.join(".git/index")).unwrap();
            h.state_mut().app.git_send_foreground(GitCommand::Refresh);
            h.state_mut().app.poll_git_ui();
            h.state_mut().pause_delivery = true;
            for _ in 0..3 {
                h.step();
            }
            assert!(h.state().app.service.git.is_busy());
            assert_eq!(h.get_all_by_role(Role::ProgressIndicator).count(), 1);
            let spinner = h.get_by_label("Atualizando Git").rect();
            let repository = h
                .get_by_label(&format!("Repository {}", root.display()))
                .rect();
            assert_eq!(spinner.height(), 2.);
            assert_eq!(spinner.bottom(), repository.bottom());
            assert!(
                spinner.left() >= repository.left() + 18. && spinner.right() <= repository.right()
            );
            for (label, rect) in labels.iter().zip(&before) {
                assert_eq!(
                    h.get_by_label(label).rect(),
                    *rect,
                    "busy {label} scale={scale} width={width}"
                );
            }
            if let Some(out) = std::env::var_os("CODISSEU_SCM_PROGRESS_CAPTURES") {
                fs::create_dir_all(&out).unwrap();
                h.render()
                    .unwrap()
                    .save(PathBuf::from(&out).join(format!("busy-{scale}-{width}.png")))
                    .unwrap();
            }
            h.state_mut().pause_delivery = false;
            settle(&mut h);
            assert!(h.query_by_role(Role::ProgressIndicator).is_none());
            for (label, rect) in labels.iter().zip(&before) {
                assert_eq!(
                    h.get_by_label(label).rect(),
                    *rect,
                    "idle {label} scale={scale} width={width}"
                );
            }
            if let Some(out) = std::env::var_os("CODISSEU_SCM_PROGRESS_CAPTURES") {
                h.render()
                    .unwrap()
                    .save(PathBuf::from(&out).join(format!("idle-{scale}-{width}.png")))
                    .unwrap();
            }
            assert_eq!(cli(&root, &["rev-parse", "HEAD"]), head);
            assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
            assert_eq!(cli(&root, &["status", "--porcelain"]), "");
        }
    }
}

#[test]
fn scm_automatic_queue_status_and_hover_prefetch_never_show_progress() {
    for scale in [1., 1.25] {
        let (_dir, root, mut h) = fixture_at_scale(scale);
        open_scm(&mut h);
        let head = cli(&root, &["rev-parse", "HEAD"]);
        let index = fs::read(root.join(".git/index")).unwrap();
        h.state_mut().pause_delivery = true;
        h.state_mut()
            .app
            .service
            .git
            .dispatch(GitCommand::Refresh)
            .unwrap();
        h.state_mut().app.git_send(GitCommand::Branches);
        h.state_mut().app.git_send(GitCommand::Tags);
        h.state_mut().app.git_insights.details.prefetch_commit(
            root.clone(),
            head.clone(),
            "app.js".into(),
            1,
        );
        assert!(h.state().app.service.git.is_busy());
        assert!(h.state().app.git_insights.is_busy());
        h.state_mut().app.git_insights.details.open_history(
            root.clone(),
            root.join("app.js"),
            1,
            std::sync::Arc::from(fs::read_to_string(root.join("app.js")).unwrap()),
            1,
            1,
        );
        assert!(
            h.state().app.git_activity_busy(),
            "explicit history lookup is foreground"
        );
        h.state_mut()
            .app
            .git_insights
            .details
            .silence_automatic_history_progress();
        for _ in 0..4 {
            h.step();
            assert!(h.query_by_role(Role::ProgressIndicator).is_none());
        }
        if let Some(out) = std::env::var_os("CODISSEU_SCM_PROGRESS_CAPTURES") {
            fs::create_dir_all(&out).unwrap();
            h.render()
                .unwrap()
                .save(PathBuf::from(out).join(format!("background-silent-{scale}.png")))
                .unwrap();
        }
        h.state_mut().pause_delivery = false;
        settle(&mut h);
        assert_eq!(cli(&root, &["rev-parse", "HEAD"]), head);
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    }
}

#[test]
fn scm_foreground_ack_cannot_lend_its_progress_to_a_following_background_read() {
    let (_dir, _root, mut h) = fixture();
    open_scm(&mut h);
    h.state_mut().app.git_send_foreground(GitCommand::Refresh);
    h.state_mut().app.poll_git_ui();
    h.state_mut().pause_delivery = true;
    h.step();
    assert!(h.query_by_role(Role::ProgressIndicator).is_some());
    let marker = h.state().app.git_ui.foreground_request.unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    while h.state().app.service.git.is_busy() {
        h.state_mut().app.service.poll();
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(2));
    }
    assert_ne!(h.state().app.service.git.revision, marker.1);
    // A passive worker request can begin before UI observes the previous ACK.
    h.state_mut()
        .app
        .service
        .git
        .dispatch(GitCommand::Refresh)
        .unwrap();
    h.step();
    assert!(h.state().app.service.git.is_busy());
    assert!(h.query_by_role(Role::ProgressIndicator).is_none());
    h.state_mut().app.git_send_foreground(GitCommand::Refresh);
    h.step();
    assert!(
        h.query_by_role(Role::ProgressIndicator).is_some(),
        "a real user request waiting behind that read must still be visible"
    );
    h.state_mut().pause_delivery = false;
    settle(&mut h);
    assert!(h.query_by_role(Role::ProgressIndicator).is_none());
    h.state_mut().app.git_send_foreground(GitCommand::Refresh);
    h.state_mut().app.poll_git_ui();
    h.state_mut().pause_delivery = true;
    h.step();
    assert!(h.query_by_role(Role::ProgressIndicator).is_some());
    h.state_mut().app.send(Command::CancelAll);
    h.step();
    assert!(h.query_by_role(Role::ProgressIndicator).is_none());
    h.state_mut().pause_delivery = false;
    settle(&mut h);
}

#[cfg(unix)]
#[test]
fn scm_repositories_stage_and_commit_independently_while_another_commit_waits() {
    use std::os::unix::fs::PermissionsExt;
    for scale in [1., 1.25] {
        let (dir, first, mut h) = fixture_at_scale(scale);
        let second = dir.path().join("second");
        second_repo(&second);
        fs::write(first.join("app.js"), "first changed\n").unwrap();
        fs::write(second.join("other.txt"), "second changed\n").unwrap();
        cli(&first, &["add", "."]);
        cli(&second, &["add", "."]);
        h.state_mut()
            .app
            .service
            .dispatch(Command::OpenWorkspace(dir.path().into()))
            .unwrap();
        settle(&mut h);
        assert_eq!(h.state().app.service.workspace_path(), Some(dir.path()));
        h.state_mut().app.send(Command::SetGitConfirmActions(false));
        open_scm(&mut h);
        h.get_by_label(&format!("Repository {}", first.display()))
            .click();
        settle(&mut h);
        replace_input(
            &mut h,
            &format!("Commit message {}", first.display()),
            "Commit first",
        );
        replace_input(
            &mut h,
            &format!("Commit message {}", second.display()),
            "Commit second",
        );
        settle(&mut h);
        // The fixture hook waits for an explicit gate, not a timing assumption.
        let hook = first.join(".git/fixture-hooks/pre-commit");
        fs::write(&hook, "#!/bin/sh\ntouch .git/hook-entered\nwhile [ ! -f .git/hook-release ]; do sleep 0.05; done\n").unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        struct Release(PathBuf);
        impl Drop for Release {
            fn drop(&mut self) {
                let _ = fs::write(&self.0, "release");
            }
        }
        let gate = Release(first.join(".git/hook-release"));
        let first_head = cli(&first, &["rev-parse", "HEAD"]);
        h.get_by_label(&format!("Commit repository {}", first.display()))
            .click();
        let deadline = Instant::now() + Duration::from_secs(15);
        while !first.join(".git/hook-entered").exists() {
            h.step();
            assert!(
                Instant::now() < deadline,
                "first hook never started: {} {:?}",
                h.state().app.git_ui.notice,
                h.state().app.service.git.error
            );
            thread::sleep(Duration::from_millis(2));
        }
        h.step();
        assert_eq!(h.get_all_by_role(Role::ProgressIndicator).count(), 1);
        let buttons = h
            .get_all_by_label("Commit Staged Changes")
            .collect::<Vec<_>>();
        assert_eq!(buttons.len(), 2);
        assert_eq!(
            buttons
                .iter()
                .filter(|b| b.accesskit_node().is_disabled())
                .count(),
            1,
            "only the repository running a commit can be disabled"
        );
        h.get_by_label(&format!("Commit repository {}", second.display()))
            .click();
        let deadline = Instant::now() + Duration::from_secs(15);
        while cli(&second, &["log", "-1", "--format=%s"]).trim() != "Commit second" {
            h.step();
            assert!(
                Instant::now() < deadline,
                "second commit blocked by first: {} {:?}",
                h.state().app.git_ui.notice,
                h.state().app.service.git.error
            );
            thread::sleep(Duration::from_millis(2));
        }
        // B really committed while A remains blocked; merely enabling B is insufficient.
        assert!(!gate.0.exists());
        assert_eq!(cli(&first, &["rev-parse", "HEAD"]), first_head);
        assert!(
            h.state()
                .app
                .service
                .git_repository_service(&first)
                .unwrap()
                .is_busy()
        );
        assert_eq!(
            h.state().app.service.git_draft(&first),
            Some("Commit first")
        );
        if let Some(out) = std::env::var_os("CODISSEU_GIT_ISOLATION_CAPTURES") {
            fs::create_dir_all(&out).unwrap();
            h.render()
                .unwrap()
                .save(PathBuf::from(out).join(format!("second-finished-first-working-{scale}.png")))
                .unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while h.state().app.git_repository_controls_busy(&second) {
            h.step();
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        assert!(
            h.state_mut()
                .app
                .service
                .dispatch(Command::OpenWorkspace(second.clone()))
                .is_err()
        );
        assert!(
            h.state_mut()
                .app
                .service
                .dispatch(Command::CloseWorkspace)
                .is_err()
        );
        assert!(
            h.state_mut()
                .app
                .service
                .dispatch(Command::OpenFile(first.join("app.js")))
                .is_err()
        );
        h.state_mut()
            .app
            .service
            .dispatch(Command::OpenFile(second.join("other.txt")))
            .unwrap();
        // Stage in B is also available while A's commit still waits.
        fs::write(second.join("other.txt"), "third change\n").unwrap();
        h.state_mut().app.git_send(GitCommand::Refresh);
        let deadline = Instant::now() + Duration::from_secs(15);
        while h.state().app.service.git.is_busy() || !h.state().app.git_ui.queue.is_empty() {
            h.step();
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        h.step();
        h.get_by_label("Stage other.txt").click();
        let deadline = Instant::now() + Duration::from_secs(15);
        while cli(&second, &["diff", "--cached", "--name-only"]).trim() != "other.txt" {
            h.step();
            assert!(
                Instant::now() < deadline,
                "stage B blocked: {}",
                h.state().app.git_ui.notice
            );
            thread::sleep(Duration::from_millis(2));
        }
        assert!(!gate.0.exists());
        drop(gate);
        settle(&mut h);
        assert_eq!(
            cli(&first, &["log", "-1", "--format=%s"]).trim(),
            "Commit first"
        );
        h.get_by_label(&format!("Repository {}", first.display()))
            .click();
        settle(&mut h);
        assert!(h.state().app.git_ui.message.is_empty());
        assert!(h.state().app.service.git_draft(&first).is_none());
        assert!(h.query_by_role(Role::ProgressIndicator).is_none());
        assert_eq!(
            cli(&second, &["diff", "--cached", "--name-only"]).trim(),
            "other.txt"
        );
    }
}

#[test]
fn scm_rapid_actions_in_three_cold_repositories_keep_each_pending_target() {
    let (dir, first, mut h) = fixture();
    let second = dir.path().join("second");
    let third = dir.path().join("third");
    second_repo(&second);
    second_repo(&third);
    cli(&third, &["mv", "other.txt", "third.txt"]);
    cli(&third, &["commit", "-m", "Third filename"]);
    fs::write(first.join("app.js"), "changed first\n").unwrap();
    fs::write(second.join("other.txt"), "changed second\n").unwrap();
    fs::write(third.join("third.txt"), "changed third\n").unwrap();
    h.state_mut()
        .app
        .service
        .dispatch(Command::OpenWorkspace(dir.path().into()))
        .unwrap();
    settle(&mut h);
    assert_eq!(h.state().app.service.workspace_path(), Some(dir.path()));
    h.state_mut().app.send(Command::SetGitConfirmActions(false));
    open_scm(&mut h);
    h.state_mut().pause_delivery = true;
    for (root, name) in [
        (&first, "app.js"),
        (&second, "other.txt"),
        (&third, "third.txt"),
    ] {
        h.get_by_label(&format!("Stage {name} in {}", root.display()))
            .click();
        h.step();
        h.run(); // settle header/error layout; worker ACK delivery is still paused
        assert_eq!(
            h.state().app.service.git_repository_target(),
            Some(root.as_path())
        );
        assert!(
            h.state().app.git_ui.scm.has_pending_stage(),
            "cold stage gesture must survive its row selection"
        );
    }
    for root in [&first, &second, &third] {
        assert_eq!(cli(root, &["diff", "--cached", "--name-only"]), "");
    }
    assert!(h.state().app.git_repository_uis_pending());
    h.state_mut().pause_delivery = false;
    settle(&mut h);
    for (root, name) in [
        (&first, "app.js"),
        (&second, "other.txt"),
        (&third, "third.txt"),
    ] {
        assert_eq!(
            cli(root, &["diff", "--cached", "--name-only"]).trim(),
            name,
            "the selection must not replace an earlier repository's pending action: service={:?}; report={:?}; snapshot={:?}; view={}",
            h.state()
                .app
                .service
                .git_repository_service(root)
                .unwrap()
                .error,
            h.state()
                .app
                .service
                .git_repository_service(root)
                .unwrap()
                .report,
            h.state()
                .app
                .service
                .git_repository_service(root)
                .unwrap()
                .snapshot
                .as_ref()
                .map(|s| &s.entries),
            h.state()
                .app
                .git_repositories
                .views
                .get(root)
                .map_or(h.state().app.git_ui.notice.as_str(), |ui| ui
                    .notice
                    .as_str())
        );
        assert!(
            h.state()
                .app
                .service
                .git_repository_service(root)
                .unwrap()
                .error
                .is_none()
        );
    }
    assert!(!h.state().app.git_repository_uis_pending());
    assert!(h.query_by_role(Role::ProgressIndicator).is_none());
}
