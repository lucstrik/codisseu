//! Real Git reads integrated with the virtual native editor and buffer lifecycle.
use super::*;
use codisseu_core::Selection;
use egui_kittest::{Harness, kittest::Queryable};
use std::{fs, process::Command as Process, thread, time::Duration};
fn git(root: &Path, args: &[&str]) -> String {
    let o = Process::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap().trim().into()
}
fn fixture() -> (tempfile::TempDir, PathBuf, Harness<'static, CodisseuApp>) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    fs::create_dir(&root).unwrap();
    git(&root, &["init", "-b", "main"]);
    git(&root, &["config", "user.name", "Ana Fixture"]);
    git(&root, &["config", "user.email", "ana@example.invalid"]);
    git(&root, &["config", "commit.gpgsign", "false"]);
    git(&root, &["config", "core.hooksPath", "/dev/null"]);
    let text = (1..=650)
        .map(|i| format!("// linha {i} ação 🌧\n"))
        .collect::<String>();
    fs::write(root.join("main.ts"), text).unwrap();
    git(&root, &["add", "main.ts"]);
    git(&root, &["commit", "-m", "Criar linhas"]);
    let mut service = AppService::new(dir.path().join("session.json"));
    service.workspace = Some(root.clone());
    service
        .dispatch(Command::OpenFile(root.join("main.ts")))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    let app = CodisseuApp::new(service);
    let h = Harness::builder()
        .with_size([1000., 700.])
        .with_max_steps(30)
        .build_ui_state(
            |ui, app: &mut CodisseuApp| {
                if !app.styled {
                    app.tokens.apply(ui.ctx());
                    app.styled = true;
                }
                app.service.poll();
                app.poll_git_ui();
                app.git_insights_tick(ui.ctx());
                if let Some(v) = app.service.active_view() {
                    app.editor(ui, v);
                }
                app.git_insights_windows(ui.ctx());
                app.git_diff_window(ui.ctx());
            },
            app,
        );
    (dir, root, h)
}
fn wait(h: &mut Harness<'_, CodisseuApp>, check: impl Fn(&CodisseuApp) -> bool) {
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        h.step();
        if check(h.state()) {
            break;
        }
        assert!(Instant::now() < end, "{:?}", h.state().git_insights.error);
        thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn cursor_authorship_dirty_buffer_and_undo_never_save_or_pollute_editor_text() {
    let (_dir, root, mut h) = fixture();
    let path = root.join("main.ts");
    let disk = fs::read(&path).unwrap();
    let head = git(&root, &["rev-parse", "HEAD"]);
    wait(&mut h, |a| a.git_insights.line(&path, 1, 1).is_some());
    assert_eq!(
        h.state().git_insights.line(&path, 1, 1).unwrap().author,
        "Ana Fixture"
    );
    assert!(!h.state().service.active_document().unwrap().can_undo());
    let id = h.state().service.active.unwrap();
    let original = h
        .state()
        .service
        .active_document()
        .unwrap()
        .text()
        .to_owned();
    h.state_mut()
        .service
        .dispatch(Command::Edit {
            id,
            expected_version: 1,
            text: format!("// nova linha\n{original}"),
        })
        .unwrap();
    h.step();
    assert!(h.state().git_insights.line(&path, 2, 1).is_none());
    wait(&mut h, |a| a.git_insights.line(&path, 2, 1).is_some());
    assert!(
        h.state()
            .git_insights
            .line(&path, 2, 1)
            .unwrap()
            .uncommitted
    );
    assert_eq!(
        h.state().git_insights.line(&path, 2, 2).unwrap().author,
        "Ana Fixture"
    );
    assert!(
        h.query_all_by_label_contains("Autoria da linha 1: Você")
            .count()
            > 0
    );
    h.state_mut()
        .service
        .dispatch(Command::Undo { id })
        .unwrap();
    let version = h.state().service.active_document().unwrap().version;
    wait(&mut h, |a| a.git_insights.line(&path, version, 1).is_some());
    assert!(
        !h.state()
            .git_insights
            .line(&path, version, 1)
            .unwrap()
            .uncommitted
    );
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        original
    );
    assert_eq!(fs::read(path).unwrap(), disk);
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), head);
}

