use super::*;
use serde_json::{Value, json};
impl CodisseuApp {
    pub(crate) fn copilot_windows(&mut self, ctx: &egui::Context) {
        if !self.copilot_ui.settings {
            return;
        }
        let mut close = false;
        let mut request = false;
        let frame = egui::Frame::popup(&ctx.style_of(ctx.theme())).inner_margin(12);
        let modal = egui::Modal::new(egui::Id::unique("codisseu.copilot.settings"))
            .frame(frame)
            .show(ctx, |ui| {
                self.tokens.form_style(ui);
                ui.set_width(540.);
                ui.heading("Copilot · Sugestões de código");
                ui.add_space(6.);
                egui::ScrollArea::vertical()
                    .id_salt("copilot.settings.body")
                    .max_height((ctx.content_rect().height() - 180.).clamp(140., 480.))
                    .auto_shrink([false, false])
                    .codisseu()
                    .show(ui, |ui| self.copilot_settings_body(ui));
                ui.add_space(8.);
                ui.separator();
                ui.add_space(6.);
                ui.horizontal(|ui| {
                    request = ui
                        .add_enabled(
                            self.service.editor_preferences().copilot_enabled
                                && self.service.active_document().is_some(),
                            egui::Button::new("Solicitar sugestão no editor"),
                        )
                        .clicked();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = ui
                            .add(self.tokens.dialog_button("Concluído", true))
                            .clicked();
                    });
                });
            });
        if close || request || modal.should_close() {
            self.copilot_ui.settings = false;
            self.focus_editor = true;
            if request {
                self.copilot_ui.manual_waiting = self.copilot_owner();
            }
        }
    }
    fn copilot_settings_body(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.label("Sugestões diretamente no código, incluindo várias linhas, para PHP, SQL, Markdown, JavaScript, TypeScript e outros textos.");
        ui.add_space(10.);
        let mut enabled = self.service.editor_preferences().copilot_enabled;
        if ui
            .checkbox(&mut enabled, "Ativar sugestões do GitHub Copilot")
            .changed()
        {
            self.send(Command::SetCopilotEnabled(enabled));
        }
        ui.label("O conteúdo do editor é enviado ao Copilot quando as sugestões estão ativadas.");
        ui.add_space(12.);
        let ready = self.copilot_ui.client.as_ref().is_some_and(|c| c.ready);
        let login_in_progress = self
            .copilot_ui
            .client
            .as_ref()
            .is_some_and(|c| c.login_in_progress);
        let authenticated = self
            .copilot_ui
            .client
            .as_ref()
            .is_some_and(|c| c.authenticated);
        ui.strong(if !enabled {
            "Sugestões desativadas"
        } else if authenticated {
            "Conta Copilot autenticada"
        } else if ready {
            "Entre na sua conta para receber sugestões"
        } else {
            "Servidor Copilot"
        });
        if let Some(c) = &self.copilot_ui.client {
            if !c.status.is_empty() {
                ui.label(&c.status);
            }
            if let Some(e) = &c.error {
                ui.colored_label(color(self.tokens.error), e);
            }
        }
        if let Some(e) = &self.copilot_ui.error {
            ui.colored_label(color(self.tokens.error), e);
        }
        ui.add_space(8.);
        ui.strong("Atividade das sugestões");
        ui.label(self.copilot_activity());
        ui.add_space(6.);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    ready && !authenticated && !login_in_progress,
                    egui::Button::new("Entrar no GitHub"),
                )
                .clicked()
                && let Some(c) = &mut self.copilot_ui.client
                && let Err(e) = c.sign_in()
            {
                self.copilot_ui.error = Some(e);
            }
            if ui
                .add_enabled(ready && authenticated, egui::Button::new("Sair da conta"))
                .clicked()
                && let Some(c) = &mut self.copilot_ui.client
                && let Err(e) = c.sign_out()
            {
                self.copilot_ui.error = Some(e);
            }
            if ui
                .add_enabled(enabled, egui::Button::new("Reconectar"))
                .clicked()
            {
                self.copilot_ui.client = None;
                self.copilot_ui.attempted = None;
            }
        });
        if let Some(login) = self
            .copilot_ui
            .client
            .as_ref()
            .and_then(|c| c.login.clone())
        {
            ui.add_space(10.);
            egui::Frame::new()
                .fill(color(self.tokens.editor))
                .inner_margin(12)
                .show(ui, |ui| {
                    ui.label("Digite este código na página do GitHub:");
                    ui.label(RichText::new(&login.code).monospace().size(22.));
                    ui.horizontal(|ui| {
                        if ui.button("Copiar código").clicked() {
                            ctx.copy_text(login.code.clone());
                        }
                        if ui.add_enabled(!login_in_progress,egui::Button::new("Continuar no GitHub")).clicked() {
                            ctx.copy_text(login.code.clone());
                            if let Some(c) = &mut self.copilot_ui.client
                                && let Err(e) = c.finish_login()
                            {
                                self.copilot_ui.error = Some(e);
                            }
                        }
                    });
                    if login_in_progress {
                        ui.add_space(6.);
                        ui.label("Conclua a autorização no navegador. Esta janela será atualizada automaticamente.");
                    }
                    ui.add_space(6.);
                    ui.hyperlink_to("Abrir página de ativação do GitHub", "https://github.com/login/device");
                });
        }
        let messages = self
            .copilot_ui
            .client
            .as_ref()
            .map(|c| c.messages.clone())
            .unwrap_or_default();
        for message in messages {
            ui.add_space(8.);
            ui.label(&message.text);
            ui.horizontal_wrapped(|ui| {
                for title in &message.actions {
                    if ui.button(title).clicked()
                        && let Some(c) = &mut self.copilot_ui.client
                    {
                        c.reply(message.id.clone(), json!({"title":title}));
                        c.messages.retain(|m| m.id != message.id);
                    }
                }
                if ui.button("Dispensar aviso").clicked()
                    && let Some(c) = &mut self.copilot_ui.client
                {
                    c.reply(message.id.clone(), Value::Null);
                    c.messages.retain(|m| m.id != message.id);
                }
            });
        }
        if let Some((id, url)) = self
            .copilot_ui
            .client
            .as_ref()
            .and_then(|c| c.open_url.clone())
        {
            ui.label(&url);
            if ui.button("Abrir link solicitado pelo Copilot").clicked() {
                ctx.open_url(egui::OpenUrl::new_tab(url));
                if let Some(c) = &mut self.copilot_ui.client {
                    c.reply(id, json!({"success":true}));
                    c.open_url = None;
                }
            }
        }
        ui.add_space(12.);
        ui.label("Tab aceita · Esc descarta · Alt+\\ solicita · Ctrl+→ aceita uma palavra");
        ui.label("Alt+[ e Alt+] alternam as sugestões disponíveis. A lista normal de autocomplete mantém prioridade sobre Tab.");
        ui.add_space(8.);
        ui.collapsing("Servidor e conexão", |ui| {
            if ui.button("Copiar diagnóstico do Copilot").clicked() {
                let diagnostic = json!({"integration_revision":"copilot-audit-20261106-v1", "state":self.copilot_diagnostic(&ctx)});
                ctx.copy_text(serde_json::to_string_pretty(&diagnostic).unwrap_or_default());
            }
            ui.label("O diagnóstico contém somente estados e contagens, sem código, caminhos ou credenciais.");
            ui.add_space(8.);
            ui.label("Caminho do servidor (opcional)");
            ui.add(
                egui::TextEdit::singleline(&mut self.copilot_ui.program)
                    .hint_text(api::default_program().display().to_string())
                    .desired_width(510.),
            );
            if ui.button("Aplicar caminho do servidor").clicked() {
                match self
                    .service
                    .dispatch(Command::SetCopilotProgram(self.copilot_ui.program.clone()))
                {
                    Ok(()) => {
                        self.copilot_ui.client = None;
                        self.copilot_ui.attempted = None;
                    }
                    Err(e) => self.copilot_ui.error = Some(e),
                }
            }
            ui.label("Use o servidor oficial instalado ou escolha seu executável de confiança.");
            ui.hyperlink_to(
                "Documentação do servidor Copilot",
                "https://github.com/github/copilot-language-server-release",
            );
        });
    }
}
