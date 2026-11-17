//! Geometry, keyboard access and clipping checks on the actual functional widgets.
use codisseu_core::{AppService, Command};
use codisseu_ui::CodisseuApp;
use egui::{
    Key,
    accesskit::{Role, Toggled},
};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

fn settle(ui: &mut Harness<'_, CodisseuApp>) {
    ui.step();
    let deadline = Instant::now() + Duration::from_secs(10);
    while ui.state().service.is_busy() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
        ui.step();
    }
    ui.run();
}

#[test]
fn toolbar_tabs_tree_and_rails_keep_geometry_at_supported_window_sizes() {
    for (width, height, scale) in [
        (1280., 800., 1.),
        (900., 600., 1.),
        (800., 500., 1.),
        (1280., 800., 2.),
        (900., 600., 2.),
        (800., 500., 2.),
    ] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        for name in ["index.php", "app.ts", "query.sql"] {
            fs::write(dir.path().join("src").join(name), "// Olá 🌧\n").unwrap();
        }
        let mut service = AppService::new(dir.path().join("session.json"));
        service
            .dispatch(Command::OpenWorkspace(dir.path().into()))
            .unwrap();
        for name in ["index.php", "app.ts", "query.sql"] {
            service
                .dispatch(Command::OpenFile(dir.path().join("src").join(name)))
                .unwrap();
        }
        let mut ui = Harness::builder()
            .with_size([width, height])
            .with_pixels_per_point(scale)
            .with_max_steps(20)
            .build_eframe(|_| CodisseuApp::new(service));
        settle(&mut ui);
        for label in [
            "Configurações",
            "Pesquisar arquivos",
            "Configurar providers",
            "Executar SQL",
        ] {
            let rect = ui.get_by_label(label).rect();
            assert!((rect.width() - 30.).abs() < 0.1, "{label}: {rect:?}");
            assert!((rect.height() - 30.).abs() < 0.1, "{label}: {rect:?}");
            assert!(
                rect.left() >= 0.
                    && rect.right() <= width
                    && rect.top() >= 0.
                    && rect.bottom() <= 40.,
                "clipped toolbar at {width}: {label}: {rect:?}"
            );
        }
        let project = ui.get_by_label("Alternar painel Projeto").rect();
        let database = ui.get_by_label("Alternar painel Database").rect();
        assert_eq!(project.width(), 40.);
        assert_eq!(project.height(), 40.);
        assert_eq!(database.width(), 40.);
        assert!(
            database.right() <= width,
            "rail clipped at {width}: {database:?}"
        );
        // File labels occur both in tree and tabs; compare only the tab row near the header.
        let tabs: Vec<_> = ["index.php", "app.ts", "query.sql"]
            .into_iter()
            .map(|label| {
                ui.get_all_by_label(label)
                    .find(|node| node.rect().top() < 90.)
                    .expect("document tab")
                    .rect()
            })
            .collect();
        for tab in &tabs {
            assert!((tab.height() - 41.).abs() < 0.1, "tab height: {tab:?}");
            assert!(
                (tab.top() - tabs[0].top()).abs() < 0.1,
                "staggered tabs: {tabs:?}"
            );
        }
        assert_eq!(ui.get_by_label("src").rect().height(), 24.);
        assert_eq!(
            ui.get_by_label("src").accesskit_node().data().is_expanded(),
            Some(true)
        );
        // Explorer selection is independent from loading a document through the
        // service. Exercise the real tree row before checking both selections.
        ui.get_all_by_label("query.sql")
            .find(|node| node.rect().top() > 90.)
            .expect("visible Explorer file row")
            .click();
        settle(&mut ui);
        for node in ui.get_all_by_label("query.sql") {
            assert_eq!(node.accesskit_node().toggled(), Some(Toggled::True));
        }
        assert_eq!(
            ui.get_by_label("Alternar painel Database")
                .accesskit_node()
                .toggled(),
            Some(Toggled::True)
        );

        let editor = ui.get_by_role(Role::MultilineTextInput).rect();
        assert!(
            editor.left() >= project.right() && editor.left() < database.left(),
            "editor inaccessible at {width}: {editor:?}"
        );
    }
}

