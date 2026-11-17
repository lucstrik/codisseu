//! Editor preferences share domain commands and session persistence with the app.
use super::*;

impl CodisseuApp {
    pub(super) fn preferences_window(&mut self, ctx: &egui::Context) {
        if !self.preferences_dialog {
            return;
        }
        let mut ligatures = self.service.editor_preferences().ligatures;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("codisseu.preferences")).show(ctx, |ui| {
            self.tokens.form_style(ui);
            ui.set_width(540.0);
            ui.heading("Configurações");
            ui.separator();
            egui::ScrollArea::vertical().id_salt("preferences.body").auto_shrink([false,false]).max_height((ctx.content_rect().height()-150.).clamp(150.,460.)).codisseu().show(ui,|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Editor").color(color(self.tokens.accent)));
                ui.label("› Geral");
            });
            ui.add_space(10.0);
            ui.strong("Salvamento automático");
            let prefs = self.service.editor_preferences();
            let mut auto_save = prefs.auto_save;
            let mut delay = prefs.auto_save_delay_ms;
            let toggle = ui.checkbox(&mut auto_save, "Salvar arquivos automaticamente");
            if toggle.clicked() { toggle.request_focus(); }
            let mut delay_changed = false;
            ui.add_enabled_ui(auto_save, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Após parar de editar por");
                    delay_changed = ui.add(egui::DragValue::new(&mut delay).range(250..=60_000).speed(100.).suffix(" ms")).changed();
                });
            });
            if toggle.changed() || delay_changed {
                self.send(Command::SetAutoSave { enabled: auto_save, delay_ms: delay });
            }
            ui.label("Novos arquivos precisam ser salvos uma vez para escolher o destino.");
            ui.add_space(12.);
            ui.strong("Linhas longas");
            let mut soft_wrap = self.service.editor_preferences().soft_wrap;
            let toggle = ui.checkbox(&mut soft_wrap, "Quebra automática de linha");
            if toggle.clicked() { toggle.request_focus(); }
            if toggle.changed() {
                self.send(Command::SetEditorSoftWrap(soft_wrap));
            }
            ui.label("Continua o texto na próxima linha visual, sem alterar o arquivo. Alt+Z no editor alterna esta opção.");
            ui.add_space(12.0);
            ui.strong("Fonte");
            ui.label(format!("JetBrains Mono · {}", self.tokens.editor_font_size));
            let toggle = ui.checkbox(&mut ligatures, "Usar ligaduras de fonte");
            if toggle.clicked() {
                toggle.request_focus();
            }
            if toggle.changed() {
                self.send(Command::SetEditorLigatures(ligatures));
            }
            ui.add_space(8.0);
            egui::Frame::new()
                .fill(color(self.tokens.editor))
                .inner_margin(12)
                .show(ui, |ui| {
                    ui.set_width(490.0);
                    editor::font_preview(ui, &self.tokens, ligatures);
                });
            ui.add_space(10.0);
            let mut python_hints=self.service.editor_preferences().python_inlay_hints;
            if ui.checkbox(&mut python_hints,"Python · Mostrar tipos e parâmetros no editor").changed(){self.send(Command::SetPythonInlayHints(python_hints));}
            let mut rust_hints=self.service.editor_preferences().rust_inlay_hints;
            if ui.checkbox(&mut rust_hints,"Rust · Mostrar tipos e parâmetros no editor").changed(){self.send(Command::SetRustInlayHints(rust_hints));}
            ui.separator();
            ui.strong("Integrações · Copilot");
            if ui.button("Configurar sugestões do Copilot…").clicked(){self.preferences_dialog=false;self.copilot_ui.settings=true;self.copilot_ui.program=self.service.editor_preferences().copilot_program.clone();}
            ui.label("Sugestões de código diretamente no editor, com acesso pela sua conta GitHub.");
            ui.add_space(10.);
            ui.strong("Integrações · Jira");
            let mut jira = self.service.jira.profile.enabled;
            let toggle = ui.add_enabled(self.service.jira.loaded,egui::Checkbox::new(&mut jira,"Mostrar Jira na barra lateral direita"));
            if toggle.clicked() { toggle.request_focus(); }
            if toggle.changed() { let mut p=self.service.jira.profile.clone();p.enabled=jira;let r=self.service.jira.set_profile(p);self.jira_result(r); }
            ui.label("Suas tarefas, histórias e horas no mesmo lugar do código.");
            if ui.add_enabled(jira,egui::Button::new("Configurar conexão Jira…")).clicked(){self.preferences_dialog=false;self.open_jira_settings();}
            ui.separator();
            ui.strong("Git · Ações");
            let mut confirm = self.service.tool_window_preferences().git_confirm_actions;
            if ui.checkbox(&mut confirm, "Confirmar ações Git antes de executar").changed() {
                self.send(Command::SetGitConfirmActions(confirm));
            }
            ui.label("Exibe uma revisão antes de aplicar commits, stage, push, pull e outras ações.");
            ui.add_space(8.);
            ui.strong("Git · Painéis");
            let mut commit = self.service.tool_window_preferences().commit;
            let toggle = ui.checkbox(&mut commit, "Mostrar painel Commit (estilo PhpStorm)");
            if toggle.clicked() {
                toggle.request_focus();
            }
            if toggle.changed() {
                self.send(Command::SetCommitToolWindowVisible(commit));
            }
            ui.label("Source Control (estilo VS Code) permanece disponível na barra lateral.");
            ui.add_space(8.);
            ui.strong("Git · autoria e histórico");
            self.git_insights_menu(ui);
            ui.collapsing("Créditos dos ícones Git", |ui| {
                ui.label("Codicons · Copyright Microsoft Corporation");
                ui.hyperlink_to("Fonte: microsoft/vscode-codicons · versão0.0.45", "https://github.com/microsoft/vscode-codicons");
                ui.hyperlink_to("Creative Commons Attribution4.0 International (CC BY4.0)", "https://creativecommons.org/licenses/by/4.0/");
                ui.label("Geometria preservada; cor adaptada para#c9c9ce e rasterização PNG transparente.");
            });
            ui.collapsing("Créditos das setas do editor e do terminal", |ui| {
                ui.label("Codicons © Microsoft Corporation · CC BY 4.0");
                ui.hyperlink_to("Fonte original: microsoft/vscode-codicons · 0.0.46-36", "https://github.com/microsoft/vscode-codicons");
                ui.hyperlink_to("Creative Commons Attribution 4.0 International", "https://creativecommons.org/licenses/by/4.0/");
                ui.label("Fonte original preservada; cor adaptada ao tema Codisseu.");
                ui.label("Terminal: Noto Sans © Google, SIL OFL 1.1; DejaVu Sans Mono © Bitstream / autores DejaVu, licença Bitstream Vera e alterações DejaVu em domínio público. Fontes preservadas sem alteração; licenças acompanham os ativos.");
            });
            });
            ui.separator();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = ui
                    .add(self.tokens.dialog_button("Concluído", true))
                    .clicked();
            });
        });
        if close || modal.should_close() {
            self.preferences_dialog = false;
            self.focus_editor = true;
        }
    }
}
