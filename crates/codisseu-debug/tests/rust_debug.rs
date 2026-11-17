use codisseu_debug::*;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

fn wait(service: &mut DebugService, pred: impl Fn(&DebugService) -> bool) {
    let end = Instant::now() + Duration::from_secs(25);
    loop {
        service.poll();
        if pred(service) {
            return;
        }
        assert!(
            !service
                .sessions
                .iter()
                .any(|s| s.state == DebugState::Failed),
            "debugger failed: {:?}",
            service.sessions
        );
        assert!(Instant::now() < end, "timed out: {:?}", service.sessions);
        thread::sleep(Duration::from_millis(10));
    }
}

fn compile(directory: &Path, text: &str) -> (PathBuf, PathBuf) {
    let source = directory.join("main.rs");
    let binary = directory.join("rust-fixture");
    fs::write(&source, text).unwrap();
    let output = Command::new("rustc")
        .arg("--edition=2024")
        .args(["-C", "debuginfo=2", "-C", "opt-level=0"])
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("Rust toolchain required for the real Rust debug corpus");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    (source, binary)
}

fn config(directory: &Path, binary: &Path) -> DebugConfiguration {
    DebugConfiguration {
        name: "Rust fixture".into(),
        runtime: DebugRuntime::Rust,
        program: Some(binary.into()),
        executable: binary.into(),
        cwd: directory.into(),
        trusted: true,
        ..DebugConfiguration::default()
    }
}

#[test]
fn real_rust_breakpoint_variables_evaluate_step_and_exit() {
    let temp = tempfile::Builder::new()
        .prefix("codisseu-rust-ação with spaces-")
        .tempdir()
        .unwrap();
    let (source, binary) = compile(
        temp.path(),
        "fn doubled(value: i32) -> i32 {\n    value * 2\n}\nfn main() {\n    let answer = 42;\n    let result = doubled(answer);\n    println!(\"result={result}\");\n}\n",
    );
    let mut service = DebugService::new();
    service
        .execute(DebugCommand::SetBreakpoints {
            path: source.clone(),
            lines: vec![6],
        })
        .unwrap();
    let id = service
        .execute(DebugCommand::Start(config(temp.path(), &binary)))
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
    assert_eq!(session.frames[0].path.as_ref(), Some(&source));
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
    service
        .execute(DebugCommand::Evaluate {
            session: id,
            expression: "answer".into(),
        })
        .unwrap();
    wait(&mut service, |s| !s.sessions[0].evaluations.is_empty());
    assert!(
        service.sessions[0].evaluations[0].error.is_none(),
        "{:?}",
        service.sessions[0].evaluations
    );
    assert_eq!(service.sessions[0].evaluations[0].result, "42");
    let epoch = service.sessions[0].stop_epoch;
    service.execute(DebugCommand::StepIn(id)).unwrap();
    wait(&mut service, |s| {
        s.sessions[0].stop_epoch > epoch
            && s.sessions[0].frames.first().is_some_and(|f| f.line == 2)
    });
    let epoch = service.sessions[0].stop_epoch;
    service.execute(DebugCommand::StepOut(id)).unwrap();
    wait(&mut service, |s| {
        s.sessions[0].stop_epoch > epoch && !s.sessions[0].frames.is_empty()
    });
    let epoch = service.sessions[0].stop_epoch;
    service.execute(DebugCommand::Next(id)).unwrap();
    wait(&mut service, |s| {
        s.sessions[0].stop_epoch > epoch
            && s.sessions[0].frames.first().is_some_and(|f| f.line == 7)
    });
    service.execute(DebugCommand::Continue(id)).unwrap();
    wait(&mut service, |s| !s.has_active_sessions());
    assert_eq!(service.sessions[0].exit_code, Some(0));
    assert!(
        service.sessions[0]
            .output
            .iter()
            .any(|line| line.text.contains("result=84"))
    );
}

#[test]
#[cfg(target_os = "linux")]
fn real_rust_stop_terminates_owned_process() {
    let temp = tempfile::tempdir().unwrap();
    let (_, binary) = compile(
        temp.path(),
        "fn main() {\n    std::fs::write(std::env::args().nth(1).unwrap(), std::process::id().to_string()).unwrap();\n    loop { std::thread::sleep(std::time::Duration::from_millis(20)); }\n}\n",
    );
    let pid_path = temp.path().join("pid");
    let mut configuration = config(temp.path(), &binary);
    configuration.args = vec![pid_path.to_string_lossy().into_owned()];
    let mut service = DebugService::new();
    let id = service
        .execute(DebugCommand::Start(configuration))
        .unwrap()
        .unwrap();
    wait(&mut service, |_| pid_path.is_file());
    let pid: u32 = fs::read_to_string(pid_path).unwrap().parse().unwrap();
    let process = PathBuf::from(format!("/proc/{pid}"));
    assert!(process.exists());
    service.execute(DebugCommand::Stop(id)).unwrap();
    wait(&mut service, |s| !s.has_active_sessions());
    let end = Instant::now() + Duration::from_secs(5);
    while process.exists() && Instant::now() < end {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!process.exists(), "owned Rust process survived Stop");
}

#[test]
fn real_rust_conditional_breakpoint_and_logpoint() {
    let temp = tempfile::tempdir().unwrap();
    let (source, binary) = compile(
        temp.path(),
        "fn main() {\n    for index in 0..5 {\n        let value = index * 2;\n        println!(\"value={value}\");\n    }\n}\n",
    );
    let mut service = DebugService::new();
    let mut log = Breakpoint::new(source.clone(), 3);
    log.options.log_message = "index={index}".into();
    service.execute(DebugCommand::SetBreakpoint(log)).unwrap();
    let mut stop = Breakpoint::new(source, 4);
    stop.options.condition = "index == 3".into();
    service.execute(DebugCommand::SetBreakpoint(stop)).unwrap();
    let id = service
        .execute(DebugCommand::Start(config(temp.path(), &binary)))
        .unwrap()
        .unwrap();
    wait(&mut service, |s| {
        s.sessions[0].state == DebugState::Stopped
            && s.sessions[0].pending == 0
            && !s.sessions[0].variables.is_empty()
    });
    assert!(
        service.sessions[0]
            .variables
            .values()
            .flatten()
            .any(|v| v.name == "index" && v.value == "3")
    );
    assert!(service.sessions[0].breakpoints.iter().all(|b| b.verified));
    let output = service.sessions[0]
        .output
        .iter()
        .map(|o| o.text.as_str())
        .collect::<String>();
    for index in 0..=3 {
        assert!(output.contains(&format!("index={index}")), "{output}");
    }
    // println! expands to several locations on one source line. Disable the
    // conditional source breakpoint before resuming through its other locations.
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

#[test]
fn rust_source_is_not_launched_as_a_binary_and_attach_is_explicitly_unsupported() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("main.rs");
    fs::write(&source, "fn main() {}\n").unwrap();
    let mut configuration = DebugConfiguration::for_file(&source, temp.path());
    assert_eq!(configuration.runtime, DebugRuntime::Rust);
    assert!(configuration.program.is_none());
    configuration.trusted = true;
    let mut service = DebugService::new();
    assert!(
        service
            .execute(DebugCommand::Start(configuration.clone()))
            .unwrap_err()
            .contains("existing program")
    );
    configuration.request = DebugRequest::Attach;
    assert!(
        service
            .execute(DebugCommand::Start(configuration))
            .unwrap_err()
            .contains("Launch")
    );
    assert!(service.sessions.is_empty());
}
