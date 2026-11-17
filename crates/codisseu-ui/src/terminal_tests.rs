use super::*;
use std::time::Instant;

fn wait_for(pane: &mut TerminalPane, condition: impl Fn(&TerminalPane) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        pane.poll();
        if condition(pane) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "terminal timeout: {:?}; screen={:?}",
            pane.message,
            pane.parser.screen().contents()
        );
        thread::sleep(Duration::from_millis(10));
    }
}
#[cfg(unix)]
fn shell(cwd: &Path) -> TerminalPane {
    let mut pane = TerminalPane {
        session: Some(Session::start(
            cwd.to_path_buf(),
            PathBuf::from("/bin/sh"),
            vec!["-i".into()],
        )),
        running: true,
        ..TerminalPane::default()
    };
    wait_for(&mut pane, |p| !p.parser.screen().contents().is_empty());
    pane
}

#[test]
fn vt_unicode_ansi_cursor_erase_alternate_screen_and_scrollback() {
    let mut parser = vt100::Parser::new(nz(3), nz(30), SCROLLBACK);
    parser.process("ação e\u{301} 界".as_bytes());
    assert!(parser.screen().contents().contains("ação e\u{301} 界"));
    parser.process(b"\x1b[2;1H\x1b[38;2;1;2;3mRGB\x1b[0m\x1b[3;1Hdelete\r\x1b[2Knew");
    assert_eq!(
        parser.screen().cell(1, 0).unwrap().fgcolor(),
        vt100::Color::Rgb(1, 2, 3)
    );
    assert!(!parser.screen().contents().contains("delete"));
    assert!(parser.screen().contents().contains("new"));
    let previous = parser.screen().contents();
    parser.process(b"\x1b[?1049hALT");
    assert!(parser.screen().alternate_screen());
    parser.process(b"\x1b[?1049l");
    assert_eq!(parser.screen().contents(), previous);
    for _ in 0..3000 {
        parser.process(b"line\r\n");
    }
    assert_eq!(parser.screen().scrollback_len(), SCROLLBACK);
    parser.screen_mut().set_scrollback(100_000);
    assert_eq!(parser.screen().scrollback(), SCROLLBACK);
}

#[test]
fn closing_one_of_multiple_sessions_keeps_all_cleanup_acknowledgements() {
    let previous = Arc::new(AtomicBool::new(false));
    let promoted = Arc::new(AtomicBool::new(false));
    let mut pane = TerminalPane {
        closing: vec![previous.clone()],
        ..Default::default()
    };
    pane.parked.push(TerminalPane {
        closing: vec![promoted.clone()],
        ..Default::default()
    });
    pane.close_active_session();
    assert!(pane.shutdown_pending());
    previous.store(true, Ordering::Release);
    assert!(pane.shutdown_pending());
    promoted.store(true, Ordering::Release);
    assert!(!pane.shutdown_pending());
}

