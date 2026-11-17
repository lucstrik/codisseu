//! Real private debugpy and disposable Python programs/venvs. No silent skips.
use codisseu_debug::*;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

fn wait(service: &mut DebugService, predicate: impl Fn(&DebugService) -> bool) {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        service.poll();
        assert!(
            !service
                .sessions
                .iter()
                .any(|s| s.state == DebugState::Failed),
            "{:?}",
            service.sessions
        );
        if predicate(service) {
            return;
        }
        assert!(Instant::now() < end, "timed out: {:?}", service.sessions);
        thread::sleep(Duration::from_millis(10));
    }
}
fn config(path: &Path) -> DebugConfiguration {
    let mut config = DebugConfiguration::for_file(path, path.parent().unwrap());
    config.trusted = true;
    config
}
fn evaluate(service: &mut DebugService, id: SessionId, expression: &str) -> String {
    service
        .execute(DebugCommand::Evaluate {
            session: id,
            expression: expression.into(),
        })
        .unwrap();
    wait(service, |s| {
        s.sessions[0]
            .evaluations
            .iter()
            .any(|e| e.expression == expression)
    });
    let result = service.sessions[0]
        .evaluations
        .iter()
        .find(|e| e.expression == expression)
        .unwrap();
    assert!(result.error.is_none(), "{result:?}");
    result.result.clone()
}

#[test]
fn real_python_breakpoint_locals_evaluate_steps_continue_and_exit() {
    let temp = tempfile::Builder::new()
        .prefix("codisseu-python-ação with spaces-")
        .tempdir()
        .unwrap();
    let path = temp.path().join("program.py");
    let source = "import sys\ndef doubled(value):\n    return value * 2\ndef main():\n    answer = 42\n    label = 'Codisseu ação 😀'\n    result = doubled(answer)\n    print(f'result={result};arg={sys.argv[1]}')\nmain()\n";
    fs::write(&path, source).unwrap();
    let mut config = config(&path);
    config.args = vec!["literal ; $(not_a_shell)".into()];
    let mut service = DebugService::new();
    service
        .execute(DebugCommand::SetBreakpoints {
            path: path.clone(),
            lines: vec![6],
        })
        .unwrap();
    let id = service
        .execute(DebugCommand::Start(config))
        .unwrap()
        .unwrap();
    wait(&mut service, |s| {
        s.sessions[0].state == DebugState::Stopped
            && s.sessions[0]
                .variables
                .values()
                .flatten()
                .any(|v| v.name == "answer")
    });
    let session = &service.sessions[0];
    assert_eq!(session.runtime, DebugRuntime::Python);
    assert_eq!(session.frames[0].path.as_ref(), Some(&path));
    assert_eq!(session.frames[0].line, 6);
    assert!(
        session
            .breakpoints
            .iter()
            .any(|b| b.line == 6 && b.verified)
    );
    assert!(
        session
            .variables
            .values()
            .flatten()
            .any(|v| v.name == "answer" && v.value == "42")
    );
    assert_eq!(evaluate(&mut service, id, "answer"), "42");
    let epoch = service.sessions[0].stop_epoch;
    service.execute(DebugCommand::Next(id)).unwrap();
    wait(&mut service, |s| {
        s.sessions[0].stop_epoch > epoch
            && s.sessions[0].frames.first().is_some_and(|f| f.line == 7)
    });
    let epoch = service.sessions[0].stop_epoch;
    service.execute(DebugCommand::StepIn(id)).unwrap();
    wait(&mut service, |s| {
        s.sessions[0].stop_epoch > epoch
            && s.sessions[0].frames.first().is_some_and(|f| f.line == 3)
    });
    let epoch = service.sessions[0].stop_epoch;
    service.execute(DebugCommand::StepOut(id)).unwrap();
    wait(&mut service, |s| {
        s.sessions[0].stop_epoch > epoch
            && s.sessions[0]
                .frames
                .first()
                .is_some_and(|f| f.name == "main")
    });
    service.execute(DebugCommand::Continue(id)).unwrap();
    wait(&mut service, |s| !s.has_active_sessions());
    assert_eq!(service.sessions[0].exit_code, Some(0));
    assert!(
        service.sessions[0]
            .output
            .iter()
            .any(|o| o.text.contains("result=84;arg=literal ; $(not_a_shell)")),
        "{:?}",
        service.sessions[0].output
    );
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}

#[test]
fn real_python_conditions_logpoints_and_hit_count() {
    for hit_count in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("loop.py");
        fs::write(
            &path,
            "for index in range(6):\n    value = index * 2\n    print(value)\n",
        )
        .unwrap();
        let mut service = DebugService::new();
        let mut stop = Breakpoint::new(path.clone(), 3);
        if hit_count {
            stop.options.hit_condition = "4".into();
        } else {
            stop.options.condition = "index == 3".into();
        }
        let mut log = Breakpoint::new(path.clone(), 2);
        log.options.log_message = "index={index}".into();
        service.execute(DebugCommand::SetBreakpoint(stop)).unwrap();
        service.execute(DebugCommand::SetBreakpoint(log)).unwrap();
        let id = service
            .execute(DebugCommand::Start(config(&path)))
            .unwrap()
            .unwrap();
        wait(&mut service, |s| {
            s.sessions[0].state == DebugState::Stopped
                && s.sessions[0].pending == 0
                && !s.sessions[0].variables.is_empty()
        });
        assert_eq!(evaluate(&mut service, id, "index"), "3");
        assert!(service.sessions[0].breakpoints.iter().all(|b| b.verified));
        let output = service.sessions[0]
            .output
            .iter()
            .map(|o| o.text.as_str())
            .collect::<String>();
        for index in 0..=3 {
            assert!(output.contains(&format!("index={index}")), "{output}");
        }
        service
            .execute(DebugCommand::SetBreakpointsEnabled(false))
            .unwrap();
        wait(&mut service, |s| {
            s.sessions[0].pending == 0 && s.sessions[0].breakpoints.iter().all(|b| !b.enabled)
        });
        service.execute(DebugCommand::Continue(id)).unwrap();
        wait(&mut service, |s| !s.has_active_sessions());
        assert_eq!(service.sessions[0].exit_code, Some(0));
    }
}

