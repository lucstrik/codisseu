use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::time::{Duration, Instant};
fn harness(root: &std::path::Path) -> Harness<'static, CodisseuApp> {
    let mut app = CodisseuApp::new(AppService::new(root.join("session.json")));
    app.ai_chat.catalog_requested = true;
    Harness::builder()
        .with_size([1280., 800.])
        .with_pixels_per_point(
            std::env::var("CODISSEU_AI_SCALE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1.),
        )
        .with_max_steps(30)
        .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app)
}
fn settle(h: &mut Harness<'_, CodisseuApp>) {
    h.step();
    let start = Instant::now();
    while h.state().service.is_busy()
        || h.state().service.ai.is_running()
        || h.state().ai_chat.mention_search.pending()
    {
        assert!(start.elapsed() < Duration::from_secs(15));
        std::thread::sleep(Duration::from_millis(2));
        h.step();
    }
    h.run();
}
#[test]
fn rail_composer_configuration_draft_and_database_are_native_widgets() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(dir.path());
    h.get_by_label("AI Chat").click();
    settle(&mut h);
    assert!(h.state().ai_chat.open);
    assert!(!h.state().show_database);
    assert!(!h.state().service.ai.is_running());
    h.get_by_role_and_label(Role::MultilineTextInput, "Mensagem AI")
        .click();
    h.event(egui::Event::Text("ação 🌧".into()));
    settle(&mut h);
    assert_eq!(h.state().ai_chat.draft, "ação 🌧");
    assert!(h.get_by_label("Enviar").accesskit_node().is_disabled());
    h.get_by_label("Configurar AI Chat").click();
    settle(&mut h);
    assert!(
        h.query_by_role_and_label(Role::TextInput, "Executável Codex")
            .is_some()
    );
    let fields: Vec<_> = ["Executável Codex", "Executável Claude", "Modelo AI"]
        .map(|name| h.get_by_role_and_label(Role::TextInput, name).rect())
        .into();
    for field in &fields {
        assert!((field.height() - h.state().tokens.control_height).abs() < 1.);
        assert!((field.min.x - fields[0].min.x).abs() < 1.);
        assert!((field.max.x - fields[0].max.x).abs() < 1.);
    }
    assert!(fields.windows(2).all(|pair| pair[0].max.y < pair[1].min.y));
    if let Some(root) = std::env::var_os("CODISSEU_AI_CAPTURES") {
        let root = PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        h.ctx.request_repaint();
        h.run_steps(2);
        h.run();
        h.render().unwrap().save(root.join("settings.png")).unwrap();
    }
    h.set_size([900., 480.].into());
    h.run();
    assert!(h.get_by_label("Concluído").rect().max.y < 480.);
    assert!(
        h.get_by_label("Fechar configurações do AI Chat")
            .rect()
            .min
            .y
            > 0.
    );
    if let Some(root) = std::env::var_os("CODISSEU_AI_CAPTURES") {
        h.ctx.request_repaint();
        h.run_steps(2);
        h.run();
        h.render()
            .unwrap()
            .save(PathBuf::from(root).join("settings-compact.png"))
            .unwrap();
    }
    h.get_by_role_and_label(Role::TextInput, "Modelo AI")
        .click();
    h.event(egui::Event::Text("modelo-teste".into()));
    h.run();
    h.key_press(Key::Escape);
    settle(&mut h);
    assert!(!h.state().ai_chat.settings);
    assert_eq!(h.state().ai_chat.model, "modelo-teste");
    assert!(!h.state().service.ai.is_running());
    h.get_by_label("Configurar AI Chat").click();
    settle(&mut h);
    h.get_by_label("Fechar configurações do AI Chat").click();
    settle(&mut h);
    assert!(!h.state().ai_chat.settings);
    h.get_by_label("Configurar AI Chat").click();
    settle(&mut h);
    h.get_by_label("Concluído").click();
    settle(&mut h);
    assert!(!h.state().ai_chat.settings);
    h.get_by_label("Ocultar AI Chat").click();
    settle(&mut h);
    assert!(!h.state().ai_chat.open);
    h.get_by_label("AI Chat").click();
    settle(&mut h);
    assert_eq!(h.state().ai_chat.draft, "ação 🌧");
    h.get_by_label("Alternar painel Database").click();
    settle(&mut h);
    assert!(!h.state().ai_chat.open);
    assert!(h.state().show_database);
}

#[test]
fn long_prompt_scrolls_inside_composer_and_keeps_controls_and_usage_visible() {
    for width in [980., 1280.] {
        let dir = tempfile::tempdir().unwrap();
        let mut h = harness(dir.path());
        h.set_size([width, 700.].into());
        h.run();
        h.get_by_label("AI Chat").click();
        settle(&mut h);
        h.state_mut().ai_chat.mode = Mode::FullAccess;
        h.state_mut().ai_chat.draft = (0..100)
            .map(|i| format!("Linha {i}: implemente a função com texto longo e Unicode 🦀.\n"))
            .collect();
        h.state_mut().ai_chat.focus = true;
        h.step();
        h.run();
        h.key_press_modifiers(Modifiers::CTRL, Key::End);
        h.run();
        let (viewport, offset) = h
            .ctx
            .data(|d| {
                d.get_temp::<(egui::Rect, egui::Vec2)>(egui::Id::unique("codisseu.ai.test.prompt"))
            })
            .unwrap();
        assert!((100. ..=180.1).contains(&viewport.height()), "{viewport:?}");
        assert!(
            offset.y > 0.,
            "cursor must reveal the end inside prompt: {offset:?}"
        );
        let panel = h
            .ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique("codisseu.ai.test.bounds")))
            .unwrap();
        for node in [
            h.get_by_label("Uso de tokens e limites"),
            h.get_by_role_and_label(Role::Label, "Full access · projeto aberto"),
            h.get_by_label("Adicionar contexto AI"),
        ] {
            let rect = node.rect();
            assert!(rect.min.y >= viewport.max.y, "{rect:?} {viewport:?}");
            assert!(rect.max.y <= panel.max.y - 4., "{rect:?} {panel:?}");
        }
        assert_eq!(h.state().ai_chat.draft.lines().count(), 100);
        if let Some(root) = std::env::var_os("CODISSEU_AI_CAPTURES") {
            let root = PathBuf::from(root);
            std::fs::create_dir_all(&root).unwrap();
            h.render()
                .unwrap()
                .save(root.join(format!("composer-expanded-{width}.png")))
                .unwrap();
        }
        h.get_by_label("Contexto do editor").click();
        h.run();
        let (collapsed, _) = h
            .ctx
            .data(|d| {
                d.get_temp::<(egui::Rect, egui::Vec2)>(egui::Id::unique("codisseu.ai.test.prompt"))
            })
            .unwrap();
        assert!(
            collapsed.height() <= 64.1,
            "blur must collapse: {collapsed:?}"
        );
        assert_eq!(h.state().ai_chat.draft.lines().count(), 100);
        if let Some(root) = std::env::var_os("CODISSEU_AI_CAPTURES") {
            let root = PathBuf::from(root);
            std::fs::create_dir_all(&root).unwrap();
            h.render()
                .unwrap()
                .save(root.join(format!("composer-collapsed-{width}.png")))
                .unwrap();
        }
    }
}

