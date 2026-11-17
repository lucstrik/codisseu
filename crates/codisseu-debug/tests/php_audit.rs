//! PHP/Xdebug audit against disposable sources only. No framework/container code.
use codisseu_debug::*;
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn wait(service: &mut DebugService, predicate: impl Fn(&DebugSession) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        service.poll();
        if service.sessions.first().is_some_and(&predicate) {
            return;
        }
        assert!(Instant::now() < deadline, "{:?}", service.sessions);
        thread::sleep(Duration::from_millis(5));
    }
}
fn stopped(service: &mut DebugService, line: u32) {
    wait(service, |s| {
        s.state == DebugState::Stopped
            && s.pending == 0
            && s.frames.first().is_some_and(|f| f.line == line)
            && !s.variables.is_empty()
    });
    assert!(
        service.sessions[0].last_error.is_none(),
        "{:?}",
        service.sessions[0].last_error
    );
}
fn evaluate(service: &mut DebugService, id: SessionId, expression: &str) -> String {
    let old = service.sessions[0].evaluations.len();
    service
        .execute(DebugCommand::Evaluate {
            session: id,
            expression: expression.into(),
        })
        .unwrap();
    wait(service, |s| s.evaluations.len() > old);
    let result = service.sessions[0].evaluations.last().unwrap();
    assert!(result.error.is_none(), "{:?}", result.error);
    result.result.clone()
}
fn launch(service: &mut DebugService, path: &Path, line: u32) -> SessionId {
    service
        .execute(DebugCommand::SetBreakpoints {
            path: path.into(),
            lines: vec![line],
        })
        .unwrap();
    let mut config = DebugConfiguration::for_file(path, path.parent().unwrap());
    config.trusted = true;
    service
        .execute(DebugCommand::Start(config))
        .unwrap()
        .unwrap()
}

#[test]
fn php_verified_breakpoint_nested_values_frame_context_live_changes_and_mute() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ação with spaces.php");
    let source = "<?php\nfunction calculate(array $parcel): int {\n    $subtotal = $parcel['count'] * 7;\n    $nested = ['label' => 'ação 😀', 'items' => [1, 2]];\n    $result = $subtotal + 1;\n    return $result;\n}\n$parcel = ['count' => 6];\n$total = calculate($parcel);\necho \"total={$total}\\n\";\necho \"finished\\n\";\n";
    fs::write(&path, source).unwrap();
    let mut service = DebugService::new();
    let id = launch(&mut service, &path, 5);
    stopped(&mut service, 5);
    assert!(
        service.sessions[0]
            .breakpoints
            .iter()
            .any(|b| b.line == 5 && b.verified)
    );
    assert_eq!(evaluate(&mut service, id, "$subtotal"), "42");
    assert!(evaluate(&mut service, id, "$nested['label']").contains("ação 😀"));
    assert!(service.sessions[0].frames.len() >= 2);
    let caller = service.sessions[0].frames[1].id;
    service
        .execute(DebugCommand::SelectFrame {
            session: id,
            frame_id: caller,
        })
        .unwrap();
    wait(&mut service, |s| {
        s.current_frame == Some(caller) && s.pending == 0 && !s.variables.is_empty()
    });
    assert_eq!(evaluate(&mut service, id, "$parcel['count']"), "6");
    assert_eq!(evaluate(&mut service, id, "$total ?? -1"), "-1");
    let frame = service.sessions[0].frames[0].id;
    service
        .execute(DebugCommand::SelectFrame {
            session: id,
            frame_id: frame,
        })
        .unwrap();
    wait(&mut service, |s| {
        s.current_frame == Some(frame) && s.pending == 0 && !s.variables.is_empty()
    });
    let nested = service.sessions[0]
        .variables
        .values()
        .flatten()
        .find(|v| v.name.contains("nested"))
        .unwrap()
        .reference;
    assert!(nested > 0);
    service
        .execute(DebugCommand::Variables {
            session: id,
            reference: nested,
        })
        .unwrap();
    wait(&mut service, |s| {
        s.variables
            .get(&nested)
            .is_some_and(|v| v.iter().any(|v| v.name.contains("items")))
    });
    let items = service.sessions[0].variables[&nested]
        .iter()
        .find(|v| v.name.contains("items"))
        .unwrap()
        .reference;
    service
        .execute(DebugCommand::Variables {
            session: id,
            reference: items,
        })
        .unwrap();
    wait(&mut service, |s| {
        s.variables
            .get(&items)
            .is_some_and(|v| v.iter().any(|v| v.value == "2"))
    });
    let epoch = service.sessions[0].stop_epoch;
    service.execute(DebugCommand::Next(id)).unwrap();
    stopped(&mut service, 6);
    assert!(service.sessions[0].stop_epoch > epoch);
    assert_eq!(evaluate(&mut service, id, "$result"), "43");
    service
        .execute(DebugCommand::SetBreakpoints {
            path: path.clone(),
            lines: vec![10, 11],
        })
        .unwrap();
    // PHP may report new breakpoints as pending until execution reaches them.
    wait(&mut service, |s| {
        s.pending == 0 && s.breakpoints.iter().any(|b| b.line == 10)
    });
    service.execute(DebugCommand::Continue(id)).unwrap();
    stopped(&mut service, 10);
    assert!(
        service.sessions[0]
            .breakpoints
            .iter()
            .any(|b| b.line == 10 && b.verified)
    );
    assert_eq!(evaluate(&mut service, id, "$total"), "43");
    service
        .execute(DebugCommand::SetBreakpointsEnabled(false))
        .unwrap();
    wait(&mut service, |s| {
        s.pending == 0 && s.breakpoints.iter().all(|b| !b.enabled)
    });
    service.execute(DebugCommand::Continue(id)).unwrap();
    wait(&mut service, |s| s.state == DebugState::Terminated);
    assert!(
        service.sessions[0]
            .output
            .iter()
            .any(|o| o.text.contains("total=43"))
    );
    assert!(
        service.sessions[0]
            .output
            .iter()
            .any(|o| o.text.contains("finished"))
    );
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}

