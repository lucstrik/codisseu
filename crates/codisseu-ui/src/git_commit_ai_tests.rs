//! One-click commit drafting through real widgets, Git and a controlled CLI.
#![cfg(unix)]
use super::*;
use codisseu_core::ai::{Config, Mode};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs, os::unix::fs::PermissionsExt, path::Path, process::Command as Process, thread,
    time::Instant,
};
type H = Harness<'static, CodisseuApp>;
const ANSWER: &str =
    "Corrija o desconto para valores negativos\n\nLimite a base do cálculo a zero.";
fn cli(root: &Path, args: &[&str]) -> String {
    let out = Process::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}
fn wait(h: &mut H, check: impl Fn(&CodisseuApp) -> bool) {
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        h.step();
        if check(h.state()) {
            return;
        }
        assert!(
            Instant::now() < end,
            "{} {:?}",
            h.state().service.status,
            h.state().git_ui.notice
        );
        thread::sleep(Duration::from_millis(3));
    }
}
fn fixture(mode: &str, scale: f32) -> (tempfile::TempDir, PathBuf, PathBuf, H) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repository");
    fs::create_dir(&root).unwrap();
    cli(&root, &["init", "-b", "main"]);
    for (k, v) in [
        ("user.name", "Commit fixture"),
        ("user.email", "fixture@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", "/dev/null"),
    ] {
        cli(&root, &["config", k, v]);
    }
    fs::write(
        root.join("discount.ts"),
        "export const discount = (base: number) => base * 0.1;\n",
    )
    .unwrap();
    cli(&root, &["add", "discount.ts"]);
    cli(&root, &["commit", "-m", "Initial"]);
    fs::write(
        root.join("discount.ts"),
        "export const discount = (base: number) => Math.max(0, base) * 0.1;\n",
    )
    .unwrap();
    let agent = dir.path().join("agent");
    fs::create_dir(&agent).unwrap();
    let program = agent.join("cli");
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../codisseu-ai/tests/fixtures/commit_message_cli.py");
    fs::copy(source, &program).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(agent.join("mode"), mode).unwrap();
    let mut app = CodisseuApp::new(AppService::new(dir.path().join("profile/session.json")));
    app.service
        .dispatch(Command::OpenWorkspace(root.clone()))
        .unwrap();
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .with_pixels_per_point(scale)
        .with_max_steps(80)
        .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
    wait(&mut h, |app| !app.service.is_busy());
    h.state_mut()
        .service
        .ai
        .dispatch(
            codisseu_core::ai::Command::Configure(Config {
                program: program.display().to_string(),
                mode: Mode::FullAccess,
                ..Default::default()
            }),
            None,
        )
        .unwrap();
    wait(&mut h, |app| !app.service.ai.profile_pending());
    h.state_mut().git_open_source_control();
    wait(&mut h, |app| {
        !app.service.is_busy() && app.git_ui.queue.is_empty()
    });
    h.step();
    h.step();
    (dir, root, agent, h)
}
fn generate(h: &mut H, root: &Path) {
    h.get_by_label(&format!("Generate commit message {}", root.display()))
        .click();
    h.step();
}
#[test]
fn commit_ai_single_click_fills_field_without_modal_or_git_mutation_at_two_scales() {
    for scale in [1., 1.25] {
        let (_dir, root, agent, mut h) = fixture("normal", scale);
        let index = fs::read(root.join(".git/index")).unwrap();
        let head = cli(&root, &["rev-parse", "HEAD"]);
        let disk = fs::read(root.join("discount.ts")).unwrap();
        let config = h.state().service.ai.config.clone();
        let entries = h.state().service.ai.entries.len();
        generate(&mut h, &root);
        assert!(h.state().git_ui.dialog.is_none());
        wait(&mut h, |app| !app.service.git_commit_messages.any_running());
        h.step();
        assert_eq!(h.state().git_ui.message, ANSWER);
        assert_eq!(h.state().service.git_draft(&root), Some(ANSWER));
        assert!(h.state().git_ui.dialog.is_none());
        assert!(h.query_by_label("Generated commit message").is_none());
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
        assert_eq!(cli(&root, &["rev-parse", "HEAD"]), head);
        assert_eq!(fs::read(root.join("discount.ts")).unwrap(), disk);
        assert_eq!(h.state().service.ai.config, config);
        assert_eq!(h.state().service.ai.entries.len(), entries);
        let requests = fs::read_to_string(agent.join("requests")).unwrap();
        assert_eq!(requests.matches("\"method\": \"turn/start\"").count(), 1);
        assert!(requests.contains("readOnly"));
        assert!(requests.contains("\"ephemeral\": true"));
    }
}
#[test]
fn commit_ai_preserves_manual_draft_and_rejects_changed_diff() {
    for manual in [true, false] {
        let (_dir, root, agent, mut h) = fixture("delay", 1.);
        h.state_mut().git_ui.message = "Rascunho original".into();
        h.state_mut().sync_git_draft();
        generate(&mut h, &root);
        wait(&mut h, |_| agent.join("started").exists());
        if manual {
            h.state_mut().git_ui.message = "Mensagem editada durante a geração".into();
            h.state_mut().sync_git_draft();
        } else {
            fs::write(root.join("discount.ts"), "export const discount = 0;\n").unwrap();
        }
        fs::write(agent.join("release"), "").unwrap();
        wait(&mut h, |app| !app.service.git_commit_messages.any_running());
        h.step();
        assert_eq!(
            h.state().git_ui.message,
            if manual {
                "Mensagem editada durante a geração"
            } else {
                "Rascunho original"
            }
        );
        assert!(
            h.state()
                .git_ui
                .commit_ai_notice
                .as_ref()
                .is_some_and(|text| text.contains("preservada"))
        );
        assert!(h.state().git_ui.dialog.is_none());
    }
}
#[test]
fn commit_ai_cancel_and_failure_leave_draft_unchanged_and_reap_owned_cli() {
    for mode in ["hang", "error", "ask"] {
        let (_dir, root, agent, mut h) = fixture(mode, 1.);
        h.state_mut().git_ui.message = "Preservar mensagem".into();
        h.state_mut().sync_git_draft();
        generate(&mut h, &root);
        if mode == "hang" {
            wait(&mut h, |_| agent.join("ready").exists());
            h.get_by_label(&format!(
                "Cancel commit message generation {}",
                root.display()
            ))
            .click();
            h.step();
        }
        wait(&mut h, |app| !app.service.git_commit_messages.any_running());
        h.step();
        assert_eq!(h.state().git_ui.message, "Preservar mensagem");
        assert!(h.state().git_ui.dialog.is_none());
        assert!(h.state().git_ui.commit_ai_notice.is_some());
        let pid = fs::read_to_string(agent.join("pid")).unwrap();
        assert!(!PathBuf::from(format!("/proc/{}", pid.trim())).exists());
        assert!(!root.join("NEVER").exists());
    }
}
#[test]
fn commit_ai_workspace_change_cancels_and_never_fills_the_new_project() {
    let (dir, root, agent, mut h) = fixture("hang", 1.);
    generate(&mut h, &root);
    wait(&mut h, |_| agent.join("ready").exists());
    let next = dir.path().join("other");
    fs::create_dir(&next).unwrap();
    h.state_mut()
        .service
        .dispatch(Command::OpenWorkspace(next.clone()))
        .unwrap();
    wait(&mut h, |app| {
        app.service.workspace_path() == Some(next.as_path())
            && !app.service.git_commit_messages.any_running()
    });
    assert!(h.state().git_ui.message.is_empty());
    assert!(h.state().service.git_draft(&next).is_none());
    let pid = fs::read_to_string(agent.join("pid")).unwrap();
    assert!(!PathBuf::from(format!("/proc/{}", pid.trim())).exists());
}