#[test]
fn slash_filters_keyboard_actions_escape_and_paths_do_not_execute() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(dir.path());
    h.get_by_label("AI Chat").click();
    settle(&mut h);
    h.get_by_label("Mensagem AI").click();
    h.event(egui::Event::Text("/".into()));
    h.run();
    assert!(h.query_by_label("Comandos do Codisseu").is_some());
    h.event(egui::Event::Text("per".into()));
    h.run();
    assert!(
        h.query_by_label("/permissions  ·  Escolher as permissões do agente")
            .is_some()
    );
    assert!(
        h.query_by_label("/new  ·  Nova conversa neste projeto")
            .is_none()
    );
    if let Some(root) = std::env::var_os("CODISSEU_AI_CAPTURES") {
        let root = PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        h.render()
            .unwrap()
            .save(root.join("slash-filtered.png"))
            .unwrap();
    }
    h.key_press(Key::Enter);
    h.run();
    assert!(h.state().ai_chat.draft.is_empty());
    assert!(h.query_by_label("Acesso total (Full access)").is_some());
    h.key_press(Key::Escape);
    h.run();
    h.get_by_label("Mensagem AI").click();
    h.event(egui::Event::Text("/set".into()));
    h.run();
    h.key_press(Key::Tab);
    h.run();
    assert!(h.state().ai_chat.settings);
    h.get_by_label("Concluído").click();
    h.run();
    h.event(egui::Event::Text("/mod".into()));
    h.run();
    h.key_press(Key::Escape);
    h.run();
    assert_eq!(h.state().ai_chat.draft, "/mod");
    assert!(h.query_by_label("Comandos do Codisseu").is_none());
    h.state_mut().ai_chat.draft = "Leia /tmp/arquivo".into();
    h.run();
    assert!(h.query_by_label("Comandos do Codisseu").is_none());
    assert!(!h.state().service.ai.is_running());
}

#[test]
fn tool_output_is_bounded_virtualized_copyable_and_keeps_footer_reachable() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(dir.path());
    h.get_by_label("AI Chat").click();
    settle(&mut h);
    let detail: String = (0..8_000)
        .map(|i| format!("linha {i}: ação 🦀\n"))
        .collect();
    h.state_mut().service.ai.entries = vec![ai::Entry {
        id: "large-tool".into(),
        role: "Ferramenta",
        text: "Comando · concluído".into(),
        detail: detail.clone(),
    }];
    h.run();
    assert!(h.query_by_label("Copiar detalhes completos").is_none());
    h.get_by_label("Comando · concluído").click();
    h.run();
    let (viewport, _) = h
        .ctx
        .data(|d| {
            d.get_temp::<(egui::Rect, egui::Vec2)>(egui::Id::unique((
                "codisseu.ai.test.tool",
                "large-tool",
            )))
        })
        .unwrap();
    assert!(viewport.height() <= 220.1, "{viewport:?}");
    let painted_lines = h.output().shapes.iter().filter(|shape| matches!(&shape.shape,egui::Shape::Text(text) if text.galley.text().starts_with("linha "))).count();
    assert!(
        painted_lines > 0 && painted_lines < 40,
        "output should be virtualized: {painted_lines}"
    );
    assert!(h.get_by_label("Adicionar contexto AI").rect().max.y < 800.);
    h.get_by_label("Copiar detalhes completos").click();
    h.step();
    assert!(h.output().platform_output.commands.iter().any(|command| matches!(command, egui::OutputCommand::CopyText(text) if text == &format!("Comando · concluído\n{detail}"))));
    let transcript_id = h
        .ctx
        .data(|d| d.get_temp::<egui::Id>(egui::Id::unique("codisseu.ai.test.scroll")))
        .unwrap();
    let outer_before = egui::scroll_area::State::load(&h.ctx, transcript_id)
        .unwrap()
        .clamped_offset();
    h.event(egui::Event::PointerMoved(viewport.center()));
    h.event(egui::Event::MouseWheel {
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0., -500.),
        modifiers: Modifiers::NONE,
    });
    h.run();
    let (_, inner_offset) = h
        .ctx
        .data(|d| {
            d.get_temp::<(egui::Rect, egui::Vec2)>(egui::Id::unique((
                "codisseu.ai.test.tool",
                "large-tool",
            )))
        })
        .unwrap();
    assert!(
        inner_offset.y > 100.,
        "inner scroll must move: {inner_offset:?}"
    );
    let outer_after = egui::scroll_area::State::load(&h.ctx, transcript_id)
        .unwrap()
        .clamped_offset();
    assert!(
        (outer_before.y - outer_after.y).abs() < 1.,
        "inner scroll moved transcript: {outer_before:?} -> {outer_after:?}"
    );
    if let Some(root) = std::env::var_os("CODISSEU_AI_CAPTURES") {
        let root = PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        h.render()
            .unwrap()
            .save(root.join("tool-expanded.png"))
            .unwrap();
    }
    h.get_by_label("Comando · concluído").click();
    h.run();
    assert!(h.query_by_label("Copiar detalhes completos").is_none());
}
#[test]
fn attach_uses_unsaved_unicode_selection_and_sends_only_explicit_context() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.sql");
    std::fs::write(&file, "SELECT 'disco';\n").unwrap();
    let mut h = harness(dir.path());
    // This scenario intentionally keeps text unsaved while exercising AI context.
    h.state_mut()
        .service
        .dispatch(Command::SetAutoSave {
            enabled: false,
            delay_ms: 1000,
        })
        .unwrap();
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(file.clone()))
        .unwrap();
    settle(&mut h);
    let id = h.state().service.active.unwrap();
    h.state_mut()
        .service
        .dispatch(Command::Edit {
            id,
            expected_version: 1,
            text: "SELECT 'ação 🌧';\n".into(),
        })
        .unwrap();
    h.state_mut()
        .service
        .dispatch(Command::SetSelection {
            id,
            expected_version: 2,
            selection: codisseu_core::Selection {
                anchor: 8,
                head: 14,
            },
        })
        .unwrap();
    h.get_by_label("AI Chat").click();
    settle(&mut h);
    h.get_by_label("Adicionar contexto AI").click();
    settle(&mut h);
    h.get_by_label("Anexar seleção ou arquivo ao AI Chat")
        .click();
    settle(&mut h);
    let (label, body) = h.state().ai_chat.context.as_ref().unwrap();
    assert!(label.contains("seleção") && label.contains("não salvo"));
    assert!(body.contains("ação 🌧"));
    assert!(!body.contains("disco"));
    assert_eq!(std::fs::read_to_string(file).unwrap(), "SELECT 'disco';\n");
    assert!(!h.state().service.ai.is_running());
    h.get_by_label("Remover contexto").click();
    settle(&mut h);
    assert!(h.state().ai_chat.context.is_none());
}
#[test]
fn composer_ctrl_enter_does_not_execute_active_sql_or_save_editor() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.sql");
    std::fs::write(&file, "SELECT 1;\n").unwrap();
    let mut h = harness(dir.path());
    h.state_mut().service.workspace = Some(dir.path().to_path_buf());
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(file.clone()))
        .unwrap();
    settle(&mut h);
    h.get_by_label("AI Chat").click();
    settle(&mut h);
    h.state_mut().ai_chat.codex = "/fixture/missing-cli".into();
    h.get_by_role_and_label(Role::MultilineTextInput, "Mensagem AI")
        .click();
    h.event(egui::Event::Text("resposta literal".into()));
    settle(&mut h);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    settle(&mut h);
    assert_eq!(h.state().service.ai.state, codisseu_core::ai::State::Failed);
    assert!(
        h.state()
            .service
            .ai
            .error
            .as_ref()
            .unwrap()
            .contains("iniciar Codex")
    );
    assert!(
        h.state().service.last_error.is_none(),
        "SQL shortcut escaped composer: {:?}",
        h.state().service.last_error
    );
    assert_eq!(std::fs::read_to_string(file).unwrap(), "SELECT 1;\n");
    assert!(
        h.state()
            .service
            .ai
            .entries
            .iter()
            .any(|e| e.text == "resposta literal")
    );
}
#[test]
fn response_fences_are_text_blocks_never_commands() {
    let blocks = blocks("Explicação\n```php\necho 'ação 🌧';\n```\nFim");
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[1], (true, "echo 'ação 🌧';\n".into()));
}

