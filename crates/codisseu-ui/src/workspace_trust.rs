//! Project trust prompt, implemented independently from the observed PhpStorm dialog.
use super::*;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrustChoice {
    Trust,
    Preview,
    Configure,
    Close,
}

impl CodisseuApp {
    pub(super) fn workspace_trust_dialog(&mut self, ctx: &egui::Context) {
        if !self.service.provider_startup().needs_trust || self.provider_dialog {
            return;
        }
        let path = self
            .service
            .workspace
            .as_deref()
            .unwrap_or_else(|| Path::new(""));
        match project_trust_modal(ctx, &self.tokens, path) {
            Some(TrustChoice::Trust) => self.send(Command::StartProviders),
            Some(TrustChoice::Preview) => self.send(Command::StopProviders),
            Some(TrustChoice::Configure) => self.provider_dialog = true,
            Some(TrustChoice::Close) => self.send(Command::CloseWorkspace),
            None => {}
        }
    }
}

fn project_trust_modal(ctx: &egui::Context, tokens: &Tokens, path: &Path) -> Option<TrustChoice> {
    let mut choice = None;
    let width = 603_f32.min((ctx.content_rect().width() - 72.).max(220.));
    let frame = egui::Frame::popup(&ctx.global_style())
        .fill(color(tokens.bar))
        .inner_margin(18.);
    let modal = egui::Modal::new(egui::Id::unique("codisseu.providers.trust"))
        .frame(frame)
        .show(ctx, |ui| {
            tokens.form_style(ui);
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 8.;
            ui.add_space(8.);
            ui.allocate_ui_with_layout(
                egui::vec2(width, 106.),
                egui::Layout::left_to_right(egui::Align::Min),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 0.;
                    let (icon, _) = ui.allocate_exact_size(egui::vec2(28., 28.), egui::Sense::hover());
                    let center = icon.center();
                    let stroke = egui::Stroke::new(1.5, color(tokens.database_completion_key));
                    ui.painter().add(egui::Shape::convex_polygon(vec![
                        center + egui::vec2(0., -11.), center + egui::vec2(12., 10.),
                        center + egui::vec2(-12., 10.)], stroke.color, egui::Stroke::NONE));
                    ui.painter().line_segment([center+egui::vec2(0.,-4.),center+egui::vec2(0.,3.)],egui::Stroke::new(1.5,color(tokens.bar)));
                    ui.painter().circle_filled(center+egui::vec2(0.,6.),1.,color(tokens.bar));
                    ui.add_space(19.);
                    ui.vertical(|ui| {
                        ui.set_width((width - 47.).max(172.));
                        let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
                        ui.add(egui::Label::new(RichText::new(format!("Trust and Open Project ‘{name}’?"))
                            .strong().size(14.)).truncate())
                            .on_hover_text(path.display().to_string());
                        ui.add_space(3.);
                        ui.label("Language servers may load project settings and plugins. Trust this project only if you know its source.");
                        ui.add(egui::Label::new(RichText::new(path.display().to_string())
                            .small().color(color(tokens.muted))).truncate())
                            .on_hover_text(path.display().to_string());
                    });
                },
            );
            ui.add_space(12.);
            if width >= 450. {
                ui.horizontal(|ui| {
                    if ui.link("Provider Settings…").clicked() {
                        choice = Some(TrustChoice::Configure);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        trust_buttons(ui, tokens, &mut choice);
                    });
                });
            } else {
                ui.vertical(|ui| {
                    trust_buttons(ui, tokens, &mut choice);
                    if ui.link("Provider Settings…").clicked() {
                        choice = Some(TrustChoice::Configure);
                    }
                });
            }
        });
    // Dismissal always preserves the untrusted state. Clicking outside the prompt
    // does not authorize anything or dismiss the decision.
    if modal.is_top_modal
        && !modal.any_popup_open
        && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        choice = Some(TrustChoice::Preview);
    }
    choice
}

