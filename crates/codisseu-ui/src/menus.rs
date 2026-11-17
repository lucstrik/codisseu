//! Menu geometry and scoped form styling. Source mapping: docs/visual-spec-v4.md.
use super::*;

fn menu_item(ui: &mut egui::Ui, accessible: &str, enabled: bool) -> egui::Response {
    let (label, shortcut) = accessible.split_once("  ").unwrap_or((accessible, ""));
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(label)
            .shortcut_text(shortcut.trim())
            .min_size(egui::vec2(260.0, 24.0)),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::accesskit::Role::Button,
            ui.is_enabled() && enabled,
            accessible,
        )
    });
    response
}

impl Tokens {
    pub(super) fn dialog_frame(&self, ctx: &egui::Context) -> egui::Frame {
        egui::Frame::window(&ctx.global_style()).inner_margin(self.dialog_content_inset)
    }

    pub(super) fn dialog_header(
        &self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        width: f32,
        title: &str,
    ) {
        self.form_style(ui);
        let available =
            (ctx.content_rect().width() - 2. * f32::from(self.dialog_content_inset) - 32.)
                .max(120.);
        ui.set_width(width.min(available));
        ui.add(egui::Label::new(RichText::new(title).strong()).wrap());
        ui.separator();
    }

    pub(super) fn dialog_footer<R>(
        &self,
        ui: &mut egui::Ui,
        actions: impl FnOnce(&mut egui::Ui) -> R,
    ) -> R {
        ui.add_space(4.);
        ui.separator();
        ui.with_layout(
            egui::Layout::right_to_left(egui::Align::Center).with_main_wrap(true),
            actions,
        )
        .inner
    }

    pub(super) fn menu_style(&self, style: &mut egui::Style, bar: bool) {
        style.spacing.interact_size.y = if bar {
            self.main_button_size
        } else {
            self.menu_row_height
        };
        style.spacing.item_spacing = egui::vec2(0.0, 0.0);
        style.spacing.button_padding = egui::vec2(if bar { 10.0 } else { 12.0 }, 4.0);
        style.spacing.menu_margin = egui::Margin::symmetric(1, 6);
        style.visuals.widgets.noninteractive.bg_stroke =
            egui::Stroke::new(1.0, color(self.popup_border));
        style.visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
        for state in [
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
            &mut style.visuals.widgets.open,
        ] {
            state.weak_bg_fill = color(if bar { self.border } else { self.selection });
            state.bg_stroke = egui::Stroke::NONE;
            state.expansion = 0.0;
            state.corner_radius = self.control_corner_radius.into();
        }
        // Keyboard focus must remain visible independently of pointer hover.
        style.visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, color(self.accent));
    }

    pub(super) fn form_style(&self, ui: &mut egui::Ui) {
        let style = ui.style_mut();
        style.spacing.interact_size.y = self.control_height;
        style.spacing.button_padding = egui::vec2(12.0, 5.0);
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.visuals.text_edit_bg_color = Some(color(self.panel));
        style.visuals.selection.stroke = egui::Stroke::new(2.0, color(self.accent));
        for state in [
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
            &mut style.visuals.widgets.open,
        ] {
            state.weak_bg_fill = color(self.panel);
            state.bg_stroke = egui::Stroke::new(1.0, color(self.control_border));
            state.corner_radius = self.control_corner_radius.into();
            state.expansion = 0.0;
        }
        style.visuals.widgets.hovered.weak_bg_fill = color(self.border);
        style.visuals.widgets.active.bg_stroke = egui::Stroke::new(2.0, color(self.accent));
    }

    pub(super) fn dialog_button(&self, label: &str, primary: bool) -> egui::Button<'static> {
        let text = RichText::new(label.to_owned());
        let button = egui::Button::new(if primary {
            text.color(Color32::WHITE)
        } else {
            text
        })
        .min_size(egui::vec2(
            self.dialog_button_min_width,
            self.control_height,
        ));
        if primary {
            button.fill(color(self.accent))
        } else {
            button
        }
    }
}