#[test]
#[cfg(target_os = "linux")]
fn real_python_stop_removes_owned_target_pid() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("long.py");
    let pid_path = temp.path().join("pid");
    fs::write(&path,"import os, sys, time\nwith open(sys.argv[1], 'w') as out:\n    out.write(str(os.getpid()))\nwhile True:\n    time.sleep(0.02)\n").unwrap();
    let mut config = config(&path);
    config.args = vec![pid_path.to_string_lossy().into_owned()];
    let mut service = DebugService::new();
    let id = service
        .execute(DebugCommand::Start(config))
        .unwrap()
        .unwrap();
    wait(&mut service, |_| pid_path.is_file());
    let pid: u32 = fs::read_to_string(pid_path).unwrap().parse().unwrap();
    let proc = PathBuf::from(format!("/proc/{pid}"));
    assert!(proc.exists());
    service.execute(DebugCommand::Stop(id)).unwrap();
    wait(&mut service, |s| !s.has_active_sessions());
    let end = Instant::now() + Duration::from_secs(5);
    while proc.exists() && Instant::now() < end {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!proc.exists(), "owned Python target survived Stop");
}

#[test]
fn real_python_explicit_venv_without_debugpy_runs_module_and_own_dependency() {
    let temp = tempfile::Builder::new()
        .prefix("codisseu-python-venv with spaces-")
        .tempdir()
        .unwrap();
    let environment = temp.path().join("environment");
    let python = codisseu_runtime::python().unwrap();
    let output = Command::new(&python)
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
    let output=Command::new(&interpreter).args(["-c","import importlib.util, sysconfig; assert importlib.util.find_spec('debugpy') is None; print(sysconfig.get_path('purelib'))"]).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let site = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    fs::write(site.join("codisseu_own_dependency.py"), "answer = 42\n").unwrap();
    fs::create_dir(temp.path().join("own_module")).unwrap();
    let source = temp.path().join("own_module/__main__.py");
    fs::write(&source,"from codisseu_own_dependency import answer\nresult = answer * 2\nprint(f'venv-result={result}')\n").unwrap();
    let config = DebugConfiguration {
        runtime: DebugRuntime::Python,
        executable: interpreter.clone(),
        program: None,
        module: Some("own_module".into()),
        cwd: temp.path().into(),
        trusted: true,
        adapter: Some(
            codisseu_runtime::python_debug_adapter()
                .unwrap()
                .1
                .join("adapter/__main__.py"),
        ),
        adapter_runtime: Some(codisseu_runtime::python().unwrap()),
        ..Default::default()
    };
    let mut service = DebugService::new();
    service
        .execute(DebugCommand::SetBreakpoints {
            path: source,
            lines: vec![2],
        })
        .unwrap();
    let id = service
        .execute(DebugCommand::Start(config))
        .unwrap()
        .unwrap();
    wait(&mut service, |s| {
        s.sessions[0].state == DebugState::Stopped && !s.sessions[0].variables.is_empty()
    });
    assert_eq!(evaluate(&mut service, id, "answer"), "42");
    let target = evaluate(&mut service, id, "__import__('sys').executable");
    assert!(target.contains(interpreter.to_str().unwrap()), "{target}");
    service.execute(DebugCommand::Continue(id)).unwrap();
    wait(&mut service, |s| !s.has_active_sessions());
    assert_eq!(service.sessions[0].exit_code, Some(0));
    assert!(
        service.sessions[0]
            .output
            .iter()
            .any(|o| o.text.contains("venv-result=84"))
    );
}

#[test]
fn python_target_validation_rejects_stubs_conflicts_and_attach_without_execution() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("type.pyi");
    fs::write(&path, "value: int\n").unwrap();
    let mut c = config(&path);
    assert_eq!(c.runtime, DebugRuntime::Python);
    assert!(c.program.is_none());
    let mut s = DebugService::new();
    assert!(s.execute(DebugCommand::Start(c.clone())).is_err());
    c.program = Some(path.clone());
    assert!(
        s.execute(DebugCommand::Start(c.clone()))
            .unwrap_err()
            .contains("stubs")
    );
    c.module = Some("unittest".into());
    assert!(
        s.execute(DebugCommand::Start(c.clone()))
            .unwrap_err()
            .contains("exactly one")
    );
    c.program = None;
    c.request = DebugRequest::Attach;
    assert!(
        s.execute(DebugCommand::Start(c))
            .unwrap_err()
            .contains("Launch")
    );
    assert!(s.sessions.is_empty());
    assert_eq!(
        DebugConfiguration::for_file(temp.path().join("app.pyw"), temp.path()).runtime,
        DebugRuntime::Python
    );
}
