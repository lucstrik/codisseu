//! Real pytest/unittest, private CPython and disposable virtual environments.
use codisseu_core::python_tests::{Config, Framework, Outcome, Scope, Service, State, TestCommand};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    thread,
    time::{Duration, Instant},
};
fn wait(service: &mut Service) {
    let end = Instant::now() + Duration::from_secs(30);
    while service.is_busy() {
        service.poll();
        assert!(
            Instant::now() < end,
            "{:?} {:?}",
            service.state,
            service.report
        );
        thread::sleep(Duration::from_millis(10));
    }
}
fn run(service: &mut Service, config: Config, scope: Scope) {
    let root = config.root.clone();
    service
        .dispatch(TestCommand::Run { config, scope }, vec![root])
        .unwrap();
    wait(service);
}

#[test]
fn real_pytest_and_unittest_results_selection_failed_rerun_and_passive_profile() {
    for framework in [Framework::Pytest, Framework::Unittest] {
        let temp = tempfile::Builder::new()
            .prefix("codisseu-tests-ação with spaces-")
            .tempdir()
            .unwrap();
        let root = temp.path().canonicalize().unwrap();
        let source = root.join("test_weather.py");
        fs::write(&source,"import unittest\nfrom pathlib import Path\nclass Weather(unittest.TestCase):\n    def test_pass(self):\n        self.assertEqual(40 + 2, 42)\n    def test_fail(self):\n        self.assertTrue(Path(__file__).with_name('fixed').exists(), 'needs fix')\n    @unittest.skip('deliberate skip ação')\n    def test_skip(self):\n        pass\n").unwrap();
        let profile = root.join("profile/tests.json");
        let mut service = Service::new(profile.clone());
        let mut config = Config::new(root.clone());
        config.framework = framework;
        service
            .dispatch(TestCommand::Inspect(root.clone()), vec![root.clone()])
            .unwrap();
        wait(&mut service);
        assert_eq!(service.state, State::Idle);
        assert!(service.report.cases.is_empty());
        run(&mut service, config.clone(), Scope::Suite);
        assert_eq!(service.state, State::Failed, "{:?}", service.report);
        assert_eq!(service.report.counts(), [1, 1, 0, 1]);
        assert_eq!(service.report.exit_codes, [Some(1)]);
        for case in &service.report.cases {
            assert_eq!(case.path.as_ref(), Some(&source), "{}", case.name);
        }
        let skipped = service
            .report
            .cases
            .iter()
            .find(|c| c.outcome == Outcome::Skipped)
            .unwrap();
        assert_eq!(
            skipped.line,
            Some(8),
            "skip decorator keeps the original source line"
        );
        let failed = service
            .report
            .cases
            .iter()
            .position(|c| c.outcome.is_failure())
            .unwrap();
        let name = service.report.cases[failed].name.clone();
        assert_eq!(service.report.cases[failed].path.as_ref(), Some(&source));
        assert!(service.report.cases[failed].line.is_some());
        assert!(service.report.cases[failed].details.contains("needs fix"));
        assert!(service.can_rerun_failed());
        let debug = service.debug_selected(failed).unwrap();
        assert_eq!(debug.runtime, codisseu_core::debugger::DebugRuntime::Python);
        assert!(debug.module.is_some());
        assert!(!debug.trusted);
        assert!(debug.args.contains(&name));
        // The mutable presentation cannot replace the private rerun selection.
        service.report.cases[failed].name = "outside.fake_test".into();
        service.report.cases[failed].path = None;
        fs::write(root.join("fixed"), "fixed").unwrap();
        service
            .dispatch(TestCommand::RerunFailed, vec![root.clone()])
            .unwrap();
        wait(&mut service);
        assert_eq!(service.state, State::Passed, "{:?}", service.report);
        assert_eq!(service.report.counts(), [1, 0, 0, 0]);
        assert_eq!(service.report.cases[0].name, name);
        service
            .dispatch(TestCommand::RunSelected(0), vec![root.clone()])
            .unwrap();
        wait(&mut service);
        assert_eq!(service.report.counts(), [1, 0, 0, 0]);
        service
            .dispatch(TestCommand::RerunLast, vec![root.clone()])
            .unwrap();
        wait(&mut service);
        assert_eq!(service.state, State::Passed);
        assert_eq!(service.report.counts(), [1, 0, 0, 0]);
        run(&mut service, config.clone(), Scope::File(source));
        assert_eq!(service.report.counts(), [2, 0, 0, 1]);
        assert_eq!(service.state, State::Passed);
        let mut restored = Service::new(profile);
        restored
            .dispatch(TestCommand::Inspect(root.clone()), vec![root])
            .unwrap();
        wait(&mut restored);
        assert_eq!(restored.config, Some(config));
        assert_eq!(restored.state, State::Idle);
        assert!(restored.report.cases.is_empty());
        assert!(restored.command.is_empty());
    }
}