#[test]
fn keyboard_and_paste_encode_terminal_input() {
    let ctrl_c = egui::Event::Key {
        key: Key::C,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::CTRL,
    };
    assert_eq!(encode_event(&ctrl_c, false, false), Some(vec![3]));
    let up = egui::Event::Key {
        key: Key::ArrowUp,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    assert_eq!(encode_event(&up, false, false), Some(b"\x1b[A".to_vec()));
    assert_eq!(encode_event(&up, true, false), Some(b"\x1bOA".to_vec()));
    let paste = egui::Event::Paste("ação\r\nnext\x1b[201~".into());
    assert_eq!(
        encode_event(&paste, false, true),
        Some("\x1b[200~ação\nnext[201~\x1b[201~".as_bytes().to_vec())
    );
    assert_eq!(
        encode_event(
            &egui::Event::Ime(egui::ImeEvent::Commit("é".into())),
            false,
            false
        ),
        Some("é".as_bytes().to_vec())
    );
}

#[test]
fn osc_clipboard_and_title_do_not_change_host_state() {
    let mut parser = vt100::Parser::new(nz(3), nz(40), 0);
    parser.process(b"before\x1b]52;c;c2VjcmV0\x07\x1b]2;untrusted title\x07after");
    assert_eq!(parser.screen().contents(), "beforeafter");
    // Default vt100 callbacks are no-ops: no clipboard or host-title capability is installed.
}

#[test]
fn selected_cells_preserve_unicode_and_line_breaks() {
    let mut pane = TerminalPane::default();
    pane.parser.process("ação\r\n界fim".as_bytes());
    pane.selection = Some((0, DEFAULT_COLS as usize + 4));
    assert_eq!(pane.selected_text(), "ação\n界fim");
}

#[cfg(unix)]
#[test]
fn real_pty_shell_cwd_unicode_ansi_and_exit() {
    let directory = tempfile::tempdir().unwrap();
    let mut pane = shell(directory.path());
    pane.send(
        "printf '\\033[31mAÇÃO界\\033[0m\\n'; pwd; printf 'TERM=%s\\n' \"$TERM\"\r"
            .as_bytes()
            .to_vec(),
    );
    wait_for(&mut pane, |p| {
        p.parser.screen().contents().contains("TERM=xterm-256color")
    });
    let contents = pane.parser.screen().contents();
    assert!(contents.contains("AÇÃO界"));
    assert!(contents.contains(directory.path().to_str().unwrap()));
    let colored = (0..DEFAULT_ROWS).any(|r| {
        (0..DEFAULT_COLS).any(|c| {
            pane.parser
                .screen()
                .cell(r, c)
                .is_some_and(|cell| cell.fgcolor() == vt100::Color::Idx(1))
        })
    });
    assert!(colored, "SGR cells not recorded: {contents:?}");
    pane.send(b"exit 7\r".to_vec());
    wait_for(&mut pane, |p| !p.running);
    assert_eq!(
        pane.message.as_deref(),
        Some("Processo encerrado (código 7).")
    );
}

#[cfg(unix)]
#[test]
fn real_pty_resize_and_ctrl_c_interrupt_foreground_command() {
    let directory = tempfile::tempdir().unwrap();
    let mut pane = shell(directory.path());
    pane.session
        .as_ref()
        .unwrap()
        .resize
        .send(size(31, 91))
        .unwrap();
    pane.parser.set_size(nz(31), nz(91));
    pane.dimensions = (31, 91);
    // Wait for resize acknowledgement by querying the actual kernel terminal size.
    pane.send(b"sleep 0.05; stty size\r".to_vec());
    wait_for(&mut pane, |p| {
        p.parser.screen().contents().contains("31 91")
    });
    pane.send(b"sleep 20\r".to_vec());
    thread::sleep(Duration::from_millis(100));
    pane.send(vec![3]);
    pane.send(b"printf 'INTERRUPTED_OK\\n'\r".to_vec());
    wait_for(&mut pane, |p| {
        p.parser
            .screen()
            .contents()
            .lines()
            .any(|line| line.ends_with("INTERRUPTED_OK"))
    });
    pane.close();
}

#[cfg(unix)]
#[test]
fn real_pty_missing_shell_is_reported_without_blocking() {
    let directory = tempfile::tempdir().unwrap();
    let mut pane = TerminalPane {
        session: Some(Session::start(
            directory.path().into(),
            PathBuf::from("/codisseu-does-not-exist/shell"),
            vec![],
        )),
        running: true,
        ..TerminalPane::default()
    };
    wait_for(&mut pane, |p| !p.running);
    assert!(
        pane.message
            .as_ref()
            .unwrap()
            .contains("codisseu-does-not-exist")
    );
}

#[cfg(target_os = "linux")]
#[test]
fn close_reaps_shell_and_terminates_foreground_process() {
    let directory = tempfile::tempdir().unwrap();
    let shell_pid = directory.path().join("shell.pid");
    let child_pid = directory.path().join("child.pid");
    let mut pane = shell(directory.path());
    pane.send(b"echo $$ > shell.pid; sleep 20 & echo $! > child.pid; wait\r".to_vec());
    let deadline = Instant::now() + Duration::from_secs(3);
    while !child_pid.exists() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    let shell_id = std::fs::read_to_string(shell_pid)
        .unwrap()
        .trim()
        .to_owned();
    let child_id = std::fs::read_to_string(child_pid)
        .unwrap()
        .trim()
        .to_owned();
    pane.close();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let shell_gone = !Path::new(&format!("/proc/{shell_id}")).exists();
        let child_stopped = std::fs::read_to_string(format!("/proc/{child_id}/stat"))
            .map_or(true, |s| s.contains(") Z "));
        if shell_gone && child_stopped {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "PTY processes remain: {shell_id}/{child_id}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn terminal_widget_owns_keyboard_focus_and_routes_unicode_paste_and_interrupt() {
    use egui_kittest::{Harness, kittest::Queryable};
    let (_output_tx, output) = mpsc::sync_channel(1);
    let (_status_tx, status) = mpsc::channel();
    let (input, input_rx) = mpsc::sync_channel(8);
    let (resize, _resize_rx) = mpsc::sync_channel(8);
    let mut pane = TerminalPane {
        session: Some(Session {
            output,
            status,
            input,
            resize,
            stop: Arc::new(AtomicBool::new(false)),
            wake_context: Arc::new(Mutex::new(None)),
            done: Arc::new(AtomicBool::new(true)),
        }),
        running: true,
        ..TerminalPane::default()
    };
    pane.request_focus();
    let mut harness = Harness::builder()
        .with_size([600., 200.])
        .build_ui_state(|ui, pane: &mut TerminalPane| pane.show(ui), pane);
    harness.step();
    assert!(
        harness
            .get_by_role(egui::accesskit::Role::Terminal)
            .is_focused()
    );
    harness.event(egui::Event::Text("ação界".into()));
    harness.step();
    assert_eq!(input_rx.try_recv().unwrap(), "ação界".as_bytes());
    harness.event(egui::Event::Paste("echo ok\n".into()));
    harness.step();
    assert_eq!(input_rx.try_recv().unwrap(), b"echo ok\r");
    harness.key_press_modifiers(egui::Modifiers::CTRL, Key::C);
    harness.step();
    assert_eq!(input_rx.try_recv().unwrap(), vec![3]);
    // Actual egui-winit translates Ctrl+C/X to clipboard events before egui sees them.
    harness.event_modifiers(egui::Event::Copy, egui::Modifiers::CTRL);
    harness.step();
    assert_eq!(input_rx.try_recv().unwrap(), vec![3]);
    harness.event_modifiers(egui::Event::Cut, egui::Modifiers::CTRL);
    harness.step();
    assert_eq!(input_rx.try_recv().unwrap(), vec![24]);
    #[cfg(not(target_os = "macos"))]
    {
        // Native Copy can be queued in the same frame as Ctrl release.
        harness.event_modifiers(egui::Event::Copy, egui::Modifiers::NONE);
        harness.step();
        assert_eq!(input_rx.try_recv().unwrap(), vec![3]);
    }
    // Modal/disabled tool windows must not consume editor/modal input.
    harness.state_mut().close();
}

#[cfg(unix)]
#[test]
fn hidden_terminal_tick_drains_more_output_than_queue_capacity() {
    let directory = tempfile::tempdir().unwrap();
    let completed = directory.path().join("hidden.done");
    let mut pane = shell(directory.path());
    pane.send(b"i=0; while [ $i -lt 15000 ]; do printf '012345678901234567890123456789\\n'; i=$((i+1)); done; printf done > hidden.done\r".to_vec());
    let ctx = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !completed.exists() {
        // No terminal widget is drawn. Root's always-on tick must keep a verbose task moving.
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| pane.tick(ui.ctx()));
        output.textures_delta.clear(); // This headless scenario intentionally does not render textures.
        assert!(
            Instant::now() < deadline,
            "hidden terminal stopped draining its bounded queue"
        );
        thread::sleep(Duration::from_millis(2));
    }
    assert!(pane.parser.screen().scrollback_len() <= SCROLLBACK);
    pane.close();
}

#[cfg(unix)]
#[test]
fn closing_a_live_pty_returns_immediately_and_acknowledges_worker_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let mut pane = TerminalPane::default();
    pane.open_command(
        dir.path(),
        "/bin/sh".into(),
        vec!["-c".into(), "printf 'WORKER_READY'; read hold".into()],
        "Owned PTY".into(),
        "owned-close".into(),
    );
    wait_for(&mut pane, |p| {
        p.parser.screen().contents().contains("WORKER_READY")
    });
    let done = pane.session.as_ref().unwrap().done.clone();
    let started = Instant::now();
    pane.close();
    assert!(started.elapsed() < Duration::from_millis(100));
    assert!(!pane.has_sessions());
    let deadline = Instant::now() + Duration::from_secs(5);
    while pane.shutdown_pending() {
        assert!(
            Instant::now() < deadline,
            "PTY cleanup acknowledgement missing"
        );
        thread::sleep(Duration::from_millis(5));
    }
    assert!(done.load(Ordering::Acquire));
}

