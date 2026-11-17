use codisseu_core::{
    AppService, Command,
    debugger::{DebugCommand, DebugState},
    js_tests::{Config, Framework, Scope, Service, State, TestCommand},
};
use std::{
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
fn root() -> PathBuf {
    PathBuf::from(
        std::env::var_os("CODISSEU_JS_TEST_FIXTURE")
            .expect("set CODISSEU_JS_TEST_FIXTURE to the installed private fixture"),
    )
    .canonicalize()
    .unwrap()
}
fn config(root: &Path, framework: Framework) -> Config {
    let mut c = Config::new(root.into());
    c.framework = framework;
    c.timeout_secs = 30;
    c.configuration = if framework == Framework::Vitest {
        "vitest.config.mjs"
    } else {
        "jest.config.cjs"
    }
    .into();
    c
}
fn idle(s: &mut Service) {
    let end = Instant::now() + Duration::from_secs(45);
    while s.is_busy() {
        s.poll();
        assert!(
            Instant::now() < end,
            "timeout {:?} {:?}",
            s.state,
            s.report.error
        );
        thread::sleep(Duration::from_millis(4));
    }
}
fn run(s: &mut Service, c: TestCommand, root: &Path) {
    s.dispatch(c, vec![root.into()]).unwrap();
    idle(s);
    assert!(
        s.report.error.is_none(),
        "{:?}\n{}",
        s.report.error,
        s.report.output
    );
}
fn exercise(framework: Framework) {
    let root = root();
    let temp = tempfile::tempdir().unwrap();
    let mut s = Service::new(temp.path().join("tests.json"));
    run(
        &mut s,
        TestCommand::Run {
            config: config(&root, framework),
            scope: Scope::Suite,
        },
        &root,
    );
    assert_eq!(s.state, State::Failed);
    assert_eq!(
        s.report.counts(),
        [
            if matches!(framework, Framework::Jest) {
                4
            } else {
                3
            },
            1,
            0,
            1
        ]
    );
    assert!(s.report.cases.iter().all(|c| c.path.is_some()));
    let failed = s
        .report
        .cases
        .iter()
        .position(|c| c.outcome.is_failure())
        .unwrap();
    assert!(s.report.cases[failed].line.is_some());
    let name = s.report.cases[failed].name.clone();
    run(&mut s, TestCommand::RunSelected(failed), &root);
    assert_eq!(s.report.cases.len(), 1);
    assert_eq!(s.report.cases[0].name, name);
    run(&mut s, TestCommand::RerunFailed, &root);
    assert_eq!(s.report.cases.len(), 1);
    run(&mut s, TestCommand::RerunLast, &root);
    assert_eq!(s.report.cases.len(), 1);
    let file = root.join(if framework == Framework::Vitest {
        "tests/example.test.ts"
    } else {
        "tests/example.test.cjs"
    });
    run(
        &mut s,
        TestCommand::Run {
            config: config(&root, framework),
            scope: Scope::File(file),
        },
        &root,
    );
    assert_eq!(s.report.counts(), [3, 1, 0, 1]);
    let dataset = s
        .report
        .cases
        .iter()
        .position(|c| c.name.contains("dataset ação"))
        .unwrap();
    run(&mut s, TestCommand::RunSelected(dataset), &root);
    assert_eq!(s.state, State::Passed);
    assert_eq!(s.report.cases.len(), 1);
}
#[test]
#[ignore = "requires private real Vitest fixture"]
fn real_vitest() {
    exercise(Framework::Vitest);
}
#[test]
#[ignore = "requires private real Jest fixture"]
fn real_jest() {
    exercise(Framework::Jest);
}
fn app_idle(a: &mut AppService) {
    let end = Instant::now() + Duration::from_secs(45);
    while a.is_busy() || a.js_tests.is_busy() {
        a.poll();
        assert!(
            Instant::now() < end,
            "timeout {:?} {:?}",
            a.js_tests.state,
            a.js_tests.report.error
        );
        thread::sleep(Duration::from_millis(4));
    }
}
#[test]
#[ignore = "requires private real JS fixture"]
fn save_reruns_and_dirty_buffers_block() {
    let root = root();
    let t = tempfile::tempdir().unwrap();
    let mut a = AppService::new(t.path().join("session"));
    a.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    app_idle(&mut a);
    a.dispatch(Command::OpenFile(root.join("value.ts")))
        .unwrap();
    app_idle(&mut a);
    a.dispatch(Command::JsTests(TestCommand::Run {
        config: config(&root, Framework::Vitest),
        scope: Scope::Suite,
    }))
    .unwrap();
    app_idle(&mut a);
    a.dispatch(Command::JsTests(TestCommand::Watch(true)))
        .unwrap();
    let doc = a.active_document().unwrap();
    let id = doc.id;
    let text = doc.text().to_string();
    let replacement = format!("{text}\n// saved fixture\n");
    a.dispatch(Command::Edit {
        id,
        expected_version: a.active_document().unwrap().version,
        text: replacement,
    })
    .unwrap();
    assert!(
        a.dispatch(Command::JsTests(TestCommand::RerunLast))
            .is_err()
    );
    let rev = a.js_tests.revision;
    a.dispatch(Command::Save { id }).unwrap();
    let end = Instant::now() + Duration::from_secs(30);
    while a.js_tests.revision <= rev + 1 || a.js_tests.is_busy() || a.js_tests.watch_pending {
        a.poll();
        assert!(Instant::now() < end, "watch did not rerun");
        thread::sleep(Duration::from_millis(8));
    }
    assert_eq!(a.js_tests.report.counts(), [3, 1, 0, 1]);
    a.dispatch(Command::JsTests(TestCommand::Cancel)).unwrap();
    assert!(!a.js_tests.watching);
    a.dispatch(Command::Edit {
        id,
        expected_version: a.active_document().unwrap().version,
        text,
    })
    .unwrap();
    a.dispatch(Command::Save { id }).unwrap();
    app_idle(&mut a);
}
fn debug(framework: Framework) {
    let root = root();
    let t = tempfile::tempdir().unwrap();
    let mut a = AppService::new(t.path().join("session"));
    a.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    app_idle(&mut a);
    a.dispatch(Command::JsTests(TestCommand::Run {
        config: config(&root, framework),
        scope: Scope::Suite,
    }))
    .unwrap();
    app_idle(&mut a);
    let index = a
        .js_tests
        .report
        .cases
        .iter()
        .position(|c| c.name.ends_with("soma"))
        .unwrap();
    let file = a.js_tests.report.cases[index].path.clone().unwrap();
    let line = if framework == Framework::Vitest { 6 } else { 4 };
    a.debugger
        .execute(DebugCommand::SetBreakpoints {
            path: file.clone(),
            lines: vec![line],
        })
        .unwrap();
    a.dispatch(Command::JsTests(TestCommand::DebugSelected(index)))
        .unwrap();
    let end = Instant::now() + Duration::from_secs(40);
    let mut paused = false;
    let mut evaluated = false;
    loop {
        a.poll();
        if let Some(s) = a.debugger.sessions.last() {
            assert!(s.state != DebugState::Failed, "{:?}", s.last_error);
            if s.state == DebugState::Stopped && !s.frames.is_empty() {
                let id = s.id;
                assert!(
                    s.frames.iter().any(|f| f.path.as_ref() == Some(&file)),
                    "{:?}",
                    s.frames
                );
                if !paused {
                    paused = true;
                    a.debugger
                        .execute(DebugCommand::Evaluate {
                            session: id,
                            expression: "answer".into(),
                        })
                        .unwrap();
                } else if !evaluated
                    && a.debugger
                        .sessions
                        .last()
                        .unwrap()
                        .evaluations
                        .iter()
                        .any(|e| e.result == "42")
                {
                    evaluated = true;
                    a.debugger.execute(DebugCommand::Continue(id)).unwrap();
                }
            }
        }
        if !a.js_tests.is_busy() {
            break;
        }
        assert!(
            Instant::now() < end,
            "debug timeout: {:?} {:?}",
            a.js_tests.report.error,
            a.debugger
                .sessions
                .last()
                .map(|s| (&s.state, &s.last_error, &s.frames))
        );
        thread::sleep(Duration::from_millis(8));
    }
    assert!(paused && evaluated);
    assert_eq!(
        a.js_tests.state,
        State::Passed,
        "{:?}\n{}",
        a.js_tests.report.error,
        a.js_tests.report.output
    );
    assert_eq!(a.js_tests.report.cases.len(), 1);
}
#[test]
#[ignore = "requires real Node DAP and Vitest"]
fn real_vitest_debug() {
    debug(Framework::Vitest);
}
#[test]
#[ignore = "requires real Node DAP and Jest"]
fn real_jest_debug() {
    debug(Framework::Jest);
}

#[test]
#[ignore = "requires real Jest/Node; creates only own temporary test file"]
fn real_timeout_and_cancel() {
    let root = root();
    let path = root.join("tests/codisseu-owned-slow.test.cjs");
    assert!(!path.exists());
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _cleanup = Cleanup(path.clone());
    std::fs::write(
        &path,
        "test('slow',async()=>{await new Promise(r=>setTimeout(r,30000))},60000);\n",
    )
    .unwrap();
    let profile = tempfile::tempdir().unwrap();
    let mut service = Service::new(profile.path().join("tests.json"));
    let mut c = config(&root, Framework::Jest);
    c.timeout_secs = 1;
    service
        .dispatch(
            TestCommand::Run {
                config: c.clone(),
                scope: Scope::File(path.clone()),
            },
            vec![root.clone()],
        )
        .unwrap();
    idle(&mut service);
    assert_eq!(service.state, State::Error);
    assert!(service.report.error.as_ref().unwrap().contains("Prazo"));
    c.timeout_secs = 30;
    service
        .dispatch(
            TestCommand::Run {
                config: c,
                scope: Scope::File(path),
            },
            vec![root],
        )
        .unwrap();
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(500) {
        service.poll();
        thread::sleep(Duration::from_millis(8));
    }
    service.dispatch(TestCommand::Cancel, vec![]).unwrap();
    idle(&mut service);
    assert_eq!(service.state, State::Cancelled);
    assert!(service.report.cases.is_empty());
}