#[test]
fn title_composer_and_content_have_explicit_insets_at_two_widths() {
    for width in [900., 1280.] {
        let dir = tempfile::tempdir().unwrap();
        let mut app = CodisseuApp::new(AppService::new(dir.path().join("session.json")));
        app.ai_chat.catalog_requested = true;
        let mut h = Harness::builder()
            .with_size([width, 700.])
            .with_max_steps(30)
            .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
        h.get_by_label("AI Chat").click();
        settle(&mut h);
        let panel = h
            .ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique("codisseu.ai.test.bounds")))
            .unwrap();
        let title = h.get_by_role_and_label(Role::Label, "AI Chat").rect();
        let composer = h
            .get_by_role_and_label(Role::MultilineTextInput, "Mensagem AI")
            .rect();
        let welcome = h.get_by_label("Vamos construir juntos").rect();
        for (label, r) in [
            ("title", title),
            ("composer", composer),
            ("welcome", welcome),
        ] {
            assert!(
                r.min.x >= panel.min.x + 16. - 0.5,
                "{label}: {r:?} panel {panel:?}"
            );
            assert!(
                r.max.x <= panel.max.x - 15.,
                "{label}: {r:?} panel {panel:?}"
            );
        }
        assert!(composer.min.x >= title.min.x + 10.);
        assert!(composer.max.y < 700. - 40.);
    }
}

#[test]
fn mention_enter_attaches_project_file_without_sending_or_changing_draft_text() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let file = project.join("hello.php");
    std::fs::write(&file, "<?php // mention fixture").unwrap();
    let mut h = harness(dir.path());
    h.state_mut()
        .service
        .dispatch(Command::OpenWorkspace(project))
        .unwrap();
    settle(&mut h);
    h.get_by_label("AI Chat").click();
    settle(&mut h);
    h.get_by_label("Mensagem AI").click();
    h.event(egui::Event::Text("Revise @hello".into()));
    settle(&mut h);
    assert!(h.query_by_label("Arquivos do projeto").is_some());
    h.key_press(Key::Enter);
    settle(&mut h);
    assert!(h.query_by_label("Remover anexo hello.php").is_some());
    assert_eq!(h.state().ai_chat.draft, "Revise ");
    assert!(h.state().service.ai.entries.is_empty());
    assert!(!h.state().service.ai.is_running());
    h.key_press_modifiers(Modifiers::COMMAND, Key::V);
    h.event(egui::Event::PasteImage(std::sync::Arc::new(
        egui::ColorImage::filled([2, 2], egui::Color32::RED),
    )));
    settle(&mut h);
    assert_eq!(h.state().ai_chat.attachments.len(), 2);
    if let Some(output) = std::env::var_os("CODISSEU_AI_ATTACHMENT_CAPTURE_DIR") {
        let output = PathBuf::from(output);
        std::fs::create_dir_all(&output).unwrap();
        h.render()
            .unwrap()
            .save(output.join("before-remove.png"))
            .unwrap();
    }
    let remove = h.get_by_label("Remover anexo Imagem colada 1").rect();
    let prompt = h.get_by_label("Mensagem AI").rect();
    assert!(
        remove.min.x >= prompt.min.x && remove.max.x <= prompt.max.x + 6.,
        "Remove action must be inside composer viewport: {remove:?}, prompt {prompt:?}"
    );
    h.get_by_label("Remover anexo Imagem colada 1").click();
    settle(&mut h);
    assert_eq!(h.state().ai_chat.attachments.len(), 1);
    assert!(h.query_by_label("Remover anexo hello.php").is_some());
    assert_eq!(h.state().ai_chat.draft, "Revise ");
    assert!(!h.state().service.ai.is_running());
    if let Some(output) = std::env::var_os("CODISSEU_AI_ATTACHMENT_CAPTURE_DIR") {
        h.render()
            .unwrap()
            .save(PathBuf::from(output).join("after-remove.png"))
            .unwrap();
    }
}

#[test]
fn mention_finds_nested_file_without_expanding_explorer() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let nested = project.join("src/never-expanded/nested");
    std::fs::create_dir_all(&nested).unwrap();
    let file = nested.join("invoice.php");
    std::fs::write(&file, "<?php // mention worker").unwrap();
    let mut h = harness(dir.path());
    h.state_mut().send(Command::OpenWorkspace(project));
    settle(&mut h);
    assert!(
        !h.state()
            .service
            .entries
            .iter()
            .any(|entry| entry.path == file)
    );
    h.get_by_label("AI Chat").click();
    settle(&mut h);
    h.get_by_label("Mensagem AI").click();
    h.event(egui::Event::Text("Revise @invoice".into()));
    settle(&mut h);
    assert!(
        h.query_by_label("src/never-expanded/nested/invoice.php")
            .is_some()
    );
    h.key_press(Key::Enter);
    settle(&mut h);
    assert!(h.query_by_label("Remover anexo invoice.php").is_some());
    assert_eq!(h.state().ai_chat.draft, "Revise ");
    assert!(h.state().service.ai.entries.is_empty());
}

#[test]
fn runtime_model_effort_and_permissions_are_selectable_from_composer() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(dir.path());
    h.state_mut().service.ai.models = vec![ai::models::Model {
        id: "fixture-model".into(),
        name: "Fixture model".into(),
        default_effort: "high".into(),
        efforts: vec!["low".into(), "high".into()],
        ..Default::default()
    }];
    h.get_by_label("AI Chat").click();
    settle(&mut h);
    h.get_by_label("Modelo e raciocínio").click();
    settle(&mut h);
    h.get_by_label("Fixture model").click();
    settle(&mut h);
    assert_eq!(h.state().ai_chat.model, "fixture-model");
    assert_eq!(h.state().ai_chat.effort, "high");
    if h.query_by_label("low").is_none() {
        h.get_by_label("Modelo e raciocínio").click();
        settle(&mut h);
    }
    h.get_by_label("low").click();
    settle(&mut h);
    assert_eq!(h.state().ai_chat.effort, "low");
    h.key_press(Key::Escape);
    settle(&mut h);
    h.get_by_label("Permissões do agente").click();
    settle(&mut h);
    h.get_by_label("Somente leitura").click();
    settle(&mut h);
    assert_eq!(h.state().ai_chat.mode, Mode::Consult);
}

