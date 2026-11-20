//! Window chrome: measurements in docs/m3-shell-reference.md, own vector artwork.
use super::*;

impl CodisseuApp {
    pub(super) fn main_toolbar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("codisseu.main.toolbar")
            .exact_size(self.tokens.main_toolbar_height)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(color(self.tokens.bar)))
            .show(ui, |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), self.tokens.main_toolbar_height),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 0.;
                        let menu = icon_button(
                            ui,
                            Icon::Menu,
                            "Menu principal",
                            false,
                            self.tokens.rail_width,
                        );
                        if menu.clicked() {
                            self.main_menu_open = !self.main_menu_open;
                        }
                        if self.main_menu_open {
                            self.main_menus(ui);
                        }
                        if !self.main_menu_open {
                            ui.add_space(12.);
                            let name = if self.service.workspace.is_some() {
                                self.service.workspace_label()
                            } else {
                                "Abrir projeto".into()
                            };
                            let badge = ui
                                .allocate_exact_size(egui::vec2(20., 20.), egui::Sense::hover())
                                .0;
                            ui.painter()
                                .rect_filled(badge, 4., color(self.tokens.accent));
                            let initial = name
                                .chars()
                                .next()
                                .unwrap_or('C')
                                .to_uppercase()
                                .to_string();
                            ui.painter().text(
                                badge.center(),
                                egui::Align2::CENTER_CENTER,
                                initial,
                                FontId::proportional(12.),
                                Color32::WHITE,
                            );
                            ui.add_space(6.);
                            let width = ui.fonts_mut(|fonts| {
                                fonts
                                    .layout_no_wrap(
                                        name.clone(),
                                        FontId::proportional(13.),
                                        color(self.tokens.text),
                                    )
                                    .size()
                                    .x
                                    + 24.
                            });
                            let title = ui.add_sized(
                                [width.min((ui.available_width() - 250.).max(50.)), 30.],
                                egui::Button::new(RichText::new(format!("{name}  ⌄")).size(13.))
                                    .frame(false)
                                    .truncate(),
                            );
                            title.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::accesskit::Role::Button,
                                    true,
                                    "Projeto atual",
                                )
                            });
                            if title.clicked() {
                                self.open_path(PathAction::Workspace);
                            }
                            title.on_hover_text(
                                self.service
                                    .workspace
                                    .as_ref()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_else(|| "Abrir pasta · Ctrl+Shift+O".into()),
                            );
                        }
                        if !self.main_menu_open {
                            self.git_branch_menu(ui);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(5.);
                            if icon_button(ui, Icon::Settings, "Configurações", false, 30.)
                                .clicked()
                            {
                                self.preferences_dialog = true;
                            }
                            if icon_button(ui, Icon::Search, "Pesquisar arquivos", false, 30.)
                                .clicked()
                            {
                                self.open_project_search();
                            }
                            if icon_button(ui, Icon::Code, "Configurar providers", false, 30.)
                                .clicked()
                            {
                                self.provider_dialog = true;
                            }
                            self.debugger_toolbar(ui);
                            // Actions are enabled only when their service is available.
                            let sql_ready = self.can_execute_query();
                            let rust_ready = self.rust_build_enabled();
                            let python_ready = self.python_file().is_some();
                            if ui
                                .add_enabled_ui(sql_ready || rust_ready || python_ready, |ui| {
                                    icon_button(
                                        ui,
                                        Icon::Play,
                                        if python_ready {
                                            "Executar Python · Shift+F10"
                                        } else if rust_ready {
                                            "Executar Rust · Shift+F10"
                                        } else if self.service.active_document().is_some_and(|d| {
                                            d.path.extension().is_some_and(|e| e == "sql")
                                        }) {
                                            "Executar SQL"
                                        } else {
                                            "Executar consulta"
                                        },
                                        false,
                                        30.,
                                    )
                                })
                                .inner
                                .clicked()
                            {
                                if python_ready {
                                    self.open_python(
                                        codisseu_core::python_project::Action::RunFile,
                                    );
                                } else if rust_ready {
                                    self.open_cargo(codisseu_core::rust_project::CargoAction::Run);
                                } else {
                                    self.execute_sql();
                                }
                            }
                            self.rust_build_toolbar(ui);
                            if icon_button(
                                ui,
                                Icon::Browser,
                                "Abrir navegador integrado",
                                false,
                                30.,
                            )
                            .clicked()
                            {
                                self.open_browser_url("about:blank");
                            }
                            let git_has_local_cancel =
                                self.show_activity && self.tool_tab == ToolTab::Git;
                            let global_cancel = self.service.python_tests.is_busy()
                                || self.service.python.is_busy()
                                || self.service.js_tests.is_busy()
                                || self.service.php_tests.is_busy()
                                || self.service.docker.is_foreground_busy()
                                || self.service.non_git_work_pending()
                                || (self.service.git.is_busy()
                                    && !self.service.git.history_pending()
                                    && !git_has_local_cancel);
                            if global_cancel
                                && icon_button(ui, Icon::Stop, "Cancelar operações", false, 30.)
                                    .clicked()
                            {
                                self.send(Command::CancelAll);
                            }
                        });
                    },
                );
            });
    }
    pub(super) fn open_terminal(&mut self) {
        if self.exit_when_saved || self.confirm.is_some() {
            return;
        }
        if self.docker_terminal_active && self.docker_terminal.has_sessions() {
            self.docker_terminal.request_focus();
            self.tool_tab = ToolTab::Terminal;
            self.show_activity = true;
            self.focus_editor = false;
        } else {
            self.open_local_terminal();
        }
    }
    pub(super) fn open_local_terminal(&mut self) {
        if self.exit_when_saved || self.confirm.is_some() {
            return;
        }
        if self.service.workspace_file.is_some() && self.service.workspace_roots().is_empty() {
            self.service.status =
                "Nenhuma pasta deste workspace está disponível para abrir o terminal".into();
            return;
        }
        self.docker_terminal_active = false;
        let cwd = self
            .service
            .active_document()
            .and_then(|d| self.service.workspace_root_for(&d.path).map(PathBuf::from))
            .or_else(|| self.service.workspace_roots().first().cloned())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        self.terminal.open(&cwd);
        self.terminal.request_focus();
        self.tool_tab = ToolTab::Terminal;
        self.show_activity = true;
    }
    pub(super) fn toggle_terminal(&mut self) {
        if self.show_activity && self.tool_tab == ToolTab::Terminal {
            self.show_activity = false;
            self.focus_editor = true;
        } else {
            self.open_terminal();
        }
    }
    pub(super) fn tool_rails(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("codisseu.left.rail")
            .exact_size(self.tokens.rail_width)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(color(self.tokens.bar)))
            .resizable(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.;
                if icon_toggle_button(
                    ui,
                    Icon::Folder,
                    "Alternar painel Projeto",
                    self.show_project
                        && !self.git_ui.commit_open
                        && !self.docker_ui.open
                        && !self.api_ui.open,
                    self.tokens.rail_width,
                )
                .clicked()
                {
                    if self.drawings_ui.open {
                        self.drawings_ui.open = false;
                        self.show_project = true;
                    } else if self.api_ui.open {
                        self.api_ui.open = false;
                        self.git_ui.commit_open = false;
                        self.docker_ui.open = false;
                        self.show_project = true;
                    } else if self.docker_ui.open {
                        self.docker_ui.open = false;
                        self.git_ui.commit_open = false;
                        self.git_ui.source_control = false;
                        self.show_project = true;
                    } else if self.git_ui.commit_open {
                        self.git_ui.commit_open = false;
                        self.git_ui.source_control = false;
                        self.show_project = true;
                    } else {
                        self.show_project = !self.show_project;
                    }
                }
                if self.service.tool_window_preferences().commit
                    && icon_toggle_button(
                        ui,
                        Icon::Changes,
                        "Commit · Ctrl+K",
                        self.show_project && self.git_ui.commit_open && !self.git_ui.source_control,
                        self.tokens.rail_width,
                    )
                    .clicked()
                {
                    if self.show_project && self.git_ui.commit_open && !self.git_ui.source_control {
                        self.show_project = false;
                    } else {
                        self.git_open_commit();
                    }
                }
                if icon_toggle_button(
                    ui,
                    Icon::Branch,
                    "Source Control",
                    self.show_project && self.git_ui.source_control,
                    self.tokens.rail_width,
                )
                .clicked()
                {
                    if self.show_project && self.git_ui.source_control {
                        self.show_project = false;
                    } else {
                        self.git_open_source_control();
                    }
                }
                if icon_toggle_button(
                    ui,
                    Icon::Docker,
                    "Docker",
                    self.show_project && self.docker_ui.open,
                    self.tokens.rail_width,
                )
                .clicked()
                {
                    if self.show_project && self.docker_ui.open {
                        self.show_project = false;
                    } else {
                        self.api_ui.open = false;
                        self.drawings_ui.open = false;
                        self.docker_ui.open = true;
                        self.git_ui.commit_open = false;
                        self.git_ui.source_control = false;
                        self.show_project = true;
                        if !self.service.docker.is_busy() {
                            if let Err(error) = self.service.dispatch(Command::Docker(
                                codisseu_core::docker::DockerCommand::Refresh,
                            )) {
                                self.service.docker.state.error = Some(error);
                            }
                            self.docker_last_refresh = std::time::Instant::now();
                        }
                    }
                }
                if icon_toggle_button(
                    ui,
                    Icon::Api,
                    "APIs · Coleções e ambientes",
                    self.show_project && self.api_ui.open,
                    self.tokens.rail_width,
                )
                .clicked()
                {
                    if self.show_project && self.api_ui.open {
                        self.show_project = false;
                    } else {
                        self.drawings_ui.open = false;
                        self.api_ui.open = true;
                        self.docker_ui.open = false;
                        self.git_ui.commit_open = false;
                        self.git_ui.source_control = false;
                        self.show_project = true;
                    }
                }
                if icon_toggle_button(
                    ui,
                    Icon::Drawing,
                    "Desenhos · Excalidraw",
                    self.show_project && self.drawings_ui.open,
                    self.tokens.rail_width,
                )
                .clicked()
                {
                    self.show_project = !(self.show_project && self.drawings_ui.open);
                    self.drawings_ui.open = self.show_project;
                    self.api_ui.open = false;
                    self.docker_ui.open = false;
                    self.git_ui.commit_open = false;
                    self.git_ui.source_control = false;
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    ui.add_space(40.);
                    if icon_toggle_button(
                        ui,
                        Icon::Debug,
                        "Debug · Alt+5",
                        self.show_activity && self.tool_tab == ToolTab::Debug,
                        self.tokens.rail_width,
                    )
                    .clicked()
                    {
                        self.show_activity =
                            !(self.show_activity && self.tool_tab == ToolTab::Debug);
                        self.tool_tab = ToolTab::Debug;
                    }
                    if icon_toggle_button(
                        ui,
                        Icon::Branch,
                        "Git · Alt+9",
                        self.show_activity && self.tool_tab == ToolTab::Git,
                        self.tokens.rail_width,
                    )
                    .clicked()
                    {
                        if self.show_activity && self.tool_tab == ToolTab::Git {
                            self.show_activity = false;
                        } else {
                            self.git_open_log();
                        }
                    }
                    if icon_toggle_button(
                        ui,
                        Icon::Problems,
                        "Mostrar Problemas",
                        self.show_activity && self.tool_tab == ToolTab::Problems,
                        self.tokens.rail_width,
                    )
                    .clicked()
                    {
                        self.show_activity =
                            !(self.show_activity && self.tool_tab == ToolTab::Problems);
                        self.tool_tab = ToolTab::Problems;
                    }
                    if icon_toggle_button(
                        ui,
                        Icon::Terminal,
                        "Terminal",
                        self.show_activity && self.tool_tab == ToolTab::Terminal,
                        self.tokens.rail_width,
                    )
                    .clicked()
                    {
                        self.toggle_terminal();
                    }
                });
            });
        egui::Panel::right("codisseu.right.rail")
            .exact_size(self.tokens.rail_width)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(color(self.tokens.bar)))
            .resizable(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.;
                if icon_toggle_button(
                    ui,
                    Icon::Ai,
                    "AI Chat",
                    self.ai_chat.open,
                    self.tokens.rail_width,
                )
                .clicked()
                {
                    self.toggle_ai_chat();
                }
                if self.service.jira.profile.enabled
                    && icon_toggle_button(
                        ui,
                        Icon::Jira,
                        "Jira · Minhas tarefas",
                        self.jira_ui.open,
                        self.tokens.rail_width,
                    )
                    .clicked()
                {
                    self.toggle_jira();
                }
                if icon_toggle_button(
                    ui,
                    Icon::Database,
                    "Alternar painel Database",
                    self.show_database,
                    self.tokens.rail_width,
                )
                .clicked()
                {
                    self.show_database = !self.show_database;
                    if self.show_database {
                        self.ai_chat.open = false;
                        self.jira_ui.open = false;
                    }
                }
            });
    }
    pub(super) fn welcome(&mut self, ui: &mut egui::Ui) {
        let rect = ui.available_rect_before_wrap();
        let width = (rect.width() - 48.).clamp(220., 440.).min(rect.width());
        let top = rect.top() + ((rect.height() - 380.) / 2.).max(16.);
        let content = egui::Rect::from_min_max(
            egui::pos2(rect.center().x - width / 2., top),
            egui::pos2(rect.center().x + width / 2., rect.bottom() - 16.),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("codisseu.welcome")
                .auto_shrink([false, false])
                .codisseu()
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.;
                    ui.label(
                        RichText::new("Codisseu")
                            .size(32.)
                            .color(color(self.tokens.text)),
                    );
                    ui.label(
                        RichText::new("Seu espaço para desenvolver.")
                            .color(color(self.tokens.muted)),
                    );
                    ui.add_space(20.);
                    ui.label(
                        RichText::new("COMEÇAR")
                            .small()
                            .color(color(self.tokens.muted)),
                    );
                    for (icon, title, shortcut, action) in [
                        (
                            Icon::Folder,
                            "Abrir projeto",
                            "Ctrl+Shift+O",
                            PathAction::Workspace,
                        ),
                        (
                            Icon::Code,
                            "Abrir workspace…",
                            "",
                            PathAction::WorkspaceFile,
                        ),
                        (Icon::OpenFile, "Abrir arquivo", "Ctrl+O", PathAction::File),
                    ] {
                        if welcome_action(ui, icon, title, shortcut, &self.tokens).clicked() {
                            self.open_path(action);
                        }
                    }
                    ui.add_space(8.);
                    ui.separator();
                    ui.add_space(8.);
                    if welcome_action(
                        ui,
                        Icon::Search,
                        "Pesquisar arquivos",
                        "Duplo Shift",
                        &self.tokens,
                    )
                    .clicked()
                    {
                        self.open_project_search();
                    }
                    if welcome_action(
                        ui,
                        Icon::Settings,
                        "Configurações",
                        "Ctrl+Alt+S",
                        &self.tokens,
                    )
                    .clicked()
                    {
                        self.preferences_dialog = true;
                    }
                });
        });
    }
}