fn trust_buttons(ui: &mut egui::Ui, tokens: &Tokens, choice: &mut Option<TrustChoice>) {
    let buttons = if ui.layout().main_dir() == egui::Direction::RightToLeft {
        [
            ("Don't Open", 105., false, TrustChoice::Close),
            ("Preview in Safe Mode", 171., false, TrustChoice::Preview),
            ("Trust Project", 113., true, TrustChoice::Trust),
        ]
    } else {
        [
            ("Trust Project", 113., true, TrustChoice::Trust),
            ("Preview in Safe Mode", 171., false, TrustChoice::Preview),
            ("Don't Open", 105., false, TrustChoice::Close),
        ]
    };
    for (label, width, primary, value) in buttons {
        if ui
            .add_sized([width, 34.], tokens.dialog_button(label, primary))
            .clicked()
        {
            *choice = Some(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    fn capture<T: 'static>(h: &mut Harness<'_, T>, name: &str) {
        if let Some(directory) = std::env::var_os("CODISSEU_WORKSPACE_WIDGET_CAPTURES") {
            let directory = PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            h.render()
                .unwrap()
                .save(directory.join(format!("{name}.png")))
                .unwrap();
        }
    }

    fn harness(size: [f32; 2]) -> Harness<'static, Option<TrustChoice>> {
        let tokens = Tokens::load();
        let mut styled = false;
        Harness::builder().with_size(size).build_ui_state(
            move |ui, choice| {
                if !styled {
                    tokens.apply(ui.ctx());
                    styled = true;
                }
                if let Some(result) =
                    project_trust_modal(ui.ctx(), &tokens, Path::new("/tmp/monolito"))
                {
                    *choice = Some(result);
                }
            },
            None,
        )
    }

    #[test]
    fn trust_modal_buttons_make_distinct_explicit_choices() {
        for (label, expected) in [
            ("Trust Project", TrustChoice::Trust),
            ("Preview in Safe Mode", TrustChoice::Preview),
            ("Provider Settings…", TrustChoice::Configure),
            ("Don't Open", TrustChoice::Close),
        ] {
            let mut h = harness([1000., 700.]);
            h.run();
            h.get_by_label(label).click();
            h.run();
            assert_eq!(*h.state(), Some(expected));
        }
    }

    #[test]
    fn trust_modal_is_centered_and_buttons_remain_visible_in_small_windows() {
        for size in [[1440., 900.], [520., 500.], [360., 500.]] {
            let mut h = harness(size);
            h.run();
            capture(&mut h, &format!("trust-{}", size[0]));
            for label in [
                "Trust Project",
                "Preview in Safe Mode",
                "Don't Open",
                "Provider Settings…",
            ] {
                let bounds = h.get_by_label(label).rect();
                assert!(
                    bounds.left() >= 0. && bounds.right() <= size[0],
                    "{label}: {bounds:?}"
                );
                assert!(
                    bounds.top() >= 0. && bounds.bottom() <= size[1],
                    "{label}: {bounds:?}"
                );
            }
            h.key_press(egui::Key::Escape);
            h.run();
            assert_eq!(*h.state(), Some(TrustChoice::Preview));
        }
    }

    #[test]
    fn empty_database_cards_open_configuration_without_connecting() {
        let dir = tempfile::tempdir().unwrap();
        let mut service = AppService::new(dir.path().join("session.json"));
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while service.remote_db.is_busy() {
            service.poll();
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let app = CodisseuApp::new(service);
        let mut h = Harness::builder().with_size([300., 640.]).build_ui_state(
            |ui, app: &mut CodisseuApp| {
                if !app.styled {
                    app.tokens.apply(ui.ctx());
                    app.styled = true;
                }
                app.remote_database_panel(ui);
            },
            app,
        );
        h.run();
        let pg = h.get_by_label("PostgreSQL").rect();
        let mysql = h.get_by_label("MySQL").rect();
        capture(&mut h, "database-empty");
        assert_eq!(pg.height(), 34.);
        assert_eq!(mysql.top() - pg.top(), 42.);
        assert!(pg.left() >= 12. && pg.right() <= 300.);
        h.get_by_label("PostgreSQL").click();
        h.run();
        assert!(h.state().database_ui.dialog);
        assert!(h.state().service.remote_db.active.is_none());
        assert!(!h.state().service.remote_db.is_busy());
        assert!(h.query_by_label("Google Cloud Explorer").is_none());
    }

    #[test]
    fn empty_git_menu_has_only_applicable_working_actions() {
        let dir = tempfile::tempdir().unwrap();
        let app = CodisseuApp::new(AppService::new(dir.path().join("session.json")));
        let mut h = Harness::builder().with_size([700., 500.]).build_ui_state(
            |ui, app: &mut CodisseuApp| {
                if !app.styled {
                    app.tokens.apply(ui.ctx());
                    app.styled = true;
                }
                app.git_branch_menu(ui);
            },
            app,
        );
        h.run();
        h.get_by_label("Git Branches").click();
        h.run();
        capture(&mut h, "git-empty");
        assert!(h.query_by_label("Create Git Repository…").is_some());
        assert!(h.query_by_label("Clone Repository…").is_some());
        assert!(h.query_by_label("Search branches and actions").is_none());
        assert!(h.query_by_label("Push…").is_none());
        h.get_by_label("Create Git Repository…").click();
        h.run();
        assert!(h.state().git_ui.dialog.is_some());
        assert!(!dir.path().join(".git").exists());
        assert!(h.state().service.git.repo.is_none());
    }
}
