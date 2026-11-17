//! Widget-level navigation regression and measured content bounds; native QA is separate.
use super::*;
use egui_kittest::{Harness, kittest::Queryable};

fn harness(size: [f32; 2]) -> (tempfile::TempDir, Harness<'static, CodisseuApp>) {
    let dir = tempfile::tempdir().unwrap();
    let mut app = CodisseuApp::new(AppService::new(dir.path().join("session.json")));
    app.show_activity = true;
    app.tool_tab = ToolTab::Activity;
    app.show_project = false;
    app.show_database = false;
    let mut h = Harness::builder()
        .with_size(size)
        .with_max_steps(30)
        .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
    h.run();
    (dir, h)
}

#[test]
fn navigation_survives_specialized_tools_and_problems_uses_the_rail() {
    for width in [800., 1280.] {
        let (_dir, mut h) = harness([width, 800.]);
        for selected in ["Debug", "Referências", "Services", "Git", "Atividade"] {
            h.get_by_role_and_label(egui::accesskit::Role::Tab, selected)
                .click();
            h.run();
            for label in [
                "Debug",
                "Referências",
                "Services",
                "Git",
                "Atividade",
                "Terminal",
            ] {
                let r = h
                    .get_by_role_and_label(egui::accesskit::Role::Tab, label)
                    .rect();
                assert!(
                    r.min.x >= 0. && r.max.x <= width,
                    "{label} after {selected}: {r:?}"
                );
            }
            assert!(h.query_by_label("Ocultar painel inferior").is_some());
        }
        h.get_by_label("Mostrar Problemas").click();
        h.run();
        assert!(h.state().tool_tab == ToolTab::Problems && h.state().show_activity);
        assert!(h.query_by_label("Problemas (0)").is_some());
        assert!(
            h.query_by_role_and_label(egui::accesskit::Role::Button, "Problemas (0)")
                .is_none()
        );
        h.get_by_label("Mostrar Problemas").click();
        h.run();
        assert!(!h.state().show_activity);
        h.get_by_label("Mostrar Problemas").click();
        h.run();
        assert!(h.state().show_activity);
    }
}

#[test]
fn database_header_has_one_settings_action_and_adjacent_add() {
    let (_dir, mut h) = harness([1280., 800.]);
    h.state_mut().show_database = true;
    h.run();
    let settings = h.get_by_label("Data Sources and Drivers").rect();
    let add = h.get_by_label("Adicionar datasource").rect();
    assert!(h.query_by_label("Data Sources…").is_none());
    assert_eq!(settings.center().y, add.center().y);
    assert!((settings.min.x - add.max.x).abs() <= 8.);
    h.get_by_label("Adicionar datasource").click();
    h.run();
    assert!(h.query_by_label("SQLite (arquivo)").is_some());
}

#[test]
fn terminal_viewport_and_activity_text_have_content_insets() {
    let (_dir, mut h) = harness([1280., 800.]);
    h.state_mut().tool_tab = ToolTab::Terminal;
    h.run();
    let terminal = h.get_by_label("Terminal integrado").rect();
    let tab = h
        .get_by_role_and_label(egui::accesskit::Role::Tab, "Terminal")
        .rect();
    assert!(
        terminal.min.y >= tab.max.y + 8.,
        "terminal {terminal:?}, tab {tab:?}"
    );
    assert!(terminal.min.x >= h.state().tokens.rail_width + 12.);
    assert!(terminal.max.x <= 1280. - h.state().tokens.rail_width - 12.);
    if let Some(out) = std::env::var_os("CODISSEU_POLISH_WIDGET_CAPTURES") {
        let out = PathBuf::from(out);
        std::fs::create_dir_all(&out).unwrap();
        h.render()
            .unwrap()
            .save(out.join("terminal-insets.png"))
            .unwrap();
    }
}