#[test]
fn full_access_and_usage_details_are_native_composer_controls() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = harness(dir.path());
    h.get_by_label("AI Chat").click();
    settle(&mut h);
    h.get_by_label("Permissões do agente").click();
    settle(&mut h);
    h.get_by_label("Acesso total (Full access)").click();
    settle(&mut h);
    h.key_press(Key::Escape);
    settle(&mut h);
    assert_eq!(h.state().ai_chat.mode, Mode::FullAccess);
    assert!(h.query_by_label("Full access · projeto aberto").is_some());
    h.get_by_label("Uso de tokens e limites").click();
    settle(&mut h);
    assert!(h.query_by_label("Uso da conversa").is_some());
    assert!(h.query_by_label("Atualizar limites").is_some());
}

#[test]
fn explicit_permission_click_updates_defaults_even_when_history_already_selected_it() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut h = harness(dir.path());
    h.state_mut()
        .service
        .dispatch(Command::OpenWorkspace(project))
        .unwrap();
    settle(&mut h);
    let full = Config {
        program: "/fixture/missing-cli".into(),
        mode: Mode::FullAccess,
        ..Default::default()
    };
    h.state_mut()
        .service
        .dispatch(Command::Ai(ai::Command::Configure(full.clone())))
        .unwrap();
    let historical = h.state().service.ai.active_chat_id().unwrap().to_owned();
    h.state_mut()
        .service
        .dispatch(Command::Ai(ai::Command::NewConversation))
        .unwrap();
    h.state_mut()
        .service
        .dispatch(Command::Ai(ai::Command::Configure(Config {
            mode: Mode::Agent,
            ..full
        })))
        .unwrap();
    h.state_mut()
        .service
        .dispatch(Command::Ai(ai::Command::SelectChat(historical)))
        .unwrap();
    assert_eq!(
        h.state()
            .service
            .ai
            .preferred_config(Provider::Codex)
            .unwrap()
            .mode,
        Mode::Agent
    );
    h.state_mut().ai_chat.open = true;
    h.state_mut().ai_chat.catalog_requested = true;
    h.run();
    assert_eq!(h.state().ai_chat.mode, Mode::FullAccess);
    h.get_by_label("Permissões do agente").click();
    h.run();
    h.get_by_label("Acesso total (Full access)").click();
    h.step();
    assert_eq!(
        h.state()
            .service
            .ai
            .preferred_config(Provider::Codex)
            .unwrap()
            .mode,
        Mode::FullAccess
    );
    h.key_press(Key::Escape);
    settle(&mut h);
    h.get_by_label("Modelo e raciocínio").click();
    h.run();
    h.get_by_label("Claude").click();
    settle(&mut h);
    assert_eq!(h.state().ai_chat.provider, Provider::Claude);
    assert_eq!(
        h.state().ai_chat.mode,
        Mode::Agent,
        "new provider cannot inherit Full access"
    );
    assert!(!h.state().service.ai.is_running());
}

#[test]
fn conversation_delete_first_click_arms_second_deletes_and_outside_cancels() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut h = harness(dir.path());
    h.state_mut()
        .service
        .dispatch(Command::OpenWorkspace(project))
        .unwrap();
    settle(&mut h);
    h.state_mut().ai_chat.catalog_requested = true;
    for name in ["Conversa anterior", "Conversa atual"] {
        h.state_mut()
            .service
            .dispatch(Command::Ai(ai::Command::NewConversation))
            .unwrap();
        h.state_mut()
            .service
            .dispatch(Command::Ai(ai::Command::Start {
                config: Config {
                    program: "/fixture/missing-cli".into(),
                    ..Default::default()
                },
                action: Action::Send(name.into()),
            }))
            .unwrap();
        settle(&mut h);
    }
    h.state_mut().ai_chat.catalog_requested = true;
    h.get_by_label("AI Chat").click();
    settle(&mut h);
    h.get_by_label("Conversas").click();
    settle(&mut h);
    h.get_by_label("Apagar conversa Conversa anterior").click();
    settle(&mut h);
    assert!(
        h.query_by_label("Confirmar exclusão Conversa anterior")
            .is_some()
    );
    assert!(h.query_by_label("Apagar conversa?").is_none());
    assert!(
        h.state()
            .service
            .ai
            .conversations()
            .iter()
            .any(|(_, t)| t == "Conversa anterior")
    );
    h.get_by_label("Mensagem AI").click();
    settle(&mut h);
    assert!(
        h.query_by_label("Confirmar exclusão Conversa anterior")
            .is_none()
    );
    h.get_by_label("Conversas").click();
    settle(&mut h);
    h.get_by_label("Apagar conversa Conversa anterior").click();
    settle(&mut h);
    assert!(
        h.state()
            .service
            .ai
            .conversations()
            .iter()
            .any(|(_, t)| t == "Conversa anterior")
    );
    h.get_by_label("Confirmar exclusão Conversa anterior")
        .click();
    settle(&mut h);
    assert!(
        !h.state()
            .service
            .ai
            .conversations()
            .iter()
            .any(|(_, t)| t == "Conversa anterior")
    );
    assert!(
        h.state()
            .service
            .ai
            .entries
            .iter()
            .any(|e| e.text == "Conversa atual")
    );
}

