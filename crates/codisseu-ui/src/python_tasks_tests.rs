//! Native egui widgets -> real private Python processes, with isolated projects/profiles.
use super::*;
use codisseu_core::{
    ShellLayout, debugger::DebugRuntime, python_project::Action, python_tests::Framework,
};
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::{fs, thread, time::Instant};

type H = Harness<'static, CodisseuApp>;
const TRUST: &str = "Confio neste projeto e autorizo esta ação Python";
const TEST_TRUST: &str = "Confio neste projeto e autorizo executar os testes Python";
const RUN: &str = "Executar ação Python";

fn wait(h: &mut H, predicate: impl Fn(&CodisseuApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        h.step();
        if predicate(h.state()) {
            h.step();
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Python UI timeout: status={:?} error={:?} python={:?} tests={:?} test_error={:?}",
            h.state().service.status,
            h.state().service.last_error,
            h.state().service.python.error,
            h.state().service.python_tests.state,
            h.state().service.python_tests.report.error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn ready(h: &mut H) {
    wait(h, |a| {
        !a.service.is_busy() && !a.service.python.is_busy() && !a.service.python_tests.is_busy()
    });
}
fn setup(scale: f32, compact: bool, source: &str) -> (tempfile::TempDir, H, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Python ação 😀");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("main.py"), source).unwrap();
    let mut service = AppService::new(dir.path().join("profile/session.json"));
    service
        .dispatch(Command::OpenWorkspace(root.clone()))
        .unwrap();
    service
        .dispatch(Command::SetShellLayout(ShellLayout {
            bottom_height: Some(350.),
            ..Default::default()
        }))
        .unwrap();
    let mut h = Harness::builder()
        .with_size(if compact { [900., 700.] } else { [1280., 900.] })
        .with_pixels_per_point(scale)
        .with_max_steps(180)
        .build_ui_state(
            |ui, a: &mut CodisseuApp| a.show(ui),
            CodisseuApp::new(service),
        );
    ready(&mut h);
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(root.join("main.py")))
        .unwrap();
    ready(&mut h);
    wait(&mut h, |a| a.service.python.interpreter.is_some());
    h.run();
    (dir, h, root)
}
fn field(h: &mut H, label: &str, value: &str) {
    let role = if label.starts_with("Argumentos") || label.starts_with("Ambiente") {
        Role::MultilineTextInput
    } else {
        Role::TextInput
    };
    h.get_by_role_and_label(role, label).focus();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(egui::Event::Text(value.into()));
    h.step();
    h.step();
}
fn capture(h: &mut H, scale: f32, name: &str) {
    let Some(base) = std::env::var_os("CODISSEU_PYTHON_TASK_CAPTURES") else {
        return;
    };
    let dir = PathBuf::from(base).join(if scale == 1. { "100" } else { "125" });
    fs::create_dir_all(&dir).unwrap();
    h.step();
    h.step();
    h.render()
        .unwrap()
        .save(dir.join(format!("{name}.png")))
        .unwrap();
}
fn scroll(h: &mut H, label: &str, delta: f32) {
    h.hover_at(h.get_by_label(label).rect().center());
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        phase: egui::TouchPhase::Move,
        delta: egui::vec2(0., delta),
        modifiers: Modifiers::NONE,
        source: egui::MouseWheelSource::Unknown,
    });
    for _ in 0..8 {
        h.step();
    }
}
fn trust_once(h: &mut H) {
    fn snapshot(h: &H, phase: &str) -> String {
        let trust = h.get_by_label(TRUST);
        let run = h.get_by_label(RUN);
        format!(
            "{phase}: trust_rect={:?} trust_disabled={} toggled={:?} run_rect={:?} run_disabled={} busy={} revision={} config={:?} interpreter={:?} error={:?}",
            trust.rect(),
            trust.accesskit_node().is_disabled(),
            trust.accesskit_node().toggled(),
            run.rect(),
            run.accesskit_node().is_disabled(),
            h.state().service.python.is_busy(),
            h.state().service.python.revision,
            h.state().service.python.config,
            h.state().service.python.interpreter,
            h.state().service.python.error
        )
    }
    let rects = (h.get_by_label(TRUST).rect(), h.get_by_label(RUN).rect());
    let point = rects.0.center();
    let mut trace = vec![snapshot(h, "before")];
    h.hover_at(point);
    h.step();
    trace.push(snapshot(h, "hover"));
    assert_eq!(
        rects,
        (h.get_by_label(TRUST).rect(), h.get_by_label(RUN).rect()),
        "{}",
        trace.join("\n")
    );
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
        trace.push(snapshot(h, if pressed { "down" } else { "up" }));
        assert_eq!(
            rects,
            (h.get_by_label(TRUST).rect(), h.get_by_label(RUN).rect()),
            "{}",
            trace.join("\n")
        );
    }
    h.step();
    h.step();
    trace.push(snapshot(h, "settled"));
    if std::env::var_os("CODISSEU_PYTHON_TRUST_TRACE").is_some() {
        println!("{}", trace.join("\n"));
    }
    assert!(
        h.get_by_label(TRUST).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True),
        "single trust click must reach the checkbox: {}",
        trace.join("\n")
    );
    assert!(
        !h.get_by_label(RUN).accesskit_node().is_disabled(),
        "{}",
        trace.join("\n")
    );
}
fn trust_run(h: &mut H) {
    trust_once(h);
    h.get_by_label(RUN).click();
    h.step();
}
fn terminal_copy(h: &mut H) -> String {
    h.get_by_role_and_label(Role::Terminal, "Terminal integrado")
        .click_secondary();
    h.step();
    h.step();
    h.get_by_label("Copiar área visível").click();
    h.step();
    h.output()
        .platform_output
        .commands
        .iter()
        .find_map(|c| match c {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
        .expect("terminal buffer copied through the user context action")
}
fn wait_terminal(h: &mut H, predicate: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        h.step();
        let text = terminal_copy(h);
        if predicate(&text) {
            return text;
        }
        assert!(
            Instant::now() < deadline,
            "terminal output did not arrive: {text}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn python_tools_review_is_passive_compact_scroll_footer_and_both_run_shortcuts() {
    for scale in [1., 1.25] {
        let (_dir, mut h, root) = setup(
            scale,
            true,
            "from pathlib import Path\nPath('UNEXPECTED').write_text('executed')\n",
        );
        assert!(
            h.get_by_label("Build Rust · Cargo build · Ctrl+F9")
                .accesskit_node()
                .is_disabled()
        );
        h.get_by_label("Executar Python · Shift+F10").click();
        ready(&mut h);
        h.run();
        assert!(h.state().python_ui.open);
        assert!(h.query_by_label("Python · Executar e ambiente").is_some());
        assert!(h.get_by_label(RUN).accesskit_node().is_disabled());
        for label in [TRUST, "Cancelar", RUN] {
            let rect = h.get_by_label(label).rect();
            assert!(
                rect.min.y >= 0. && rect.max.y <= 700.,
                "footer clipped: {label} {rect:?}"
            );
        }
        capture(&mut h, scale, "tools-review-compact");
        scroll(&mut h, "Diretório de execução", -700.);
        assert!(
            h.query_by_role_and_label(
                Role::MultilineTextInput,
                "Ambiente Python (NOME=valor, um por linha)"
            )
            .is_some()
        );
        capture(&mut h, scale, "tools-review-scroll-footer");
        h.get_by_label(TRUST).click();
        h.step();
        h.step();
        field(
            &mut h,
            "Ambiente Python (NOME=valor, um por linha)",
            "DEMO=ação",
        );
        assert!(
            h.get_by_label(RUN).accesskit_node().is_disabled(),
            "draft changes revoke trust"
        );
        h.get_by_label("Cancelar").click();
        h.run();
        assert!(!h.state().python_ui.open);
        assert!(
            h.get_by_role_and_label(
                Role::MultilineTextInput,
                root.join("main.py").to_string_lossy().as_ref()
            )
            .is_focused(),
            "closing Python tools restores editor focus"
        );
        assert!(!h.state().terminal.has_sessions());
        assert!(!root.join("UNEXPECTED").exists());
        h.key_press_modifiers(Modifiers::SHIFT, Key::F10);
        h.step();
        h.step();
        assert!(h.state().python_ui.open);
        assert!(h.query_by_label("Python · Executar e ambiente").is_some());
        assert!(h.get_by_label(RUN).accesskit_node().is_disabled());
        h.key_press(Key::Escape);
        h.run();
        assert!(!h.state().python_ui.open);
        assert!(
            h.get_by_role_and_label(
                Role::MultilineTextInput,
                root.join("main.py").to_string_lossy().as_ref()
            )
            .is_focused(),
            "closing Python tools restores editor focus"
        );
        assert!(!h.state().terminal.has_sessions());
        assert!(!root.join("UNEXPECTED").exists());
        h.state_mut().open_python(Action::CreateVenv);
        ready(&mut h);
        h.run();
        h.get_by_label("Cancelar").click();
        h.run();
        assert!(
            !root.join(".venv").exists(),
            "review/cancel cannot create a venv"
        );
        fs::write(root.join("test_passive.py"), "from pathlib import Path\nPath('UNEXPECTED').write_text('imported')\ndef test_passive(): pass\n").unwrap();
        h.state_mut().open_python_tests(true);
        ready(&mut h);
        h.run();
        assert!(
            h.get_by_label("Executar suite Python")
                .accesskit_node()
                .is_disabled()
        );
        capture(&mut h, scale, "test-review-passive");
        h.get_by_label("Cancelar").click();
        h.run();
        assert!(!h.state().python_tests_ui.settings);
        assert!(
            h.get_by_role_and_label(
                Role::MultilineTextInput,
                root.join("main.py").to_string_lossy().as_ref()
            )
            .is_focused(),
            "closing Python tests restores editor focus"
        );
        assert!(h.state().service.python_tests.report.cases.is_empty());
        assert!(!root.join("UNEXPECTED").exists());
    }
}

#[test]
fn python_run_widgets_preserve_literal_arguments_environment_and_visible_terminal_output() {
    for scale in [1., 1.25] {
        let (_dir, mut h, root) = setup(
            scale,
            false,
            "import json, os, sys\nfrom pathlib import Path\ndata={'args':sys.argv[1:],'env':os.environ.get('CODISSEU_GUI_VALUE'),'executable':sys.executable}\nPath('result.json').write_text(json.dumps(data),encoding='utf-8')\nprint('PYTHON_LITERAL_OK',json.dumps(data,ensure_ascii=False),flush=True)\n",
        );
        h.get_by_label("Executar Python · Shift+F10").click();
        ready(&mut h);
        h.run();
        field(
            &mut h,
            "Argumentos Python (um por linha)",
            "literal space\n$(touch SHELL_EXECUTED)\n; touch SHELL_EXECUTED\nα😀",
        );
        field(
            &mut h,
            "Ambiente Python (NOME=valor, um por linha)",
            "CODISSEU_GUI_VALUE=valor com espaço $HOME ; 😀",
        );
        capture(&mut h, scale, "run-literal-review");
        trust_run(&mut h);
        wait(&mut h, |a| {
            !a.python_ui.open && a.terminal.has_sessions() && root.join("result.json").exists()
        });
        let result: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("result.json")).unwrap()).unwrap();
        assert_eq!(
            result["args"],
            serde_json::json!([
                "literal space",
                "$(touch SHELL_EXECUTED)",
                "; touch SHELL_EXECUTED",
                "α😀"
            ])
        );
        assert_eq!(result["env"], "valor com espaço $HOME ; 😀");
        assert!(!root.join("SHELL_EXECUTED").exists());
        assert!(h.state().tool_tab == ToolTab::Terminal);
        // Copy the actual terminal buffer through its user-visible context action.
        for _ in 0..30 {
            h.step();
            thread::sleep(Duration::from_millis(5));
        }
        h.get_by_role_and_label(Role::Terminal, "Terminal integrado")
            .click_secondary();
        h.step();
        h.step();
        h.get_by_label("Copiar área visível").click();
        h.step();
        let copied = h
            .output()
            .platform_output
            .commands
            .iter()
            .find_map(|c| match c {
                egui::OutputCommand::CopyText(s) => Some(s.clone()),
                _ => None,
            })
            .expect("terminal copy command");
        assert!(copied.contains("PYTHON_LITERAL_OK"), "{copied}");
        assert!(copied.contains("$(touch SHELL_EXECUTED)"), "{copied}");
        assert!(copied.contains("valor com espaço $HOME ;"), "{copied}");
        capture(&mut h, scale, "run-literal-terminal");
    }
}

#[test]
fn python_module_repl_and_syntax_check_widgets_use_real_processes_without_shell_or_source_side_effects()
 {
    let (_dir, mut h, root) = setup(
        1.,
        false,
        "from pathlib import Path\nPath('SOURCE_EXECUTED').write_text('unexpected')\n",
    );
    fs::write(root.join("tasks_entry.py"), "import json,sys\nfrom pathlib import Path\nPath('module.json').write_text(json.dumps(sys.argv[1:]))\nprint('MODULE_LITERAL_OK',json.dumps(sys.argv[1:],ensure_ascii=False),flush=True)\n").unwrap();
    h.get_by_label("Executar Python · Shift+F10").click();
    ready(&mut h);
    h.run();
    h.get_by_label("Executar módulo").click();
    h.step();
    h.step();
    field(&mut h, "Módulo Python (-m)", "tasks_entry");
    field(
        &mut h,
        "Argumentos Python (um por linha)",
        "literal space\n$(touch MODULE_SHELL)\nação 😀",
    );
    capture(&mut h, 1., "module-review");
    trust_run(&mut h);
    wait(&mut h, |a| {
        !a.python_ui.open && root.join("module.json").is_file()
    });
    let args: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("module.json")).unwrap()).unwrap();
    assert_eq!(
        args,
        serde_json::json!(["literal space", "$(touch MODULE_SHELL)", "ação 😀"])
    );
    let output = wait_terminal(&mut h, |t| t.contains("MODULE_LITERAL_OK"));
    assert!(output.contains("$(touch MODULE_SHELL)"));
    assert!(!root.join("MODULE_SHELL").exists());
    assert!(!root.join("SOURCE_EXECUTED").exists());
    capture(&mut h, 1., "module-terminal-literal-output");

    h.state_mut().open_python(Action::RunFile);
    ready(&mut h);
    h.run();
    h.get_by_label("Console Python").click();
    h.step();
    h.step();
    trust_run(&mut h);
    wait(&mut h, |a| {
        !a.python_ui.open && a.terminal.session_label() == "Python · Console Python"
    });
    wait_terminal(&mut h, |t| t.contains(">>>"));
    h.get_by_role_and_label(Role::Terminal, "Terminal integrado")
        .click();
    h.step();
    assert!(
        h.get_by_role_and_label(Role::Terminal, "Terminal integrado")
            .is_focused()
    );
    h.event(egui::Event::Text("print(6*7)".into()));
    h.key_press(Key::Enter);
    h.step();
    let output = wait_terminal(&mut h, |t| t.lines().any(|l| l.trim() == "42"));
    assert!(output.contains("print(6*7)"));
    capture(&mut h, 1., "repl-evaluated-42");
    h.get_by_role_and_label(Role::Terminal, "Terminal integrado")
        .click();
    h.step();
    h.event(egui::Event::Text("exit()".into()));
    h.key_press(Key::Enter);
    h.step();
    let deadline = Instant::now() + Duration::from_secs(20);
    while h.query_by_label("Reiniciar").is_none() {
        h.step();
        assert!(
            Instant::now() < deadline,
            "exit() did not finish the REPL process"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!root.join("SOURCE_EXECUTED").exists());
    capture(&mut h, 1., "repl-exited");

    h.state_mut().open_python(Action::RunFile);
    ready(&mut h);
    h.run();
    h.get_by_label("Verificar sintaxe").click();
    h.step();
    h.step();
    trust_run(&mut h);
    wait(&mut h, |a| {
        !a.python_ui.open && a.terminal.session_label() == "Python · Verificar sintaxe"
    });
    wait_terminal(&mut h, |t| t.contains("Sintaxe Python válida"));
    assert!(
        !root.join("SOURCE_EXECUTED").exists(),
        "syntax verification cannot execute the source"
    );
    capture(&mut h, 1., "syntax-check-without-source-execution");
}