fn welcome_action(
    ui: &mut egui::Ui,
    icon: Icon,
    title: &str,
    shortcut: &str,
    tokens: &Tokens,
) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(
            ui.available_width(),
            if ui.available_width() < 340. {
                50.
            } else {
                38.
            },
        ),
        egui::Sense::hover(),
    );
    let response = ui.interact(
        rect,
        ui.make_persistent_id(("welcome.action", title)),
        egui::Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::accesskit::Role::Button,
            true,
            if shortcut.is_empty() {
                title.to_owned()
            } else {
                format!("{title}    {shortcut}")
            },
        )
    });
    if response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(rect, tokens.control_corner_radius, color(tokens.selection));
    }
    let tint = color(tokens.text);
    components::paint_icon_monochrome(
        ui,
        egui::Rect::from_center_size(
            rect.left_center() + egui::vec2(20., 0.),
            egui::vec2(16., 16.),
        ),
        icon,
        tint,
    );
    let compact = rect.width() < 340.;
    ui.painter().text(
        rect.left_center() + egui::vec2(40., if compact { -9. } else { 0. }),
        egui::Align2::LEFT_CENTER,
        title,
        FontId::proportional(tokens.ui_font_size),
        tint,
    );
    ui.painter().text(
        if compact {
            rect.left_center() + egui::vec2(40., 10.)
        } else {
            rect.right_center() - egui::vec2(12., 0.)
        },
        if compact {
            egui::Align2::LEFT_CENTER
        } else {
            egui::Align2::RIGHT_CENTER
        },
        shortcut,
        FontId::proportional(12.),
        color(tokens.muted),
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
