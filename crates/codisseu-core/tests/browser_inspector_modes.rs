//! Real CEF arbitration: native paused -> SDK frontend -> native, without stale cleanup.
use codisseu_browser::{
    Command as BrowserCommand, EngineConfig, EngineState, Surface,
    inspector::{Action, Lifecycle},
};
use codisseu_core::{AppService, Command};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, Command as Process, Stdio},
    thread,
    time::{Duration, Instant},
};
struct Fixture(Child);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn wait(app: &mut AppService, p: impl Fn(&AppService) -> bool) {
    let end = Instant::now() + Duration::from_secs(25);
    loop {
        app.poll();
        assert_ne!(
            app.browser.state,
            EngineState::Failed,
            "{:?}",
            app.browser.last_error
        );
        if p(app) {
            return;
        }
        assert!(
            Instant::now() < end,
            "native={:?} bridge={}",
            app.browser_inspector.state,
            app.browser_devtools_diagnostics()
        );
        thread::sleep(Duration::from_millis(3));
    }
}
fn evaluate(app: &mut AppService, expression: &str, answer: &str) {
    app.dispatch_browser_inspector(Action::Evaluate {
        expression: expression.into(),
    })
    .unwrap();
    wait(app, |a| {
        a.browser_inspector
            .state
            .evaluation
            .as_ref()
            .is_some_and(|e| {
                e.expression == expression
                    && e.value
                        .as_ref()
                        .is_some_and(|v| v.description.contains(answer))
            })
    });
}
#[test]
#[ignore = "actual CEF, own Xvfb and private HTTP fixture"]
fn real_native_pause_sdk_native_epochs_resume_and_cleanup_preserve_new_owner() {
    assert_eq!(std::env::var("CODISSEU_INSPECTOR_REAL").as_deref(), Ok("1"));
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let mut child = Process::new(codisseu_runtime::python().unwrap())
        .arg("-I")
        .arg(root.join("fixtures/browser-inspector/server.py"))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let fixture = Fixture(child);
    let origin = serde_json::from_str::<serde_json::Value>(&line).unwrap()["origin"]
        .as_str()
        .unwrap()
        .to_owned();
    let private = tempfile::tempdir().unwrap();
    let mut app = AppService::new(private.path().join("session.json"));
    app.browser
        .configure(EngineConfig {
            program: root.join(".tools/browser/bin/codisseu-browser-engine"),
            runtime_dir: root.join(".tools/browser/cef"),
        })
        .unwrap();
    app.dispatch(Command::Browser(BrowserCommand::Open {
        id: 1,
        url: origin,
    }))
    .unwrap();
    app.dispatch(Command::Browser(BrowserCommand::Resize {
        id: 1,
        surface: Surface::Page,
        width: 1000,
        height: 700,
        scale: 1.,
    }))
    .unwrap();
    app.dispatch(Command::Browser(BrowserCommand::Visible {
        id: 1,
        surface: Surface::Page,
        visible: true,
    }))
    .unwrap();
    wait(&mut app, |a| {
        a.browser.state == EngineState::Ready
            && !a.browser.tabs[&1].loading
            && a.browser.tabs[&1].title.contains("Inspector")
    });
    app.start_browser_inspector(1).unwrap();
    wait(&mut app, |a| {
        a.browser_inspector.state.lifecycle == Lifecycle::Ready
    });
    let first_epoch = app.browser_inspector.state.session.as_ref().unwrap().epoch;
    evaluate(&mut app, "setTimeout(()=>computeProbe(),0)", "");
    wait(&mut app, |a| a.browser_inspector.state.paused.is_some());
    app.dispatch(Command::Browser(BrowserCommand::DevTools {
        id: 1,
        open: true,
    }))
    .unwrap();
    assert_eq!(app.browser_inspector.state.lifecycle, Lifecycle::Stopped);
    wait(&mut app, |a| {
        a.browser_devtools_diagnostics()["gateways"]
            .as_array()
            .is_some_and(|g| g.iter().any(|g| g["connected"] == true))
    });
    // A newly enabled native Debugger must keep working after old workers/bridge drop asynchronously.
    app.start_browser_inspector(1).unwrap();
    wait(&mut app, |a| {
        a.browser_inspector.state.lifecycle == Lifecycle::Ready
    });
    let second_epoch = app.browser_inspector.state.session.as_ref().unwrap().epoch;
    assert!(second_epoch > first_epoch);
    evaluate(&mut app, "window.lastResult", "42");
    evaluate(&mut app, "setTimeout(()=>computeProbe(),0)", "");
    wait(&mut app, |a| a.browser_inspector.state.paused.is_some());
    app.stop_browser_inspector();
    app.start_browser_inspector(1).unwrap();
    wait(&mut app, |a| {
        a.browser_inspector.state.lifecycle == Lifecycle::Ready
    });
    assert!(app.browser_inspector.state.session.as_ref().unwrap().epoch > second_epoch);
    evaluate(&mut app, "window.lastResult", "42");
    evaluate(&mut app, "setTimeout(()=>computeProbe(),0)", "");
    wait(&mut app, |a| a.browser_inspector.state.paused.is_some());
    app.dispatch_browser_inspector(Action::Resume).unwrap();
    wait(&mut app, |a| a.browser_inspector.state.paused.is_none());
    evaluate(&mut app, "19+23", "42");
    app.shutdown_browser();
    wait(&mut app, |a| {
        a.browser.state == EngineState::Stopped && a.browser.engine_pid().is_none()
    });
    assert!(app.browser_inspector.state.session.is_none());
    assert!(
        app.browser_devtools_diagnostics()["gateways"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    drop(fixture);
    std::fs::write(root.join("artifacts/browser-inspector-20261102/backend/core-modes-result.json"),"{\"status\":\"passed\",\"native_paused_to_sdk_to_native\":true,\"stop_paused_immediate_new_epoch\":true,\"new_debugger_survived_old_cleanup\":true,\"helper_reaped\":true}\n").unwrap();
}
