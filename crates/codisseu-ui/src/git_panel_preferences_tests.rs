//! Real egui controls; disposable repository and isolated session only.
use super::*;
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn git(root: &Path, args: &[&str]) -> String {
    let result = std::process::Command::new("git")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}
fn service() -> (tempfile::TempDir, AppService) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    fs::create_dir(&root).unwrap();
    git(&root, &["-c", "init.templateDir=", "init", "-b", "main"]);
    fs::write(root.join("file.txt"), "owned fixture\n").unwrap();
    git(&root, &["add", "file.txt"]);
    git(
        &root,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-m",
            "Initial",
        ],
    );
    let mut service = AppService::new(dir.path().join("session.json"));
    service.dispatch(Command::OpenWorkspace(root)).unwrap();
    let end = Instant::now() + Duration::from_secs(15);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(2));
    }
    assert!(service.git.error.is_none());
    (dir, service)
}
fn settle(h: &mut Harness<'_, CodisseuApp>) {
    let end = Instant::now() + Duration::from_secs(15);
    let mut idle = 0;
    while idle < 5 {
        h.step();
        idle = if h.state().service.is_busy() {
            0
        } else {
            idle + 1
        };
        assert!(Instant::now() < end, "{:?}", h.state().service.last_error);
        thread::sleep(Duration::from_millis(4));
    }
}
fn harness(service: AppService, scale: f32) -> Harness<'static, CodisseuApp> {
    Harness::builder()
        .with_size([1000., 800.])
        .with_pixels_per_point(scale)
        .with_max_steps(40)
        .build_ui_state(
            |ui, app: &mut CodisseuApp| app.show(ui),
            CodisseuApp::new(service),
        )
}
fn capture(h: &mut Harness<'_, CodisseuApp>, name: &str) {
    if let Some(out) = std::env::var_os("CODISSEU_GIT_PANEL_CAPTURES") {
        fs::create_dir_all(&out).unwrap();
        h.render()
            .unwrap()
            .save(PathBuf::from(out).join(format!("{name}.png")))
            .unwrap();
    }
}

fn scroll_to_commit_preference(h: &mut Harness<'_, CodisseuApp>) {
    // General options can grow. Scroll the actual target into the preferences
    // viewport rather than assuming a fixed wheel distance still reveals it.
    for _ in 0..8 {
        let y = h
            .get_by_label("Mostrar painel Commit (estilo PhpStorm)")
            .rect()
            .center()
            .y;
        if (300. ..=500.).contains(&y) {
            return;
        }
        h.event(egui::Event::PointerMoved(egui::pos2(500., 380.)));
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            delta: egui::vec2(0., 440. - y),
            modifiers: egui::Modifiers::NONE,
        });
        settle(h);
    }
    panic!("Commit preference did not enter the visible viewport");
}