#[test]
fn python_create_venv_is_explicit_and_subsequent_run_uses_the_new_environment() {
    for scale in [1., 1.25] {
        let (_dir, mut h, root) = setup(
            scale,
            false,
            "import json,sys\nfrom pathlib import Path\nPath('venv-result.json').write_text(json.dumps({'prefix':sys.prefix,'base':sys.base_prefix}))\nprint('VENV_SELECTED',sys.prefix,flush=True)\n",
        );
        h.state_mut().open_python(Action::CreateVenv);
        ready(&mut h);
        h.run();
        assert!(!root.join(".venv").exists());
        assert!(h.get_by_label(RUN).accesskit_node().is_disabled());
        capture(&mut h, scale, "venv-explicit-review");
        trust_run(&mut h);
        wait(&mut h, |a| {
            !a.python_ui.open
                && a.service
                    .python
                    .interpreter
                    .as_ref()
                    .is_some_and(|p| p.starts_with(root.join(".venv")))
        });
        assert!(root.join(".venv/pyvenv.cfg").is_file());
        assert!(!root.join("venv-result.json").exists());
        h.state_mut().open_python(Action::RunFile);
        ready(&mut h);
        h.run();
        capture(&mut h, scale, "venv-detected-review");
        trust_run(&mut h);
        wait(&mut h, |a| {
            !a.python_ui.open && root.join("venv-result.json").exists()
        });
        let result: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("venv-result.json")).unwrap()).unwrap();
        assert_eq!(
            result["prefix"],
            root.join(".venv").to_string_lossy().as_ref()
        );
        assert_ne!(result["prefix"], result["base"]);
        capture(&mut h, scale, "venv-run-terminal");
    }
}