#[test]
fn icon_action_has_a_name_and_activates_from_keyboard() {
    let dir = tempfile::tempdir().unwrap();
    let mut ui = Harness::builder()
        .with_size([1280., 800.])
        .with_max_steps(20)
        .build_eframe(|_| CodisseuApp::new(AppService::new(dir.path().join("state.json"))));
    for _ in 0..40 {
        if ui.get_by_label("Menu principal").is_focused() {
            break;
        }
        ui.key_press(Key::Tab);
        settle(&mut ui);
    }
    assert!(
        ui.get_by_label("Menu principal").is_focused(),
        "named icon must participate in keyboard focus order"
    );
    ui.key_press(Key::Enter);
    settle(&mut ui);
    assert!(
        ui.query_by_label("Arquivo").is_some(),
        "Enter must open the compact menu"
    );
}

#[test]
fn closing_passive_tab_keeps_active_document_and_dirty_close_requires_resolution() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("first.ts");
    let b = dir.path().join("second.php");
    fs::write(&a, "export const value = 1;\n").unwrap();
    fs::write(&b, "<?php echo 'saved';\n").unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service.dispatch(Command::OpenFile(a.clone())).unwrap();
    service.dispatch(Command::OpenFile(b.clone())).unwrap();
    let mut ui = Harness::builder()
        .with_size([1280., 800.])
        .with_max_steps(30)
        .build_eframe(|_| CodisseuApp::new(service));
    settle(&mut ui);
    let active = ui.state().service.active_document().unwrap().id;
    ui.get_by_label("Fechar first.ts").click();
    settle(&mut ui);
    assert_eq!(ui.state().service.active_document().unwrap().id, active);
    assert_eq!(ui.state().service.documents.len(), 1);
    ui.get_by_role(Role::MultilineTextInput).click();
    settle(&mut ui);
    ui.event(egui::Event::Text("X".into()));
    settle(&mut ui);
    assert!(ui.state().service.active_document().unwrap().is_dirty());
    ui.get_by_label("Fechar second.php").click();
    settle(&mut ui);
    assert_eq!(
        ui.state().service.documents.len(),
        1,
        "dirty tab requires resolution"
    );
    assert_eq!(fs::read_to_string(b).unwrap(), "<?php echo 'saved';\n");
}

#[test]
fn long_tree_name_scrolls_horizontally_without_changing_the_document() {
    let dir = tempfile::tempdir().unwrap();
    let name = format!("{}.php", "arquivo_longo_".repeat(15));
    let path = dir.path().join(&name);
    fs::write(&path, "<?php echo 'Olá';\n").unwrap();
    let mut service = AppService::new(dir.path().join("state.json"));
    service
        .dispatch(Command::OpenWorkspace(dir.path().into()))
        .unwrap();
    service.dispatch(Command::OpenFile(path.clone())).unwrap();
    let mut ui = Harness::builder()
        .with_size([1280., 800.])
        .with_max_steps(30)
        .build_eframe(|_| CodisseuApp::new(service));
    settle(&mut ui);
    let tree_rect = |ui: &Harness<'_, CodisseuApp>| {
        ui.get_all_by_label(&name)
            .find(|node| node.rect().top() > 90.)
            .expect("tree row")
            .rect()
    };
    let before = tree_rect(&ui);
    assert!(
        before.width() > 500.,
        "tree content must include the painted name's width: {before:?}"
    );
    ui.hover_at(egui::pos2(180., before.center().y));
    ui.event(egui::Event::MouseWheel {
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(-160., 0.),
        modifiers: egui::Modifiers::NONE,
    });
    settle(&mut ui);
    let after = tree_rect(&ui);
    assert!(
        after.left() < before.left() - 30.,
        "horizontal wheel must reveal the clipped name: {before:?} -> {after:?}"
    );
    assert_eq!(ui.state().service.active_document().unwrap().path, path);
}

