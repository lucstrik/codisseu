//! Short EOF caret must remain visible when split groups are resized.
use codisseu_core::{AppService, Command};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

#[test]
fn short_eof_caret_remains_visible_and_idle_in_both_split_orientations() {
    for scale in [1., 1.25] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("short.ts");
        let source = "const n = 1;\nlast";
        fs::write(&path, source).unwrap();
        let mut service = AppService::new(dir.path().join("session.json"));
        service.dispatch(Command::OpenFile(path)).unwrap();
        let end = Instant::now() + Duration::from_secs(10);
        while service.is_busy() {
            service.poll();
            assert!(Instant::now() < end);
            thread::sleep(Duration::from_millis(1));
        }
        let mut h = Harness::builder()
            .with_size([1280., 800.])
            .with_max_steps(60)
            .build_ui_state(
                |ui, a: &mut CodisseuApp| a.show(ui),
                CodisseuApp::new(service),
            );
        h.ctx.set_pixels_per_point(scale);
        h.step();
        h.get_by_role(Role::MultilineTextInput).click();
        h.key_press_modifiers(Modifiers::COMMAND, Key::End);
        h.run();
        let selection = h.state().service.active_document().unwrap().selection();
        assert_eq!(selection.head, source.chars().count());
        h.get_by_label("Mais ações do grupo").click();
        h.run();
        h.get_by_label("Dividir à direita").click();
        h.run();
        for horizontal in [true, false, true] {
            h.get_all_by_label("Mais ações do grupo")
                .next()
                .unwrap()
                .click();
            h.run();
            h.get_by_label("Alterar orientação da divisão").click();
            h.run(); // Regression: previously repainted forever at short EOF.
            assert_eq!(h.state().service.editor_split().horizontal, horizontal);
            assert_eq!(h.state().editor_view_metrics().len(), 2);
            for view in h.state().service.document_views() {
                assert_eq!(view.selection(), selection);
                assert!(h.state().editor_view_metrics()[&view.id].caret_in_viewport);
            }
            let scroll: Vec<_> = h
                .state()
                .service
                .document_views()
                .iter()
                .map(|v| v.scroll())
                .collect();
            h.run();
            assert_eq!(
                scroll,
                h.state()
                    .service
                    .document_views()
                    .iter()
                    .map(|v| v.scroll())
                    .collect::<Vec<_>>()
            );
        }
        assert_eq!(h.state().service.active_document().unwrap().text(), source);
        assert!(!h.state().service.active_document().unwrap().is_dirty());
    }
}