#[test]
fn conversation_search_has_compact_rows_adapts_to_panel_and_selects_with_keyboard() {
    for (width, panel_width) in [(900., 295.), (1280., 440.)] {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let mut h = harness(dir.path());
        h.set_size([width, 800.].into());
        h.state_mut()
            .service
            .dispatch(Command::SetShellLayout(codisseu_core::ShellLayout {
                right_width: Some(panel_width),
                ..Default::default()
            }))
            .unwrap();
        h.state_mut().send(Command::OpenWorkspace(project));
        settle(&mut h);
        for title in [
            "Revisar contexto 🦀",
            "Validar arquivos TypeScript",
            "Ajustar espaços na lista de conversas e manter títulos longos sem cortar controles",
        ] {
            h.state_mut().ai_command(ai::Command::NewConversation);
            h.state_mut().service.ai.entries.push(ai::Entry {
                id: "user-1".into(),
                role: "Você",
                text: title.into(),
                detail: String::new(),
            });
            h.state_mut().ai_flush_draft();
            // NewConversation checkpoints the preceding transcript.
        }
        h.state_mut().ai_command(ai::Command::NewConversation);
        h.state_mut().ai_chat.catalog_requested = true;
        h.get_by_label("AI Chat").click();
        settle(&mut h);
        assert!(h.query_by_label("Conversas recentes").is_some());
        assert!(h.query_by_label("Vamos construir juntos").is_none());
        let recent_header = h.get_by_label("Conversas recentes").rect();
        let content_bounds = h
            .ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique("codisseu.ai.test.bounds")))
            .unwrap();
        assert!(recent_header.top() - content_bounds.top() < 100.);
        let previous = h
            .state()
            .service
            .ai
            .conversations()
            .into_iter()
            .find(|(_, title)| title == "Revisar contexto 🦀")
            .unwrap()
            .0;
        h.state_mut().ai_command(ai::Command::SelectChat(previous));
        settle(&mut h);
        h.get_by_label("Conversas").click();
        settle(&mut h);
        let search = h.get_by_role_and_label(Role::TextInput, "Buscar conversas AI");
        assert!(search.is_focused());
        let row = h
            .get_by_role_and_label(Role::Button, "Validar arquivos TypeScript")
            .rect();
        assert!((row.height() - 29.).abs() < 0.5);
        let panel = h
            .ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique("codisseu.ai.test.bounds")))
            .unwrap();
        assert!(row.min.x > panel.min.x && row.max.x < panel.max.x);
        assert!(search.rect().width() < panel.width() - 16.);
        assert!(search.rect().width() > panel.width() - 90.);
        if let Some(root) = std::env::var_os("CODISSEU_AI_CAPTURES") {
            let root = PathBuf::from(root);
            std::fs::create_dir_all(&root).unwrap();
            h.run_steps(2);
            h.render()
                .unwrap()
                .save(root.join(format!("history-{width}.png")))
                .unwrap();
        }
        h.event(egui::Event::Text("typescript".into()));
        settle(&mut h);
        assert!(
            h.query_by_role_and_label(Role::Button, "Revisar contexto 🦀")
                .is_none()
        );
        h.key_press(Key::Enter);
        settle(&mut h);
        assert!(h.query_by_label("Buscar conversas AI").is_none());
        assert!(
            h.state()
                .service
                .ai
                .entries
                .iter()
                .any(|entry| entry.text == "Validar arquivos TypeScript")
        );
        h.get_by_label("Conversas").click();
        settle(&mut h);
        h.event(egui::Event::Text("zzzzfixture".into()));
        settle(&mut h);
        assert!(h.query_by_label("Nenhuma conversa encontrada").is_some());
        h.key_press(Key::Escape);
        settle(&mut h);
        assert!(h.query_by_label("Buscar conversas AI").is_none());
        assert!(h.query_by_label("Mensagem AI").is_some());
    }
}

#[test]
fn declared_root_switch_preserves_recent_draft_and_attachment_before_same_frame_action() {
    let dir = tempfile::tempdir().unwrap();
    let carrier = dir.path().join("carrier");
    let roots = [dir.path().join("first"), dir.path().join("second")];
    std::fs::create_dir(&carrier).unwrap();
    for root in &roots {
        std::fs::create_dir(root).unwrap();
        std::fs::write(root.join("note.txt"), root.to_string_lossy().as_bytes()).unwrap();
    }
    let file = carrier.join("team.code-workspace");
    std::fs::write(&file, serde_json::json!({"folders":roots.iter().map(|root| serde_json::json!({"path":root})).collect::<Vec<_>>()}).to_string()).unwrap();
    let mut h = harness(dir.path());
    h.state_mut().send(Command::OpenWorkspace(file));
    settle(&mut h);
    h.state_mut()
        .send(Command::OpenFile(roots[0].join("note.txt")));
    settle(&mut h);
    h.state_mut().ai_chat.catalog_requested = true;
    h.get_by_label("AI Chat").click();
    settle(&mut h);
    h.state_mut().ai_chat.draft = "rascunho recente da primeira 🦀".into();
    h.state_mut().ai_attach();
    let first_context = h.state().ai_chat.context.clone();
    h.state_mut()
        .send(Command::OpenFile(roots[1].join("note.txt")));
    settle(&mut h);
    h.state_mut().ai_chat.catalog_requested = true;
    assert_eq!(h.state().service.ai.workspace(), Some(roots[1].as_path()));
    assert!(h.state().ai_chat.draft.is_empty());
    assert!(h.state().ai_chat.context.is_none());
    h.state_mut().ai_chat.codex = "/fixture/missing-cli".into();
    let second_config = h.state().ai_config();
    h.state_mut()
        .ai_command(ai::Command::Configure(second_config));
    h.state_mut().ai_chat.draft = "rascunho da segunda pasta".into();
    h.state_mut().ai_flush_draft();
    let first_id = h
        .state()
        .service
        .documents
        .iter()
        .find(|doc| doc.path == roots[0].join("note.txt"))
        .unwrap()
        .id;
    // Direct activation bypasses the app send helper, as editor split input does.
    h.state_mut()
        .service
        .dispatch(Command::Activate { id: first_id })
        .unwrap();
    h.state_mut().ai_sync_workspace();
    assert_eq!(h.state().service.ai.workspace(), Some(roots[0].as_path()));
    assert_eq!(h.state().ai_chat.draft, "rascunho recente da primeira 🦀");
    assert_eq!(h.state().ai_chat.context, first_context);
    assert_eq!(
        h.state().service.ai.saved_draft(),
        Some("rascunho recente da primeira 🦀")
    );
    assert_ne!(h.state().service.ai.workspace(), Some(carrier.as_path()));
    let second_id = h
        .state()
        .service
        .documents
        .iter()
        .find(|doc| doc.path == roots[1].join("note.txt"))
        .unwrap()
        .id;
    h.state_mut()
        .service
        .dispatch(Command::Activate { id: second_id })
        .unwrap();
    // No frame or panel synchronization between activation and submission.
    h.state_mut().ai_send_message();
    settle(&mut h);
    assert_eq!(h.state().service.ai.workspace(), Some(roots[1].as_path()));
    assert_eq!(h.state().service.ai.config.program, "/fixture/missing-cli");
    assert!(
        h.state()
            .service
            .ai
            .entries
            .iter()
            .any(|entry| entry.text == "rascunho da segunda pasta")
    );
    assert!(
        !h.state()
            .service
            .ai
            .entries
            .iter()
            .any(|entry| entry.text.contains("primeira"))
    );
    h.state_mut()
        .service
        .dispatch(Command::Activate { id: first_id })
        .unwrap();
    h.state_mut().ai_sync_workspace();
    assert_eq!(h.state().ai_chat.draft, "rascunho recente da primeira 🦀");
    assert_eq!(h.state().ai_chat.context, first_context);
}

