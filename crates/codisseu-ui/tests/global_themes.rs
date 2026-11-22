//! Global palette selection exercises the same settings widgets as the app.
use codisseu_core::{AppService, Command, ThemePreference};
use codisseu_ui::CodisseuApp;
use egui::{Color32, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

fn settle(h: &mut Harness<'_, CodisseuApp>) {
    h.step();
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().service.is_busy() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
        h.step();
    }
    h.run();
    // Modals settle their centered rectangle after measuring the scrolled
    // contents. Refresh AccessKit bounds before clicking a nested control.
    h.step();
    h.run();
}

#[test]
fn settings_switches_all_widget_colors_live_without_editing_the_document() {
    for scale in [1., 1.25] {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sample.php");
        let source = "<?php // ação 🦀\nfunction example() { return 42; }\n";
        fs::write(&file, source).unwrap();
        let mut service = AppService::new(dir.path().join("session.json"));
        service.dispatch(Command::OpenFile(file.clone())).unwrap();
        let mut h = Harness::builder()
            .with_size([1280., 800.])
            .with_pixels_per_point(scale)
            .with_max_steps(30)
            .build_eframe(|_| CodisseuApp::new(service));
        settle(&mut h);
        let version = h.state().service.active_document().unwrap().version;
        h.get_by_label("Configurações").click();
        settle(&mut h);
        for (label, expected, panel, background) in [
            (
                "Codisseu Clássico",
                ThemePreference::Classic,
                [25, 26, 28],
                [25, 26, 28],
            ),
            (
                "Codisseu Noturno",
                ThemePreference::Midnight,
                [16, 22, 29],
                [12, 17, 23],
            ),
        ] {
            let pos = h.get_by_role(Role::ComboBox).rect().center();
            h.event(egui::Event::PointerMoved(pos));
            h.step();
            h.event(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
            h.step();
            h.event(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
            h.step();
            h.step();
            h.get_by_label(label).click();
            settle(&mut h);
            assert_eq!(h.state().service.editor_preferences().theme, expected);
            let style = h.ctx.global_style();
            assert_eq!(
                style.visuals.panel_fill,
                Color32::from_rgb(panel[0], panel[1], panel[2])
            );
            assert_eq!(
                style.visuals.extreme_bg_color,
                Color32::from_rgb(background[0], background[1], background[2])
            );
            let doc = h.state().service.active_document().unwrap();
            assert_eq!(doc.text(), source);
            assert_eq!(doc.version, version);
            assert!(!doc.is_dirty() && !doc.can_undo());
        }
        h.get_by_label("Concluído").click();
        settle(&mut h);
        assert_eq!(fs::read_to_string(file).unwrap(), source);
        assert!(h.query_by_label("Tema do aplicativo").is_none());
    }
}
