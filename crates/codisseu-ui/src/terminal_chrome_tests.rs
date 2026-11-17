//! Own local PTYs only. Reproduce the three user screenshots through real widgets.
use super::*;
use codisseu_core::AppService;
use egui_kittest::{Harness, kittest::Queryable};
use std::time::Instant;
fn fixture(scale: f32) -> (tempfile::TempDir, Harness<'static, CodisseuApp>) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir
        .path()
        .join("ics-faturamento-lv-development-php84-long-project-name".repeat(3));
    std::fs::create_dir(&root).unwrap();
    let mut app = CodisseuApp::new(AppService::new(dir.path().join("profile/session.json")));
    app.terminal.open(&root);
    app.show_activity = true;
    app.tool_tab = ToolTab::Terminal;
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .with_pixels_per_point(scale)
        .with_max_steps(100)
        .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
    wait(&mut h, |a| a.terminal.running);
    h.run();
    (dir, h)
}
fn wait(h: &mut Harness<CodisseuApp>, condition: impl Fn(&CodisseuApp) -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
    while !condition(h.state()) {
        h.step();
        std::thread::sleep(Duration::from_millis(4));
        assert!(Instant::now() < until);
    }
}
fn capture(h: &mut Harness<CodisseuApp>, name: &str, scale: f32) {
    if let Some(out) = std::env::var_os("CODISSEU_TERMINAL_CHROME_CAPTURES") {
        std::fs::create_dir_all(&out).unwrap();
        h.render()
            .unwrap()
            .save(PathBuf::from(out).join(format!("{name}-{scale}.png")))
            .unwrap();
    }
}
fn stop(h: &mut Harness<CodisseuApp>) {
    h.state_mut().terminal.close();
    wait(h, |a| !a.terminal.shutdown_pending());
}
#[test]
fn long_title_keeps_entire_icon_and_all_tools_share_tab_geometry_and_fonts() {
    for scale in [1., 1.25] {
        let (_dir, mut h) = fixture(scale);
        let mut icons = 0;
        for clipped in &h.output().shapes {
            if let egui::epaint::Shape::Text(text) = &clipped.shape
                && text.galley.text() == "\u{ea85}"
            {
                icons += 1;
                let bounds = clipped.shape.visual_bounding_rect();
                assert!(
                    clipped.clip_rect.expand(1.).contains_rect(bounds),
                    "icon clipped: {:?} in {:?}",
                    bounds,
                    clipped.clip_rect
                );
            }
        }
        assert_eq!(icons, 1);
        capture(&mut h, "single-long-title", scale);
        let labels = [
            "Git",
            "Debug",
            "Terminal",
            "Services",
            "Atividade",
            "Referências",
            "Docker",
        ];
        let rects = labels
            .iter()
            .map(|label| {
                h.get_by_role_and_label(egui::accesskit::Role::Tab, label)
                    .rect()
            })
            .collect::<Vec<_>>();
        let active_id = h.state().terminal.identity;
        for tab in [
            ToolTab::Git,
            ToolTab::Debug,
            ToolTab::Results,
            ToolTab::Activity,
            ToolTab::References,
            ToolTab::Docker,
        ] {
            h.state_mut().tool_tab = tab;
            h.run();
            for (index, label) in labels.iter().enumerate() {
                let current = h
                    .get_by_role_and_label(egui::accesskit::Role::Tab, label)
                    .rect();
                assert!(
                    (current.min - rects[index].min).length() < 0.1
                        && (current.size() - rects[index].size()).length() < 0.1,
                    "{label}: {current:?} vs {:?}",
                    rects[index]
                );
            }
            assert_eq!(h.state().terminal.identity, active_id);
            assert!(h.get_by_label("Ocultar painel inferior").rect().width() >= 22.);
            for clipped in &h.output().shapes {
                if let egui::epaint::Shape::Text(text) = &clipped.shape
                    && text.galley.text() == "\u{ea76}"
                {
                    assert!(
                        clipped
                            .clip_rect
                            .expand(1.)
                            .contains_rect(clipped.shape.visual_bounding_rect())
                    );
                }
            }
            if tab == ToolTab::Activity {
                capture(&mut h, "activity-header", scale);
            }
            if tab == ToolTab::Results {
                capture(&mut h, "services-header", scale);
            }
        }
        stop(&mut h);
    }
}
fn drag(h: &mut Harness<CodisseuApp>, distance: f32) {
    let position = h
        .get_by_label("Largura da lista de terminais")
        .rect()
        .center();
    h.hover_at(position);
    h.step();
    h.drag_at(position);
    h.step();
    h.hover_at(position + egui::vec2(distance / 2., 0.));
    h.step();
    h.hover_at(position + egui::vec2(distance, 0.));
    h.step();
    h.drop_at(position + egui::vec2(distance, 0.));
    h.run();
}
#[test]
fn sidebar_drag_both_directions_and_hover_actions_target_real_sessions() {
    for scale in [1., 1.25] {
        let (_dir, mut h) = fixture(scale);
        let root = h.state().terminal.cwd.clone();
        h.state_mut().terminal.open_directory(&root);
        h.run();
        wait(&mut h, |a| a.terminal.running);
        let active = h.state().terminal.identity;
        let inactive = h
            .state()
            .terminal
            .tabs()
            .into_iter()
            .find(|t| !t.active)
            .unwrap();
        let initial_cols = h.state().terminal.dimensions.1;
        drag(&mut h, -100.);
        let narrow_cols = h.state().terminal.dimensions.1;
        assert!(narrow_cols < initial_cols);
        assert!((h.state().terminal_ui.sidebar_width.unwrap() - 220.).abs() < 2.);
        drag(&mut h, 60.);
        assert!(h.state().terminal.dimensions.1 > narrow_cols);
        assert!((h.state().terminal_ui.sidebar_width.unwrap() - 160.).abs() < 2.);
        let width = h.state().terminal_ui.sidebar_width;
        h.state_mut().tool_tab = ToolTab::Activity;
        h.run();
        h.state_mut().tool_tab = ToolTab::Terminal;
        h.run();
        assert_eq!(h.state().terminal_ui.sidebar_width, width);
        let label = format!(
            "Terminal {} em {}",
            inactive.id,
            root.file_name().unwrap().to_string_lossy()
        );
        h.get_by_role_and_label(egui::accesskit::Role::Tab, &label)
            .hover();
        h.run();
        capture(&mut h, "hover-actions", scale);
        h.get_by_label(&format!("Dividir terminal {}", inactive.id))
            .click();
        h.run();
        assert_eq!(h.state().terminal.session_count(), 3);
        assert_eq!(h.state().terminal.identity, inactive.group);
        assert_eq!(h.state().terminal.splits[0].cwd, root);
        let split = h.state().terminal.splits[0].identity;
        h.state_mut().terminal.splits[0].send(b"printf 'HOVER_SPLIT_OK\\n'\r".to_vec());
        wait(&mut h, |a| {
            a.terminal.splits[0]
                .parser
                .screen()
                .contents()
                .lines()
                .any(|line| line == "HOVER_SPLIT_OK")
        });
        capture(&mut h, "split-created", scale);
        let previous = h
            .state()
            .terminal
            .tabs()
            .into_iter()
            .find(|t| t.id == active)
            .unwrap();
        h.state_mut().terminal.activate(&previous);
        h.run();
        let label = format!(
            "Terminal {split} em {}",
            root.file_name().unwrap().to_string_lossy()
        );
        h.get_by_role_and_label(egui::accesskit::Role::Tab, &label)
            .hover();
        h.run();
        h.get_by_label(&format!("Excluir terminal {split}")).click();
        h.run();
        assert_eq!(h.state().terminal.session_count(), 2);
        assert_eq!(h.state().terminal.identity, active);
        assert!(!h.state().terminal.tabs().iter().any(|t| t.id == split));
        stop(&mut h);
    }
}
