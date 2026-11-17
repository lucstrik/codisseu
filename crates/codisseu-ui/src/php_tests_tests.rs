//! Real egui widget acceptance; framework executions are separate ignored tests.
use super::*;
use codisseu_core::{AppService, Command, ShellLayout};
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

type H = Harness<'static, CodisseuApp>;

fn js_capture(h: &mut H, scale: f32, name: &str) {
    let Some(base) = std::env::var_os("CODISSEU_JS_TEST_CAPTURES") else {
        return;
    };
    let path = PathBuf::from(base).join(if scale == 1. { "100" } else { "125" });
    fs::create_dir_all(&path).unwrap();
    h.state()
        .service
        .js_tests
        .report
        .cases
        .iter()
        .for_each(|c| assert!(c.path.is_some()));
    h.step();
    h.step();
    h.render()
        .unwrap()
        .save(path.join(format!("{name}.png")))
        .unwrap();
}
fn js_workflow(framework: codisseu_core::js_tests::Framework) {
    let root = PathBuf::from(
        std::env::var_os("CODISSEU_JS_TEST_FIXTURE").expect("installed private JS fixture"),
    )
    .canonicalize()
    .unwrap();
    for scale in [1., 1.25] {
        let profile = tempfile::tempdir().unwrap();
        let mut h = harness(&root, profile.path(), scale);
        menu(&mut h, "Testes JavaScript/TypeScript · Vitest/Jest…");
        wait(&mut h, |a| !a.service.js_tests.is_busy());
        assert!(h.state().js_tests_ui.settings);
        assert!(h.state().service.js_tests.report.cases.is_empty());
        h.get_by_role_and_label(Role::ComboBox, "Framework JS/TS")
            .click();
        h.step();
        h.step();
        h.get_by_label(framework.label()).click();
        h.step();
        h.step();
        field(
            &mut h,
            "Configuração JS/TS",
            if framework == codisseu_core::js_tests::Framework::Vitest {
                "vitest.config.mjs"
            } else {
                "jest.config.cjs"
            },
        );
        js_capture(
            &mut h,
            scale,
            &format!("{}-configuration", framework.label()),
        );
        h.get_by_label("Executar suite JS/TS").click();
        h.step();
        wait(&mut h, |a| !a.service.js_tests.is_busy());
        assert_eq!(
            h.state().service.js_tests.report.counts(),
            [
                if framework == codisseu_core::js_tests::Framework::Jest {
                    4
                } else {
                    3
                },
                1,
                0,
                1
            ],
            "{:?}",
            h.state().service.js_tests.report.error
        );
        assert!(h.state().tool_tab == ToolTab::JsTests);
        js_capture(&mut h, scale, &format!("{}-suite", framework.label()));
        let failed = h
            .state()
            .service
            .js_tests
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
        assert!(
            h.state()
                .php_tests_ui
                .console
                .as_ref()
                .unwrap()
                .text
                .contains("41")
        );
        h.get_by_label("Show Passed").click();
        h.step();
        h.step();
        assert!(h.query_by_label("Teste Cálculo 😀 soma").is_none());
        js_capture(&mut h, scale, &format!("{}-failure", framework.label()));
        h.get_by_label(&format!("Teste {failed}")).click_secondary();
        h.step();
        h.step();
        h.get_by_label("Abrir fonte do teste").click();
        wait(&mut h, |a| !a.service.is_busy());
        let path = h.state().service.active_document().unwrap().path.clone();
        assert!(path.starts_with(&root));
        assert!(path.extension().is_some_and(|e| e == "ts" || e == "cjs"));
        h.get_by_label("Rerun Failed Tests").click();
        h.step();
        wait(&mut h, |a| !a.service.js_tests.is_busy());
        assert_eq!(h.state().service.js_tests.report.counts(), [0, 1, 0, 0]);
        js_capture(
            &mut h,
            scale,
            &format!("{}-rerun-failed", framework.label()),
        );
        h.get_by_label("Rerun Automatically · after Save").click();
        h.step();
        h.step();
        assert!(h.state().service.js_tests.watching);
        js_capture(&mut h, scale, &format!("{}-auto-save", framework.label()));
        h.key_press_modifiers(Modifiers::CTRL, Key::F2);
        h.step();
        assert!(!h.state().service.js_tests.watching);
        let case = &h.state().service.js_tests.report.cases[0];
        let source = case.path.clone().unwrap();
        let line = case.line.unwrap();
        h.state_mut()
            .service
            .debugger
            .execute(codisseu_core::debugger::DebugCommand::SetBreakpoints {
                path: source,
                lines: vec![line],
            })
            .unwrap();
        h.get_by_label("Debug selected test").click();
        h.step();
        wait(&mut h, |a| {
            a.service.debugger.sessions.last().is_some_and(|s| {
                s.state == codisseu_core::debugger::DebugState::Stopped && !s.frames.is_empty()
            })
        });
        assert!(h.state().tool_tab == ToolTab::Debug);
        js_capture(
            &mut h,
            scale,
            &format!("{}-debug-paused", framework.label()),
        );
        h.get_by_label("Resume Program  F9").click();
        h.step();
        wait(&mut h, |a| !a.service.js_tests.is_busy());
        assert_eq!(
            h.state().service.js_tests.state,
            State::Failed,
            "{:?}",
            h.state().service.js_tests.report.error
        );
        h.get_by_label("Run").click();
        h.step();
        h.step();
        assert!(h.state().tool_tab == ToolTab::JsTests);
        // Same renderer must preserve independent PHP reports/configuration.
        assert!(h.state().service.php_tests.report.cases.is_empty());
    }
}
#[test]
#[ignore = "real Vitest installed in explicitly supplied private fixture"]
fn real_js_vitest_widgets() {
    js_workflow(codisseu_core::js_tests::Framework::Vitest);
}
#[test]
#[ignore = "real Jest installed in explicitly supplied private fixture"]
fn real_js_jest_widgets() {
    js_workflow(codisseu_core::js_tests::Framework::Jest);
}

