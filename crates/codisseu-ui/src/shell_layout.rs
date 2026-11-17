//! Fixed panel viewports: content does not resize the user's chosen tool area.
use super::*;
use codisseu_core::ShellLayout;

#[derive(Clone, Copy)]
pub(super) enum ShellSide {
    Bottom,
    Left,
    Right,
}

impl ShellSide {
    fn axis(self) -> usize {
        match self {
            Self::Bottom => 1,
            Self::Left | Self::Right => 0,
        }
    }
    fn salt(self) -> &'static str {
        match self {
            Self::Bottom => "codisseu.activity",
            Self::Left => "codisseu.project",
            Self::Right => "codisseu.database",
        }
    }
}

impl CodisseuApp {
    pub(super) fn prepare_shell_size(&self, ui: &egui::Ui, side: ShellSide) {
        let layout = self.service.shell_layout();
        let size = match side {
            ShellSide::Bottom => layout
                .bottom_height
                .unwrap_or(self.tokens.shell_bottom_height),
            ShellSide::Left => layout.left_width.unwrap_or(self.tokens.sidebar_width),
            ShellSide::Right => layout.right_width.unwrap_or(self.tokens.shell_right_width),
        };
        let id = ui.scope_id().with(side.salt());
        // Seed from the domain every frame, including after an asynchronous restore.
        // Temporary viewport clamps never replace the persisted preference.
        ui.ctx().data_mut(|data| {
            let mut state = data
                .get_persisted::<egui::containers::panel::PanelState>(id)
                .unwrap_or(egui::containers::panel::PanelState {
                    outer_rect: egui::Rect::ZERO,
                });
            state.outer_rect.max[side.axis()] = state.outer_rect.min[side.axis()] + size;
            data.insert_persisted(id, state);
        });
    }

    pub(super) fn remember_shell_size(&mut self, ui: &egui::Ui, side: ShellSide, rect: egui::Rect) {
        let resize_id = ui.scope_id().with(side.salt()).with("__resize");
        let resized = ui
            .read_response(resize_id)
            .is_some_and(|r| r.dragged() || r.drag_stopped());
        let accessible_resize = ui.input(|i| {
            [
                egui::accesskit::Action::Increment,
                egui::accesskit::Action::Decrement,
                egui::accesskit::Action::SetValue,
            ]
            .iter()
            .any(|a| i.num_accesskit_action_requests(resize_id, *a) > 0)
        });
        if resized || accessible_resize {
            let mut layout: ShellLayout = self.service.shell_layout();
            let size = rect.size()[side.axis()];
            match side {
                ShellSide::Bottom => layout.bottom_height = Some(size.clamp(160., 500.)),
                ShellSide::Left => layout.left_width = Some(size.clamp(150., 500.)),
                ShellSide::Right => layout.right_width = Some(size.clamp(220., 600.)),
            }
            if layout != self.service.shell_layout() {
                self.send(Command::SetShellLayout(layout));
            }
        }
    }
}

