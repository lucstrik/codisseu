use super::*;
use codisseu_core::debugger::{DebugRuntime, DebugThread, Scope, StackFrame, Variable};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

fn session(id: u64, state: DebugState) -> DebugSession {
    DebugSession {
        id,
        name: format!("fixture-{id}"),
        runtime: DebugRuntime::Node,
        state,
        threads: vec![DebugThread {
            id: 1,
            name: "Main Thread".into(),
        }],
        frames: vec![StackFrame {
            id: 1,
            name: "main".into(),
            path: None,
            line: 3,
            column: 1,
        }],
        scopes: vec![Scope {
            name: "Local".into(),
            reference: 10,
            expensive: false,
        }],
        variables: BTreeMap::from([(
            10,
            vec![Variable {
                name: format!("session{id}"),
                value: format!("{}", id * 42),
                type_name: Some("number".into()),
                reference: 0,
                evaluate_name: None,
            }],
        )]),
        output: VecDeque::new(),
        breakpoints: vec![],
        last_error: None,
        exit_code: None,
        stop_epoch: 1,
        current_frame: Some(1),
        current_thread: Some(1),
        exception_filters: vec![],
        capabilities: serde_json::json!({}),
        evaluations: vec![],
        stopped_reason: Some("breakpoint".into()),
        pending: 0,
    }
}
fn harness(size: [f32; 2]) -> (tempfile::TempDir, Harness<'static, CodisseuApp>) {
    let dir = tempfile::tempdir().unwrap();
    let mut app = CodisseuApp::new(AppService::new(dir.path().join("session.json")));
    app.service.debugger.sessions = vec![
        session(1, DebugState::Stopped),
        session(2, DebugState::Stopped),
    ];
    app.debugger_ui.selected_session = Some(1);
    let mut h = Harness::builder().with_size(size).build_ui_state(
        |ui, app: &mut CodisseuApp| {
            if !app.styled {
                app.tokens.apply(ui.ctx());
                app.styled = true;
            }
            app.debugger_panel(ui);
        },
        app,
    );
    h.run();
    (dir, h)
}
#[test]
fn debug_sessions_keep_variables_and_console_selection_scoped() {
    let (_dir, mut h) = harness([1200., 350.]);
    h.state_mut().service.debugger.sessions[1].frames[0].path =
        Some("/tmp/selected-session.js".into());
    assert!(h.query_by_label("session1 = 42  {number}").is_some());
    assert!(h.query_by_label("session2 = 84  {number}").is_none());
    h.get_by_label("Debug session 2").click();
    h.run();
    assert_eq!(h.state().debugger_ui.selected_session, Some(2));
    assert_eq!(
        h.state()
            .jump_to
            .as_ref()
            .map(|target| (&target.0, target.1)),
        Some((&PathBuf::from("/tmp/selected-session.js"), 2))
    );
    assert!(h.query_by_label("session2 = 84  {number}").is_some());
    assert!(h.query_by_label("session1 = 42  {number}").is_none());
    h.get_by_label("Debugger Console").click();
    h.run();
    assert!(h.state().debugger_ui.console);
    h.get_by_label("Process Console").click();
    h.run();
    assert!(h.state().debugger_ui.process_console);
    h.get_by_label("Threads & Variables").click();
    h.run();
    assert!(!h.state().debugger_ui.console);
}
#[test]
fn debug_narrow_panel_and_empty_frames_do_not_panic() {
    let (_dir, mut h) = harness([520., 220.]);
    h.state_mut().service.debugger.sessions[0].frames.clear();
    h.state_mut().service.debugger.sessions[0].current_frame = None;
    h.run();
    h.state_mut().service.debugger.sessions[0].state = DebugState::Running;
    h.run();
    assert!(
        h.query_by_label("Frames are available when paused")
            .is_some()
    );
}
#[test]
fn narrow_debug_frames_and_thread_remain_single_line() {
    let (_dir, mut h) = harness([520., 300.]);
    h.state_mut().service.debugger.sessions[0].frames[0].name =
        "very_long_namespace_and_method_name_to_truncate".into();
    h.state_mut().service.debugger.sessions[0].threads[0].name =
        "very_long_thread_caption_to_truncate".into();
    h.run();
    let frame =
        h.get_by_label("very_long_namespace_and_method_name_to_truncate  <source unavailable>:3");
    assert!(frame.rect().height() <= 25.);
    assert!(h.get_by_label("Debug Thread").rect().height() <= 28.);
}
#[test]
fn debug_mute_controls_real_breakpoint_state_without_starting_a_runtime() {
    let (_dir, mut h) = harness([1200., 350.]);
    h.state_mut()
        .service
        .debugger
        .breakpoints
        .push(codisseu_core::debugger::Breakpoint {
            path: "/tmp/fixture.js".into(),
            line: 3,
            enabled: true,
            verified: false,
            options: Default::default(),
            revision: 0,
            message: None,
        });
    h.get_by_label("Mute Breakpoints").click();
    h.run();
    assert!(h.state().debugger_ui.muted);
    assert!(h.state().service.debugger.breakpoints[0].enabled);
    assert!(!h.state().service.debugger.breakpoints_enabled());
    h.get_by_label("Mute Breakpoints").click();
    h.run();
    assert!(h.state().service.debugger.breakpoints[0].enabled);
}
#[test]
fn watch_expressions_never_cross_session_boundaries() {
    let (_dir, mut h) = harness([1200., 350.]);
    h.state_mut()
        .debugger_ui
        .watches
        .insert(1, vec!["php_only()".into()]);
    h.get_by_label("Debug session 2").click();
    h.run();
    assert!(h.query_by_label("Remove Watch php_only()").is_none());
    h.get_by_label("Debug session 1").click();
    h.run();
    assert!(h.query_by_label("Remove Watch php_only()").is_some());
}