#[test]
fn external_declared_folder_supports_authorship_and_line_history_without_widening_workspace() {
    let (dir, root, mut h) = fixture();
    wait(&mut h, |a| !a.service.is_busy());
    let config = dir.path().join("config");
    fs::create_dir(&config).unwrap();
    let file = config.join("team.code-workspace");
    fs::write(
        &file,
        r#"{"folders":[{"name":"Service","path":"../repo"}]}"#,
    )
    .unwrap();
    h.state_mut()
        .service
        .dispatch(Command::OpenWorkspace(file.clone()))
        .unwrap();
    wait(&mut h, |a| {
        !a.service.is_busy() && a.service.workspace_path() == Some(file.as_path())
    });
    let path = root.join("main.ts");
    let original = fs::read(&path).unwrap();
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(path.clone()))
        .unwrap();
    wait(&mut h, |a| a.git_insights.line(&path, 1, 1).is_some());
    assert_eq!(
        h.state().service.workspace.as_deref(),
        Some(config.as_path())
    );
    assert_eq!(
        h.state().git_insights.identity.as_ref().unwrap().workspace,
        root
    );
    assert_eq!(
        h.state().git_insights.line(&path, 1, 1).unwrap().author,
        "Ana Fixture"
    );
    h.state_mut().git_insights_open_history();
    wait_label(&mut h, "main.ts · linhas 1–1");
    assert!(
        h.state().service.git.error.is_none(),
        "{:?}",
        h.state().service.git.error
    );
    if let Some(out) = std::env::var_os("CODISSEU_WORKSPACE_GIT_CAPTURES") {
        fs::create_dir_all(&out).unwrap();
        h.render()
            .unwrap()
            .save(PathBuf::from(out).join("external-authorship-history.png"))
            .unwrap();
    }
    let outside = dir.path().join("outside.ts");
    fs::write(&outside, "// not a declared source\n").unwrap();
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(outside.clone()))
        .unwrap();
    wait(&mut h, |a| {
        a.service
            .active_document()
            .is_some_and(|d| d.path == outside)
            && a.git_insights.identity.is_none()
    });
    assert_eq!(fs::read(path).unwrap(), original);
    assert!(!h.state().service.active_document().unwrap().is_dirty());
}
#[test]
fn gutter_queries_beyond_500_and_retains_selection_and_layout_budget() {
    let (_dir, root, mut h) = fixture();
    let path = root.join("main.ts");
    wait(&mut h, |a| a.git_insights.line(&path, 1, 1).is_some());
    let id = h.state().service.active.unwrap();
    let snapshot = h.state().service.active_document().unwrap().snapshot();
    let selection = Selection::caret(snapshot.line_to_char(610));
    h.state_mut()
        .service
        .dispatch(Command::SetSelection {
            id,
            expected_version: 1,
            selection,
        })
        .unwrap();
    h.state_mut()
        .service
        .dispatch(Command::SetGitInsights {
            inline: true,
            hover: true,
            gutter: true,
        })
        .unwrap();
    wait(&mut h, |a| {
        a.git_insights.line(&path, 1, 611).is_some() && a.editor_metrics().visible_start > 500
    });
    assert_eq!(
        h.state().git_insights.line(&path, 1, 611).unwrap().author,
        "Ana Fixture"
    );
    assert!(h.state().editor_metrics().layouts_built < 100);
    assert_eq!(h.state().service.active_document().unwrap().version, 1);
    assert!(!h.state().service.active_document().unwrap().is_dirty());
    assert_eq!(
        h.state()
            .service
            .view(h.state().service.active_view().unwrap())
            .unwrap()
            .selection(),
        selection
    );
    assert!(
        h.query_all_by_label_contains("Autoria da linha 611:")
            .count()
            > 0
    );
}
#[test]
fn switching_document_and_disabling_annotations_cannot_reuse_previous_author() {
    let (_dir, root, mut h) = fixture();
    let path = root.join("main.ts");
    wait(&mut h, |a| a.git_insights.line(&path, 1, 1).is_some());
    let other = root.join("local.sql");
    fs::write(&other, "SELECT 1;\n").unwrap();
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(other.clone()))
        .unwrap();
    wait(&mut h, |a| {
        a.service.active_document().unwrap().path == other
            && a.git_insights.line(&other, 1, 1).is_some()
    });
    assert!(h.state().git_insights.line(&path, 1, 1).is_none());
    assert!(
        h.state()
            .git_insights
            .line(&other, 1, 1)
            .unwrap()
            .uncommitted
    );
    h.state_mut()
        .service
        .dispatch(Command::SetGitInsights {
            inline: false,
            hover: true,
            gutter: false,
        })
        .unwrap();
    h.step();
    assert_eq!(h.query_all_by_label_contains("Autoria da linha").count(), 0);
    assert!(!h.state().service.active_document().unwrap().is_dirty());
}