#[test]
fn preference_mouse_and_keyboard_switch_presentations_without_losing_draft() {
    for scale in [1., 1.25] {
        let (_dir, service) = service();
        let root = service.workspace.clone().unwrap();
        let mut h = harness(service, scale);
        settle(&mut h);
        assert!(h.query_by_label("Commit · Ctrl+K").is_none());
        assert!(h.query_by_label("Source Control").is_some());
        h.state_mut().preferences_dialog = true;
        settle(&mut h);
        scroll_to_commit_preference(&mut h);
        assert!(
            !h.state()
                .service
                .tool_window_preferences()
                .git_confirm_actions
        );
        h.get_by_label("Confirmar ações Git antes de executar")
            .click();
        settle(&mut h);
        assert!(
            h.state()
                .service
                .tool_window_preferences()
                .git_confirm_actions
        );
        h.get_by_label("Confirmar ações Git antes de executar")
            .click();
        settle(&mut h);
        assert!(
            !h.state()
                .service
                .tool_window_preferences()
                .git_confirm_actions
        );
        h.get_by_label("Mostrar painel Commit (estilo PhpStorm)")
            .click();
        settle(&mut h);
        assert!(h.state().service.tool_window_preferences().commit);
        h.key_press(egui::Key::Space);
        settle(&mut h);
        assert!(!h.state().service.tool_window_preferences().commit);
        h.key_press(egui::Key::Space);
        settle(&mut h);
        assert!(h.state().service.tool_window_preferences().commit);
        h.get_by_label("Concluído").click();
        settle(&mut h);
        h.get_by_label("Commit · Ctrl+K").click();
        settle(&mut h);
        assert!(!h.state().git_ui.source_control);
        h.state_mut().git_ui.message = "Rascunho preservado 😀".into();
        h.state_mut().preferences_dialog = true;
        settle(&mut h);
        capture(&mut h, &format!("settings-enabled-{scale}"));
        scroll_to_commit_preference(&mut h);
        h.get_by_label("Mostrar painel Commit (estilo PhpStorm)")
            .click();
        settle(&mut h);
        h.get_by_label("Concluído").click();
        settle(&mut h);
        assert!(!h.state().service.tool_window_preferences().commit);
        assert!(h.state().git_ui.source_control);
        assert!(h.query_by_label("Commit · Ctrl+K").is_none());
        assert_eq!(h.state().git_ui.message, "Rascunho preservado 😀");
        h.state_mut().preferences_dialog = true;
        settle(&mut h);
        scroll_to_commit_preference(&mut h);
        let toggle = h.get_by_label("Mostrar painel Commit (estilo PhpStorm)");
        toggle.focus();
        h.step();
        h.key_press(egui::Key::Space);
        settle(&mut h);
        assert!(h.state().service.tool_window_preferences().commit);
        h.get_by_label("Concluído").click();
        settle(&mut h);
        h.get_by_label("Commit · Ctrl+K").click();
        settle(&mut h);
        assert!(!h.state().git_ui.source_control);
        assert_eq!(h.state().git_ui.message, "Rascunho preservado 😀");
        assert_eq!(git(&root, &["status", "--porcelain"]), "");
        assert!(h.state().service.last_error.is_none());
    }
}

#[test]
fn hidden_commit_routes_shortcuts_and_hides_its_menu_while_stash_remains_available() {
    let (_dir, service) = service();
    let mut h = harness(service, 1.);
    settle(&mut h);
    for modifiers in [
        egui::Modifiers::CTRL.plus(egui::Modifiers::ALT),
        egui::Modifiers::CTRL,
    ] {
        h.key_press_modifiers(modifiers, egui::Key::K);
        settle(&mut h);
        assert!(h.state().git_ui.source_control && h.state().show_project);
        assert!(h.query_by_label("Commit Message").is_none());
    }
    h.get_by_label("Source Control actions").click();
    settle(&mut h);
    assert!(h.query_by_label("Commit Tool Window").is_none());
    h.key_press(egui::Key::Escape);
    settle(&mut h);
    h.state_mut()
        .git_action(crate::git::GitActionKind::Stashes, String::new());
    settle(&mut h);
    assert_eq!(h.state().git_ui.commit_tab, 1);
    assert!(!h.state().git_ui.source_control);
    assert!(h.query_by_label("Commit Message").is_none());
}

#[test]
fn empty_commit_text_uses_content_inset_at_both_scales() {
    for scale in [1., 1.25] {
        let (_dir, mut service) = service();
        service
            .dispatch(Command::SetCommitToolWindowVisible(true))
            .unwrap();
        let mut app = CodisseuApp::new(service);
        app.git_ui.commit_open = true;
        let mut h = Harness::builder()
            .with_size([298., 800.])
            .with_pixels_per_point(scale)
            .build_ui_state(
                |ui, app: &mut CodisseuApp| {
                    if !app.styled {
                        app.tokens.apply(ui.ctx());
                        app.styled = true;
                    }
                    app.git_commit_panel(ui);
                },
                app,
            );
        h.run();
        let empty = h.get_by_label("No changes").rect();
        // egui_kittest's root UI has its regular 8-point outer margin.
        assert!(
            (empty.left() - 8. - f32::from(h.state().tokens.panel_content_inset_x)).abs() < 1.,
            "{empty:?}"
        );
        assert!(empty.right() < 298. - 8.);
        capture(&mut h, &format!("commit-empty-{scale}"));
    }
}
