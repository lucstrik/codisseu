use super::*;
use egui_kittest::{Harness, kittest::Queryable};
#[test]
fn breakpoint_gutter_receives_real_pointer_input_outside_code_clip() {
    for scale in [1., 1.25] {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("gutter.php");
        std::fs::write(
            &source,
            "<?php\n$value = 42;\nfunction run() {\n    echo 'fixture';\n}\n",
        )
        .unwrap();
        let mut service = AppService::new(temp.path().join("profile/session.json"));
        service
            .dispatch(Command::OpenWorkspace(temp.path().into()))
            .unwrap();
        let until = std::time::Instant::now() + Duration::from_secs(10);
        while service.is_busy() {
            service.poll();
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(Duration::from_millis(5));
        }
        service.dispatch(Command::OpenFile(source.clone())).unwrap();
        while service.is_busy() {
            service.poll();
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut app = CodisseuApp::new(service);
        app.show_project = false;
        app.show_database = false;
        let mut h = Harness::builder()
            .with_size([1200., 800.])
            .with_pixels_per_point(scale)
            .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
        h.run();
        while h.state().editor_metrics().folding_pending {
            h.step();
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(Duration::from_millis(2));
        }
        let bp = h
            .get_by_label(&format!("Add breakpoint: {}:3", source.display()))
            .rect();
        let fold = h.get_by_label("Fold lines 3–5").rect();
        assert_eq!(bp.width(), 18.);
        assert_eq!(fold.width(), 13.);
        assert!((fold.center().x - bp.center().x - 16.).abs() < 0.1);
        assert!(bp.right() < fold.left());
        h.get_by_label(&format!("Add breakpoint: {}:3", source.display()))
            .click();
        h.run();
        assert!(
            h.state()
                .service
                .debugger
                .breakpoints
                .iter()
                .any(|point| point.path == source && point.line == 3)
        );
        if let Some(path) = std::env::var_os("CODISSEU_BREAKPOINT_CAPTURE") {
            while h.state().editor_metrics().folding_pending {
                h.step();
                assert!(std::time::Instant::now() < until);
                std::thread::sleep(Duration::from_millis(2));
            }
            h.event(egui::Event::PointerGone);
            h.run();
            h.render()
                .unwrap()
                .save(PathBuf::from(path).with_extension(format!("{scale}.png")))
                .unwrap();
        }
        h.get_by_label(&format!("Remove breakpoint: {}:3", source.display()))
            .click();
        h.run();
        assert!(h.state().service.debugger.breakpoints.is_empty());
    }
}