fn wait_label(h: &mut Harness<'_, CodisseuApp>, label: &str) {
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        h.step();
        if h.query_by_label(label).is_some() {
            return;
        }
        assert!(Instant::now() < end, "Label absent: {label}");
        thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn line_history_follows_editor_cursor_and_pin_keeps_the_selected_range() {
    let (_dir, root, mut h) = fixture();
    let path = root.join("main.ts");
    wait(&mut h, |a| a.git_insights.line(&path, 1, 1).is_some());
    h.state_mut().git_insights_open_history();
    wait_label(&mut h, "main.ts · linhas 1–1");
    let id = h.state().service.active.unwrap();
    let snapshot = h.state().service.active_document().unwrap().snapshot();
    h.state_mut()
        .service
        .dispatch(Command::SetSelection {
            id,
            expected_version: 1,
            selection: Selection::caret(snapshot.line_to_char(1)),
        })
        .unwrap();
    wait_label(&mut h, "main.ts · linhas 2–2");
    h.get_by_label("Fixar seleção").click();
    h.step();
    assert!(!h.state().git_insights.details.follows_cursor());
    h.state_mut()
        .service
        .dispatch(Command::SetSelection {
            id,
            expected_version: 1,
            selection: Selection::caret(snapshot.line_to_char(2)),
        })
        .unwrap();
    thread::sleep(Duration::from_millis(350));
    h.step();
    assert!(h.query_by_label("main.ts · linhas 2–2").is_some());
    assert_eq!(
        h.state().git_insights.history_requested,
        Some((id, 1, 2, 2))
    );
    assert!(!h.state().service.active_document().unwrap().is_dirty());
}

#[test]
fn revealing_an_old_commit_anchors_the_graph_at_that_oid_instead_of_only_loading_latest_page() {
    let (_dir, root, mut h) = fixture();
    let first = git(&root, &["rev-parse", "HEAD"]);
    let tree = git(&root, &["rev-parse", "HEAD^{tree}"]);
    let mut tip = first.clone();
    for _ in 0..205 {
        tip = git(
            &root,
            &[
                "commit-tree",
                &tree,
                "-p",
                &tip,
                "-m",
                "later fixture commit",
            ],
        );
    }
    git(&root, &["update-ref", "refs/heads/main", &tip]);
    wait(&mut h, |a| {
        a.service.git.repo.is_some() && !a.service.git.is_busy() && a.git_ui.queue.is_empty()
    });
    h.state_mut().git_insights_action(Action::RevealCommit {
        root: root.clone(),
        oid: first.clone(),
    });
    wait(&mut h, |a| {
        a.service.git.history.as_ref().is_some_and(|p| {
            p.resolved_reference.as_ref() == Some(&first)
                && p.commits.first().is_some_and(|c| c.oid == first)
        })
    });
    assert_eq!(h.state().git_ui.commit.as_ref(), Some(&first));
    assert_eq!(h.state().git_ui.log_branch, first);
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), tip);
}

#[test]
fn symbolic_revision_comparison_displays_real_patch_and_rejects_another_request() {
    let (_dir, root, mut h) = fixture();
    let first = git(&root, &["rev-parse", "HEAD"]);
    let path = root.join("main.ts");
    let text = fs::read_to_string(&path).unwrap().replacen(
        "// linha 1 ação 🌧",
        "// ALTERACAO-COMPARADA",
        1,
    );
    fs::write(&path, text).unwrap();
    git(&root, &["add", "main.ts"]);
    git(&root, &["commit", "-m", "Alterar primeira linha"]);
    let second = git(&root, &["rev-parse", "HEAD"]);
    wait(&mut h, |a| {
        a.service.git.repo.is_some() && !a.service.git.is_busy() && a.git_ui.queue.is_empty()
    });
    h.state_mut().git_insights_action(Action::OpenDiff {
        root: root.clone(),
        base: Some("HEAD~1".into()),
        head: "HEAD".into(),
        path: None,
    });
    wait(&mut h, |a| {
        a.git_ui
            .diff_data
            .as_ref()
            .is_some_and(|d| d.text.contains("+// ALTERACAO-COMPARADA"))
    });
    let result = h.state().service.git.revision_diff.as_ref().unwrap();
    assert_eq!(result.query.head, "HEAD");
    assert_eq!(result.query.base.as_deref(), Some("HEAD~1"));
    assert_eq!(result.head, second);
    assert_eq!(result.base.as_deref(), Some(first.as_str()));
    assert!(result.query.path.is_none());
    h.state_mut().git_ui.diff_range = Some((Some("HEAD~1".into()), first.clone(), None));
    h.state_mut().git_ui.commit = Some(first);
    h.step();
    assert!(
        h.state().git_ui.diff_data.as_ref().unwrap().text.is_empty(),
        "A resposta do pedido HEAD não pertence a um pedido diferente"
    );
}