#[test]
fn python_pytest_unittest_widgets_reuse_run_cases_rerun_navigation_and_debug_review() {
    for (scale, framework) in [
        (1., Framework::Pytest),
        (1.25, Framework::Pytest),
        (1., Framework::Unittest),
        (1.25, Framework::Unittest),
    ] {
        let (_dir, mut h, root) = setup(scale, false, "# Python GUI fixture\n");
        let source = root.join("test_gui.py");
        fs::write(&source,"from pathlib import Path\nimport unittest\nPath('IMPORTED').write_text('yes')\nclass GuiTests(unittest.TestCase):\n    def test_pass(self): self.assertEqual(42,42)\n    def test_fail(self): self.assertEqual(41,42)\n    @unittest.skip('GUI skipped')\n    def test_skip(self): pass\n").unwrap();
        h.state_mut().open_python_tests(true);
        ready(&mut h);
        h.run();
        assert!(!root.join("IMPORTED").exists());
        h.get_by_role(Role::ComboBox).click();
        h.step();
        h.step();
        h.get_by_label(framework.label()).click();
        h.step();
        h.step();
        capture(
            &mut h,
            scale,
            &format!("{}-configuration", framework.label()),
        );
        h.get_by_label(TEST_TRUST).click();
        h.step();
        h.step();
        h.get_by_label("Executar suite Python").click();
        h.step();
        wait(&mut h, |a| !a.service.python_tests.is_busy());
        h.run();
        assert_eq!(
            h.state().service.python_tests.report.counts(),
            [1, 1, 0, 1],
            "{:?}",
            h.state().service.python_tests.report
        );
        assert!(root.join("IMPORTED").exists());
        assert!(h.state().tool_tab == ToolTab::PythonTests);
        for case in &h.state().service.python_tests.report.cases {
            assert_eq!(case.path.as_ref(), Some(&source), "{}", case.name);
            assert!(case.line.is_some());
        }
        let failed = h
            .state()
            .service
            .python_tests
            .report
            .cases
            .iter()
            .find(|c| c.outcome.is_failure())
            .unwrap()
            .name
            .clone();
        h.get_by_label(&format!("Teste {failed}")).click();
        h.step();
        h.step();
        capture(
            &mut h,
            scale,
            &format!("{}-shared-run-failure", framework.label()),
        );
        h.get_by_label(&format!("Teste {failed}")).click_secondary();
        h.step();
        h.step();
        h.get_by_label("Abrir fonte do teste").click();
        wait(&mut h, |a| !a.service.is_busy());
        assert_eq!(h.state().service.active_document().unwrap().path, source);
        h.get_by_label("Rerun Failed Tests").click();
        h.step();
        wait(&mut h, |a| !a.service.python_tests.is_busy());
        h.run();
        assert_eq!(h.state().service.python_tests.report.counts(), [0, 1, 0, 0]);
        h.get_by_label(&format!("Teste {failed}")).click();
        h.step();
        h.step();
        h.get_by_label("Run selected test").click();
        h.step();
        wait(&mut h, |a| !a.service.python_tests.is_busy());
        h.run();
        assert_eq!(h.state().service.python_tests.report.counts(), [0, 1, 0, 0]);
        h.get_by_label(&format!("Teste {failed}")).click();
        h.step();
        h.step();
        h.get_by_label("Debug selected Python test").click();
        h.step();
        h.step();
        assert!(h.state().debugger_ui.dialog);
        let draft = h.state().debugger_ui.draft.as_ref().unwrap();
        assert_eq!(draft.runtime, DebugRuntime::Python);
        assert_eq!(draft.module.as_deref(), Some(framework.label()));
        assert!(!draft.trusted);
        assert!(draft.program.is_none());
        assert!(h.state().service.debugger.sessions.is_empty());
        capture(
            &mut h,
            scale,
            &format!("{}-debug-selected-review", framework.label()),
        );
        h.get_by_label("Cancel").click();
        h.run();
        assert!(!h.state().debugger_ui.dialog);
        assert!(h.state().service.debugger.sessions.is_empty());
        assert!(h.state().service.php_tests.report.cases.is_empty());
        assert!(h.state().service.js_tests.report.cases.is_empty());
    }
}