#[test]
fn evaluation_drafts_are_scoped_and_enter_cannot_execute_while_running() {
    let (_dir, mut h) = harness([1200., 350.]);
    h.state_mut()
        .debugger_ui
        .expressions
        .insert(1, "first()".into());
    h.state_mut()
        .debugger_ui
        .expressions
        .insert(2, "second()".into());
    h.run();
    h.get_by_label("Debug session 2").click();
    h.run();
    assert_eq!(h.state().debugger_ui.expressions[&1], "first()");
    assert_eq!(h.state().debugger_ui.expressions[&2], "second()");
    h.state_mut().service.debugger.sessions[1].state = DebugState::Running;
    h.run();
    h.get_by_label("Evaluate expression").click();
    h.key_press(Key::Enter);
    h.run();
    // These synthetic sessions have no worker: sending Evaluate would produce an error.
    assert!(h.state().service.debugger.error.is_none());
    assert!(h.get_by_label("Evaluate").accesskit_node().is_disabled());
}

#[test]
fn thread_caption_uses_backend_selection_and_exception_filters_are_negotiated() {
    let (_dir, mut h) = harness([1200., 350.]);
    let s = &mut h.state_mut().service.debugger.sessions[0];
    s.threads.push(DebugThread {
        id: 2,
        name: "Worker Thread".into(),
    });
    s.current_thread = Some(2);
    s.capabilities = serde_json::json!({"exceptionBreakpointFilters":[
        {"filter":"custom-engine-id","label":"Custom caught errors"}
    ]});
    h.state_mut().debugger_ui.selected_threads.insert(1, 1);
    h.run();
    assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Text(text) if text.galley.text() == "Worker Thread")));
    h.get_by_label("Exception Breakpoints").click();
    h.run();
    assert!(h.query_by_label("Custom caught errors").is_some());
    h.key_press(Key::Escape);
    h.run();
    h.state_mut().service.debugger.sessions[0].capabilities = serde_json::json!({});
    h.run();
    assert!(
        h.get_by_label("Exception Breakpoints")
            .accesskit_node()
            .is_disabled()
    );
}