impl CodisseuApp {
    pub(super) fn main_menus(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens.clone();
        let popup = tokens.clone();
        let width = ui.fonts_mut(|fonts| {
            ["Arquivo", "Editar", "Exibir", "Navegar"]
                .iter()
                .map(|label| {
                    fonts
                        .layout_no_wrap(
                            (*label).into(),
                            FontId::proportional(tokens.ui_font_size),
                            color(tokens.text),
                        )
                        .size()
                        .x
                        + 20.0
                })
                .sum::<f32>()
        });
        ui.allocate_ui_with_layout(
            egui::vec2(width, tokens.main_button_size),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                egui::MenuBar::new()
                    .style(move |style: &mut egui::Style| tokens.menu_style(style, true))
                    .config(
                        egui::menu::MenuConfig::new()
                            .style(move |style: &mut egui::Style| popup.menu_style(style, false)),
                    )
                    .ui(ui, |ui| {
                        ui.menu_button("Arquivo", |ui| {
                            if menu_item(ui, "Nova janela      Ctrl+Shift+N", true).clicked() {
                                self.open_new_window();
                                ui.close();
                            }
                            if menu_item(ui, "Novo arquivo…    Ctrl+N", true).clicked() {
                                self.send(Command::NewUntitled);
                                self.focus_editor = true;
                                ui.close();
                            }
                            if menu_item(ui, "Abrir pasta…     Ctrl+Shift+O", true).clicked() {
                                self.open_path(PathAction::Workspace);
                                ui.close();
                            }
                            if menu_item(ui, "Abrir arquivo…   Ctrl+O", true).clicked() {
                                self.open_path(PathAction::File);
                                ui.close();
                            }
                            if menu_item(
                                ui,
                                "Salvar           Ctrl+S",
                                self.service.active.is_some()
                                    || self
                                        .api_ui
                                        .active_in_view(self.service.active_view().unwrap_or(0)),
                            )
                            .clicked()
                            {
                                self.active_command("save");
                                ui.close();
                            }
                            if menu_item(
                                ui,
                                "Fechar aba       Ctrl+W",
                                self.service.active.is_some()
                                    || self
                                        .api_ui
                                        .active_in_view(self.service.active_view().unwrap_or(0)),
                            )
                            .clicked()
                            {
                                self.active_command("close");
                                ui.close();
                            }
                            ui.separator();
                            ui.menu_button("Local History", |ui| {
                                let enabled = self
                                    .service
                                    .active_document()
                                    .is_some_and(|d| !d.is_untitled());
                                if menu_item(ui, "Show History", enabled).clicked() {
                                    self.open_file_history(false);
                                    ui.close();
                                }
                                if menu_item(ui, "Put Label…", enabled).clicked() {
                                    self.open_file_history(true);
                                    ui.close();
                                }
                            });
                            ui.separator();
                            ui.horizontal(|ui| {
                                if icon_button(ui, Icon::Folder, "Abrir pasta", false, 30.)
                                    .clicked()
                                {
                                    self.open_path(PathAction::Workspace);
                                    ui.close();
                                }
                                if icon_button(ui, Icon::File, "Abrir arquivo", false, 30.)
                                    .clicked()
                                {
                                    self.open_path(PathAction::File);
                                    ui.close();
                                }
                                if ui
                                    .add_enabled_ui(
                                        self.service.active.is_some()
                                            || self.api_ui.active_in_view(
                                                self.service.active_view().unwrap_or(0),
                                            ),
                                        |ui| icon_button(ui, Icon::Save, "Salvar", false, 30.),
                                    )
                                    .inner
                                    .clicked()
                                {
                                    self.active_command("save");
                                    ui.close();
                                }
                            });
                            ui.separator();
                            if menu_item(ui, "Sair", true).clicked() {
                                self.request_exit();
                                ui.close();
                            }
                        });
                        ui.menu_button("Git", |ui| {
                            self.git_insights_menu(ui);
                            ui.separator();
                            if ui.button("Grafo de commits").clicked() {
                                self.git_open_log();
                                ui.close();
                            }
                            if ui.button("Source Control").clicked() {
                                self.git_open_source_control();
                                ui.close();
                            }
                        });
                        ui.menu_button("Editar", |ui| {
                            if self
                                .api_ui
                                .active_in_view(self.service.active_view().unwrap_or(0))
                            {
                                ui.disable();
                            }
                            self.editor_actions_menu(ui);
                            ui.separator();
                            let undo = self
                                .service
                                .active
                                .is_some_and(|id| self.service.can_undo_document(id));
                            let redo = self
                                .service
                                .active
                                .is_some_and(|id| self.service.can_redo_document(id));
                            if menu_item(ui, "Desfazer         Ctrl+Z", undo).clicked() {
                                self.active_command("undo");
                                ui.close();
                            }
                            if menu_item(ui, "Refazer          Ctrl+Shift+Z", redo).clicked() {
                                self.active_command("redo");
                                ui.close();
                            }
                            ui.separator();
                            if menu_item(
                                ui,
                                "Renomear símbolo…    Shift+F6",
                                self.service.active.is_some(),
                            )
                            .clicked()
                            {
                                self.request_rename();
                                ui.close();
                            }
                            if menu_item(
                                ui,
                                "Renomear arquivo…",
                                self.service.active_document().is_some(),
                            )
                            .clicked()
                            {
                                if let Some(document) = self.service.active_document() {
                                    self.request_file_rename(document.path.clone());
                                }
                                ui.close();
                            }
                            if menu_item(
                                ui,
                                "Completar código    Ctrl+Space",
                                self.service.active.is_some(),
                            )
                            .clicked()
                            {
                                self.request_completion();
                                ui.close();
                            }
                            if menu_item(
                                ui,
                                "Localizar        Ctrl+F",
                                self.service.active.is_some(),
                            )
                            .clicked()
                            {
                                self.open_search(false);
                                ui.close();
                            }
                            if menu_item(
                                ui,
                                "Substituir       Ctrl+H",
                                self.service.active.is_some(),
                            )
                            .clicked()
                            {
                                self.open_search(true);
                                ui.close();
                            }
                            ui.separator();
                            if menu_item(ui, "Configurações…    Ctrl+Alt+S", true).clicked() {
                                self.preferences_dialog = true;
                                ui.close();
                            }
                        });
                        ui.menu_button("Exibir", |ui| {
                            let mut soft_wrap = self.service.editor_preferences().soft_wrap;
                            if ui
                                .checkbox(&mut soft_wrap, "Quebra automática de linha    Alt+Z")
                                .changed()
                            {
                                self.send(Command::SetEditorSoftWrap(soft_wrap));
                                self.focus_editor = true;
                                ui.close();
                            }
                            ui.separator();
                            if menu_item(ui, "Navegador integrado…", true).clicked() {
                                self.open_browser_url("about:blank");
                                ui.close();
                            }
                            ui.separator();
                            if menu_item(ui, "Terminal    Alt+F12", true).clicked() {
                                self.toggle_terminal();
                                ui.close();
                            }
                            ui.separator();
                            if menu_item(
                                ui,
                                "Informações de parâmetros    Ctrl+P",
                                self.service.active.is_some(),
                            )
                            .clicked()
                            {
                                self.request_signature(
                                    codisseu_core::SignatureHelpTrigger::Invoked,
                                    false,
                                );
                                ui.close();
                            }
                            if menu_item(
                                ui,
                                "Documentação do símbolo    Ctrl+Q",
                                self.service.active.is_some(),
                            )
                            .clicked()
                            {
                                self.request_documentation();
                                ui.close();
                            }
                            ui.separator();
                            if menu_item(ui, "Dividir à direita", self.service.active.is_some())
                                .clicked()
                            {
                                self.split_editor(false);
                                ui.close();
                            }
                            if menu_item(ui, "Dividir abaixo", self.service.active.is_some())
                                .clicked()
                            {
                                self.split_editor(true);
                                ui.close();
                            }
                            if menu_item(
                                ui,
                                "Alternar grupo    Ctrl+Alt+Tab",
                                self.service.document_views().len() > 1,
                            )
                            .clicked()
                            {
                                self.next_editor_group();
                                ui.close();
                            }
                            if menu_item(
                                ui,
                                "Fechar divisão",
                                self.service.document_views().len() > 1,
                            )
                            .clicked()
                            {
                                self.close_editor_group();
                                ui.close();
                            }
                            ui.separator();
                            ui.checkbox(&mut self.show_project, "Painel do projeto");
                            ui.checkbox(&mut self.show_activity, "Painel de atividade");
                            ui.checkbox(&mut self.show_database, "Painel Database");
                        });
                        ui.menu_button("Executar", |ui| {
                            if menu_item(ui, "Python · Executar e ambiente…", true).clicked() {
                                self.open_python(codisseu_core::python_project::Action::RunFile);
                                ui.close();
                            }
                            if menu_item(ui, "Testes Python · pytest/unittest…", true).clicked()
                            {
                                self.open_python_tests(true);
                                ui.close();
                            }
                            ui.separator();
                            if menu_item(ui, "Testes JavaScript/TypeScript · Vitest/Jest…", true)
                                .clicked()
                            {
                                self.open_js_tests(true);
                                ui.close();
                            }

                            if menu_item(
                                ui,
                                "Testes PHP · PHPUnit/Pest…",
                                self.service.workspace.is_some(),
                            )
                            .clicked()
                            {
                                self.open_php_tests(true);
                                ui.close();
                            }
                            if menu_item(
                                ui,
                                "Executar arquivo de testes PHP",
                                self.service.active_document().is_some_and(|d| {
                                    d.path.extension().is_some_and(|e| e == "php")
                                }),
                            )
                            .clicked()
                            {
                                self.run_php_test_file();
                                ui.close();
                            }
                            ui.separator();
                            if menu_item(
                                ui,
                                "Rust · Cargo…    Ctrl+F9",
                                self.service.workspace.is_some(),
                            )
                            .clicked()
                            {
                                self.open_cargo(codisseu_core::rust_project::CargoAction::Build);
                                ui.close();
                            }
                            ui.separator();
                            if menu_item(ui, "Debug…    Shift+F9", true).clicked() {
                                self.debugger_start_selected();
                                ui.close();
                            }
                            if menu_item(ui, "Configurações de execução/debug…", true).clicked()
                            {
                                self.debugger_open_configurations();
                                ui.close();
                            }
                            ui.separator();
                            if menu_item(ui, "Mostrar Debug    Alt+5", true).clicked() {
                                self.tool_tab = ToolTab::Debug;
                                self.show_activity = true;
                                ui.close();
                            }
                            if menu_item(
                                ui,
                                "Parar todas as sessões de debug",
                                self.service.debugger_has_sessions(),
                            )
                            .clicked()
                            {
                                self.service.stop_debugger();
                                ui.close();
                            }
                        });
                        ui.menu_button("Navegar", |ui| {
                            if menu_item(ui, "Pesquisar arquivos…    Duplo Shift", true).clicked()
                            {
                                self.open_project_search();
                                ui.close();
                            }
                            ui.separator();
                            if menu_item(ui, "Ir à definição    F12", self.service.active.is_some())
                                .clicked()
                            {
                                self.go_to_definition();
                                ui.close();
                            }
                            if menu_item(
                                ui,
                                "Ir à implementação    Ctrl+F12",
                                self.service.active.is_some(),
                            )
                            .clicked()
                            {
                                self.go_to_implementation();
                                ui.close();
                            }
                            if menu_item(
                                ui,
                                "Ir ao supermétodo/supertipo    Ctrl+Alt+U",
                                self.service.active_document().is_some_and(|d| {
                                    codisseu_core::Language::for_path(&d.path)
                                        == Some(codisseu_core::Language::Php)
                                }),
                            )
                            .clicked()
                            {
                                self.go_to_super_definition();
                                ui.close();
                            }
                            if menu_item(
                                ui,
                                "Ir ao tipo    Ctrl+Shift+B",
                                self.service.active.is_some(),
                            )
                            .clicked()
                            {
                                self.go_to_type_definition();
                                ui.close();
                            }
                            if menu_item(
                                ui,
                                "Buscar referências    Shift+F12",
                                self.service.active.is_some(),
                            )
                            .clicked()
                            {
                                self.request_references();
                                ui.close();
                            }
                            ui.separator();
                            if menu_item(
                                ui,
                                "Voltar    Ctrl+Alt+Left",
                                self.service.navigation.can_back(),
                            )
                            .clicked()
                            {
                                self.navigate_history(false);
                                ui.close();
                            }
                            if menu_item(
                                ui,
                                "Avançar    Ctrl+Alt+Right",
                                self.service.navigation.can_forward(),
                            )
                            .clicked()
                            {
                                self.navigate_history(true);
                                ui.close();
                            }
                        });
                    });
            },
        );
    }
}