#[test]
fn root_completion_checkpoints_last_keystrokes_of_the_parked_composer() {
    let dir = tempfile::tempdir().unwrap();
    let roots = [dir.path().join("first"), dir.path().join("second")];
    for root in &roots {
        std::fs::create_dir(root).unwrap();
        std::fs::write(root.join("note.txt"), "fixture").unwrap();
    }
    let file = dir.path().join("team.code-workspace");
    std::fs::write(&file, serde_json::json!({"folders": roots.iter().map(|root| serde_json::json!({"path":root})).collect::<Vec<_>>()}).to_string()).unwrap();
    let mut h = harness(dir.path());
    h.state_mut().send(Command::OpenWorkspace(file));
    settle(&mut h);
    for root in &roots {
        h.state_mut().send(Command::OpenFile(root.join("note.txt")));
        settle(&mut h);
    }
    let ids: Vec<_> = roots
        .iter()
        .map(|root| {
            h.state()
                .service
                .documents
                .iter()
                .find(|doc| doc.path == root.join("note.txt"))
                .unwrap()
                .id
        })
        .collect();
    h.state_mut().send(Command::Activate { id: ids[0] });
    settle(&mut h);
    h.state_mut().ai_chat.draft = "checkpoint inicial".into();
    h.state_mut().ai_flush_draft();
    let owner = h.state().service.ai.active_chat_id().unwrap().to_owned();
    // The user types after the OpenFile checkpoint, just before its reply.
    h.state_mut().ai_chat.draft.push_str(" + última tecla 🦀");
    h.state_mut()
        .service
        .dispatch(Command::Activate { id: ids[1] })
        .unwrap();
    h.state_mut().service.poll();
    assert_eq!(h.state().service.ai.workspace(), Some(roots[1].as_path()));
    h.state_mut().ai_sync_editor(false);
    settle(&mut h);
    let deadline = Instant::now() + Duration::from_secs(5);
    while h.state().service.ai.profile_pending() {
        assert!(Instant::now() < deadline);
        h.state_mut().service.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    let profile: ai::profiles::Profile =
        serde_json::from_slice(&std::fs::read(dir.path().join("session.ai.json")).unwrap())
            .unwrap();
    let parked = profile.workspaces[&roots[0]]
        .chats
        .iter()
        .find(|chat| chat.id == owner)
        .unwrap();
    assert_eq!(parked.draft, "checkpoint inicial + última tecla 🦀");
    assert!(h.state().ai_chat.draft.is_empty());
}

#[test]
fn usage_footer_keeps_status_and_counter_inside_content_insets_at_both_widths() {
    for (width, panel_width) in [(900., 295.), (1280., 440.)] {
        let dir = tempfile::tempdir().unwrap();
        let mut h = harness(dir.path());
        h.state_mut()
            .service
            .dispatch(Command::SetShellLayout(codisseu_core::ShellLayout {
                right_width: Some(panel_width),
                ..Default::default()
            }))
            .unwrap();
        h.set_size([width, 700.].into());
        h.run();
        h.get_by_label("AI Chat").click();
        settle(&mut h);
        let b = ai::usage::Breakdown {
            total_tokens: 8421,
            input_tokens: 8000,
            cached_input_tokens: 4000,
            output_tokens: 421,
            reasoning_output_tokens: 100,
        };
        h.state_mut().service.ai.usage = Some(ai::usage::Tokens {
            total: b.clone(),
            last: b,
            model_context_window: Some(100000),
        });
        h.state_mut().service.ai.limits = Some(ai::usage::Limits {
            buckets: vec![ai::usage::Bucket {
                id: "codex".into(),
                name: "codex".into(),
                primary: Some(ai::usage::Window {
                    used_percent: 28.,
                    duration_mins: Some(300),
                    resets_at: Some(ai::usage::now() + 500),
                }),
                secondary: None,
            }],
            observed_at: ai::usage::now(),
        });
        h.state_mut().ai_chat.mode = Mode::Agent;
        settle(&mut h);
        let panel = h
            .ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique("codisseu.ai.test.bounds")))
            .unwrap();
        let status = h
            .get_by_role_and_label(Role::Label, "Agente · projeto aberto")
            .rect();
        let counter = h.get_by_label("Uso de tokens e limites").rect();
        let composer = h
            .ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique("codisseu.ai.test.composer")))
            .unwrap();
        let painted = h
            .output()
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape
                    && text.galley.text() == "Agente · projeto aberto"
                {
                    Some(text.pos)
                } else {
                    None
                }
            })
            .expect("status text must be painted");
        assert!(
            (painted.x - composer.min.x).abs() <= 0.5,
            "status text {painted:?}, composer {composer:?}"
        );
        assert!(status.min.x >= panel.min.x + 15.5);
        assert!(status.max.x < counter.min.x);
        assert!(counter.max.x <= panel.max.x - 15.5, "{counter:?} {panel:?}");
        assert!((status.center().y - counter.center().y).abs() < 2.);
        let bottom_gap = panel.max.y - status.max.y.max(counter.max.y);
        assert!(
            (3.5..=4.5).contains(&bottom_gap),
            "bottom gap: {bottom_gap}, {panel:?}, {counter:?}"
        );
        if let Some(root) = std::env::var_os("CODISSEU_AI_CAPTURES") {
            let root = PathBuf::from(root);
            std::fs::create_dir_all(&root).unwrap();
            h.ctx.request_repaint();
            h.run_steps(2);
            h.run();
            h.render()
                .unwrap()
                .save(root.join(format!("footer-{width}.png")))
                .unwrap();
            std::fs::write(
                root.join(format!("geometry-{width}.txt")),
                format!(
                    "panel={panel:?}\nstatus={status:?}\ncounter={counter:?}\nbottom_gap={}\n",
                    panel.max.y - status.max.y.max(counter.max.y)
                ),
            )
            .unwrap();
        }
    }
}