#[test]
fn frames_splitter_resizes_with_pointer_and_keeps_variables_visible() {
    let (_dir, mut h) = harness([1200., 350.]);
    let start = h.get_by_label("Resize debug frames").rect().center();
    h.event(egui::Event::PointerMoved(start));
    h.event(egui::Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.step();
    let end = start + egui::vec2(110., 0.);
    h.event(egui::Event::PointerMoved(end));
    h.step();
    h.event(egui::Event::PointerButton {
        pos: end,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run();
    assert!(h.get_by_label("Resize debug frames").rect().center().x > start.x + 80.);
    assert!(h.get_by_label("Evaluate").rect().max.x < 1200.);
}

#[test]
fn breakpoint_popup_uses_selected_session_confirmation_and_mute() {
    let (_dir, mut h) = harness([1200., 350.]);
    let bp = codisseu_core::debugger::Breakpoint {
        path: "/tmp/shared.js".into(),
        line: 3,
        enabled: true,
        verified: true,
        options: Default::default(),
        revision: 0,
        message: None,
    };
    h.state_mut().service.debugger.breakpoints.push(bp.clone());
    let mut pending = bp;
    pending.verified = false;
    h.state_mut().service.debugger.sessions[0]
        .breakpoints
        .push(pending);
    h.get_by_label("View Breakpoints").click();
    h.run();
    assert!(
        h.query_by_label("shared.js:3 · Breakpoint pending verification")
            .is_some()
    );
    assert!(
        h.query_by_label("shared.js:3 · Breakpoint verified")
            .is_none()
    );
    h.key_press(Key::Escape);
    h.run();
    h.get_by_label("Mute Breakpoints").click();
    h.run();
    h.get_by_label("View Breakpoints").click();
    h.run();
    assert!(
        h.query_by_label("shared.js:3 · Breakpoint muted; execution will not stop here")
            .is_some()
    );
}

#[test]
fn exception_draft_survives_an_earlier_evaluation_error_and_expires_if_unconfirmed() {
    let (_dir, mut h) = harness([1200., 350.]);
    h.state_mut().service.debugger.sessions[0].last_error =
        Some("Earlier evaluation failed".into());
    h.state_mut()
        .debugger_ui
        .exception_drafts
        .insert(1, ExceptionDraft::new(vec!["all".into()]));
    h.state_mut().debugger_tick(&egui::Context::default());
    assert_eq!(h.state().debugger_ui.exception_drafts[&1].filters, ["all"]);
    h.state_mut()
        .debugger_ui
        .exception_drafts
        .get_mut(&1)
        .unwrap()
        .sent -= Duration::from_secs(36);
    h.state_mut().debugger_tick(&egui::Context::default());
    assert!(!h.state().debugger_ui.exception_drafts.contains_key(&1));
}
#[test]
fn configuration_requires_explicit_execution_authorization_and_cancel_discards_draft() {
    for size in [[1200., 800.], [800., 700.]] {
        let dir = tempfile::tempdir().unwrap();
        let mut app = CodisseuApp::new(AppService::new(dir.path().join("session.json")));
        app.debugger_open_configurations();
        let mut h = Harness::builder().with_size(size).build_ui_state(
            |ui, app: &mut CodisseuApp| {
                if !app.styled {
                    app.tokens.apply(ui.ctx());
                    app.styled = true;
                }
                app.debugger_dialog(ui.ctx());
            },
            app,
        );
        h.run();
        assert!(
            h.get_by_label("Start Debug Configuration")
                .accesskit_node()
                .is_disabled()
        );
        assert!(h.query_by_label("Name").is_some());
        assert!(h.query_by_label("Node interpreter:").is_some());
        assert!(h.query_by_label("File:").is_some());
        assert!(
            h.get_by_label("Close Run/Debug Configurations")
                .rect()
                .min
                .y
                >= 0.
        );
        assert!(h.get_by_label("Cancel").rect().max.y <= size[1]);
        assert!(
            h.get_by_label("I trust this configuration and authorize executing its code")
                .rect()
                .max
                .y
                < h.get_by_label("Cancel").rect().min.y
        );
        h.get_by_label("I trust this configuration and authorize executing its code")
            .click();
        h.run();
        assert!(
            !h.get_by_label("Start Debug Configuration")
                .accesskit_node()
                .is_disabled()
        );
        h.get_by_label("Cancel").click();
        h.run();
        assert!(!h.state().debugger_ui.dialog);
        assert!(h.state().debugger_ui.draft.is_none());
        assert!(h.state().service.debugger.sessions.is_empty());
    }
}

#[test]
fn step_with_reused_frame_id_reveals_the_new_source_location() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fixture.js");
    std::fs::write(&path, "let n = 1;\nn++;\nn++;\nn++;\n").unwrap();
    let mut app = CodisseuApp::new(AppService::new(dir.path().join("session.json")));
    let mut paused = session(1, DebugState::Stopped);
    paused.frames[0].path = Some(path.clone());
    app.service.debugger.sessions.push(paused);
    let ctx = egui::Context::default();
    app.debugger_tick(&ctx);
    assert_eq!(app.jump_to, Some((path.clone(), 2, 0)));
    app.service.debugger.sessions[0].frames[0].line = 4;
    app.debugger_tick(&ctx);
    assert_eq!(app.jump_to, Some((path.clone(), 3, 0)));
    app.jump_to = None;
    app.debugger_ui.requested_variables.insert((1, 999));
    app.service.debugger.sessions[0].stop_epoch += 1;
    app.debugger_tick(&ctx);
    assert_eq!(app.jump_to, Some((path, 3, 0)));
    assert!(!app.debugger_ui.requested_variables.contains(&(1, 999)));
}

#[test]
fn python_debug_configuration_file_module_interpreter_and_footer_in_both_scales() {
    for scale in [1., 1.25] {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("entry.py");
        std::fs::write(&source, "print(42)\n").unwrap();
        let mut app = CodisseuApp::new(AppService::new(temp.path().join("profile/session.json")));
        app.debugger_edit_configuration(DebugConfiguration::for_file(&source, temp.path()));
        app.debugger_ui.dialog = true;
        let size = [1000., 760.];
        let mut h = Harness::builder()
            .with_size(size)
            .with_pixels_per_point(scale)
            .with_max_steps(100)
            .build_ui_state(
                |ui, app: &mut CodisseuApp| {
                    if !app.styled {
                        app.tokens.apply(ui.ctx());
                        app.styled = true;
                    }
                    app.debugger_dialog(ui.ctx());
                },
                app,
            );
        h.run();
        assert!(h.query_by_label("Python interpreter:").is_some());
        assert!(h.query_by_label("File:").is_some());
        assert!(
            h.get_by_label("Start Debug Configuration")
                .accesskit_node()
                .is_disabled()
        );
        h.get_by_label("Target:").click();
        h.run();
        h.get_by_label("Module (-m)").click();
        h.run();
        h.get_by_label("Module:").click();
        h.event(egui::Event::Text("pytest".into()));
        h.run();
        let draft = h.state().debugger_ui.draft.as_ref().unwrap();
        assert_eq!(draft.runtime, DebugRuntime::Python);
        assert_eq!(draft.module.as_deref(), Some("pytest"));
        assert!(draft.program.is_none());
        for label in ["Cancel", "Apply", "Start Debug Configuration"] {
            let node = h.get_by_label(label);
            assert!(
                node.rect().min.y >= 0. && node.rect().max.y <= size[1],
                "{label}: {:?}",
                node.rect()
            );
        }
        if let Some(out) = std::env::var_os("CODISSEU_DEBUG_CAPTURES") {
            std::fs::create_dir_all(&out).unwrap();
            h.event(egui::Event::PointerGone);
            h.run();
            h.render()
                .unwrap()
                .save(PathBuf::from(out).join(format!("python-configuration-module-{scale}.png")))
                .unwrap();
        }
        h.get_by_label("Cancel").click();
        h.run();
        assert!(!h.state().debugger_ui.dialog);
        assert!(h.state().service.debugger.sessions.is_empty());
        assert_eq!(std::fs::read_to_string(source).unwrap(), "print(42)\n");
    }
}