#[cfg(unix)]
#[test]
fn explicit_program_pty_preserves_arguments_reuses_sessions_and_restarts_the_same_program() {
    let dir = tempfile::tempdir().unwrap();
    let program = PathBuf::from("/usr/bin/python3");
    let args = vec![
        "-c".into(),
        "import sys; print(repr(sys.argv[1]), flush=True); sys.stdin.readline()".into(),
        "literal ; $(not-a-shell) ' spaces".into(),
    ];
    let mut pane = TerminalPane::default();
    assert!(pane.open_command(
        dir.path(),
        program.clone(),
        args.clone(),
        "Docker · own".into(),
        "context:id:sh".into()
    ));
    wait_for(&mut pane, |p| {
        p.parser.screen().contents().contains("$(not-a-shell)")
    });
    assert!(pane.open_command(
        dir.path(),
        program,
        args,
        "Docker · own".into(),
        "context:id:sh".into()
    ));
    assert!(pane.parked.is_empty());
    pane.send(b"\r".to_vec());
    wait_for(&mut pane, |p| !p.running);
    pane.restart();
    wait_for(&mut pane, |p| {
        p.parser.screen().contents().contains("$(not-a-shell)")
    });
    assert_eq!(pane.session_label(), "Docker · own");
    assert!(pane.launch.is_some());
    pane.close();
}