fn wait(h: &mut H, predicate: impl Fn(&CodisseuApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        h.step();
        if predicate(h.state()) {
            h.step();
            return;
        }
        assert!(
            Instant::now() < deadline,
            "PHP tests UI timeout: status={:?}, error={:?}, test_state={:?}, test_error={:?}, config_error={:?}",
            h.state().service.status,
            h.state().service.last_error,
            h.state().service.php_tests.state,
            h.state().service.php_tests.report.error,
            h.state().php_tests_ui.config_error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn idle(h: &mut H) {
    wait(h, |a| {
        !a.service.is_busy() && !a.service.php_tests.is_busy()
    });
}
fn harness(root: &Path, profile: &Path, scale: f32) -> H {
    let mut service = AppService::new(profile.join("session.json"));
    service
        .dispatch(Command::OpenWorkspace(root.to_path_buf()))
        .unwrap();
    service
        .dispatch(Command::SetShellLayout(ShellLayout {
            bottom_height: Some(360.),
            ..Default::default()
        }))
        .unwrap();
    let mut h = Harness::builder()
        .with_size([1280., 900.])
        .with_pixels_per_point(scale)
        .with_max_steps(150)
        .build_ui_state(
            |ui, app: &mut CodisseuApp| app.show(ui),
            CodisseuApp::new(service),
        );
    idle(&mut h);
    h
}
fn menu(h: &mut H, label: &str) {
    if h.query_by_label("Executar").is_none() {
        h.get_by_label("Menu principal").click();
        h.step();
        h.step();
    }
    h.get_by_label("Executar").click();
    h.step();
    h.step();
    h.get_by_label(label).click();
    h.step();
    h.step();
}
fn settings(h: &mut H) {
    menu(h, "Testes PHP · PHPUnit/Pest…");
    idle(h);
    assert!(h.state().php_tests_ui.settings);
}
fn field(h: &mut H, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(egui::Event::Text(value.into()));
    h.step();
    h.step();
}
fn screenshot(h: &mut H, scale: f32, name: &str) {
    if std::env::var_os("CODISSEU_PHP_TEST_CAPTURES").is_none() {
        return;
    }
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/php-tests-20261030/ui")
        .join(if scale == 1. { "100" } else { "125" });
    fs::create_dir_all(&directory).unwrap();
    h.step();
    h.step();
    h.render()
        .unwrap()
        .save(directory.join(format!("{name}.png")))
        .unwrap();
}
fn open_file(h: &mut H, path: &Path) {
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(path.to_path_buf()))
        .unwrap();
    idle(h);
}
fn choose_framework(h: &mut H, framework: Framework) {
    h.get_by_role(Role::ComboBox).click();
    h.step();
    h.step();
    h.query_all_by_label(framework.label())
        .last()
        .unwrap()
        .click();
    h.step();
    h.step();
    assert_eq!(
        h.state().php_tests_ui.draft.as_ref().unwrap().framework,
        framework
    );
}
fn configure(h: &mut H, framework: Framework, xml: &str) {
    settings(h);
    choose_framework(h, framework);
    field(h, "Interpretador PHP", "php");
    field(
        h,
        "Runner",
        if framework == Framework::Pest {
            "vendor/bin/pest"
        } else {
            "vendor/bin/phpunit"
        },
    );
    field(h, "Configuração XML", xml);
}
fn execute_suite(h: &mut H) {
    h.get_by_label("Executar suite").click();
    h.step();
    wait(h, |a| {
        !a.service.php_tests.is_busy() && !a.php_tests_ui.settings
    });
    assert_eq!(
        h.state().service.php_tests.state,
        State::Failed,
        "{:?}",
        h.state().service.php_tests.report.error
    );
    assert_eq!(h.state().service.php_tests.report.counts(), [3, 1, 1, 1]);
}

#[test]
fn php_tests_open_and_configuration_do_not_execute_project_code() {
    for scale in [1., 1.25] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("PHP testes ação");
        fs::create_dir_all(root.join("vendor/bin")).unwrap();
        fs::write(
            root.join("composer.json"),
            "{\"name\":\"codisseu/no-execution\"}",
        )
        .unwrap();
        fs::write(
            root.join("vendor/bin/phpunit"),
            "<?php file_put_contents(__DIR__.'/UNEXPECTED', 'executed');",
        )
        .unwrap();
        let mut h = harness(&root, &dir.path().join("profile"), scale);
        settings(&mut h);
        assert_eq!(h.state().service.php_tests.state, State::Idle);
        assert!(h.state().service.php_tests.report.cases.is_empty());
        assert!(h.state().service.php_tests.report.exit_codes.is_empty());
        assert!(h.state().service.php_tests.command.is_empty());
        field(
            &mut h,
            "Interpretador PHP",
            "php configured without execution",
        );
        field(&mut h, "Runner", "vendor/bin/phpunit");
        field(&mut h, "Configuração XML", "phpunit.xml");
        screenshot(&mut h, scale, "configuration-no-execution");
        h.get_by_label("Cancelar").click();
        h.step();
        h.step();
        assert!(!h.state().php_tests_ui.settings);
        assert!(!root.join("vendor/bin/UNEXPECTED").exists());
        assert!(h.state().service.php_tests.command.is_empty());
        assert!(h.state().service.php_tests.report.exit_codes.is_empty());
        assert!(h.state().tool_tab == ToolTab::PhpTests);
        screenshot(&mut h, scale, "empty-panel");
    }
}

/// Controlled presentation model: this test does not execute PHPUnit or Pest.
#[test]
fn partial_controlled_report_keeps_cancel_and_runner_error_visible() {
    use codisseu_core::php_tests::{RunReport, TestCase};
    for scale in [1., 1.25] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("controlled partial presentation");
        fs::create_dir_all(&root).unwrap();
        let mut h = harness(&root, &dir.path().join("profile"), scale);
        {
            let app = h.state_mut();
            app.tool_tab = ToolTab::PhpTests;
            app.show_activity = true;
            app.php_tests_ui.selected = Some(1);
            app.service.php_tests.state = State::Cancelled;
            app.service.php_tests.report = RunReport {
                cases: [Outcome::Passed, Outcome::Failed]
                    .into_iter()
                    .enumerate()
                    .map(|(i, outcome)| TestCase {
                        ancestors: vec![],
                        title: String::new(),
                        name: format!("controlled_case_{i}"),
                        class: "ControlledPartial".into(),
                        path: None,
                        line: None,
                        outcome,
                        seconds: 0.001,
                        details: if outcome.is_failure() {
                            "Controlled earlier assertion failure".into()
                        } else {
                            String::new()
                        },
                    })
                    .collect(),
                output: "Saída parcial antes do cancelamento".into(),
                ..Default::default()
            };
            app.service.php_tests.revision = app.service.php_tests.revision.wrapping_add(1);
        }
        h.step();
        h.step();
        assert!(h.query_by_label("Execução cancelada").is_some());
        assert!(h.state().php_tests_ui.selected.is_none());
        assert_eq!(h.state().service.php_tests.report.counts(), [1, 1, 0, 0]);
        assert!(
            h.state()
                .php_tests_ui
                .console
                .as_ref()
                .unwrap()
                .text
                .contains("Saída parcial antes do cancelamento")
        );
        assert!(
            !h.state()
                .php_tests_ui
                .console
                .as_ref()
                .unwrap()
                .text
                .contains("Controlled earlier assertion failure")
        );
        assert!(h.state().service.php_tests.report.exit_codes.is_empty());
        screenshot(&mut h, scale, "controlled-partial-cancelled");

        {
            let app = h.state_mut();
            app.php_tests_ui.selected = Some(1);
            app.service.php_tests.state = State::Error;
            app.service.php_tests.report.error =
                Some("bootstrap controlado: relatório parcial indisponível".into());
            app.service.php_tests.revision = app.service.php_tests.revision.wrapping_add(1);
        }
        h.step();
        h.step();
        assert!(h.query_by_label("Erro de execução").is_some());
        assert!(
            h.query_by_label("bootstrap controlado: relatório parcial indisponível")
                .is_some()
        );
        assert!(h.state().php_tests_ui.selected.is_none());
        assert!(
            h.state()
                .php_tests_ui
                .console
                .as_ref()
                .unwrap()
                .text
                .contains("bootstrap controlado:")
        );
        assert!(
            !h.state()
                .php_tests_ui
                .console
                .as_ref()
                .unwrap()
                .text
                .contains("Controlled earlier assertion failure")
        );
        assert_eq!(h.state().service.php_tests.report.counts(), [1, 1, 0, 0]);
        assert!(h.state().service.php_tests.report.exit_codes.is_empty());
        screenshot(&mut h, scale, "controlled-partial-runner-error");
    }
}