#[cfg(unix)]
fn parallel_fixture(root: &std::path::Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join("ui-agent");
    std::fs::write(&path,r#"#!/usr/bin/python3
import json,sys,os,time
from pathlib import Path
def send(value): print(json.dumps(value),flush=True)
for line in sys.stdin:
 value=json.loads(line); method=value.get('method'); id=value.get('id')
 if method=='initialize': send({'id':id,'result':{}})
 elif method in ['thread/start','thread/resume']: send({'id':id,'result':{'thread':{'id':value.get('params',{}).get('threadId','thread-'+str(os.getpid()))}}})
 elif method in ['account/rateLimits/read','thread/delete']: send({'id':id,'result':{}})
 elif method=='account/read': send({'id':id,'result':{'account':None}})
 elif method=='model/list': send({'id':id,'result':{'data':[],'nextCursor':None}})
 elif method=='turn/start':
  name=value['params']['input'][0]['text'].splitlines()[0]
  send({'id':id,'result':{'turn':{'id':'turn-same','status':'inProgress'}}})
  send({'method':'item/agentMessage/delta','params':{'itemId':'same-answer','delta':'Início '+name}})
  if name=='A':
   send({'id':'same-approval','method':'item/commandExecution/requestApproval','params':{'itemId':'same-tool','command':'fixture','reason':'A only'}})
   continue
  while not Path('release-'+name).exists(): time.sleep(.01)
  send({'method':'item/completed','params':{'item':{'id':'same-answer','type':'agentMessage','text':'Resposta '+name}}})
  send({'method':'turn/completed','params':{'turn':{'id':'turn-same','status':'completed'}}})
"#).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

#[cfg(unix)]
#[track_caller]
fn while_running(h: &mut Harness<'_, CodisseuApp>, condition: impl Fn(&CodisseuApp) -> bool) {
    let start = Instant::now();
    loop {
        h.step();
        if condition(h.state()) {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "AI: {:?} {:?}; entries {:?}; cli {}",
            h.state().service.ai.state,
            h.state().service.ai.error,
            h.state()
                .service
                .ai
                .entries
                .iter()
                .map(|e| e.text.as_str())
                .collect::<Vec<_>>(),
            h.state().service.ai.config.program,
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(unix)]
#[test]
fn parallel_chats_keep_drafts_context_images_config_and_cancel_selected_only() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let program = parallel_fixture(dir.path());
    let mut h = harness(dir.path());
    h.state_mut()
        .service
        .dispatch(Command::OpenWorkspace(project.clone()))
        .unwrap();
    settle(&mut h);
    h.state_mut().ai_chat.catalog_requested = true;
    h.get_by_label("AI Chat").click();
    settle(&mut h);
    h.state_mut().ai_chat.codex = program.display().to_string();
    h.state_mut().ai_chat.model = "model-a".into();
    h.state_mut().ai_chat.mode = Mode::Agent;
    h.state_mut().ai_chat.draft = "A".into();
    h.state_mut().ai_chat.catalog_requested = true;
    h.state_mut().ai_chat.quota_requested_at = ai::usage::now();
    settle(&mut h);
    assert!(h.state().ai_chat.open, "panel closed");
    assert!(
        !h.state().service.ai.is_running(),
        "unexpected metadata process: {:?} {:?}",
        h.state().service.ai.state,
        h.state().service.ai.notice
    );
    h.get_by_label("Enviar").click();
    while_running(&mut h, |app| !app.service.ai.approvals.is_empty());
    let a = h.state().service.ai.active_chat_id().unwrap().to_owned();
    h.state_mut().ai_chat.draft = "Rascunho A 🦀".into();
    h.state_mut().ai_chat.context = Some(("Contexto A".into(), "conteúdo A".into()));
    h.state_mut().ai_chat.ide_context = false;
    h.state_mut()
        .ai_chat
        .attachments
        .add_image(std::sync::Arc::new(egui::ColorImage::filled(
            [2, 2],
            egui::Color32::RED,
        )));
    h.step();
    let scroll_a = h
        .ctx
        .data(|data| data.get_temp::<egui::Id>(egui::Id::unique("codisseu.ai.test.scroll")))
        .unwrap();
    let mut text_a =
        egui::text_edit::TextEditState::load(&h.ctx, egui::Id::unique("codisseu.ai.composer"))
            .unwrap();
    text_a
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(2),
            egui::text::CCursor::new(5),
        )));
    text_a.store(&h.ctx, egui::Id::unique("codisseu.ai.composer"));
    assert!(
        !h.get_by_label("Nova conversa AI")
            .accesskit_node()
            .is_disabled()
    );
    assert!(!h.get_by_label("Conversas").accesskit_node().is_disabled());
    h.get_by_label("Nova conversa AI").click();
    h.step();
    assert!(h.state().ai_chat.draft.is_empty());
    assert!(h.state().ai_chat.context.is_none());
    assert!(h.state().ai_chat.attachments.is_empty());
    assert!(h.state().ai_chat.ide_context);
    let scroll_b = h
        .ctx
        .data(|data| data.get_temp::<egui::Id>(egui::Id::unique("codisseu.ai.test.scroll")))
        .unwrap();
    assert_ne!(scroll_a, scroll_b);
    assert!(!h.state().service.ai.is_running());
    assert_eq!(h.state().service.ai.running_count(), 1);
    assert!(h.query_by_label("1 em segundo plano").is_some());
    h.state_mut().ai_chat.model = "model-b".into();
    h.state_mut().ai_chat.mode = Mode::FullAccess;
    h.state_mut().ai_chat.draft = "B".into();
    settle(&mut h);
    h.get_by_label("Enviar").click();
    while_running(&mut h, |app| {
        app.service
            .ai
            .entries
            .iter()
            .any(|entry| entry.text == "Início B")
    });
    let b = h.state().service.ai.active_chat_id().unwrap().to_owned();
    assert_ne!(a, b);
    assert_eq!(h.state().service.ai.running_count(), 2);
    assert!(h.state().service.ai.approvals.is_empty());
    h.state_mut().ai_chat.draft = "Rascunho B".into();
    h.state_mut().ai_chat.context = Some(("Contexto B".into(), "conteúdo B".into()));
    h.state_mut()
        .ai_chat
        .attachments
        .add_project_file(project.join("b.txt"));
    h.step();
    h.get_by_label("Conversas").click();
    h.step();
    h.step();
    assert_eq!(
        h.get_by_role_and_label(Role::Button, "A")
            .accesskit_node()
            .description()
            .as_deref(),
        Some("Aguardando aprovação"),
        "labels {:?}; conversations {:?}; approval {}",
        h.query_all_by_role(Role::Label)
            .map(|node| node.accesskit_node().value())
            .collect::<Vec<_>>(),
        h.state().service.ai.conversations(),
        h.state().service.ai.chat_approval_count(&a),
    );
    assert_eq!(
        h.get_by_role_and_label(Role::Button, "B")
            .accesskit_node()
            .description()
            .as_deref(),
        Some("Trabalhando…")
    );
    assert!(
        h.get_by_label("Apagar conversa A")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        h.get_by_label("Apagar conversa B")
            .accesskit_node()
            .is_disabled()
    );
    h.get_by_label("A").click();
    h.step();
    assert_eq!(h.state().ai_chat.draft, "Rascunho A 🦀");
    assert_eq!(h.state().ai_chat.context.as_ref().unwrap().0, "Contexto A");
    assert_eq!(h.state().ai_chat.attachments.len(), 1);
    assert!(!h.state().ai_chat.ide_context);
    assert_eq!(h.state().ai_chat.model, "model-a");
    assert_eq!(h.state().ai_chat.mode, Mode::Agent);
    assert_eq!(
        h.ctx
            .data(|data| data.get_temp::<egui::Id>(egui::Id::unique("codisseu.ai.test.scroll")))
            .unwrap(),
        scroll_a
    );
    let restored =
        egui::text_edit::TextEditState::load(&h.ctx, egui::Id::unique("codisseu.ai.composer"))
            .unwrap()
            .cursor
            .char_range()
            .unwrap();
    assert_eq!(
        (restored.primary.index.0, restored.secondary.index.0),
        (5, 5)
    );
    assert_eq!(h.state().service.ai.approvals.len(), 1);
    h.get_by_label("Interromper").click();
    while_running(&mut h, |app| !app.service.ai.is_running());
    assert_eq!(h.state().service.ai.running_count(), 1);
    assert_eq!(
        h.state().service.ai.chat_status(&a),
        Some(ai::State::Cancelled)
    );
    assert_eq!(
        h.state().service.ai.chat_status(&b),
        Some(ai::State::Running)
    );
    h.get_by_label("Conversas").click();
    h.step();
    h.step();
    h.get_by_label("B").click();
    h.step();
    assert_eq!(h.state().ai_chat.draft, "Rascunho B");
    assert_eq!(h.state().ai_chat.context.as_ref().unwrap().0, "Contexto B");
    assert_eq!(h.state().ai_chat.attachments.len(), 1);
    assert_eq!(h.state().ai_chat.model, "model-b");
    assert_eq!(h.state().ai_chat.mode, Mode::FullAccess);
    std::fs::write(project.join("release-B"), "").unwrap();
    while_running(&mut h, |app| !app.service.ai.any_running());
    assert_eq!(h.state().service.ai.state, ai::State::Completed);
    assert!(
        h.state()
            .service
            .ai
            .entries
            .iter()
            .any(|entry| entry.text == "Resposta B")
    );
    assert!(
        !h.state()
            .service
            .ai
            .entries
            .iter()
            .any(|entry| entry.text.contains("Início A"))
    );
    h.get_by_label("Conversas").click();
    h.step();
    h.step();
    assert_eq!(
        h.get_by_role_and_label(Role::Button, "B")
            .accesskit_node()
            .description()
            .as_deref(),
        Some("Concluído")
    );
}

