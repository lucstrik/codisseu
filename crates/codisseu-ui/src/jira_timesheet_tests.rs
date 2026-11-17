//! Real egui widgets with explicit synthetic snapshots; HTTP/author filtering is tested in Jira crate.
use super::*;
use codisseu_core::jira::{
    self as jira,
    period::{Mode, Period},
    timesheet::{Entry, Snapshot, State},
};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::{sync::Arc, thread, time::Instant};
fn settle(h: &mut Harness<'_, CodisseuApp>) {
    let end = Instant::now() + Duration::from_secs(10);
    let mut idle = 0;
    while idle < 4 {
        h.step();
        idle = if h.state().service.is_busy() {
            0
        } else {
            idle + 1
        };
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(4));
    }
}
fn shot(h: &mut Harness<'_, CodisseuApp>, name: &str) {
    if let Some(dir) = std::env::var_os("CODISSEU_JIRA_HOURS_CAPTURES") {
        std::fs::create_dir_all(&dir).unwrap();
        h.render()
            .unwrap()
            .save(std::path::Path::new(&dir).join(format!("{name}.png")))
            .unwrap();
    }
}
fn period() -> Period {
    Period {
        mode: Mode::Month,
        anchor: "2026-11-03".into(),
    }
}
fn entries() -> Vec<Entry> {
    (0..12)
        .map(|i| Entry {
            id: i.to_string(),
            key: format!("DEMO-{}", i % 3 + 1),
            title: "Acompanhar entregas com contexto".into(),
            date: format!("2026-11-{:02}", i / 3 + 3),
            time: format!("{:02}:00", 9 + i % 3),
            seconds: 3600,
            note: "Revisão e implementação · fixture própria".into(),
            instant: i,
        })
        .collect()
}
#[test]
fn personal_hours_month_week_day_navigation_filter_and_code_preservation() {
    for (scale, width, height) in [
        (1., 1280., 850.),
        (1.25, 1280., 850.),
        (1., 1000., 700.),
        (1.25, 1000., 700.),
    ] {
        let d = tempfile::tempdir().unwrap();
        let service = AppService::new(d.path().join("session.json"));
        let mut h = Harness::builder()
            .with_size([width, height])
            .with_pixels_per_point(scale)
            .with_max_steps(80)
            .build_ui_state(
                |ui, a: &mut CodisseuApp| a.show(ui),
                CodisseuApp::new(service),
            );
        settle(&mut h);
        let mut profile = h.state().service.jira.profile.clone();
        profile.enabled = true;
        profile.timesheet = period();
        h.state_mut().service.jira.set_profile(profile).unwrap();
        settle(&mut h);
        h.state_mut()
            .service
            .dispatch(Command::NewUntitled)
            .unwrap();
        settle(&mut h);
        let doc = h.state().service.active_document().unwrap();
        let (id, version) = (doc.id, doc.version);
        h.state_mut()
            .service
            .dispatch(Command::Edit {
                id,
                expected_version: version,
                text: "Uncommitted work stays intact".into(),
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
        {
            let a = h.state_mut();
            a.service.jira.connected = true;
            a.service.jira.identity = Some(jira::client::Identity {
                name: "Lucas · demonstração".into(),
                account: "me".into(),
                avatar: String::new(),
                team_field: None,
                team_fields: vec![],
            });
            a.service.jira.timesheet.view = Some(Arc::new(Snapshot::from_entries(
                period(),
                entries(),
                State::Complete,
                15,
                17,
                None,
            )));
        }
        assert!(
            !h.state()
                .jira_ui
                .tabs
                .contains(crate::jira_tabs::Kind::Hours)
        );
        h.get_by_label("Jira · Minhas tarefas").click();
        h.run();
        assert!(
            !h.state()
                .jira_ui
                .tabs
                .contains(crate::jira_tabs::Kind::Hours)
        );
        h.get_by_label("Abrir Minhas horas").click();
        h.run();
        assert!(
            h.state()
                .jira_ui
                .tabs
                .contains(crate::jira_tabs::Kind::Hours)
        );
        assert!(!h.state().service.jira.timesheet.busy);
        h.get_by_label("Horas no período");
        assert!(
            h.query_all(egui_kittest::kittest::By::new().label("Código"))
                .next()
                .is_none()
        );
        let native_tab = h
            .get_by_role_and_label(egui::accesskit::Role::Button, "Minhas horas")
            .rect();
        let file_tab = h.get_by_label(&code_label).rect();
        assert!((native_tab.top() - file_tab.top()).abs() < 1.);
        assert!((native_tab.height() - file_tab.height()).abs() < 1.);

        h.get_by_label("12h");
        let refresh = h.get_by_label("Atualizar horas").rect();
        let locate = h.get_by_label("Localizar cards para apontar").rect();
        if locate.top() >= refresh.bottom() - 1. {
            assert!(
                locate.top() - refresh.bottom() >= 7.5,
                "wrapped action gap: {refresh:?} {locate:?}"
            );
        } else {
            assert!(
                locate.left() - refresh.right() >= 7.5,
                "inline action gap: {refresh:?} {locate:?}"
            );
        }

        let first = h.get_by_label("Horas em 2026-11-03").rect();
        let next = h.get_by_label("Horas em 2026-11-04").rect();
        assert!((first.min.y - next.min.y).abs() < 1.);
        assert!(next.min.x - first.max.x >= 5.);
        shot(&mut h, &format!("month-{scale}-{width}"));
        // Recording is independent of report completion and card creation date.
        h.state_mut().service.jira.timesheet.busy = true;
        h.get_by_label("Registrar horas…").click();
        h.run_steps(3);
        assert!(h.state().jira_ui.worklog.is_some());
        h.get_by_label("Chave do card").click();
        h.run_steps(3);
        h.event(egui::Event::Text("OLD-42".into()));
        h.run_steps(3);
        let started = h.get_by_label("Início com fuso");
        assert!(
            started
                .accesskit_node()
                .value()
                .unwrap()
                .starts_with("2026-11-03T")
        );
        shot(&mut h, &format!("calendar-draft-{scale}-{width}"));
        h.get_by_label("Revisar apontamento").click();
        h.run_steps(3);
        assert_eq!(h.state().service.jira.selected.as_deref(), Some("OLD-42"));
        let deadline = Instant::now() + Duration::from_secs(5);
        while h.state().service.jira.main_busy() {
            assert!(Instant::now() < deadline);
            h.step();
            thread::sleep(Duration::from_millis(4));
        }
        // This widget harness has no authenticated HTTP worker. Supply the
        // confirmed card as a synthetic UI snapshot, like the report above.
        h.state_mut().service.jira.error = None;
        h.state_mut().service.jira.detail = Some(jira::Detail {
            issue: jira::Issue {
                key: "OLD-42".into(),
                title: "Card criado antes do mês selecionado".into(),
                ..Default::default()
            },
            ..Default::default()
        });
        h.run_steps(4);

        h.get_by_label("Confirmar e registrar");
        shot(&mut h, &format!("calendar-review-{scale}-{width}"));
        h.get_by_label("Voltar").click();
        h.run_steps(3);
        h.get_by_label("Cancelar").click();
        h.run_steps(3);
        h.state_mut().service.jira.timesheet.busy = false;
        h.run();

        h.get_by_label("Horas em 2026-11-04").click();
        h.run();
        assert_eq!(h.state().jira_ui.hours.selected, "2026-11-04");
        let expected = jira::period::Period {
            mode: Mode::Week,
            anchor: "2026-11-04".into(),
        };
        h.state_mut().service.jira.timesheet.view = Some(Arc::new(Snapshot::from_entries(
            expected,
            entries(),
            State::Complete,
            4,
            5,
            None,
        )));
        h.get_by_label("Semana").click();
        h.run();
        assert_eq!(h.state().service.jira.profile.timesheet.mode, Mode::Week);
        assert!(!h.state().service.jira.timesheet.busy);
        shot(&mut h, &format!("week-{scale}-{width}"));
        let expected = jira::period::Period {
            mode: Mode::Day,
            anchor: "2026-11-04".into(),
        };
        h.state_mut().service.jira.timesheet.view = Some(Arc::new(Snapshot::from_entries(
            expected,
            entries(),
            State::Complete,
            1,
            2,
            None,
        )));
        h.get_by_label("Dia").click();
        h.run();
        assert_eq!(h.state().service.jira.profile.timesheet.mode, Mode::Day);
        h.get_by_label("3h");
        shot(&mut h, &format!("day-{scale}-{width}"));
        h.get_by_label("DEMO-1 · Acompanhar entregas com contexto")
            .click();
        settle(&mut h);
        assert!(h.state().jira_tool_active());
        assert_eq!(h.state().service.jira.selected.as_deref(), Some("DEMO-1"));
        h.get_by_label("Minhas horas").click();
        h.run();
        assert!(
            h.state()
                .jira_ui
                .tabs
                .is_active(crate::jira_tabs::Kind::Hours)
        );
        h.get_by_label("Escolher data…").click();
        h.run();
        h.get_by_label("Data · dd/mm/aaaa").click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.run();
        h.event(egui::Event::Text("31/02/2026".into()));
        h.run();
        h.get_by_label("Ir para data").click();
        h.run();
        assert!(h.state().jira_ui.hours.date_picker);
        shot(&mut h, &format!("date-invalid-{scale}-{width}"));
        h.get_by_label("Data · dd/mm/aaaa").click();
        h.run();
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.event(egui::Event::Text("04/11/2026".into()));
        h.run();
        h.get_by_label("Ir para data").click();
        h.run();
        assert!(!h.state().jira_ui.hours.date_picker);
        assert_eq!(
            h.state().service.jira.profile.timesheet.anchor,
            "2026-11-04"
        );
        h.get_by_label("Localizar cards para apontar").click();
        settle(&mut h);
        let f = &h.state().service.jira.profile.filters;
        assert!(f.mine);
        assert_eq!(f.status, jira::Status::All);
        assert_eq!(f.created_month.as_deref(), Some("2026-11"));
        shot(&mut h, &format!("created-filter-{scale}-{width}"));
        h.get_by_label(&code_label).scroll_to_me();
        h.run();
        h.get_by_label(&code_label).click();
        h.run();
        assert!(
            !h.state().jira_tool_active(),
            "file tab activation at {scale} {width}"
        );
        let doc = h.state().service.active_document().unwrap();
        assert_eq!(doc.id, id);
        assert_eq!(doc.text(), "Uncommitted work stays intact");
        assert!(doc.is_dirty());
        h.get_by_label("Minhas horas").click();
        h.run();
        h.key_press(egui::Key::Escape);
        h.run();
        assert!(!h.state().jira_tool_active());
        assert!(
            h.state()
                .jira_ui
                .tabs
                .contains(crate::jira_tabs::Kind::Hours)
        );
        h.get_by_label("Minhas horas").click();
        h.run();
        h.state_mut().service.jira.timesheet.view = Some(Arc::new(Snapshot::from_entries(
            period(),
            entries(),
            State::Paused,
            7,
            10,
            Some("Parcial: continue a conferência.".into()),
        )));
        h.run();
        h.get_by_label("Horas encontradas");
        shot(&mut h, &format!("partial-{scale}-{width}"));
        h.get_by_label("Fechar aba Minhas horas").click();
        h.run();
        assert!(
            !h.state()
                .jira_ui
                .tabs
                .contains(crate::jira_tabs::Kind::Hours)
        );
        assert!(!h.state().service.jira.timesheet.busy);
        h.state_mut().service.jira.disconnect();
        settle(&mut h);
        assert!(h.state().service.jira.timesheet.view.is_none());
        let profile: jira::Profile =
            serde_json::from_slice(&std::fs::read(d.path().join("session.jira.json")).unwrap())
                .unwrap();
        assert_eq!(profile.filters.created_month.as_deref(), Some("2026-11"));
        assert_eq!(profile.timesheet.anchor, "2026-11-04");
    }
}

#[test]
fn jira_native_tabs_keep_groups_buffers_and_other_resource_selection_independent() {
    for scale in [1., 1.25] {
        let d = tempfile::tempdir().unwrap();
        let service = AppService::new(d.path().join("session.json"));
        let mut h = Harness::builder()
            .with_size([1440., 900.])
            .with_pixels_per_point(scale)
            .with_max_steps(80)
            .build_ui_state(
                |ui, a: &mut CodisseuApp| a.show(ui),
                CodisseuApp::new(service),
            );
        settle(&mut h);
        let mut p = h.state().service.jira.profile.clone();
        p.enabled = true;
        h.state_mut().service.jira.set_profile(p).unwrap();
        settle(&mut h);
        h.state_mut()
            .service
            .dispatch(Command::NewUntitled)
            .unwrap();
        settle(&mut h);
        let doc = h.state().service.active_document().unwrap();
        let (id, version) = (doc.id, doc.version);
        h.state_mut()
            .service
            .dispatch(Command::Edit {
                id,
                expected_version: version,
                text: "Dirty code, untouched by Jira tabs".into(),
            })
            .unwrap();
        settle(&mut h);
        let label = format!(
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
        h.state_mut().split_editor(false);
        settle(&mut h);
        let views: Vec<_> = h
            .state()
            .service
            .document_views()
            .iter()
            .map(|v| v.id)
            .collect();
        assert_eq!(views.len(), 2);
        let owner = h.state().service.active_view().unwrap();
        h.state_mut().open_jira_hours();
        h.run();
        assert_eq!(
            h.state().jira_ui.tabs.owner(crate::jira_tabs::Kind::Hours),
            Some(owner)
        );
        assert!(
            h.query_all(egui_kittest::kittest::By::new().label("Código"))
                .next()
                .is_none()
        );
        shot(&mut h, &format!("native-tabs-split-{scale}"));
        // The code tab in the other group must remain independently selectable.
        let tab = h
            .query_all(egui_kittest::kittest::By::new().label(&label))
            .min_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
            .unwrap();
        tab.click();
        h.run();
        assert_eq!(h.state().service.active_view(), Some(views[0]));
        assert!(!h.state().jira_tool_active());
        assert!(
            h.state()
                .jira_ui
                .tabs
                .is_active(crate::jira_tabs::Kind::Hours)
        );
        h.state_mut().open_jira_hours();
        h.run();
        assert_eq!(h.state().service.active_view(), Some(owner));
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::W);
        h.run();
        assert!(
            !h.state()
                .jira_ui
                .tabs
                .contains(crate::jira_tabs::Kind::Hours)
        );
        assert_eq!(
            h.state().service.active_document().unwrap().text(),
            "Dirty code, untouched by Jira tabs"
        );
        assert!(h.state().service.active_document().unwrap().is_dirty());
        h.state_mut().open_jira_hours();
        h.run();
        h.state_mut()
            .api_open_tab(crate::api_client::ApiTab::History);
        h.run();
        assert!(!h.state().jira_tool_active());
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Minhas horas")
            .click();
        h.run();
        assert!(h.state().jira_tool_active());
        assert!(!h.state().api_ui.active_in_view(owner));
        h.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Tab);
        h.run();
        assert!(!h.state().jira_tool_active());
        assert_eq!(h.state().service.active_document().unwrap().id, id);
        h.key_press_modifiers(
            egui::Modifiers::CTRL.plus(egui::Modifiers::SHIFT),
            egui::Key::Tab,
        );
        h.run();
        assert!(h.state().jira_tool_active());
        h.state_mut().send(Command::NewUntitled);
        settle(&mut h);
        let fresh = h.state().service.active_document().unwrap().id;
        assert_ne!(fresh, id);
        assert!(!h.state().jira_tool_active());
        h.get_by_label("Fechar aba Minhas horas").click();
        h.run();
        assert_eq!(h.state().service.active_document().unwrap().id, fresh);
        let old = h
            .state()
            .service
            .documents
            .iter()
            .find(|d| d.id == id)
            .unwrap();
        assert!(old.is_dirty());
        assert_eq!(old.text(), "Dirty code, untouched by Jira tabs");
        shot(&mut h, &format!("native-tabs-code-{scale}"));
    }
}
