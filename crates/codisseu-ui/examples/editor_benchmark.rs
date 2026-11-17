//! CPU-side egui benchmark. It excludes GPU submission/presentation and OS input latency.
use codisseu_core::{AppService, Command};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn stats(mut samples: Vec<u128>) -> serde_json::Value {
    samples.sort_unstable();
    serde_json::json!({"n":samples.len(),"p50_us":samples[samples.len()/2],"p95_us":samples[(samples.len()*95/100).min(samples.len()-1)],"max_us":samples.last()})
}
fn main() {
    let mut results = vec![];
    for bytes in [1024 * 1024, 4 * 1024 * 1024, 16 * 1024 * 1024] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.txt");
        let row = "// large document weather 0123456789\n";
        let mut source = row.repeat(bytes / row.len());
        source.extend(std::iter::repeat_n('x', bytes - source.len()));
        fs::write(&path, &source).unwrap();
        let mut s = AppService::new(dir.path().join("session.json"));
        s.dispatch(Command::OpenFile(path)).unwrap();
        let cold = Instant::now();
        let mut h = Harness::builder()
            .with_size([1280., 800.])
            .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), CodisseuApp::new(s));
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            h.step();
            if !h.state().service.is_busy() {
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        h.get_by_role(Role::MultilineTextInput).focus();
        h.step();
        let cold_us = cold.elapsed().as_micros();
        let mut frames = vec![];
        let mut editor = vec![];
        let mut end_seek = vec![];
        for index in 0..400 {
            if index % 40 == 0 {
                h.key_press_modifiers(
                    Modifiers::CTRL,
                    if index % 80 == 0 { Key::End } else { Key::Home },
                );
            } else if index % 5 == 0 {
                h.key_press(Key::PageDown);
            } else if index % 5 == 1 {
                h.event(egui::Event::Text("a".into()));
            } else if index % 5 == 2 {
                h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
            }
            let start = Instant::now();
            h.step();
            let micros = start.elapsed().as_micros();
            frames.push(micros);
            editor.push(h.state().editor_metrics().frame_micros);
            if index % 40 == 0 {
                end_seek.push(micros);
            }
        }
        let metrics = serde_json::to_value(h.state().editor_metrics()).unwrap();
        results.push(serde_json::json!({"bytes":bytes,"lines":source.lines().count(),"cold_open_and_first_ui_us":cold_us,"egui_step_cpu":stats(frames),"editor_build_cpu":stats(editor),"home_end_cpu":stats(end_seek),"last_editor":metrics}));
    }
    let mut long_lines = vec![];
    for bytes in [1024 * 1024, 16 * 1024 * 1024] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("single-line.txt");
        fs::write(&path, "a".repeat(bytes)).unwrap();
        let mut service = AppService::new(dir.path().join("session.json"));
        service.dispatch(Command::OpenFile(path)).unwrap();
        let mut h = Harness::builder().with_size([1280., 800.]).build_ui_state(
            |ui, a: &mut CodisseuApp| a.show(ui),
            CodisseuApp::new(service),
        );
        let until = Instant::now() + Duration::from_secs(30);
        while h.state().service.is_busy() {
            h.step();
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(1));
        }
        h.get_by_role(Role::MultilineTextInput).focus();
        h.key_press_modifiers(Modifiers::CTRL, Key::End);
        let started = Instant::now();
        let mut samples = vec![];
        for _ in 0..5000 {
            let frame = Instant::now();
            h.step();
            samples.push(frame.elapsed().as_micros());
            if !h.state().editor_metrics().layout_pending {
                break;
            }
        }
        assert!(!h.state().editor_metrics().layout_pending);
        long_lines.push(serde_json::json!({"bytes":bytes,"cpu_steps_until_measured_end":samples.len(),"cpu_total_us":started.elapsed().as_micros(),"step_cpu":stats(samples),"last_editor":h.state().editor_metrics(),"note":"Unthrottled CPU loop; a displayed/vsynced window may take substantially longer. Prefix layout remains pending until measured; no instant-End claim."}));
    }
    println!("{}",serde_json::to_string_pretty(&serde_json::json!({"scope":"release egui_kittest CPU command/build/accessibility passes; excludes GPU/presentation/input-to-photon latency", "profile":if cfg!(debug_assertions){"debug"}else{"release"},"samples":results,"single_lines":long_lines})).unwrap());
}