#[test]
fn providers_form_keeps_long_paths_inside_window_without_starting_processes() {
    for width in [800., 1280.] {
        let (_dir, mut h) = harness([width, 800.]);
        h.state_mut()
            .service
            .dispatch(Command::ConfigureProviders(vec![
                codisseu_core::ProviderConfig {
                    language: codisseu_core::Language::TypeScript,
                    program: PathBuf::from("/fixture/language-providers/typescript/node/bin/node"),
                    args: vec![
                        "/fixture/node_modules/typescript-language-server/lib/cli.mjs".into(),
                        "--stdio".into(),
                    ],
                },
            ]))
            .unwrap();
        h.state_mut().provider_dialog = true;
        h.run();
        let window = h
            .ctx
            .memory(|m| m.area_rect(egui::Id::unique("codisseu.providers.settings")))
            .unwrap();
        assert!(window.min.x >= 0. && window.max.x <= width, "{window:?}");
        for label in [
            "Configuração dos providers",
            "Arquivo de configuração",
            "Carregar configuração",
            "Iniciar providers",
            "Parar providers",
        ] {
            let r = h.get_by_label(label).rect();
            assert!(
                r.max.y <= window.max.y - 16.,
                "{label}: {r:?} window {window:?}"
            );
            assert!(
                r.min.x >= window.min.x + 16. && r.max.x <= window.max.x - 16.,
                "{label}: {r:?} window {window:?}"
            );
        }
        assert!(!h.state().service.providers_running());
        if width == 1280.
            && let Some(out) = std::env::var_os("CODISSEU_POLISH_WIDGET_CAPTURES")
        {
            let out = PathBuf::from(out);
            std::fs::create_dir_all(&out).unwrap();
            h.render().unwrap().save(out.join("providers.png")).unwrap();
        }
    }
}

#[test]
fn welcome_commands_align_and_references_have_insets_and_real_navigation_icons() {
    for (size, scale) in [([1280., 800.], 1.), ([900., 650.], 1.25)] {
        let (dir, mut h) = harness(size);
        h.ctx.set_pixels_per_point(scale);
        h.state_mut().show_activity = false;
        h.run();
        let names = [
            "Abrir projeto    Ctrl+Shift+O",
            "Abrir workspace…",
            "Abrir arquivo    Ctrl+O",
            "Pesquisar arquivos    Duplo Shift",
            "Configurações    Ctrl+Alt+S",
        ];
        let mut previous: Option<egui::Rect> = None;
        for name in names {
            let rect = h.get_by_label(name).rect();
            assert!(rect.height() >= 38.);
            if let Some(before) = previous {
                assert!((rect.left() - before.left()).abs() < 1.);
                assert!((rect.width() - before.width()).abs() < 1.);
                assert!(rect.top() - before.bottom() >= 7.5);
            }
            previous = Some(rect);
        }
        if let Some(out) = std::env::var_os("CODISSEU_POLISH_WIDGET_CAPTURES") {
            let out = PathBuf::from(out);
            std::fs::create_dir_all(&out).unwrap();
            h.render()
                .unwrap()
                .save(out.join(format!("welcome-{scale}.png")))
                .unwrap();
        }
        h.get_by_label("Abrir arquivo    Ctrl+O").click();
        h.run();
        assert!(matches!(h.state().path_dialog, Some(PathAction::File)));
        h.key_press(egui::Key::Escape);
        h.run();
        h.get_by_label("Pesquisar arquivos    Duplo Shift").click();
        h.run();
        assert!(h.state().project_search.open);
        h.key_press(egui::Key::Escape);
        h.run();
        h.get_by_label("Configurações    Ctrl+Alt+S").click();
        h.run();
        assert!(h.state().preferences_dialog);
        h.key_press(egui::Key::Escape);
        h.run();
        h.state_mut().show_activity = true;
        h.state_mut().tool_tab = ToolTab::References;
        h.run();
        let hint = h
            .get_by_label("Buscar referências no editor: Alt+Shift+7")
            .rect();
        let refresh = h.get_by_label("Atualizar referências").rect();
        let up = h.get_by_label("Uso anterior").rect();
        let down = h.get_by_label("Próximo uso").rect();
        assert!(hint.left() >= h.state().tokens.rail_width + 12.);
        assert!(hint.top() >= h.get_by_label("Find").rect().bottom() + 8.);
        assert_eq!(up.size(), egui::vec2(24., 24.));
        assert_eq!(down.size(), up.size());
        assert_eq!(up.center().x, refresh.center().x);
        assert_eq!(down.center().x, refresh.center().x);
        assert!(up.top() >= hint.bottom() + 4.);
        if let Some(out) = std::env::var_os("CODISSEU_POLISH_WIDGET_CAPTURES") {
            h.render()
                .unwrap()
                .save(PathBuf::from(out).join(format!("references-{scale}.png")))
                .unwrap();
        }
        drop(h);
        drop(dir);
    }
}