#[test]
fn php_listen_port_collision_has_explicit_failure() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let temp = tempfile::tempdir().unwrap();
    let mut config = DebugConfiguration::for_file(temp.path().join("unused.php"), temp.path());
    config.request = DebugRequest::Listen;
    config.program = None;
    config.port = port;
    config.trusted = true;
    let mut service = DebugService::new();
    service.execute(DebugCommand::Start(config)).unwrap();
    wait(&mut service, |s| s.state == DebugState::Failed);
    assert!(
        service.sessions[0]
            .last_error
            .as_ref()
            .is_some_and(|e| e.contains("EADDRINUSE") || e.contains("in use")),
        "{:?}",
        service.sessions[0].last_error
    );
    assert!(!service.has_active_sessions());
}

#[cfg(unix)]
#[test]
fn php_launch_without_xdebug_fails_before_executing_selected_program() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("executed");
    let path = temp.path().join("no-extension.php");
    fs::write(
        &path,
        format!(
            "<?php\nfile_put_contents('{}','must not run');\n",
            marker.display()
        ),
    )
    .unwrap();
    let runtime = temp.path().join("php-without-xdebug");
    fs::write(&runtime, "#!/bin/sh\nexec /usr/bin/php -n \"$@\"\n").unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
    let mut config = DebugConfiguration::for_file(&path, temp.path());
    config.executable = runtime;
    config.trusted = true;
    let mut service = DebugService::new();
    service.execute(DebugCommand::Start(config)).unwrap();
    wait(&mut service, |s| s.state == DebugState::Failed);
    assert!(
        service.sessions[0]
            .last_error
            .as_ref()
            .is_some_and(|e| e.contains("Xdebug"))
    );
    assert!(!marker.exists(), "PHP source ran before verifying Xdebug");
}

#[cfg(unix)]
#[test]
fn php_preflight_disabled_mode_timeout_and_oversized_output_are_bounded() {
    use std::os::unix::fs::PermissionsExt;
    for mode in ["disabled", "timeout", "oversized"] {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("executed");
        let path = temp.path().join("must-not-run.php");
        fs::write(
            &path,
            format!(
                "<?php file_put_contents('{}','must not run');",
                marker.display()
            ),
        )
        .unwrap();
        let mut config = DebugConfiguration::for_file(&path, temp.path());
        config.trusted = true;
        if mode == "disabled" {
            config.env.insert("XDEBUG_MODE".into(), "off".into());
        } else {
            let wrapper = temp.path().join("interpreter-probe-wrapper");
            let content = if mode == "timeout" {
                "#!/bin/sh\nsleep 30\n"
            } else {
                "#!/bin/sh\nprintf 'PRIVATE_PROBE_OUTPUT'\nhead -c 100000 /dev/zero\n"
            };
            fs::write(&wrapper, content).unwrap();
            fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
            config.executable = wrapper;
        }
        let start = Instant::now();
        let mut service = DebugService::new();
        service.execute(DebugCommand::Start(config)).unwrap();
        wait(&mut service, |s| s.state == DebugState::Failed);
        assert!(start.elapsed() < Duration::from_secs(4), "{mode}");
        let error = service.sessions[0].last_error.as_deref().unwrap();
        let expected = match mode {
            "disabled" => "debug mode",
            "timeout" => "timed out",
            _ => "8 KiB",
        };
        assert!(error.contains(expected), "{mode}: {error}");
        assert!(!error.contains("PRIVATE_PROBE_OUTPUT"));
        assert!(
            !marker.exists(),
            "{mode}: selected PHP source ran during preflight"
        );
    }
}

#[cfg(unix)]
#[test]
fn stop_and_drop_cancel_php_preflight_and_reap_own_interpreter() {
    use std::os::unix::fs::PermissionsExt;
    for action in ["stop", "drop"] {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("must-not-run.php");
        let marker = temp.path().join("source-executed");
        fs::write(
            &source,
            format!(
                "<?php file_put_contents('{}','must not run');",
                marker.display()
            ),
        )
        .unwrap();
        let pidfile = temp.path().join("preflight.pid");
        let wrapper = temp.path().join("slow-interpreter");
        fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nprintf '%s' $$ > '{}'\nsleep 30\n",
                pidfile.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
        let mut config = DebugConfiguration::for_file(&source, temp.path());
        config.executable = wrapper;
        config.trusted = true;
        let mut service = DebugService::new();
        let id = service
            .execute(DebugCommand::Start(config))
            .unwrap()
            .unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        while !pidfile.exists() || fs::read_to_string(&pidfile).unwrap_or_default().is_empty() {
            service.poll();
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(5));
        }
        let pid = fs::read_to_string(pidfile).unwrap();
        let start = Instant::now();
        if action == "stop" {
            service.execute(DebugCommand::Stop(id)).unwrap();
            wait(&mut service, |s| s.state == DebugState::Terminated);
            assert!(service.sessions[0].last_error.is_none());
        }
        drop(service);
        assert!(
            start.elapsed() < Duration::from_millis(1500),
            "{action}: cancellation waited for timeout"
        );
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "{action}: preflight interpreter not reaped"
        );
        assert!(!marker.exists(), "{action}: selected PHP source executed");
    }
}
