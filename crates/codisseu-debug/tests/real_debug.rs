use codisseu_debug::*;
use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
fn wait(s: &mut DebugService, mut pred: impl FnMut(&DebugService) -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        s.poll();
        if pred(s) {
            return;
        }
        if Instant::now() > end {
            panic!("timed out; {:?}", s.sessions)
        }
        thread::sleep(Duration::from_millis(10));
    }
}
fn start(s: &mut DebugService, file: &str, line: u32) -> SessionId {
    let root = root();
    let file = root.join("fixtures/debug").join(file);
    s.execute(DebugCommand::ToggleBreakpoint {
        path: file.clone(),
        line,
    })
    .unwrap();
    let mut c = DebugConfiguration::for_file(file, root);
    c.trusted = true;
    s.execute(DebugCommand::Start(c)).unwrap().unwrap()
}
fn paused(s: &mut DebugService, id: SessionId) {
    wait(s, |s| {
        s.sessions
            .iter()
            .any(|x| x.id == id && x.state == DebugState::Stopped && !x.variables.is_empty())
    });
}
#[test]
fn real_php_and_node_simultaneous() {
    let mut s = DebugService::new();
    let php = start(&mut s, "backend.php", 5);
    let node = start(&mut s, "frontend.js", 4);
    paused(&mut s, php);
    paused(&mut s, node);
    for id in [php, node] {
        let session = s.sessions.iter().find(|x| x.id == id).unwrap();
        assert!(session.frames[0].path.is_some());
        assert!(
            session
                .variables
                .values()
                .flatten()
                .any(|v| v.name.contains("answer") && v.value == "42"),
            "{:?}",
            session.variables
        );
        let expression = if id == php { "$answer" } else { "answer" };
        s.execute(DebugCommand::Evaluate {
            session: id,
            expression: expression.into(),
        })
        .unwrap();
    }
    wait(&mut s, |s| {
        s.sessions.iter().all(|x| !x.evaluations.is_empty())
    });
    assert!(s.sessions.iter().all(|x| x.evaluations[0].result == "42"));
    s.execute(DebugCommand::Next(php)).unwrap();
    wait(&mut s, |s| {
        s.sessions
            .iter()
            .find(|x| x.id == php)
            .unwrap()
            .frames
            .first()
            .is_some_and(|x| x.line == 6)
    });
    assert_eq!(
        s.sessions.iter().find(|x| x.id == node).unwrap().state,
        DebugState::Stopped
    );
    for id in [php, node] {
        s.execute(DebugCommand::Continue(id)).unwrap();
    }
    wait(&mut s, |s| {
        s.sessions.iter().all(|x| x.state == DebugState::Terminated)
    });
    assert!(
        s.sessions
            .iter()
            .all(|x| x.output.iter().any(|x| x.text.contains("84")))
    );
}
#[test]
fn rejects_untrusted_and_invalid_program() {
    let mut s = DebugService::new();
    let c = DebugConfiguration::for_file(root().join("fixtures/debug/backend.php"), root());
    assert!(s.execute(DebugCommand::Start(c.clone())).is_err());
    let mut c = c;
    c.trusted = true;
    c.program = Some(root().join("nonexistent.php"));
    assert!(s.execute(DebugCommand::Start(c)).is_err());
}
#[test]
fn missing_adapter_is_reported() {
    let mut s = DebugService::new();
    let mut c = DebugConfiguration::for_file(root().join("fixtures/debug/backend.php"), root());
    c.trusted = true;
    c.adapter = Some(root().join("nonexistent-adapter.js"));
    let id = s.execute(DebugCommand::Start(c)).unwrap().unwrap();
    wait(&mut s, |s| s.sessions[0].state == DebugState::Failed);
    assert_eq!(s.sessions[0].id, id);
    assert!(
        s.sessions[0]
            .last_error
            .as_ref()
            .unwrap()
            .contains("adapter not found")
    );
}
#[test]
fn real_node_stop_and_unverifiable_breakpoint() {
    let mut s = DebugService::new();
    let root = root();
    s.execute(DebugCommand::SetBreakpoints {
        path: root.join("fixtures/debug/missing.js"),
        lines: vec![999],
    })
    .unwrap();
    let id = start(&mut s, "frontend.js", 4);
    paused(&mut s, id);
    assert!(s.breakpoints.iter().any(|x| x.line == 999 && !x.verified));
    s.execute(DebugCommand::Stop(id)).unwrap();
    wait(&mut s, |s| !s.has_active_sessions());
}
#[test]
fn real_typescript_source_map() {
    let root = root();
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("frontend.ts");
    std::fs::copy(root.join("fixtures/debug/frontend.ts"), &source).unwrap();
    let status = std::process::Command::new(root.join(".tools/node-v24.22.0-linux-x64/bin/node"))
        .arg(root.join(".tools/node_modules/typescript/bin/tsc"))
        .arg("--sourceMap")
        .arg("--target")
        .arg("ES2022")
        .arg("--outDir")
        .arg(temp.path().join("out"))
        .arg(&source)
        .status()
        .unwrap();
    assert!(status.success());
    let mut s = DebugService::new();
    s.execute(DebugCommand::SetBreakpoints {
        path: source.clone(),
        lines: vec![4],
    })
    .unwrap();
    let mut c = DebugConfiguration::for_file(temp.path().join("out/frontend.js"), temp.path());
    c.trusted = true;
    let id = s.execute(DebugCommand::Start(c)).unwrap().unwrap();
    paused(&mut s, id);
    assert_eq!(s.sessions[0].frames[0].path.as_ref(), Some(&source));
    assert_eq!(s.sessions[0].frames[0].line, 4);
    assert!(
        s.sessions[0]
            .variables
            .values()
            .flatten()
            .any(|v| v.name == "answer" && v.value == "42")
    );
    s.execute(DebugCommand::Continue(id)).unwrap();
    wait(&mut s, |s| !s.has_active_sessions());
}
struct OwnedChild(std::process::Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
#[test]
fn real_node_attach_disconnect_preserves_external_process() {
    let root = root();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("attach.js");
    std::fs::write(&path,"function run() {\nconst answer=42;\nconsole.log(answer);\nsetInterval(()=>{},1000);\n}\nrun();\n").unwrap();
    let port = free_port();
    let child = std::process::Command::new(root.join(".tools/node-v24.22.0-linux-x64/bin/node"))
        .arg(format!("--inspect-brk=127.0.0.1:{port}"))
        .arg(&path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut child = OwnedChild(child);
    let deadline = Instant::now() + Duration::from_secs(5);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    let mut s = DebugService::new();
    s.execute(DebugCommand::SetBreakpoints {
        path: path.clone(),
        lines: vec![3],
    })
    .unwrap();
    let mut c = DebugConfiguration::for_file(&path, temp.path());
    c.request = DebugRequest::Attach;
    c.port = port;
    c.trusted = true;
    let id = s.execute(DebugCommand::Start(c)).unwrap().unwrap();
    paused(&mut s, id);
    if s.sessions[0].frames[0].line != 3 {
        s.execute(DebugCommand::Continue(id)).unwrap();
        wait(&mut s, |s| {
            s.sessions[0].state == DebugState::Stopped
                && s.sessions[0].frames.first().is_some_and(|x| x.line == 3)
        });
    }
    s.execute(DebugCommand::Stop(id)).unwrap();
    wait(&mut s, |s| !s.has_active_sessions());
    assert!(
        child.0.try_wait().unwrap().is_none(),
        "Attach stop killed external Node process"
    );
}
#[test]
fn real_php_listen_disconnect_preserves_external_process() {
    let root = root();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("listen.php");
    std::fs::write(
        &path,
        "<?php\n$answer=42;\n$label='listen';\necho $answer;\nsleep(30);\n",
    )
    .unwrap();
    let port = free_port();
    let mut s = DebugService::new();
    s.execute(DebugCommand::SetBreakpoints {
        path: path.clone(),
        lines: vec![4],
    })
    .unwrap();
    let mut c = DebugConfiguration::for_file(&path, &root);
    c.request = DebugRequest::Listen;
    c.port = port;
    c.program = None;
    c.trusted = true;
    let id = s.execute(DebugCommand::Start(c)).unwrap().unwrap();
    wait(&mut s, |s| {
        s.sessions[0].state == DebugState::Running && s.sessions[0].pending == 0
    });
    let child = std::process::Command::new("php")
        .args([
            "-d",
            "xdebug.mode=debug",
            "-d",
            "xdebug.start_with_request=yes",
            "-d",
            "xdebug.client_host=127.0.0.1",
            "-d",
        ])
        .arg(format!("xdebug.client_port={port}"))
        .arg(&path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut child = OwnedChild(child);
    paused(&mut s, id);
    assert_eq!(s.sessions[0].frames[0].line, 4);
    s.execute(DebugCommand::Stop(id)).unwrap();
    wait(&mut s, |s| !s.has_active_sessions());
    assert!(
        child.0.try_wait().unwrap().is_none(),
        "Listen stop killed external PHP process"
    );
}
#[test]
fn real_browser_javascript_breakpoint() {
    let root = root();
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("frontend.js");
    std::fs::copy(root.join("fixtures/debug/frontend.js"), &source).unwrap();
    let page = temp.path().join("index.html");
    std::fs::write(&page,"<!doctype html><title>Codisseu debugger fixture</title><script src='frontend.js'></script>").unwrap();
    let mut s = DebugService::new();
    s.execute(DebugCommand::SetBreakpoints {
        path: source.clone(),
        lines: vec![4],
    })
    .unwrap();
    let mut c = DebugConfiguration::for_file(&source, temp.path());
    c.runtime = DebugRuntime::Chrome;
    c.executable = "/usr/bin/google-chrome".into();
    c.url = format!("file://{}", page.display());
    c.args = vec![
        "--headless=new".into(),
        "--disable-gpu".into(),
        "--disable-dev-shm-usage".into(),
    ];
    c.trusted = true;
    let id = s.execute(DebugCommand::Start(c)).unwrap().unwrap();
    paused(&mut s, id);
    assert_eq!(s.sessions[0].frames[0].path.as_ref(), Some(&source));
    assert_eq!(s.sessions[0].frames[0].line, 4);
    assert!(
        s.sessions[0]
            .variables
            .values()
            .flatten()
            .any(|x| x.name == "answer" && x.value == "42")
    );
    s.execute(DebugCommand::Stop(id)).unwrap();
    wait(&mut s, |s| !s.has_active_sessions());
}
#[test]
fn real_node_pause_and_unexpected_exit() {
    let root = root();
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("running.js");
    std::fs::write(
        &source,
        "let answer=42;\nsetInterval(()=>{answer+=1},100);\n",
    )
    .unwrap();
    let port = free_port();
    let child = std::process::Command::new(root.join(".tools/node-v24.22.0-linux-x64/bin/node"))
        .arg(format!("--inspect=127.0.0.1:{port}"))
        .arg(&source)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut child = OwnedChild(child);
    let until = Instant::now() + Duration::from_secs(5);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(10));
    }
    let mut s = DebugService::new();
    let mut c = DebugConfiguration::for_file(&source, temp.path());
    c.request = DebugRequest::Attach;
    c.port = port;
    c.trusted = true;
    let id = s.execute(DebugCommand::Start(c)).unwrap().unwrap();
    wait(&mut s, |s| {
        s.sessions[0].state == DebugState::Running && s.sessions[0].pending == 0
    });
    s.execute(DebugCommand::Pause(id)).unwrap();
    paused(&mut s, id);
    s.execute(DebugCommand::Evaluate {
        session: id,
        expression: "process.exit(7)".into(),
    })
    .unwrap();
    wait(&mut s, |s| !s.has_active_sessions());
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert_eq!(status.code(), Some(7));
            break;
        }
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn real_php_listener_accepts_successive_requests() {
    let root = root();
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("requests.php");
    std::fs::write(&source, "<?php\n$answer=42;\necho $answer;\n").unwrap();
    let port = free_port();
    let mut s = DebugService::new();
    s.execute(DebugCommand::SetBreakpoints {
        path: source.clone(),
        lines: vec![3],
    })
    .unwrap();
    let mut c = DebugConfiguration::for_file(&source, &root);
    c.request = DebugRequest::Listen;
    c.port = port;
    c.program = None;
    c.listen_host = "127.0.0.1".into();
    c.trusted = true;
    let id = s.execute(DebugCommand::Start(c)).unwrap().unwrap();
    wait(&mut s, |s| {
        s.sessions[0].state == DebugState::Running && s.sessions[0].pending == 0
    });
    for _ in 0..2 {
        let child = std::process::Command::new("php")
            .args([
                "-d",
                "xdebug.mode=debug",
                "-d",
                "xdebug.start_with_request=yes",
                "-d",
                "xdebug.client_host=127.0.0.1",
                "-d",
            ])
            .arg(format!("xdebug.client_port={port}"))
            .arg(&source)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let mut child = OwnedChild(child);
        paused(&mut s, id);
        assert_eq!(s.sessions[0].frames[0].line, 3);
        s.execute(DebugCommand::Continue(id)).unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            s.poll();
            if child.0.try_wait().unwrap().is_some() {
                break;
            }
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(10));
        }
        wait(&mut s, |s| {
            s.sessions[0].state == DebugState::Running && s.sessions[0].frames.is_empty()
        });
    }
    s.execute(DebugCommand::Stop(id)).unwrap();
    wait(&mut s, |s| !s.has_active_sessions());
}
#[test]
fn bounded_breakpoints_and_muting() {
    let mut s = DebugService::new();
    let path = root().join("fixtures/debug/frontend.js");
    s.execute(DebugCommand::SetBreakpoints {
        path: path.clone(),
        lines: vec![2],
    })
    .unwrap();
    s.execute(DebugCommand::SetBreakpointsEnabled(false))
        .unwrap();
    s.execute(DebugCommand::ToggleBreakpoint {
        path: path.clone(),
        line: 3,
    })
    .unwrap();
    assert!(s.breakpoints.iter().all(|x| x.enabled));
    assert!(!s.breakpoints_enabled());
    assert!(
        s.execute(DebugCommand::SetBreakpoints {
            path: path.clone(),
            lines: (1..=513).collect()
        })
        .is_err()
    );
    assert_eq!(s.breakpoints.len(), 2);
    s.execute(DebugCommand::SetBreakpointsEnabled(true))
        .unwrap();
    assert!(s.breakpoints.iter().all(|x| x.enabled));
}
#[test]
fn drop_with_full_command_queue_cannot_hang() {
    let mut s = DebugService::new();
    let id = start(&mut s, "frontend.js", 4);
    paused(&mut s, id);
    for _ in 0..1000 {
        let _ = s.execute(DebugCommand::Evaluate {
            session: id,
            expression: "answer".into(),
        });
    }
    let now = Instant::now();
    drop(s);
    assert!(now.elapsed() < Duration::from_secs(4));
}
#[test]
fn same_breakpoint_loop_refreshes_stop_epoch_and_variables() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("loop.js");
    std::fs::write(
        &source,
        "for (let answer=0;answer<3;answer++) {\n console.log(answer);\n}\n",
    )
    .unwrap();
    let mut s = DebugService::new();
    s.execute(DebugCommand::SetBreakpoints {
        path: source.clone(),
        lines: vec![2],
    })
    .unwrap();
    let mut c = DebugConfiguration::for_file(&source, temp.path());
    c.trusted = true;
    let id = s.execute(DebugCommand::Start(c)).unwrap().unwrap();
    let mut epoch = 0;
    for expected in ["0", "1", "2"] {
        wait(&mut s, |s| {
            s.sessions[0].state == DebugState::Stopped
                && s.sessions[0].stop_epoch > epoch
                && s.sessions[0]
                    .variables
                    .values()
                    .flatten()
                    .any(|v| v.name == "answer" && v.value == expected)
        });
        assert_eq!(s.sessions[0].frames[0].line, 2);
        epoch = s.sessions[0].stop_epoch;
        s.execute(DebugCommand::Continue(id)).unwrap();
    }
    wait(&mut s, |s| !s.has_active_sessions());
    assert_eq!(epoch, 3);
}
#[test]
fn real_php_path_mapping_translates_remote_source() {
    let root = root();
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("server");
    let local = temp.path().join("client");
    std::fs::create_dir(&remote).unwrap();
    std::fs::create_dir(&local).unwrap();
    let code = "<?php\n$answer=42;\n$label='mapped';\necho $answer;\n";
    std::fs::write(remote.join("mapped.php"), code).unwrap();
    std::fs::write(local.join("mapped.php"), code).unwrap();
    let port = free_port();
    let mut s = DebugService::new();
    s.execute(DebugCommand::SetBreakpoints {
        path: local.join("mapped.php"),
        lines: vec![4],
    })
    .unwrap();
    let mut c = DebugConfiguration::for_file(local.join("mapped.php"), &root);
    c.request = DebugRequest::Listen;
    c.port = port;
    c.program = None;
    c.trusted = true;
    c.path_mappings.insert(
        remote.to_string_lossy().into_owned(),
        local.to_string_lossy().into_owned(),
    );
    let id = s.execute(DebugCommand::Start(c)).unwrap().unwrap();
    wait(&mut s, |s| {
        s.sessions[0].state == DebugState::Running && s.sessions[0].pending == 0
    });
    let child = std::process::Command::new("php")
        .args([
            "-d",
            "xdebug.mode=debug",
            "-d",
            "xdebug.start_with_request=yes",
            "-d",
            "xdebug.client_host=127.0.0.1",
            "-d",
        ])
        .arg(format!("xdebug.client_port={port}"))
        .arg(remote.join("mapped.php"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut child = OwnedChild(child);
    paused(&mut s, id);
    assert_eq!(s.sessions[0].frames[0].path, Some(local.join("mapped.php")));
    assert_eq!(s.sessions[0].frames[0].line, 4);
    s.execute(DebugCommand::Continue(id)).unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while child.0.try_wait().unwrap().is_none() {
        s.poll();
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(10));
    }
    s.execute(DebugCommand::Stop(id)).unwrap();
    wait(&mut s, |s| !s.has_active_sessions());
}
#[test]
fn real_browser_typescript_and_php_simultaneously() {
    let root = root();
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("frontend.ts");
    std::fs::copy(root.join("fixtures/debug/frontend.ts"), &source).unwrap();
    let status = std::process::Command::new(root.join(".tools/node-v24.22.0-linux-x64/bin/node"))
        .arg(root.join(".tools/node_modules/typescript/bin/tsc"))
        .args(["--sourceMap", "--target", "ES2022", "--outDir"])
        .arg(temp.path().join("out"))
        .arg(&source)
        .status()
        .unwrap();
    assert!(status.success());
    let page = temp.path().join("index.html");
    std::fs::write(
        &page,
        "<!doctype html><title>TS debug fixture</title><script src='out/frontend.js'></script>",
    )
    .unwrap();
    let mut s = DebugService::new();
    let php = start(&mut s, "backend.php", 5);
    paused(&mut s, php);
    s.execute(DebugCommand::SetBreakpoints {
        path: source.clone(),
        lines: vec![4],
    })
    .unwrap();
    let mut c = DebugConfiguration::for_file(&source, temp.path());
    c.runtime = DebugRuntime::Chrome;
    c.executable = "/usr/bin/google-chrome".into();
    c.url = format!("file://{}", page.display());
    c.args = vec![
        "--headless=new".into(),
        "--disable-gpu".into(),
        "--disable-dev-shm-usage".into(),
    ];
    c.trusted = true;
    let browser = s.execute(DebugCommand::Start(c)).unwrap().unwrap();
    paused(&mut s, browser);
    let browser_session = s.sessions.iter().find(|x| x.id == browser).unwrap();
    assert_eq!(browser_session.frames[0].path, Some(source));
    assert_eq!(browser_session.frames[0].line, 4);
    assert!(s.sessions.iter().all(|x| x.state == DebugState::Stopped));
    s.execute(DebugCommand::Stop(browser)).unwrap();
    wait(&mut s, |s| {
        s.sessions.iter().find(|x| x.id == browser).unwrap().state == DebugState::Terminated
    });
    assert_eq!(
        s.sessions.iter().find(|x| x.id == php).unwrap().state,
        DebugState::Stopped
    );
    s.execute(DebugCommand::Continue(php)).unwrap();
    wait(&mut s, |s| !s.has_active_sessions());
}
#[test]
fn real_php_and_node_step_into_out() {
    for (file, line, inside) in [("backend.php", 8, 3), ("frontend.js", 7, 2)] {
        let mut s = DebugService::new();
        let id = start(&mut s, file, line);
        wait(&mut s, |s| {
            s.sessions[0].state == DebugState::Stopped && !s.sessions[0].frames.is_empty()
        });
        let epoch = s.sessions[0].stop_epoch;
        s.execute(DebugCommand::StepIn(id)).unwrap();
        wait(&mut s, |s| {
            s.sessions[0].state == DebugState::Stopped
                && s.sessions[0].stop_epoch > epoch
                && !s.sessions[0].frames.is_empty()
        });
        assert_eq!(s.sessions[0].frames[0].line, inside);
        assert!(s.sessions[0].frames.len() >= 2);
        let epoch = s.sessions[0].stop_epoch;
        s.execute(DebugCommand::StepOut(id)).unwrap();
        wait(&mut s, |s| {
            s.sessions[0].state == DebugState::Stopped
                && s.sessions[0].stop_epoch > epoch
                && !s.sessions[0].frames.is_empty()
        });
        assert_eq!(
            s.sessions[0].frames[0].line,
            if file == "backend.php" { 6 } else { line }
        );
        if file == "backend.php" {
            let epoch = s.sessions[0].stop_epoch;
            s.execute(DebugCommand::StepOut(id)).unwrap();
            wait(&mut s, |s| {
                s.sessions[0].state == DebugState::Stopped
                    && s.sessions[0].stop_epoch > epoch
                    && !s.sessions[0].frames.is_empty()
            });
            assert_eq!(s.sessions[0].frames[0].line, 8);
            assert_eq!(s.sessions[0].frames.len(), 1);
        }
        let path = root().join("fixtures/debug").join(file);
        s.execute(DebugCommand::SetBreakpoints {
            path,
            lines: vec![],
        })
        .unwrap();
        s.execute(DebugCommand::Continue(id)).unwrap();
        wait(&mut s, |s| !s.has_active_sessions());
    }
}
#[test]
fn real_php_pause_via_xdebug_control_socket() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("running.php");
    std::fs::write(
        &source,
        "<?php\n$answer=42;\nwhile (true) {\n $answer++;\n usleep(25000);\n}\n",
    )
    .unwrap();
    let mut s = DebugService::new();
    let mut c = DebugConfiguration::for_file(&source, temp.path());
    c.trusted = true;
    let id = s.execute(DebugCommand::Start(c)).unwrap().unwrap();
    wait(&mut s, |s| {
        s.sessions[0].state == DebugState::Running
            && s.sessions[0].pending == 0
            && !s.sessions[0].threads.is_empty()
    });
    s.execute(DebugCommand::Pause(id)).unwrap();
    paused(&mut s, id);
    assert_eq!(s.sessions[0].frames[0].path, Some(source));
    assert!(
        s.sessions[0]
            .variables
            .values()
            .flatten()
            .any(|x| x.name.contains("answer"))
    );
    s.execute(DebugCommand::Stop(id)).unwrap();
    wait(&mut s, |s| !s.has_active_sessions());
}