#[test]
#[cfg(target_os = "linux")]
fn python_shared_run_stop_widget_cancels_and_reaps_the_real_test_process() {
    let (_dir, mut h, root) = setup(1.25, false, "# cancellation fixture\n");
    fs::write(root.join("test_wait.py"), "import os,time,unittest\nfrom pathlib import Path\nclass Waiting(unittest.TestCase):\n    def test_wait(self):\n        Path('test.pid').write_text(str(os.getpid()))\n        while True: time.sleep(0.02)\n").unwrap();
    h.state_mut().open_python_tests(true);
    ready(&mut h);
    h.run();
    h.get_by_role(Role::ComboBox).click();
    h.step();
    h.step();
    h.get_by_label("unittest").click();
    h.step();
    h.step();
    h.get_by_label(TEST_TRUST).click();
    h.step();
    h.step();
    h.get_by_label("Executar suite Python").click();
    h.step();
    wait(&mut h, |a| {
        a.service.python_tests.is_busy() && root.join("test.pid").is_file()
    });
    let pid: u32 = fs::read_to_string(root.join("test.pid"))
        .unwrap()
        .parse()
        .unwrap();
    let process = PathBuf::from(format!("/proc/{pid}"));
    assert!(process.exists());
    assert!(
        !h.get_by_label("Stop · Ctrl+F2")
            .accesskit_node()
            .is_disabled()
    );
    capture(&mut h, 1.25, "unittest-running-stop-enabled");
    h.get_by_label("Stop · Ctrl+F2").click();
    h.step();
    wait(&mut h, |a| !a.service.python_tests.is_busy());
    assert_eq!(
        h.state().service.python_tests.state,
        codisseu_core::python_tests::State::Cancelled
    );
    assert!(!process.exists(), "Stop must reap the target test process");
    capture(&mut h, 1.25, "unittest-stopped-process-reaped");
}

#[test]
fn python_tools_first_presentation_keeps_footer_stable_through_one_pointer_click() {
    for (scale, compact) in [(1., false), (1.25, false), (1., true), (1.25, true)] {
        let (_dir, mut h, root) = setup(
            scale,
            compact,
            "from pathlib import Path\nPath('UNEXPECTED').write_text('executed')\n",
        );
        for action in [Action::CreateVenv, Action::RunFile] {
            h.state_mut().open_python(action);
            ready(&mut h);
            h.run();
            // First interaction: no geometry settling or screenshot frames before the click.
            trust_once(&mut h);
            assert!(!h.state().terminal.has_sessions());
            assert!(!root.join(".venv").exists());
            assert!(!root.join("UNEXPECTED").exists());
            capture(
                &mut h,
                scale,
                &format!(
                    "first-click-{}-{}",
                    if compact { "compact" } else { "normal" },
                    if action == Action::CreateVenv {
                        "venv"
                    } else {
                        "run"
                    }
                ),
            );
            h.get_by_label("Cancelar").click();
            h.run();
            assert!(!h.state().python_ui.open);
        }
    }
}
