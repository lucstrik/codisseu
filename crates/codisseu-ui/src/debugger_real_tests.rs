//! Real egui input -> AppService -> installed DAP -> owned PHP/Node fixtures.
use super::*;
use codisseu_core::debugger::{DebugCommand as D, DebugConfiguration, DebugState};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::time::Instant;

fn wait(h: &mut Harness<CodisseuApp>, ready: impl Fn(&CodisseuApp) -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        h.step();
        for s in &h.state().service.debugger.sessions {
            assert_ne!(s.state, DebugState::Failed, "{:?}", s.last_error);
        }
        if ready(h.state()) {
            return;
        }
        assert!(
            Instant::now() < end,
            "{:?}",
            h.state().service.debugger.sessions
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

// DAP requests and PHP declaration analysis advance on real workers, while
// Harness::run can consume hundreds of synthetic frames before either replies.
// Live debug sessions also intentionally schedule periodic repainting.
fn settle(h: &mut Harness<CodisseuApp>) {
    let end = Instant::now() + Duration::from_secs(20);
    let mut idle_frames = 0;
    while idle_frames < 8 {
        h.step();
        assert!(Instant::now() < end, "debugger UI did not settle");
        let app = h.state();
        if app.service.is_busy()
            || app.php_navigation.pending()
            || app
                .service
                .debugger
                .sessions
                .iter()
                .any(|session| session.pending != 0)
        {
            idle_frames = 0;
        } else {
            idle_frames += 1;
        }
        std::thread::sleep(Duration::from_millis(3));
    }
}

#[test]
fn debugger_real_php_javascript_and_python_pointer_keyboard_evaluate_watch_and_step() {
    for (extension, text, expression, scale) in [
        (
            "php",
            "<?php\nfunction calculate(int $input): int {\n    $answer = $input + 2;\n    $label = 'Codisseu PHP';\n    $result = $answer * 2;\n    return $result;\n}\necho calculate(40);\n",
            "$answer",
            1.,
        ),
        (
            "js",
            "function calculate(input) {\n    const answer = input + 2;\n    const label = 'Codisseu JS';\n    const result = answer * 2;\n    return result;\n}\nconsole.log(calculate(40));\n",
            "answer",
            1.25,
        ),
        (
            "py",
            "def calculate(input):\n    answer = input + 2\n    label = 'Codisseu Python'\n    result = answer * 2\n    return result\nprint(calculate(40))\n",
            "answer",
            1.,
        ),
        (
            "pyw",
            "def calculate(input):\n    answer = input + 2\n    label = 'Codisseu Python'\n    result = answer * 2\n    return result\nprint(calculate(40))\n",
            "answer",
            1.25,
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(format!("calculate.{extension}"));
        std::fs::write(&path, text).unwrap();
        let mut service = AppService::new(temp.path().join("profile/session.json"));
        service.dispatch(Command::OpenFile(path.clone())).unwrap();
        let mut app = CodisseuApp::new(service);
        app.show_project = false;
        app.show_database = false;
        app.show_activity = true;
        app.tool_tab = ToolTab::Debug;
        let mut h = Harness::builder()
            .with_size([1280., 900.])
            .with_pixels_per_point(scale)
            .with_max_steps(100)
            .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
        wait(&mut h, |a| {
            !a.service.is_busy() && a.service.active_document().is_some()
        });
        settle(&mut h);
        let line = if extension == "php" { 5 } else { 4 };
        h.get_by_label(&format!("Add breakpoint: {}:{line}", path.display()))
            .click();
        settle(&mut h);
        let mut config = DebugConfiguration::for_file(&path, temp.path());
        config.trusted = true;
        h.state_mut()
            .service
            .dispatch(Command::Debug(D::Start(config)))
            .unwrap();
        wait(&mut h, |a| {
            a.service.debugger.sessions.first().is_some_and(|s| {
                s.state == DebugState::Stopped
                    && !s.frames.is_empty()
                    && s.variables
                        .values()
                        .flatten()
                        .any(|v| v.name.contains("answer") && v.value == "42")
            })
        });
        let id = h.state().service.debugger.sessions[0].id;
        assert_eq!(h.state().service.debugger.sessions[0].frames[0].line, line);
        assert!(
            h.state().service.debugger.sessions[0]
                .breakpoints
                .iter()
                .any(|b| b.line == line && b.verified)
        );
        h.state_mut()
            .debugger_ui
            .expressions
            .insert(id, expression.into());
        settle(&mut h);
        h.key_press_modifiers(Modifiers::ALT, Key::F8);
        settle(&mut h);
        assert!(
            h.get_by_label("Evaluate expression")
                .accesskit_node()
                .is_focused(),
            "Alt+F8 must focus the evaluator"
        );
        h.key_press(Key::Enter);
        wait(&mut h, |a| {
            a.service.debugger.sessions[0]
                .evaluations
                .iter()
                .any(|e| e.expression == expression && e.result == "42" && e.error.is_none())
        });
        h.get_by_label("Add to Watches").click();
        settle(&mut h);
        assert_eq!(h.state().debugger_ui.watches[&id], [expression]);
        if extension == "js" {
            // Model the first menu change awaiting ACK, then click a second filter.
            // The final combined policy must be acknowledged by the real adapter.
            h.state_mut()
                .debugger_ui
                .exception_drafts
                .insert(id, debugger::ExceptionDraft::new(vec!["all".into()]));
            h.get_by_label("Exception Breakpoints").click();
            settle(&mut h);
            h.get_by_label("Uncaught Exceptions").click();
            wait(&mut h, |a| {
                let s = &a.service.debugger.sessions[0];
                s.pending == 0
                    && s.exception_filters.contains(&"all".into())
                    && s.exception_filters.contains(&"uncaught".into())
            });
            h.key_press(Key::Escape);
            settle(&mut h);
        }
        if let Some(out) = std::env::var_os("CODISSEU_DEBUG_CAPTURES") {
            std::fs::create_dir_all(&out).unwrap();
            h.event(egui::Event::PointerGone);
            settle(&mut h);
            h.render()
                .unwrap()
                .save(PathBuf::from(out).join(format!("paused-{extension}-{scale}.png")))
                .unwrap();
        }
        let epoch = h.state().service.debugger.sessions[0].stop_epoch;
        h.key_press(Key::F8);
        wait(&mut h, |a| {
            let s = &a.service.debugger.sessions[0];
            s.state == DebugState::Stopped && s.stop_epoch > epoch && !s.frames.is_empty()
        });
        assert_eq!(
            h.state().service.debugger.sessions[0].frames[0].line,
            line + 1
        );
        h.get_by_label("Stop Debug Session  Shift+F5").click();
        wait(&mut h, |a| {
            !a.service.debugger.sessions[0].state.is_active()
        });
        assert!(h.state().service.debugger.sessions[0].frames.is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
}