pub(super) fn bounded_panel(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    let rect = ui.available_rect_before_wrap();
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("bounded-panel")
            .max_rect(rect),
    );
    child.shrink_clip_rect(rect);
    content(&mut child);
    // Allocate only the chosen viewport, irrespective of the content's minimum rect.
    ui.allocate_rect(rect, egui::Sense::hover());
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Pos2, Rect, Vec2, accesskit::Role};
    use egui_kittest::{Harness, kittest::Queryable};
    use std::time::{Duration, Instant};
    type H = Harness<'static, CodisseuApp>;
    fn fixture(path: &std::path::Path, restore: bool) -> H {
        let mut service = AppService::new(path.into());
        if restore {
            service.dispatch(Command::RestoreSession).unwrap();
        } else {
            service
                .dispatch(Command::SetCommitToolWindowVisible(true))
                .unwrap();
        }
        let mut app = CodisseuApp::new(service);
        app.show_activity = true;
        let mut h = Harness::builder()
            .with_size([1280., 800.])
            .with_max_steps(40)
            .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
        settle(&mut h);
        h
    }
    fn settle(h: &mut H) {
        let deadline = Instant::now() + Duration::from_secs(10);
        // These panels intentionally repaint periodically. Advance real frames
        // until their independent workers finish and layout has had eight idle
        // frames; Harness::run would fast-forward into the next monitor refresh.
        let mut idle_frames = 0;
        while idle_frames < 8 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
            h.step();
            if h.state().service.is_busy()
                || h.state().service.docker.is_busy()
                || h.state().git_insights.is_busy()
            {
                idle_frames = 0;
            } else {
                idle_frames += 1;
            }
        }
        assert!(
            h.state().service.last_error.is_none(),
            "{:?}",
            h.state().service.last_error
        );
    }
    fn splitter(h: &H, side: ShellSide) -> Rect {
        let mut vertical: Vec<_> = h
            .get_all_by_role(Role::Splitter)
            .map(|n| n.rect())
            .filter(|r| r.height() > r.width())
            .collect();
        vertical.sort_by(|a, b| a.left().total_cmp(&b.left()));
        match side {
            ShellSide::Left => vertical[0],
            ShellSide::Right => *vertical.last().unwrap(),
            ShellSide::Bottom => h
                .get_all_by_role(Role::Splitter)
                .map(|n| n.rect())
                .find(|r| r.width() > r.height())
                .unwrap(),
        }
    }
    fn drag(h: &mut H, point: Pos2, delta: Vec2) {
        h.hover_at(point);
        h.drag_at(point);
        h.step();
        h.hover_at(point + delta);
        h.step();
        h.drop_at(point + delta);
        settle(h);
    }
    fn close(a: f32, b: f32) {
        assert!((a - b).abs() <= 1., "{a} != {b}");
    }
    #[test]
    fn shared_dimensions_survive_switch_hide_restore_and_viewport_clamp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        let mut h = fixture(&path, false);
        let workspace = dir.path().join("project");
        std::fs::create_dir(&workspace).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "--quiet"])
                .arg(&workspace)
                .status()
                .unwrap()
                .success()
        );
        h.state_mut()
            .service
            .dispatch(Command::OpenWorkspace(workspace))
            .unwrap();
        settle(&mut h);
        for (side, delta) in [
            (ShellSide::Bottom, egui::vec2(0., 65.)),
            (ShellSide::Left, egui::vec2(65., 0.)),
            (ShellSide::Right, egui::vec2(-55., 0.)),
        ] {
            let point = splitter(&h, side).center();
            drag(&mut h, point, delta);
        }
        let layout = h.state().service.shell_layout();
        assert!(
            layout.bottom_height.is_some()
                && layout.left_width.is_some()
                && layout.right_width.is_some(),
            "{layout:?}"
        );
        let bottom = splitter(&h, ShellSide::Bottom).center().y;
        let left = splitter(&h, ShellSide::Left).center().x;
        let right = splitter(&h, ShellSide::Right).center().x;
        for label in [
            "Git",
            "Debug",
            "Terminal",
            "Services",
            "Atividade",
            "Referências",
            "Docker",
        ] {
            h.get_by_role_and_label(Role::Tab, label).click();
            settle(&mut h);
            close(splitter(&h, ShellSide::Bottom).center().y, bottom);
        }
        for label in [
            "Docker",
            "Alternar painel Projeto",
            "Commit · Ctrl+K",
            "Alternar painel Projeto",
        ] {
            h.get_by_role_and_label(Role::Button, label).click();
            settle(&mut h);
            close(splitter(&h, ShellSide::Left).center().x, left);
        }
        for label in ["AI Chat", "Alternar painel Database"] {
            h.get_by_label(label).click();
            settle(&mut h);
            close(splitter(&h, ShellSide::Right).center().x, right);
        }
        for (label, side, expected) in [
            ("Alternar painel Projeto", ShellSide::Left, left),
            ("Alternar painel Database", ShellSide::Right, right),
        ] {
            h.get_by_label(label).click();
            settle(&mut h);
            h.get_by_label(label).click();
            settle(&mut h);
            close(splitter(&h, side).center().x, expected);
        }
        h.get_by_label("Ocultar painel inferior").click();
        settle(&mut h);
        h.get_by_label("Mostrar Problemas").click();
        settle(&mut h);
        close(splitter(&h, ShellSide::Bottom).center().y, bottom);
        assert_eq!(h.state().service.shell_layout(), layout);
        h.set_size(egui::vec2(800., 400.));
        settle(&mut h);
        assert_eq!(h.state().service.shell_layout(), layout);
        h.set_size(egui::vec2(1280., 800.));
        settle(&mut h);
        close(splitter(&h, ShellSide::Bottom).center().y, bottom);
        h.state_mut()
            .service
            .dispatch(Command::PersistSession)
            .unwrap();
        settle(&mut h);
        drop(h);
        let h = fixture(&path, true);
        assert_eq!(h.state().service.shell_layout(), layout);
        close(splitter(&h, ShellSide::Bottom).center().y, bottom);
        close(splitter(&h, ShellSide::Left).center().x, left);
        close(splitter(&h, ShellSide::Right).center().x, right);
    }
    #[test]
    fn encoding_respects_right_rail_without_and_with_document() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = fixture(&dir.path().join("session.json"), false);
        let right = 1280. - h.state().tokens.rail_width - 12.;
        assert!(h.get_by_label("UTF-8").rect().right() <= right);
        let path = dir.path().join("probe.php");
        std::fs::write(&path, "<?php // ação\n").unwrap();
        h.state_mut()
            .service
            .dispatch(Command::OpenFile(path))
            .unwrap();
        settle(&mut h);
        assert!(h.get_by_label("UTF-8").rect().right() <= right);
    }
}