#[test]
fn workspace_navigation_restores_unsent_state_and_chat_deletion_releases_images() {
    let dir = tempfile::tempdir().unwrap();
    let projects = [dir.path().join("one"), dir.path().join("two")];
    for project in &projects {
        std::fs::create_dir(project).unwrap();
    }
    let mut h = harness(dir.path());
    h.state_mut()
        .service
        .dispatch(Command::OpenWorkspace(projects[0].clone()))
        .unwrap();
    settle(&mut h);
    h.get_by_label("AI Chat").click();
    settle(&mut h);
    h.state_mut().ai_command(ai::Command::NewConversation);
    settle(&mut h);
    let a = h.state().service.ai.active_chat_id().unwrap().to_owned();
    h.state_mut().ai_chat.draft = "Rascunho projeto um".into();
    h.state_mut()
        .ai_chat
        .attachments
        .add_image(std::sync::Arc::new(egui::ColorImage::filled(
            [2, 2],
            egui::Color32::RED,
        )));
    h.state_mut()
        .service
        .dispatch(Command::OpenWorkspace(projects[1].clone()))
        .unwrap();
    settle(&mut h);
    assert!(h.state().ai_chat.draft.is_empty());
    assert!(h.state().ai_chat.attachments.is_empty());
    h.state_mut().ai_chat.draft = "Rascunho projeto dois".into();
    h.state_mut()
        .service
        .dispatch(Command::OpenWorkspace(projects[0].clone()))
        .unwrap();
    settle(&mut h);
    assert_eq!(h.state().ai_chat.draft, "Rascunho projeto um");
    assert_eq!(h.state().ai_chat.attachments.len(), 1);
    h.state_mut().ai_command(ai::Command::DeleteChat(a.clone()));
    settle(&mut h);
    assert!(h.state().ai_chat.attachments.is_empty());
    assert!(
        !h.state()
            .ai_chat
            .editors
            .keys()
            .any(|key| key.workspace.as_ref() == Some(&projects[0])
                && key.chat.as_deref() == Some(&a))
    );
    h.state_mut()
        .service
        .dispatch(Command::OpenWorkspace(projects[1].clone()))
        .unwrap();
    settle(&mut h);
    assert_eq!(h.state().ai_chat.draft, "Rascunho projeto dois");
}

#[test]
fn text_draft_checkpoint_survives_restart_and_navigation_flushes_before_project_switch() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let other = dir.path().join("other");
    std::fs::create_dir(&project).unwrap();
    std::fs::create_dir(&other).unwrap();
    let mut h = harness(dir.path());
    h.state_mut().send(Command::OpenWorkspace(project.clone()));
    settle(&mut h);
    h.get_by_label("AI Chat").click();
    settle(&mut h);
    h.state_mut().ai_chat.draft = "Rascunho persistido 🦀".into();
    h.state_mut()
        .ai_chat
        .attachments
        .add_image(std::sync::Arc::new(egui::ColorImage::filled(
            [2, 2],
            egui::Color32::RED,
        )));
    h.step();
    h.state_mut().ai_chat.draft_edited_at = Some(Instant::now() - Duration::from_millis(751));
    h.step();
    settle(&mut h);
    assert_eq!(
        h.state().service.ai.saved_draft(),
        Some("Rascunho persistido 🦀")
    );
    let original = h.state().service.ai.active_chat_id().unwrap().to_owned();
    drop(h);
    let mut h = harness(dir.path());
    h.state_mut().send(Command::OpenWorkspace(project.clone()));
    settle(&mut h);
    h.get_by_label("AI Chat").click();
    settle(&mut h);
    assert_eq!(h.state().ai_chat.draft, "Rascunho persistido 🦀");
    assert!(h.state().ai_chat.attachments.is_empty());
    assert_eq!(
        h.state().service.ai.active_chat_id(),
        Some(original.as_str())
    );
    h.state_mut().ai_chat.draft = "Último texto antes da troca".into();
    h.state_mut().send(Command::OpenWorkspace(other));
    settle(&mut h);
    assert!(h.state().ai_chat.draft.is_empty());
    h.state_mut().send(Command::OpenWorkspace(project));
    settle(&mut h);
    assert_eq!(
        h.state().service.ai.saved_draft(),
        Some("Último texto antes da troca")
    );
    assert_eq!(h.state().ai_chat.draft, "Último texto antes da troca");
    h.state_mut().ai_chat.draft.clear();
    h.state_mut().ai_flush_draft();
    settle(&mut h);
    assert_eq!(h.state().service.ai.saved_draft(), Some(""));
}

#[test]
fn programmatic_chat_switch_preserves_text_selection_without_cross_chat_undo() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut h = harness(dir.path());
    h.state_mut().send(Command::OpenWorkspace(project));
    settle(&mut h);
    h.state_mut().ai_command(ai::Command::NewConversation);
    settle(&mut h);
    let a = h.state().service.ai.active_chat_id().unwrap().to_owned();
    h.state_mut().ai_chat.draft = "Rascunho A".into();
    let id = egui::Id::unique("codisseu.ai.composer");
    let range =
        egui::text::CCursorRange::two(egui::text::CCursor::new(2), egui::text::CCursor::new(5));
    let mut state = egui::text_edit::TextEditState::load(&h.ctx, id).unwrap();
    state.cursor.set_char_range(Some(range));
    let mut undo = state.undoer();
    undo.add_undo(&(range, "Texto anterior de A".into()));
    state.set_undoer(undo);
    state.store(&h.ctx, id);
    h.state_mut().ai_command(ai::Command::NewConversation);
    assert!(h.state().ai_chat.draft.is_empty());
    let fresh = egui::text_edit::TextEditState::load(&h.ctx, id).unwrap();
    assert!(!fresh.undoer().has_undo(&(range, "Texto B".into())));
    h.state_mut().ai_chat.draft = "Rascunho B".into();
    h.state_mut().ai_command(ai::Command::SelectChat(a));
    assert_eq!(h.state().ai_chat.draft, "Rascunho A");
    let restored = egui::text_edit::TextEditState::load(&h.ctx, id).unwrap();
    assert_eq!(restored.cursor.char_range(), Some(range));
    assert!(restored.undoer().has_undo(&(range, "Rascunho A".into())));
}

#[test]
fn conversation_switch_reloads_missing_catalog_and_preserves_loaded_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut h = harness(dir.path());
    h.state_mut().send(Command::OpenWorkspace(project));
    settle(&mut h);
    h.state_mut().ai_chat.catalog_requested = true;
    h.state_mut().ai_command(ai::Command::NewConversation);
    assert!(!h.state().ai_chat.catalog_requested);
    h.state_mut().service.ai.models = vec![ai::models::Model {
        id: "fixture-model".into(),
        ..Default::default()
    }];
    h.state_mut().ai_chat.catalog_requested = false;
    h.state_mut().ai_command(ai::Command::NewConversation);
    assert!(h.state().ai_chat.catalog_requested);
    assert_eq!(h.state().service.ai.models[0].id, "fixture-model");
}
