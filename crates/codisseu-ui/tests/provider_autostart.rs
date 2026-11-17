use codisseu_core::{AppService, Command, Language, ProviderConfig};
use codisseu_ui::CodisseuApp;
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn settle(ui: &mut Harness<'_, CodisseuApp>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    for _ in 0..3 {
        ui.step();
    }
    while ui.state().service.is_busy() {
        assert!(Instant::now() < deadline, "{}", ui.state().service.status);
        thread::sleep(Duration::from_millis(1));
        ui.step();
    }
    ui.run();
    ui.check_accessibility();
}
fn configured(root: &Path, project: &Path) -> AppService {
    let mut service = AppService::new(root.join("session.json"));
    service
        .dispatch(Command::OpenWorkspace(project.into()))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    service
        .dispatch(Command::ConfigureProviders(vec![ProviderConfig {
            language: Language::TypeScript,
            program: root.join("missing-provider"),
            args: vec!["--stdio".into()],
        }]))
        .unwrap();
    service
}

#[test]
fn trust_once_with_widgets_and_restore_without_opening_provider_dialog() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let profile = root.path().join("profile/policy.json");
    fs::create_dir(&project).unwrap();
    let mut app = CodisseuApp::new(configured(root.path(), &project));
    app.configure_provider_autostart(profile.clone(), None);
    let mut ui = Harness::builder()
        .with_size([1280.0, 800.0])
        .with_max_steps(30)
        .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
    settle(&mut ui);
    assert!(ui.query_by_label("Trust Project").is_some());
    let trust = ui
        .ctx
        .memory(|m| m.area_rect(egui::Id::unique("codisseu.providers.trust")))
        .unwrap();
    let viewport = ui.ctx.content_rect();
    assert!(
        trust.center().distance(viewport.center()) <= 1.0,
        "trust={trust:?}, viewport={viewport:?}"
    );
    assert!(ui.query_by_label("Iniciar providers").is_none());
    assert!(ui.state().service.language.statuses.is_empty());
    ui.get_by_label("Trust Project").click();
    settle(&mut ui);
    assert!(ui.state().service.provider_startup().trusted);
    assert!(profile.exists());
    assert!(ui.query_by_label("Trust Project").is_none());
    assert!(
        ui.state().service.language.last_error.is_some(),
        "missing tool must be an actual failure"
    );
    drop(ui);

    let mut app = CodisseuApp::new(configured(root.path(), &project));
    app.configure_provider_autostart(profile, None);
    let mut ui = Harness::builder()
        .with_size([1280.0, 800.0])
        .with_max_steps(30)
        .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
    settle(&mut ui);
    assert!(ui.state().service.provider_startup().trusted);
    assert!(ui.query_by_label("Trust Project").is_none());
    assert!(ui.query_by_label("Iniciar providers").is_none());
    assert!(
        ui.state().service.language.last_error.is_some(),
        "automatically attempted the remembered tool"
    );
    ui.get_by_label("Configurar providers").click();
    settle(&mut ui);
    ui.get_by_label("Iniciar automaticamente em projetos confiáveis")
        .click();
    settle(&mut ui);
    assert!(!ui.state().service.provider_startup().enabled);
}

#[test]
fn continue_without_analysis_does_not_store_trust_or_execute_tools() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let profile = root.path().join("profile/policy.json");
    fs::create_dir(&project).unwrap();
    let mut app = CodisseuApp::new(configured(root.path(), &project));
    app.configure_provider_autostart(profile.clone(), None);
    let mut ui = Harness::builder()
        .with_size([1280.0, 800.0])
        .with_max_steps(30)
        .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
    settle(&mut ui);
    ui.get_by_label("Preview in Safe Mode").click();
    settle(&mut ui);
    assert!(ui.query_by_label("Trust Project").is_none());
    assert!(ui.state().service.provider_startup().suspended);
    assert!(!ui.state().service.provider_startup().trusted);
    assert!(!ui.state().service.providers_running());
    assert_eq!(
        ui.state().service.status,
        "Providers parados; análise pausada até iniciar ou reabrir"
    );
    assert!(ui.state().service.language.last_error.is_none());
    assert!(!profile.exists());
}

#[test]
fn trust_click_after_switching_project_authorizes_new_folder_without_an_open_file() {
    for scale in [1., 1.25] {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("previous-project");
        let second = root.path().join("conflicts-demo");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        let mut service = configured(root.path(), &first);
        let profile = root.path().join("policy/provider-policy.json");
        service
            .dispatch(Command::EnableProviderAutostart {
                profile: profile.clone(),
                fallback_config: None,
            })
            .unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        while service.is_busy() {
            service.poll();
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(2));
        }
        service.dispatch(Command::StartProviders).unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        while service.is_busy() {
            service.poll();
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(service.active_language_project(), Some(first.as_path()));
        service
            .dispatch(Command::OpenWorkspace(second.clone()))
            .unwrap();
        while service.is_busy() {
            service.poll();
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(2));
        }
        assert!(service.active_document().is_none());
        let app = CodisseuApp::new(service);
        let mut ui = Harness::builder()
            .with_size([1280., 800.])
            .with_pixels_per_point(scale)
            .with_max_steps(40)
            .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
        settle(&mut ui);
        assert!(!ui.state().service.provider_startup().trusted);
        ui.get_by_label("Trust Project").click();
        settle(&mut ui);
        assert!(
            ui.state().service.provider_startup().trusted,
            "{:?}",
            ui.state().service.last_error
        );
        assert!(ui.query_by_label("Trust Project").is_none());
        assert_eq!(
            ui.state().service.active_language_project(),
            Some(second.as_path())
        );
        assert!(profile.exists());
    }
}