#[test]
fn regular_mouse_wheel_follows_tabs_and_editor_under_pointer() {
    for scale in [1., 1.25] {
        let dir = tempfile::tempdir().unwrap();
        let mut service = AppService::new(dir.path().join("session.json"));
        let names = (0..9)
            .map(|n| format!("documento_de_faturamento_{n}.php"))
            .collect::<Vec<_>>();
        let source = "<?php\n".to_owned() + &"// conteúdo de rolagem 😀\n".repeat(200);
        for name in &names {
            let path = dir.path().join(name);
            fs::write(&path, &source).unwrap();
            service.dispatch(Command::OpenFile(path)).unwrap();
        }
        let mut h = Harness::builder()
            .with_size([1280., 800.])
            .with_pixels_per_point(scale)
            .with_max_steps(100)
            .build_eframe(|_| CodisseuApp::new(service));
        settle(&mut h);
        let document = h.state().service.active_document().unwrap();
        let (id, version, selection) = (document.id, document.version, document.selection());
        let view = h.state().service.active_view().unwrap();
        let before = h.get_by_label(&names[1]).rect();
        let capture = |h: &mut Harness<'_, CodisseuApp>, name: &str| {
            if let Some(out) = std::env::var_os("CODISSEU_WHEEL_CAPTURES") {
                let out = std::path::PathBuf::from(out);
                fs::create_dir_all(&out).unwrap();
                h.render()
                    .unwrap()
                    .save(out.join(format!("{name}-{scale}.png")))
                    .unwrap();
            }
        };
        let wheel = |h: &mut Harness<'_, CodisseuApp>, delta, modifiers| {
            h.event(egui::Event::MouseWheel {
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Line,
                delta,
                modifiers,
            });
            settle(h);
        };
        capture(&mut h, "tabs-before");
        h.hover_at(before.center());
        wheel(&mut h, egui::vec2(0., -3.), egui::Modifiers::NONE);
        let right = h.get_by_label(&names[1]).rect();
        assert!(
            right.left() < before.left() - 20.,
            "wheel down reveals tabs to the right: {before:?} -> {right:?}"
        );
        assert_eq!(
            h.state().editor_view_metrics()[&view].visible_start,
            0,
            "hovering tabs must not scroll editor"
        );
        capture(&mut h, "tabs-wheel-right");
        wheel(&mut h, egui::vec2(0., 2.), egui::Modifiers::NONE);
        let left = h.get_by_label(&names[1]).rect();
        assert!(
            left.left() > right.left() + 10.,
            "wheel up returns toward left"
        );
        wheel(&mut h, egui::vec2(-2., 0.), egui::Modifiers::NONE);
        assert!(
            h.get_by_label(&names[1]).rect().left() < left.left() - 10.,
            "horizontal gesture stays supported"
        );
        let before_shift = h.get_by_label(&names[1]).rect().left();
        wheel(&mut h, egui::vec2(0., -2.), egui::Modifiers::SHIFT);
        assert!(
            h.get_by_label(&names[1]).rect().left() < before_shift - 10.,
            "Shift+wheel stays supported"
        );
        let strip = h.get_by_label(&names[1]).rect().left();
        let editor = h.get_by_role(Role::MultilineTextInput).rect();
        h.hover_at(editor.center());
        wheel(&mut h, egui::vec2(0., -8.), egui::Modifiers::NONE);
        assert!(
            h.state().editor_view_metrics()[&view].visible_start > 0,
            "wheel over editor must scroll vertically"
        );
        assert!(
            (h.get_by_label(&names[1]).rect().left() - strip).abs() < 1.,
            "editor wheel must not move tabs"
        );
        capture(&mut h, "editor-wheel-down");
        let document = h.state().service.active_document().unwrap();
        assert_eq!(
            (document.id, document.version, document.selection()),
            (id, version, selection)
        );
        assert_eq!(document.text(), source);
        assert_eq!(fs::read_to_string(&document.path).unwrap(), source);
    }
}
