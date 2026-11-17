//! Real PHPUnit/Pest integrations require the explicitly installed private fixture.
//! No Composer/npm installation or third-party project is performed by these tests.
use codisseu_core::{
    AppService, Command,
    php_tests::{Config, Framework, Outcome, Scope, Service, State, TestCommand},
};
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

fn fixture() -> PathBuf {
    let path = std::env::var_os("CODISSEU_PHP_TEST_FIXTURE")
        .expect("set CODISSEU_PHP_TEST_FIXTURE to the installed fixture");
    let root = PathBuf::from(path).canonicalize().unwrap();
    assert!(root.join("vendor/bin/phpunit").is_file());
    assert!(root.join("vendor/bin/pest").is_file());
    root
}
fn config(root: &std::path::Path, framework: Framework) -> Config {
    let mut config = Config::new(root.into());
    config.framework = framework;
    config.php = std::env::var("CODISSEU_PHP_TEST_PHP").unwrap_or_else(|_| "/usr/bin/php".into());
    config.xml = if framework == Framework::Pest {
        "pest.xml"
    } else {
        "phpunit.xml"
    }
    .into();
    config.timeout_secs = 30;
    config
}
fn idle(service: &mut Service) {
    let deadline = Instant::now() + Duration::from_secs(45);
    while service.is_busy() {
        service.poll();
        assert!(
            Instant::now() < deadline,
            "timeout {:?} {:?}",
            service.state,
            service.report.error
        );
        thread::sleep(Duration::from_millis(3));
    }
}
fn dispatch(service: &mut Service, command: TestCommand, root: &std::path::Path) {
    service.dispatch(command, vec![root.into()]).unwrap();
    idle(service);
    assert!(
        service.report.error.is_none(),
        "{:?}\n{}",
        service.report.error,
        service.report.output
    );
}
fn exercise(framework: Framework) {
    let root = fixture();
    let profile = tempfile::tempdir().unwrap();
    let file = root.join(if framework == Framework::Pest {
        "tests/Pest/ExampleTest.php"
    } else {
        "tests/PhpUnit/ExampleTest.php"
    });
    let bytes_before = fs::read(&file).unwrap();
    let mut service = Service::new(profile.path().join("tests.json"));
    let conf = config(&root, framework);
    dispatch(
        &mut service,
        TestCommand::Run {
            config: conf.clone(),
            scope: Scope::Suite,
        },
        &root,
    );
    assert_eq!(service.state, State::Failed);
    assert_eq!(service.report.counts(), [3, 1, 1, 1]);
    assert_eq!(service.report.exit_codes.len(), 1);
    assert!(service.report.exit_codes[0].is_some_and(|c| c != 0));
    assert!(
        service
            .report
            .cases
            .iter()
            .all(|c| c.path.as_ref() == Some(&file))
    );
    let failed = service
        .report
        .cases
        .iter()
        .find(|c| c.outcome == Outcome::Failed)
        .unwrap();
    assert!(failed.line.is_some());
    assert!(failed.details.contains("41"));
    assert!(service.can_rerun_failed());
    dispatch(&mut service, TestCommand::RerunFailed, &root);
    assert_eq!(service.state, State::Failed);
    assert_eq!(service.report.counts(), [0, 1, 1, 0]);
    assert_eq!(
        service.report.cases.len(),
        2,
        "rerun must exclude passed/skipped"
    );
    dispatch(&mut service, TestCommand::RunSelected(0), &root);
    assert_eq!(service.report.cases.len(), 1);
    assert!(service.report.cases[0].outcome.is_failure());
    dispatch(
        &mut service,
        TestCommand::Run {
            config: conf,
            scope: Scope::File(file.clone()),
        },
        &root,
    );
    assert_eq!(service.report.counts(), [3, 1, 1, 1]);
    let dataset = service
        .report
        .cases
        .iter()
        .position(|c| c.name.contains("data set") && c.name.contains("ação 😀"))
        .unwrap();
    let dataset_name = service.report.cases[dataset].name.clone();
    dispatch(&mut service, TestCommand::RunSelected(dataset), &root);
    assert_eq!(service.state, State::Passed);
    assert_eq!(service.report.counts(), [1, 0, 0, 0]);
    assert_eq!(service.report.cases[0].name, dataset_name);
    dispatch(&mut service, TestCommand::RerunLast, &root);
    assert_eq!(service.report.cases.len(), 1);
    assert_eq!(service.report.cases[0].name, dataset_name);
    assert!(!service.can_rerun_failed());
    let mut restored = Service::new(profile.path().join("tests.json"));
    restored
        .dispatch(TestCommand::Inspect(root.clone()), vec![root.clone()])
        .unwrap();
    idle(&mut restored);
    assert_eq!(restored.config.as_ref().unwrap().framework, framework);
    assert_eq!(
        fs::read(&file).unwrap(),
        bytes_before,
        "the runner must preserve fixture source"
    );
}
#[test]
#[ignore = "requires real installed PHPUnit/Pest fixture"]
fn real_phpunit_suite_file_selected_dataset_failed_and_configuration() {
    exercise(Framework::PhpUnit);
}
#[test]
#[ignore = "requires real installed PHPUnit/Pest fixture"]
fn real_pest_suite_file_selected_dataset_failed_and_configuration() {
    exercise(Framework::Pest);
}