#[test]
fn real_unittest_explicit_venv_uses_its_own_dependency() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let environment = root.join(".venv");
    let python = codisseu_runtime::python().unwrap();
    let output = Command::new(python)
        .args(["-I", "-m", "venv", "--without-pip"])
        .arg(&environment)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let interpreter = environment.join("bin/python");
    let output = Command::new(&interpreter)
        .args([
            "-I",
            "-c",
            "import sysconfig; print(sysconfig.get_path('purelib'))",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let site = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    fs::write(site.join("codisseu_dependency.py"), "answer = 42\n").unwrap();
    fs::write(root.join("test_dependency.py"),"import unittest\nfrom codisseu_dependency import answer\nclass OwnEnvironment(unittest.TestCase):\n    def test_dependency(self):\n        self.assertEqual(answer, 42)\n").unwrap();
    let mut config = Config::new(root.clone());
    config.framework = Framework::Unittest;
    config.interpreter = interpreter.to_string_lossy().into_owned();
    let mut service = Service::new(root.join("profile/tests.json"));
    run(&mut service, config, Scope::Suite);
    assert_eq!(service.state, State::Passed, "{:?}", service.report);
    assert_eq!(service.report.counts(), [1, 0, 0, 0]);
    assert!(service.command.contains(interpreter.to_str().unwrap()));
    service.set_project_interpreter(root.clone(), Some(interpreter.clone()));
    let mut inherited = Config::new(root.clone());
    inherited.framework = Framework::Unittest;
    run(&mut service, inherited, Scope::Suite);
    assert_eq!(service.state, State::Passed, "{:?}", service.report);
    assert!(service.command.contains(interpreter.to_str().unwrap()));
    let profile: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("profile/tests.json")).unwrap()).unwrap();
    assert_eq!(
        profile["projects"][root.to_str().unwrap()]["interpreter"],
        "",
        "resolved project fallback must not become a persistent test override"
    );
    service.set_project_interpreter(root.clone(), Some(codisseu_runtime::python().unwrap()));
    let mut missing = Config::new(root.clone());
    missing.framework = Framework::Pytest;
    missing.interpreter = interpreter.to_string_lossy().into_owned();
    run(&mut service, missing, Scope::Suite);
    assert_eq!(service.state, State::Error);
    assert!(service.report.error.as_ref().unwrap().contains("pytest"));
    assert!(
        service.report.cases.is_empty(),
        "missing pytest must not fall back to a different environment"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn cancel_and_timeout_terminate_only_owned_python_test_process() {
    for cancel in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let pid = root.join("pid");
        fs::write(root.join("test_wait.py"),"import unittest, os, time\nfrom pathlib import Path\nclass Waiting(unittest.TestCase):\n    def test_wait(self):\n        Path('pid').write_text(str(os.getpid()))\n        while True:\n            time.sleep(0.02)\n").unwrap();
        let mut config = Config::new(root.clone());
        config.framework = Framework::Unittest;
        config.timeout_secs = if cancel { 30 } else { 1 };
        let mut service = Service::new(root.join("profile/tests.json"));
        service
            .dispatch(
                TestCommand::Run {
                    config,
                    scope: Scope::Suite,
                },
                vec![root.clone()],
            )
            .unwrap();
        let end = Instant::now() + Duration::from_secs(15);
        while !pid.is_file() {
            service.poll();
            assert!(service.is_busy(), "{:?}", service.report);
            assert!(Instant::now() < end);
            thread::sleep(Duration::from_millis(10));
        }
        let pid: u32 = fs::read_to_string(pid).unwrap().parse().unwrap();
        let proc = PathBuf::from(format!("/proc/{pid}"));
        assert!(proc.exists());
        if cancel {
            service.dispatch(TestCommand::Cancel, vec![root]).unwrap();
        }
        wait(&mut service);
        assert_eq!(
            service.state,
            if cancel {
                State::Cancelled
            } else {
                State::Error
            }
        );
        assert!(!proc.exists());
        assert_eq!(service.report.exit_codes, [None]);
        if !cancel {
            assert!(service.report.error.as_ref().unwrap().contains("tempo"));
        }
    }
}

#[test]
fn invalid_profile_and_outside_file_do_not_execute_tests_or_overwrite_profile() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let profile = root.join("tests.json");
    let original = "{\"schema\":999,\"projects\":{}}";
    fs::write(&profile, original).unwrap();
    fs::write(
        root.join("test_never.py"),
        "from pathlib import Path\nPath('EXECUTED').write_text('bad')\n",
    )
    .unwrap();
    let mut service = Service::new(profile.clone());
    run(&mut service, Config::new(root.clone()), Scope::Suite);
    assert_eq!(service.state, State::Error);
    assert_eq!(fs::read_to_string(profile).unwrap(), original);
    assert!(!root.join("EXECUTED").exists());
    let outside = tempfile::tempdir().unwrap();
    let file = outside.path().join("test_external.py");
    fs::write(&file, "raise RuntimeError('must not run')\n").unwrap();
    let mut service = Service::new(root.join("valid-profile/tests.json"));
    run(&mut service, Config::new(root.clone()), Scope::File(file));
    assert_eq!(service.state, State::Error);
    assert!(
        service
            .report
            .error
            .as_ref()
            .unwrap()
            .contains("fora do projeto")
    );
    assert!(!root.join("EXECUTED").exists());
}

#[test]
fn real_collection_error_and_zero_tests_are_not_success() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut service = Service::new(root.join("profile/tests.json"));
    run(&mut service, Config::new(root.clone()), Scope::Suite);
    assert_eq!(service.state, State::Error);
    assert!(service.report.error.as_ref().unwrap().contains("Nenhum"));
    fs::write(root.join("test_broken.py"), "this is not valid python!\n").unwrap();
    run(&mut service, Config::new(root.clone()), Scope::Suite);
    assert_eq!(service.state, State::Failed, "{:?}", service.report);
    assert!(
        service
            .report
            .cases
            .iter()
            .any(|c| c.outcome == codisseu_core::python_tests::Outcome::Error)
    );
    fs::write(root.join("test_broken.py"), "import unittest\nclass BrokenClass(unittest.TestCase):\n    @classmethod\n    def setUpClass(cls):\n        raise RuntimeError('class setup error')\n    def test_case(self):\n        pass\n").unwrap();
    let mut config = Config::new(root.clone());
    config.framework = Framework::Unittest;
    run(&mut service, config, Scope::Suite);
    assert_eq!(service.state, State::Failed, "{:?}", service.report);
    assert_eq!(service.report.counts(), [0, 0, 1, 0]);
}
