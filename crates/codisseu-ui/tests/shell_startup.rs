use codisseu_core::AppService;
use codisseu_ui::CodisseuApp;
use egui_kittest::{Harness, kittest::Queryable};

#[test]
fn explicit_problems_startup_opens_panel_despite_the_quiet_default_shell() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = CodisseuApp::new(AppService::new(dir.path().join("session.json")));
    app.configure_startup(None, None, false, false, true);
    let mut ui = Harness::builder()
        .with_size([1280., 800.])
        .build_eframe(|_| app);
    ui.run();
    assert!(ui.query_by_label("Problemas (0)").is_some());
    assert!(ui.query_by_label("Ocultar painel inferior").is_some());
}
