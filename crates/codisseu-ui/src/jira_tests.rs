use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::{thread, time::Instant};
fn settle(h: &mut Harness<'_, CodisseuApp>) {
    let end = Instant::now() + Duration::from_secs(10);
    let mut idle = 0;
    while idle < 5 {
        h.step();
        idle = if h.state().service.is_busy() {
            0
        } else {
            idle + 1
        };
        assert!(Instant::now() < end, "{:?}", h.state().service.jira.error);
        thread::sleep(Duration::from_millis(5));
    }
}
fn shot(h: &mut Harness<'_, CodisseuApp>, name: &str) {
    if let Some(out) = std::env::var_os("CODISSEU_JIRA_CAPTURES") {
        std::fs::create_dir_all(&out).unwrap();
        h.render()
            .unwrap()
            .save(std::path::Path::new(&out).join(format!("{name}.png")))
            .unwrap()
    }
}
#[test]
fn jira_optional_rail_settings_and_reader_widgets() {
    for scale in [1., 1.25] {
        let d = tempfile::tempdir().unwrap();
        let service = AppService::new(d.path().join("session.json"));
        let mut h = Harness::builder()
            .with_size([1280., 850.])
            .with_pixels_per_point(scale)
            .with_max_steps(40)
            .build_ui_state(
                |ui, a: &mut CodisseuApp| a.show(ui),
                CodisseuApp::new(service),
            );
        settle(&mut h);
        assert!(
            h.query_all(egui_kittest::kittest::By::new().label("Jira · Minhas tarefas"))
                .next()
                .is_none()
        );
        let mut p = h.state().service.jira.profile.clone();
        p.enabled = true;
        h.state_mut().service.jira.set_profile(p).unwrap();
        settle(&mut h);
        h.get_by_label("Jira · Minhas tarefas").click();
        h.run();
        assert!(h.state().jira_ui.open);
        shot(&mut h, &format!("onboarding-{scale}"));
        h.get_by_label("Conectar ao Jira…").click();
        h.run();
        assert!(h.state().jira_ui.settings);
        assert_eq!(
            h.get_by_label("Guardar token no cofre do sistema")
                .accesskit_node()
                .toggled(),
            Some(egui::accesskit::Toggled::False)
        );
        h.get_by_label("Com o token salvo, o Jira conecta automaticamente ao abrir a IDE.");
        shot(&mut h, &format!("settings-{scale}"));
        h.get_by_label("Concluído").click();
        h.run();
        // Render saved preferences without touching a real vault or network.
        let profile = h.state().service.jira.profile.clone();
        h.state_mut().service.jira.profile.connection = codisseu_core::jira::Connection {
            site: "https://fixture.atlassian.net".into(),
            api: "https://fixture.atlassian.net".into(),
            email: "fixture@example.invalid".into(),
        };
        h.state_mut().open_jira_settings();
        h.run();
        assert_eq!(
            h.get_by_label("Guardar token no cofre do sistema")
                .accesskit_node()
                .toggled(),
            Some(egui::accesskit::Toggled::True)
        );
        shot(&mut h, &format!("settings-saved-{scale}"));
        h.state_mut().service.jira.profile.auto_connect = false;
        h.state_mut().open_jira_settings();
        h.run();
        assert_eq!(
            h.get_by_label("Guardar token no cofre do sistema")
                .accesskit_node()
                .toggled(),
            Some(egui::accesskit::Toggled::False)
        );
        h.get_by_label("Concluído").click();
        h.run();
        h.state_mut().service.jira.profile = profile;
        h.state_mut()
            .service
            .dispatch(Command::NewUntitled)
            .unwrap();
        settle(&mut h);
        let code_id = h.state().service.active_document().unwrap().id;
        let code_version = h.state().service.active_document().unwrap().version;
        h.state_mut()
            .service
            .dispatch(Command::Edit {
                id: code_id,
                expected_version: code_version,
                text: "Uncommitted code stays intact".into(),
            })
            .unwrap();
        settle(&mut h);
        let code_label = format!(
            "{} •",
            h.state()
                .service
                .active_document()
                .unwrap()
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
        );
        h.state_mut().service.jira.connected = true;
        h.state_mut().service.jira.identity = Some(codisseu_core::jira::client::Identity {
            name: "Lucas · conta de demonstração".into(),
            account: "fixture".into(),
            avatar: String::new(),
            team_field: None,
            team_fields: vec![],
        });
        h.state_mut().service.jira.page.issues = (1..=50)
            .map(|i| codisseu_core::jira::Issue {
                key: format!("DEMO-{i}"),
                title: format!("Melhorar rastreabilidade das entregas · tarefa{i}"),
                status: "Em andamento".into(),
                kind: "História".into(),
                project: "Projeto demonstração".into(),
                assignee: codisseu_core::jira::Choice {
                    id: format!("person{i}"),
                    name: format!("Pessoa {i}"),
                },
                assignee_avatar: String::new(),
                ..Default::default()
            })
            .collect();
        h.state_mut().service.jira.media.completed.push_back((
            "https://fixture.atlassian.net/avatar.png".into(),
            Ok(codisseu_core::jira::media::Raster {
                width: 48,
                height: 48,
                rgba: [90u8, 150, 210, 255].repeat(48 * 48),
            }),
        ));
        h.run();
        let avatar = h.get_by_label("Avatar de Pessoa 1").rect();
        let title = h
            .get_by_label("Melhorar rastreabilidade das entregas · tarefa1")
            .rect();
        assert!(
            (avatar.center().y - title.center().y).abs() < 1.,
            "avatar and task title should share a center: {avatar:?} / {title:?}"
        );
        let status = h.get_by_label("Status Jira").rect();
        let filter = h.get_by_label("Filtros Jira").rect();
        let reset = h.get_by_label("Redefinir filtros Jira").rect();
        assert!(
            (status.center().y - filter.center().y).abs() < 1.,
            "status {status:?}; filter {filter:?}"
        );
        assert!((filter.center().y - reset.center().y).abs() < 1.);
        shot(&mut h, &format!("tasks-{scale}"));
        h.get_by_label("Filtros Jira").click();
        h.run();
        let mut previous = None;
        for name in [
            "Projeto: todos",
            "Tipo: todos",
            "História / épico: todos",
            "Responsável: todos",
            "Equipe: todos",
        ] {
            let rect = h.get_by_label(name).rect();
            if let Some(before) = previous {
                assert!(rect.top() - before >= 7.5, "filter gap at {scale}: {name}");
            }
            assert!(rect.height() >= 28.);
            previous = Some(rect.bottom());
        }
        shot(&mut h, &format!("filters-{scale}"));
        h.get_by_label("Projeto: todos").click();
        h.run();
        assert!(h.state().jira_ui.picker.is_some());
        h.key_press(egui::Key::Escape);
        h.run();
        h.get_by_label("Filtros Jira").click();
        h.run();

        h.get_by_label("Abrir tarefa DEMO-1").click();
        settle(&mut h);
        assert!(h.state().jira_tool_active());
        assert!(
            !h.state()
                .jira_ui
                .tabs
                .contains(crate::jira_tabs::Kind::Hours)
        );
        h.state_mut().service.jira.error = None;
        h.state_mut().service.jira.selected = Some("DEMO-1".into());
        h.state_mut().service.jira.detail=Some(codisseu_core::jira::Detail{issue:codisseu_core::jira::Issue{key:"DEMO-1".into(),title:"Acompanhar entregas com mais clareza".into(),project:"Experiência de entregas".into(),kind:"História".into(),status:"Em progresso".into(),assignee:codisseu_core::jira::Choice{id:"fixture".into(),name:"Lucas · demonstração".into()},description:"## Objetivo\nAcompanhar cada **entrega** com contexto, sem interromper o trabalho.\n\n### Critérios de aceite\n- Exibir o histórico de eventos\n- Preservar o código aberto\n\n| Campo | Origem |\n| --- | --- |\n| Status | API |\n\n```php\nreturn $entrega->status;\n```\n\n> Fixture própria. Nenhum dado empresarial.\n".into(),..Default::default()},..Default::default()});
        {
            let issue = &mut h.state_mut().service.jira.detail.as_mut().unwrap().issue;
            issue.assignee_avatar = "https://fixture.atlassian.net/avatar.png".into();
            issue.reporter.name = "Autora da história".into();
            issue.reporter_avatar = "https://fixture.atlassian.net/avatar.png".into();
        }
        h.run();
        for _ in 0..15 {
            h.step();
            thread::sleep(Duration::from_millis(10));
        }
        shot(&mut h, &format!("reader-{scale}"));
        h.get_by_label("Pessoas e vínculos").click();
        h.run();
        h.get_by_label("Avatar de Autora da história");
        shot(&mut h, &format!("people-{scale}"));
        h.get_by_label("Comentários").click();
        settle(&mut h);
        h.state_mut().service.jira.error = None;
        {
            let service = &mut h.state_mut().service.jira;
            let d = service.detail.as_mut().unwrap();
            d.comments=vec![codisseu_core::jira::Comment{author:"Maria · demonstração".into(),avatar:"https://fixture.atlassian.net/avatar.png".into(),created:"03/11/2026 · 10:30".into(),body:"**Validado** na fixture. Podemos seguir com a história.\n\n- Critério revisado\n- Evidência anexada".into()}];
            d.comment_total = 1;
            service.revision += 1;
        }
        for _ in 0..15 {
            h.step();
            thread::sleep(Duration::from_millis(10));
        }
        shot(&mut h, &format!("comments-{scale}"));
        h.get_by_label("Horas").click();
        settle(&mut h);
        h.state_mut().service.jira.error = None;
        h.run();
        {
            let service = &mut h.state_mut().service.jira;
            service.detail.as_mut().unwrap().worklogs = vec![
                codisseu_core::jira::Worklog {
                    account: "fixture".into(),
                    author: "Lucas".into(),
                    avatar: String::new(),
                    started: "2026-11-03T09:00:00-03:00".into(),
                    seconds: 3600,
                    body: "Registro pessoal visível".into(),
                },
                codisseu_core::jira::Worklog {
                    account: "other".into(),
                    author: "Outra pessoa".into(),
                    avatar: String::new(),
                    started: "2026-11-03T09:00:00-03:00".into(),
                    seconds: 7200,
                    body: "Registro de outra pessoa proibido na tela".into(),
                },
            ];
            service.revision += 1;
        }
        h.run();
        h.get_by_label("Registro pessoal visível");
        assert!(
            h.query_all(
                egui_kittest::kittest::By::new().label("Registro de outra pessoa proibido na tela")
            )
            .next()
            .is_none()
        );
        h.get_by_label("Registrar horas…").click();
        h.run();
        shot(&mut h, &format!("worklog-{scale}"));
        h.get_by_label("Revisar apontamento").click();
        h.run();
        shot(&mut h, &format!("worklog-review-{scale}"));
        h.get_by_label("Voltar").click();
        h.run();
        h.get_by_label("Cancelar").click();
        h.run();
        h.get_by_label(&code_label).click();
        h.run();
        assert!(!h.state().jira_tool_active());
        let code = h.state().service.active_document().unwrap();
        assert_eq!(code.id, code_id);
        assert_eq!(code.text(), "Uncommitted code stays intact");
        assert!(code.is_dirty());
        let mut p = h.state().service.jira.profile.clone();
        p.enabled = false;
        h.state_mut().service.jira.set_profile(p).unwrap();
        settle(&mut h);
        assert!(!h.state().jira_ui.open);
        assert!(h.state().service.jira.detail.is_none());
        assert!(!h.state().service.jira.connected);
    }
}