#[test]
fn commit_ai_result_remains_bound_to_its_repository_when_selection_changes() {
    let (dir, root, agent, mut h) = fixture("delay", 1.);
    let second = dir.path().join("second");
    fs::create_dir(&second).unwrap();
    cli(&second, &["init", "-b", "main"]);
    for (k, v) in [
        ("user.name", "Second fixture"),
        ("user.email", "second@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", "/dev/null"),
    ] {
        cli(&second, &["config", k, v]);
    }
    fs::write(second.join("other.ts"), "export const other = true;\n").unwrap();
    cli(&second, &["add", "other.ts"]);
    cli(&second, &["commit", "-m", "Second initial"]);
    let workspace = dir.path().join("team.code-workspace");
    fs::write(
        &workspace,
        serde_json::to_string(&serde_json::json!({"folders":[{"path":root},{"path":second}]}))
            .unwrap(),
    )
    .unwrap();
    h.state_mut()
        .send(Command::OpenWorkspace(workspace.clone()));
    wait(&mut h, |app| {
        app.service.workspace_path() == Some(workspace.as_path())
            && !app.workspace_switch_pending()
            && !app.service.is_busy()
    });
    h.state_mut()
        .service
        .dispatch(Command::SelectGitRepository(root.clone()))
        .unwrap();
    wait(&mut h, |app| !app.service.is_busy());
    h.step();
    h.step();
    generate(&mut h, &root);
    wait(&mut h, |_| agent.join("started").exists());
    h.state_mut()
        .service
        .dispatch(Command::SelectGitRepository(second.clone()))
        .unwrap();
    wait(&mut h, |app| !app.service.is_busy());
    h.step();
    h.step();
    h.state_mut().git_ui.message = "Mensagem do segundo repositório".into();
    h.state_mut().sync_git_draft();
    fs::write(agent.join("release"), "").unwrap();
    wait(&mut h, |app| !app.service.git_commit_messages.any_running());
    h.step();
    assert_eq!(h.state().git_ui.message, "Mensagem do segundo repositório");
    assert_eq!(h.state().service.git_draft(&root), Some(ANSWER));
    assert_eq!(
        h.state().service.git_draft(&second),
        Some("Mensagem do segundo repositório")
    );
}