#[test]
fn historical_revision_view_binds_the_selected_commit_and_preserves_the_dirty_editor() {
    let (_dir, root, mut h) = fixture();
    let path = root.join("main.ts");
    let original = fs::read_to_string(&path).unwrap();
    let first = git(&root, &["rev-parse", "HEAD"]);
    fs::write(&path, "// nova revisão em disco\n").unwrap();
    git(&root, &["add", "main.ts"]);
    git(&root, &["commit", "-m", "Nova revisão"]);
    let id = h.state().service.active.unwrap();
    h.state_mut()
        .service
        .dispatch(Command::Edit {
            id,
            expected_version: 1,
            text: format!("// LOCAL NÃO SALVO\n{original}"),
        })
        .unwrap();
    wait(&mut h, |a| {
        a.service.git.repo.is_some() && !a.service.git.is_busy() && a.git_ui.queue.is_empty()
    });
    h.state_mut()
        .git_insights_action(Action::OpenHistoricalFile {
            root: root.clone(),
            oid: first.clone(),
            path: PathBuf::from("main.ts"),
            line: 1,
        });
    wait(&mut h, |a| {
        a.git_ui
            .diff_data
            .as_ref()
            .is_some_and(|d| d.text == original)
    });
    assert_eq!(h.state().git_ui.commit.as_ref(), Some(&first));
    assert!(h.state().git_ui.file_version_open);
    assert!(h.state().git_ui.diff_range.is_none());
    assert!(h.state().service.active_document().unwrap().is_dirty());
    assert!(
        h.state()
            .service
            .active_document()
            .unwrap()
            .text()
            .starts_with("// LOCAL NÃO SALVO\n")
    );
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "// nova revisão em disco\n"
    );
}

#[test]
fn status_authorship_aligns_with_encoding_and_opens_the_real_commit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    fs::create_dir(&root).unwrap();
    git(&root, &["init", "-b", "main"]);
    git(&root, &["config", "user.name", "Footer Fixture"]);
    git(&root, &["config", "user.email", "fixture@example.invalid"]);
    git(&root, &["config", "commit.gpgsign", "false"]);
    git(&root, &["config", "core.hooksPath", "/dev/null"]);
    let path = root.join("footer.ts");
    fs::write(&path, "export const footer = 1;\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "Footer fixture"]);
    for scale in [1., 1.25] {
        let mut service = AppService::new(dir.path().join(format!("session-{scale}.json")));
        service
            .dispatch(Command::OpenWorkspace(root.clone()))
            .unwrap();
        service.dispatch(Command::OpenFile(path.clone())).unwrap();
        let mut h = Harness::builder()
            .with_size([1280., 800.])
            .with_pixels_per_point(scale)
            .with_max_steps(40)
            .build_ui_state(
                |ui, a: &mut CodisseuApp| a.show(ui),
                CodisseuApp::new(service),
            );
        wait(&mut h, |a| a.git_insights.line(&path, 1, 1).is_some());
        h.run();
        let author = h.get_by_label("Autoria Git da linha atual").rect();
        let encoding = h.get_by_label("UTF-8").rect();
        assert!(
            (author.center().y - encoding.center().y).abs() <= 0.8,
            "scale {scale}: {author:?} {encoding:?}"
        );
        h.get_by_label("Autoria Git da linha atual").click();
        wait(&mut h, |a| !a.git_insights.is_busy());
        for _ in 0..10 {
            h.step();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(h.query_by_label("Detalhes do commit").is_some());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "export const footer = 1;\n"
        );
    }
}