#[cfg(unix)]
#[test]
#[ignore = "explicit owned Docker fixture only; CODISSEU_DOCKER_TEST_CONTAINER"]
fn real_docker_exec_pty_unicode_resize_ctrl_c_and_exit() {
    let id = std::env::var("CODISSEU_DOCKER_TEST_CONTAINER").expect("owned fixture ID required");
    let mut docker = codisseu_core::docker::DockerService::default();
    let wait = |docker: &mut codisseu_core::docker::DockerService| {
        let start = Instant::now();
        while docker.is_busy() {
            docker.poll();
            assert!(start.elapsed() < Duration::from_secs(45));
            thread::sleep(Duration::from_millis(10));
        }
        assert!(docker.state.error.is_none(), "{:?}", docker.state.error);
    };
    docker
        .dispatch(codisseu_core::docker::DockerCommand::Refresh)
        .unwrap();
    wait(&mut docker);
    let container = docker
        .state
        .containers
        .iter()
        .find(|c| c.id == id)
        .expect("fixture missing");
    assert!(container.name.starts_with("codisseu-docker-test-"));
    assert_eq!(container.state, "running");
    docker
        .dispatch(codisseu_core::docker::DockerCommand::Select(id.clone()))
        .unwrap();
    wait(&mut docker);
    let spec = docker.exec_spec(&id, "sh").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut pane = TerminalPane::default();
    pane.open_command(
        dir.path(),
        spec.program,
        spec.args,
        "Docker · fixture".into(),
        format!("{}:{id}", spec.context),
    );
    pane.send(
        "printf 'DOCKER_PTY=%s\\n' 'ação界'; pwd\r"
            .as_bytes()
            .to_vec(),
    );
    wait_for(&mut pane, |p| {
        p.parser.screen().contents().contains("DOCKER_PTY=ação界")
    });
    pane.session
        .as_ref()
        .unwrap()
        .resize
        .try_send(size(17, 73))
        .unwrap();
    thread::sleep(Duration::from_millis(50));
    pane.send(b"stty size; sleep 30\r".to_vec());
    wait_for(&mut pane, |p| {
        p.parser.screen().contents().contains("17 73")
    });
    pane.send(vec![3]);
    pane.send(b"printf '%s' 'INTERRUPTED_' 'OWN_EXEC'; printf '\\n'\r".to_vec());
    wait_for(&mut pane, |p| {
        p.parser
            .screen()
            .contents()
            .contains("INTERRUPTED_OWN_EXEC")
    });
    pane.send(b"exit\r".to_vec());
    wait_for(&mut pane, |p| !p.running);
    pane.close();
    docker.shutdown();
}