#[test]
fn commit_ai_shutdown_cancels_the_owned_cli_and_preserves_the_draft_in_session() {
    let (dir, root, agent, mut h) = fixture("hang", 1.);
    h.state_mut().git_ui.message = "Mensagem preservada ao sair".into();
    h.state_mut().sync_git_draft();
    generate(&mut h, &root);
    wait(&mut h, |_| agent.join("ready").exists());
    h.state_mut().request_exit();
    wait(&mut h, |app| app.allow_close);
    let pid = fs::read_to_string(agent.join("pid")).unwrap();
    assert!(!PathBuf::from(format!("/proc/{}", pid.trim())).exists());
    let session: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("profile/session.json")).unwrap())
            .unwrap();
    assert_eq!(
        session["git_drafts"][root.display().to_string()],
        "Mensagem preservada ao sair"
    );
}

#[test]
fn commit_ai_ready_result_does_not_overwrite_text_entered_in_the_same_frame() {
    let (_dir, root, agent, mut h) = fixture("delay", 1.);
    h.state_mut().git_ui.message = "Rascunho original".into();
    h.state_mut().sync_git_draft();
    generate(&mut h, &root);
    wait(&mut h, |_| agent.join("started").exists());
    h.get_by_label(&format!("Commit message {}", root.display()))
        .click();
    h.step();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.step();
    fs::write(agent.join("release"), "").unwrap();
    // Let the real worker finish without letting a GUI frame consume its result.
    let end = Instant::now() + Duration::from_secs(15);
    while h.state().service.git_commit_messages.any_running() {
        h.state_mut().service.poll();
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(3));
    }
    h.event(egui::Event::Text("Mensagem digitada neste frame".into()));
    h.step();
    h.step();
    assert_eq!(h.state().git_ui.message, "Mensagem digitada neste frame");
    assert_eq!(
        h.state().service.git_draft(&root),
        Some("Mensagem digitada neste frame")
    );
}

#[test]
fn commit_ai_repository_menu_generates_directly_without_a_form() {
    let (_dir, root, _agent, mut h) = fixture("normal", 1.);
    h.get_by_label(&format!("Repository actions {}", root.display()))
        .click();
    h.step();
    h.step();
    h.get_by_label("Generate Commit Message").click();
    h.step();
    assert!(h.state().git_ui.dialog.is_none());
    wait(&mut h, |app| !app.service.git_commit_messages.any_running());
    h.step();
    assert_eq!(h.state().git_ui.message, ANSWER);
    assert!(h.state().git_ui.dialog.is_none());
}