fn real_fixture() -> PathBuf {
    let root = std::env::var_os("CODISSEU_PHP_TEST_FIXTURE")
        .map(PathBuf::from)
        .expect("Set CODISSEU_PHP_TEST_FIXTURE to the installed own PHPUnit/Pest fixture");
    let root = root.canonicalize().unwrap();
    assert!(root.join("vendor/bin/phpunit").is_file());
    assert!(root.join("vendor/bin/pest").is_file());
    root
}
fn real_workflow(framework: Framework) {
    let root = real_fixture();
    let (xml, relative, failed, skipped, passed, dataset) = if framework == Framework::Pest {
        (
            "pest.xml",
            "tests/Pest/ExampleTest.php",
            "it fails",
            "skipped",
            "passes ação 😀",
            "dataset with data set \"dataset \"ação 😀\"\"",
        )
    } else {
        (
            "phpunit.xml",
            "tests/PhpUnit/ExampleTest.php",
            "testFails",
            "testSkips",
            "testPasses",
            "testDataset with data set \"ação 😀\"",
        )
    };
    let path = root.join(relative);
    let original = fs::read(&path).unwrap();
    for scale in [1., 1.25] {
        let profile = tempfile::tempdir().unwrap();
        let mut h = harness(&root, profile.path(), scale);
        open_file(&mut h, &path);
        configure(&mut h, framework, xml);
        screenshot(
            &mut h,
            scale,
            &format!("{}-configuration", framework.label()),
        );
        execute_suite(&mut h);
        screenshot(&mut h, scale, &format!("{}-suite", framework.label()));

        h.get_by_label("Show Passed").click();
        h.step();
        h.step();
        h.get_by_label("Show Ignored").click();
        h.step();
        h.step();
        assert!(h.query_by_label(&format!("Teste {passed}")).is_none());
        assert!(h.query_by_label(&format!("Teste {skipped}")).is_none());
        assert!(h.query_by_label(&format!("Teste {failed}")).is_some());
        h.get_by_label(&format!("Teste {failed}")).click();
        h.step();
        h.step();
        let selected = h.state().php_tests_ui.selected.unwrap();
        assert_eq!(
            h.state().service.php_tests.report.cases[selected].name,
            failed
        );
        assert!(
            h.state()
                .php_tests_ui
                .console
                .as_ref()
                .unwrap()
                .text
                .contains("41")
        );
        screenshot(
            &mut h,
            scale,
            &format!("{}-failed-filter", framework.label()),
        );

        // Context menu uses the same source navigation as the link, through real clicks.
        h.get_by_label(&format!("Teste {failed}")).click_secondary();
        h.step();
        h.step();
        h.get_by_label("Abrir fonte do teste").click();
        idle(&mut h);
        let doc = h.state().service.active_document().unwrap();
        assert_eq!(doc.path, path);
        assert_eq!(doc.text().as_bytes(), original);
        let expected_line = h.state().service.php_tests.report.cases[selected]
            .line
            .unwrap();
        assert!(expected_line > 0);
        assert_eq!(
            doc.text()
                .chars()
                .take(doc.selection().head)
                .filter(|c| *c == '\n')
                .count() as u32
                + 1,
            expected_line
        );
        screenshot(&mut h, scale, &format!("{}-source", framework.label()));

        h.get_by_label("Rerun Failed Tests").click();
        h.step();
        wait(&mut h, |a| !a.service.php_tests.is_busy());
        assert_eq!(h.state().service.php_tests.report.counts(), [0, 1, 1, 0]);
        assert_eq!(h.state().service.php_tests.report.cases.len(), 2);
        h.get_by_label(&format!("Teste {failed}")).click();
        h.step();
        h.step();
        h.get_by_label("Run selected test").click();
        h.step();
        wait(&mut h, |a| !a.service.php_tests.is_busy());
        assert_eq!(h.state().service.php_tests.report.cases.len(), 1);
        assert_eq!(h.state().service.php_tests.report.cases[0].name, failed);
        screenshot(&mut h, scale, &format!("{}-individual", framework.label()));

        // Run active file via the menu. No fabricated cases are inserted into the model.
        menu(&mut h, "Executar arquivo de testes PHP");
        wait(&mut h, |a| !a.service.php_tests.is_busy());
        assert_eq!(h.state().service.php_tests.report.counts(), [3, 1, 1, 1]);
        h.get_by_label("Show Passed").click();
        h.step();
        h.step();
        h.get_by_label("Show Ignored").click();
        h.step();
        h.step();
        assert!(
            h.query_by_label(&format!("Teste {dataset}")).is_some(),
            "dataset not visible: {}",
            dataset
        );
        h.get_by_label(&format!("Teste {dataset}")).click();
        h.step();
        h.step();
        h.get_by_label("Run selected test").click();
        h.step();
        wait(&mut h, |a| !a.service.php_tests.is_busy());
        assert_eq!(h.state().service.php_tests.report.counts(), [1, 0, 0, 0]);
        assert_eq!(h.state().service.php_tests.report.cases[0].name, dataset);
        screenshot(&mut h, scale, &format!("{}-dataset", framework.label()));

        // A dirty buffer must prevent execution and must never be saved by the panel.
        let doc = h.state().service.active_document().unwrap();
        let command = Command::Edit {
            id: doc.id,
            expected_version: doc.version,
            text: format!("{}\n// dirty preserved\n", doc.text()),
        };
        h.state_mut().service.dispatch(command).unwrap();
        h.step();
        h.get_by_label("Rerun · Ctrl+F5").click();
        h.step();
        h.step();
        assert!(!h.state().service.php_tests.is_busy());
        assert!(
            h.state()
                .php_tests_ui
                .config_error
                .as_ref()
                .is_some_and(|e| e.contains("Salve"))
        );
        assert!(h.state().service.active_document().unwrap().is_dirty());
        assert_eq!(fs::read(&path).unwrap(), original);
        screenshot(
            &mut h,
            scale,
            &format!("{}-dirty-blocked", framework.label()),
        );
    }
    assert_eq!(fs::read(&path).unwrap(), original);
}
#[test]
#[ignore = "requires the installed own real PHPUnit/Pest fixture and PHP"]
fn phpunit_real_widgets_suite_file_individual_failed_dataset_and_dirty() {
    real_workflow(Framework::PhpUnit);
}
#[test]
#[ignore = "requires the installed own real PHPUnit/Pest fixture and PHP"]
fn pest_real_widgets_suite_file_individual_failed_dataset_and_dirty() {
    real_workflow(Framework::Pest);
}