fn app_idle(app: &mut AppService) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while app.is_busy() || app.php_tests.is_busy() {
        app.poll();
        assert!(
            Instant::now() < deadline,
            "app timeout {:?}",
            app.last_error
        );
        thread::sleep(Duration::from_millis(3));
    }
}
#[test]
fn app_inspection_cannot_execute_dirty_source_or_an_unopened_project() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("own project ação");
    fs::create_dir(&root).unwrap();
    let path = root.join("ExampleTest.php");
    fs::write(&path, "<?php // initial\n").unwrap();
    let mut app = AppService::new(dir.path().join("profile/session.json"));
    app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    app_idle(&mut app);
    app.dispatch(Command::OpenFile(path.clone())).unwrap();
    app_idle(&mut app);
    let document = app.active_document().unwrap();
    let id = document.id;
    let version = document.version;
    app.dispatch(Command::Edit {
        id,
        expected_version: version,
        text: "<?php // unsaved ação 😀\n".into(),
    })
    .unwrap();
    let config = Config::new(root.clone());
    let error = app
        .dispatch(Command::PhpTests(TestCommand::Run {
            config,
            scope: Scope::Suite,
        }))
        .unwrap_err();
    assert!(error.contains("Salve"));
    assert!(!app.php_tests.is_busy());
    assert!(app.active_document().unwrap().is_dirty());
    assert_eq!(fs::read_to_string(&path).unwrap(), "<?php // initial\n");
    app.dispatch(Command::PhpTests(TestCommand::Inspect(root)))
        .unwrap();
    app_idle(&mut app);
    assert_eq!(app.php_tests.state, State::Idle);
    let outsider = tempfile::tempdir().unwrap();
    let error = app
        .dispatch(Command::PhpTests(TestCommand::Inspect(
            outsider.path().into(),
        )))
        .unwrap_err();
    assert!(error.contains("Abra o projeto"));
}

#[test]
#[ignore = "requires real installed PHPUnit/Pest fixture"]
fn real_phpunit_same_method_names_in_two_classes_rerun_by_file() {
    let installed = fixture();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("isolated tests ação");
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join("tests")).unwrap();
    let php_quote = |path: &std::path::Path| {
        format!(
            "'{}'",
            path.to_str()
                .unwrap()
                .replace('\\', "\\\\")
                .replace('\'', "\\'")
        )
    };
    // The private project bridge loads the reviewed installed dependency; sources
    // and configuration under test remain owned temporary files.
    fs::write(
        root.join("runner.php"),
        format!(
            "<?php require {};\n",
            php_quote(&installed.join("vendor/bin/phpunit"))
        ),
    )
    .unwrap();
    fs::write(
        root.join("bootstrap.php"),
        format!(
            "<?php require {};\n",
            php_quote(&installed.join("vendor/autoload.php"))
        ),
    )
    .unwrap();
    fs::write(root.join("phpunit.xml"), "<phpunit bootstrap='bootstrap.php' cacheResult='false'><testsuites><testsuite name='own'><directory>tests</directory></testsuite></testsuites></phpunit>").unwrap();
    let first = root.join("tests/FirstTest.php");
    let second = root.join("tests/SecondTest.php");
    fs::write(&first, "<?php\nuse PHPUnit\\Framework\\TestCase;\nfinal class FirstTest extends TestCase { public function testSameName():void { self::assertSame(42, 41); } public function testOther():void { self::assertTrue(true); } }\n").unwrap();
    fs::write(&second, "<?php\nuse PHPUnit\\Framework\\TestCase;\nfinal class SecondTest extends TestCase { public function testSameName():void { self::assertSame(42, 40); } }\n").unwrap();
    let mut conf = config(&root, Framework::PhpUnit);
    conf.runner = "runner.php".into();
    let mut service = Service::new(dir.path().join("profile/tests.json"));
    dispatch(
        &mut service,
        TestCommand::Run {
            config: conf,
            scope: Scope::Suite,
        },
        &root,
    );
    assert_eq!(service.report.counts(), [1, 2, 0, 0]);
    dispatch(&mut service, TestCommand::RerunFailed, &root);
    assert_eq!(service.report.counts(), [0, 2, 0, 0]);
    assert_eq!(service.report.exit_codes, vec![Some(1), Some(1)]);
    assert_eq!(
        service
            .report
            .cases
            .iter()
            .map(|c| c.class.as_str())
            .collect::<Vec<_>>(),
        vec!["FirstTest", "SecondTest"]
    );
    dispatch(&mut service, TestCommand::RunSelected(1), &root);
    assert_eq!(service.report.cases.len(), 1);
    assert_eq!(service.report.cases[0].class, "SecondTest");
    assert_eq!(service.report.cases[0].path.as_ref(), Some(&second));
    assert_eq!(service.report.counts(), [0, 1, 0, 0]);
}
