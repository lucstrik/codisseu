#![cfg(target_os = "linux")]
use codisseu_browser::{
    BrowserService, Command, DevToolsEvent, EngineConfig, EngineState, Surface,
};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
fn fixture(mode: &str) -> (tempfile::TempDir, BrowserService) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let program = root.join("helper");
    fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/engine.py"),
        &program,
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(root.join("mode"), mode).unwrap();
    let mut s = BrowserService::new();
    s.configure(EngineConfig {
        program,
        runtime_dir: root,
    })
    .unwrap();
    (dir, s)
}
fn wait(s: &mut BrowserService, f: impl Fn(&BrowserService) -> bool) {
    let end = Instant::now() + Duration::from_secs(7);
    loop {
        s.poll();
        if f(s) {
            return;
        }
        assert!(Instant::now() < end, "{:?} {:?}", s.state, s.last_error);
        thread::sleep(Duration::from_millis(5));
    }
}
fn open(s: &mut BrowserService, id: u64) {
    s.dispatch(Command::Open {
        id,
        url: "localhost:5173/ação".into(),
    })
    .unwrap();
    wait(s, |s| s.state == EngineState::Ready);
    s.dispatch(Command::Resize {
        id,
        surface: Surface::Page,
        width: 2,
        height: 1,
        scale: 1.,
    })
    .unwrap();
    s.dispatch(Command::Visible {
        id,
        surface: Surface::Page,
        visible: true,
    })
    .unwrap();
}
fn dead(pid: u32) -> bool {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"));
    stat.is_err() || stat.is_ok_and(|s| s.split(") ").nth(1).is_some_and(|s| s.starts_with('Z')))
}
#[test]
fn opens_only_on_request_private_authenticated_ipc_real_pixels_and_last_close_reaps() {
    let (dir, mut s) = fixture("foreign-peer");
    assert!(!dir.path().join("starts").exists());
    assert!(!s.is_busy());
    open(&mut s, 1);
    wait(&mut s, |s| s.frame(1, Surface::Page).is_some());
    assert!(!s.is_busy(), "ongoing page loading is not global busy");
    assert!(s.tabs[&1].loading);
    let f = s.frame(1, Surface::Page).unwrap();
    assert_eq!(&f.bgra[..4], &[1, 2, 3, 255]);
    assert_eq!(&f.rgba[..4], &[3, 2, 1, 255]);
    let info: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("info.json")).unwrap()).unwrap();
    assert_eq!(info["dir_mode"], 448);
    assert_eq!(info["profile_mode"], 448);
    assert_eq!(info["socket_mode"], 384);
    assert_eq!(info["env_leak"], false);
    let pid = s.engine_pid().unwrap();
    let socket = PathBuf::from(info["socket"].as_str().unwrap());
    s.dispatch(Command::Close { id: 1 }).unwrap();
    wait(&mut s, |s| s.state == EngineState::Stopped);
    assert!(dead(pid));
    assert!(!socket.exists());
    assert!(s.frames.is_empty());
}
#[test]
fn hidden_and_closed_surfaces_drop_late_frames_and_ids_cannot_be_reused() {
    let (_dir, mut s) = fixture("normal");
    open(&mut s, 1);
    open(&mut s, 2);
    wait(&mut s, |s| s.frame(2, Surface::Page).is_some());
    s.dispatch(Command::Visible {
        id: 2,
        surface: Surface::Page,
        visible: false,
    })
    .unwrap();
    assert!(s.frame(2, Surface::Page).is_none());
    s.dispatch(Command::Close { id: 1 }).unwrap();
    thread::sleep(Duration::from_millis(100));
    s.poll();
    assert!(!s.tabs.contains_key(&1));
    assert!(s.frame(1, Surface::Page).is_none());
    assert!(
        s.dispatch(Command::Open {
            id: 1,
            url: "https://example.com".into()
        })
        .is_err()
    );
    s.shutdown();
    wait(&mut s, |s| s.state == EngineState::Stopped);
}
#[test]
fn frame_burst_is_coalesced_to_latest_sequence_without_global_busy() {
    let (_dir, mut s) = fixture("burst");
    let wake = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = wake.clone();
    s.set_waker(std::sync::Arc::new(move || {
        count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }));
    open(&mut s, 1);
    thread::sleep(Duration::from_millis(150));
    wait(&mut s, |s| {
        s.frame(1, Surface::Page).is_some_and(|f| f.seq == 100)
    });
    assert_eq!(s.frames.len(), 1);
    assert!(wake.load(std::sync::atomic::Ordering::Relaxed) > 0);
    s.shutdown();
    wait(&mut s, |s| s.state == EngineState::Stopped);
}
#[test]
fn invalid_engine_output_and_crashes_fail_once_and_explicit_reload_recovers() {
    for mode in [
        "crash-before",
        "crash-ready",
        "old-protocol",
        "badurl",
        "oversize",
        "badframe",
    ] {
        let (dir, mut s) = fixture(mode);
        s.dispatch(Command::Open {
            id: 1,
            url: "https://example.com".into(),
        })
        .unwrap();
        if mode == "badframe" {
            wait(&mut s, |s| s.state == EngineState::Ready);
            s.dispatch(Command::Resize {
                id: 1,
                surface: Surface::Page,
                width: 2,
                height: 1,
                scale: 1.,
            })
            .unwrap();
            s.dispatch(Command::Visible {
                id: 1,
                surface: Surface::Page,
                visible: true,
            })
            .unwrap();
        }
        wait(&mut s, |s| s.state == EngineState::Failed);
        assert!(s.last_error.is_some(), "{mode}");
        if mode == "old-protocol" {
            assert!(
                s.last_error
                    .as_deref()
                    .unwrap()
                    .contains("Incompatible browser protocol")
            );
        }
        thread::sleep(Duration::from_millis(100));
        s.poll();
        assert_eq!(
            fs::read_to_string(dir.path().join("starts"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        fs::write(dir.path().join("mode"), "normal").unwrap();
        s.dispatch(Command::Reload {
            id: 1,
            ignore_cache: false,
        })
        .unwrap();
        wait(&mut s, |s| s.state == EngineState::Ready);
        assert_eq!(
            fs::read_to_string(dir.path().join("starts"))
                .unwrap()
                .lines()
                .count(),
            2
        );
        s.shutdown();
        wait(&mut s, |s| s.state == EngineState::Stopped);
    }
}
#[test]
fn forced_shutdown_reaps_actual_helper_and_its_process_group_before_five_seconds() {
    let (dir, mut s) = fixture("hang");
    open(&mut s, 1);
    wait(&mut s, |s| s.engine_pid().is_some());
    let pid = s.engine_pid().unwrap();
    let end = Instant::now() + Duration::from_secs(2);
    while !dir.path().join("child").exists() {
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(5));
    }
    let child = fs::read_to_string(dir.path().join("child"))
        .unwrap()
        .parse::<u32>()
        .unwrap();
    let started = Instant::now();
    s.shutdown();
    wait(&mut s, |s| s.state == EngineState::Stopped);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(dead(pid));
    assert!(dead(child));
}
#[test]
fn cdp_is_explicit_private_bounded_and_closes_on_tab_teardown() {
    let (_dir, mut s) = fixture("normal");
    assert!(s.devtools(1, 1).is_err());
    open(&mut s, 1);
    assert!(
        s.dispatch(Command::Clipboard {
            id: 1,
            surface: Surface::Page,
            action: codisseu_browser::ClipboardAction::Paste
        })
        .is_err()
    );
    let endpoint = || {
        codisseu_browser::SecretUrl::devtools(format!(
            "devtools://devtools/bundled/inspector.html?can_dock=false&ws=127.0.0.1:45678/{}",
            "a".repeat(64)
        ))
        .unwrap()
    };
    assert!(
        s.dispatch(Command::DevToolsFrontend {
            id: 1,
            url: endpoint()
        })
        .is_err()
    );
    assert!(s.open_devtools_frontend(1, endpoint()).is_err());
    assert_eq!(s.tabs[&1].frontend_epoch, 0);
    let c = s.devtools(1, 7).unwrap();
    s.open_devtools_frontend(1, endpoint()).unwrap();
    assert_eq!(s.tabs[&1].frontend_epoch, 7);
    assert!(s.devtools(1, 8).is_err());
    c.sender
        .send(r#"{"id":1,"method":"Runtime.evaluate","params":{"expression":"21*2"}}"#)
        .unwrap();
    let event = c.events.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(matches!(event,DevToolsEvent::Message{json} if json.contains("42")));
    assert!(c.sender.send("[]").is_err());
    assert!(c.sender.send("x".repeat(4 * 1024 * 1024 + 1)).is_err());
    s.dispatch(Command::Close { id: 1 }).unwrap();
    assert!(matches!(
        c.events.recv_timeout(Duration::from_secs(1)).unwrap(),
        DevToolsEvent::Closed { .. }
    ));
    assert!(c.sender.send("{}").is_err());
    wait(&mut s, |s| s.state == EngineState::Stopped);
}
#[test]
fn drop_never_waits_for_a_hung_browser_on_the_calling_thread() {
    let (_dir, mut s) = fixture("hang");
    open(&mut s, 1);
    let pid = s.engine_pid().unwrap();
    let started = Instant::now();
    drop(s);
    assert!(started.elapsed() < Duration::from_millis(100));
    let end = Instant::now() + Duration::from_secs(6);
    while !dead(pid) {
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn native_last_page_close_is_graceful_not_reported_as_a_crash() {
    let (_dir, mut service) = fixture("native-close");
    service
        .dispatch(Command::Open {
            id: 1,
            url: "about:blank".into(),
        })
        .unwrap();
    wait(&mut service, |s| {
        s.state == EngineState::Stopped && s.tabs.is_empty()
    });
    assert!(service.last_error.is_none(), "{:?}", service.last_error);
    assert!(service.engine_pid().is_none());
}
#[test]
fn eight_tab_bound_and_deferred_missing_engine_errors_do_not_start_at_configuration() {
    let (_dir, mut service) = fixture("normal");
    open(&mut service, 1);
    for id in 2..=8 {
        service
            .dispatch(Command::Open {
                id,
                url: "https://example.com".into(),
            })
            .unwrap();
    }
    assert!(
        service
            .dispatch(Command::Open {
                id: 9,
                url: "https://example.com".into()
            })
            .is_err()
    );
    assert_eq!(service.tabs.len(), 8);
    service.shutdown();
    wait(&mut service, |s| s.state == EngineState::Stopped);
    let dir = tempfile::tempdir().unwrap();
    let mut missing = BrowserService::new();
    missing
        .configure(EngineConfig {
            program: dir.path().join("missing"),
            runtime_dir: dir.path().into(),
        })
        .unwrap();
    assert_eq!(missing.state, EngineState::Stopped);
    assert!(missing.last_error.is_none());
    missing
        .dispatch(Command::Open {
            id: 1,
            url: "about:blank".into(),
        })
        .unwrap();
    wait(&mut missing, |s| s.state == EngineState::Failed);
    assert!(missing.last_error.is_some());
    assert!(missing.engine_pid().is_none());
}

#[test]
fn dropping_a_debug_channel_delivers_unsubscribe_while_page_stays_open() {
    let (dir, mut service) = fixture("normal");
    open(&mut service, 1);
    let channel = service.devtools(1, 11).unwrap();
    channel
        .sender
        .send(r#"{"id":1,"method":"Debugger.pause"}"#)
        .unwrap();
    let _ = channel.events.recv_timeout(Duration::from_secs(2)).unwrap();
    drop(channel.events);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let wire = fs::read_to_string(dir.path().join("wire.jsonl")).unwrap_or_default();
        if wire.lines().any(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            v["type"] == "DevToolsUnsubscribe" && v["epoch"] == 11
        }) {
            break;
        }
        service.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(channel.sender.send("{}").is_err());
    let next = service.devtools(1, 12).unwrap();
    drop(next);
    assert_eq!(service.tabs.len(), 1);
    service.shutdown();
    wait(&mut service, |s| s.state == EngineState::Stopped);
}

#[test]
fn immediate_owned_bridge_replacement_revokes_old_sender_and_preserves_new_epoch() {
    let (_dir, mut s) = fixture("normal");
    open(&mut s, 1);
    let old = s.devtools(1, 20).unwrap();
    s.end_devtools(1);
    assert!(
        old.sender
            .send(r#"{"id":1,"method":"Runtime.enable"}"#)
            .is_err()
    );
    let new = s.devtools(1, 21).unwrap();
    drop(old);
    new.sender
        .send(r#"{"id":2,"method":"Runtime.evaluate","params":{"expression":"40+2"}}"#)
        .unwrap();
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        if let Ok(DevToolsEvent::Message { json }) =
            new.events.recv_timeout(Duration::from_millis(20))
            && serde_json::from_str::<serde_json::Value>(&json).unwrap()["id"] == 2
        {
            break;
        }
        assert!(
            Instant::now() < end,
            "replacement epoch did not receive actual fixture reply"
        );
    }
    s.shutdown();
    wait(&mut s, |s| s.state == EngineState::Stopped);
}

#[test]
fn second_tab_opened_after_ready_survives_first_close_and_remains_usable() {
    let (_dir, mut s) = fixture("normal");
    open(&mut s, 1);
    let pid = s.engine_pid().unwrap();
    open(&mut s, 2);
    wait(&mut s, |s| s.frame(2, Surface::Page).is_some());
    s.dispatch(Command::Close { id: 1 }).unwrap();
    thread::sleep(Duration::from_millis(150));
    s.poll();
    assert_eq!(
        s.state,
        EngineState::Ready,
        "closing first tab stopped the engine"
    );
    assert_eq!(s.engine_pid(), Some(pid));
    assert!(!dead(pid));
    assert_eq!(s.tabs.keys().copied().collect::<Vec<_>>(), [2]);
    let c = s.devtools(2, 1).unwrap();
    c.sender
        .send(r#"{"id":5,"method":"Runtime.evaluate","params":{"expression":"6*7"}}"#)
        .unwrap();
    assert!(
        matches!(c.events.recv_timeout(Duration::from_secs(2)).unwrap(),DevToolsEvent::Message{json} if json.contains("42"))
    );
    drop(c);
    s.dispatch(Command::Close { id: 2 }).unwrap();
    wait(&mut s, |s| {
        s.state == EngineState::Stopped && s.engine_pid().is_none()
    });
    assert!(dead(pid));
}
