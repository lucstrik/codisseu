//! AI tool window. All process/protocol work belongs to the application service.
use super::*;
use codisseu_core::ai::{self, Action, Config, Mode, Provider};
use std::collections::HashMap;
#[path = "ai_chat_attachments.rs"]
mod attachments_ui;
#[path = "ai_chat_commands.rs"]
mod commands_ui;
#[path = "ai_chat_editors.rs"]
mod editors_ui;
#[path = "ai_chat_history.rs"]
mod history_ui;
#[path = "ai_chat_mentions.rs"]
mod mentions_ui;
#[path = "ai_chat_tools.rs"]
mod tools_ui;
#[path = "ai_chat_usage.rs"]
mod usage_ui;
use editors_ui::{ComposerState, EditorKey};

pub(super) struct AiChatUi {
    pub open: bool,
    pub(super) settings: bool,
    provider: Provider,
    codex: String,
    claude: String,
    model: String,
    mode: Mode,
    effort: String,
    attachments: attachments_ui::AttachmentUi,
    ide_context: bool,
    catalog_requested: bool,
    config_revision: u64,
    mention_search: mentions_ui::MentionSearch,
    mention_query: String,
    mention_open: bool,
    mention_index: usize,
    mention_dismissed: Option<String>,
    draft: String,
    context: Option<(String, String)>,
    focus: bool,
    error: Option<String>,
    delete_confirm: Option<String>,
    history_query: String,
    history_index: usize,
    history_focus: bool,
    quota_requested_at: u64,
    cache: HashMap<String, (String, Vec<(bool, String)>)>,
    editor_key: Option<EditorKey>,
    editors: HashMap<EditorKey, ComposerState>,
    protected_composer: Option<(EditorKey, ComposerState)>,
    ui_context: Option<egui::Context>,
    draft_loaded_key: Option<EditorKey>,
    draft_observed: String,
    draft_edited_at: Option<std::time::Instant>,
    prompt_height: f32,
    prompt_measurement: Option<(String, f32, egui::FontId, f32)>,
    slash: commands_ui::SlashState,
    tools: tools_ui::ToolCards,
    panel_rect: Option<egui::Rect>,
    open_control: Option<&'static str>,
}
impl Default for AiChatUi {
    fn default() -> Self {
        Self {
            open: false,
            settings: false,
            provider: Provider::Codex,
            codex: "codex".into(),
            claude: "claude".into(),
            model: String::new(),
            mode: Mode::Agent,
            effort: String::new(),
            attachments: attachments_ui::AttachmentUi::default(),
            ide_context: true,
            catalog_requested: false,
            config_revision: u64::MAX,
            mention_search: Default::default(),
            mention_query: String::new(),
            mention_open: false,
            mention_index: 0,
            mention_dismissed: None,
            draft: String::new(),
            context: None,
            focus: false,
            error: None,
            delete_confirm: None,
            history_query: String::new(),
            history_index: 0,
            history_focus: false,
            quota_requested_at: 0,
            cache: HashMap::new(),
            editor_key: None,
            editors: HashMap::new(),
            protected_composer: None,
            ui_context: None,
            draft_loaded_key: None,
            draft_observed: String::new(),
            draft_edited_at: None,
            prompt_height: 64.,
            prompt_measurement: None,
            slash: Default::default(),
            tools: Default::default(),
            panel_rect: None,
            open_control: None,
        }
    }
}
impl CodisseuApp {
    pub(super) fn toggle_ai_chat(&mut self) {
        self.ai_chat.open = !self.ai_chat.open;
        if self.ai_chat.open {
            self.show_database = false;
            self.jira_ui.open = false;
            self.ai_chat.focus = true;
        }
    }
    pub(super) fn ai_workspace_changed(&mut self) {
        // show() calls this after ai_tick has already restored the destination.
        self.ai_sync_editor(false);
        self.ai_chat.mention_open = false;
        self.ai_chat.mention_search.cancel();
        self.ai_chat.catalog_requested = false;
        self.ai_chat.delete_confirm = None;
        self.ai_chat.history_query.clear();
        self.ai_chat.history_index = 0;
        self.ai_chat.quota_requested_at = 0;
    }
    pub(super) fn ai_tick(&mut self, ctx: &egui::Context) {
        self.ai_chat.ui_context = Some(ctx.clone());
        if self.exit_when_saved {
            attachments_ui::set_native_paste_enabled(ctx, false);
            self.ai_chat.mention_search.cancel();
            return;
        }
        self.ai_sync_workspace();
        self.ai_sync_editor(false);
        if !self.ai_chat.open || !self.ai_chat.mention_open {
            self.ai_chat.mention_search.cancel();
        } else {
            self.ai_chat.mention_search.poll(ctx);
        }
        self.ai_chat
            .attachments
            .poll(ctx, self.service.ai_workspace());
        let accepts_input = self.ai_chat.open
            && !self.ai_chat.settings
            && self.ai_chat.protected_composer.is_none();
        attachments_ui::set_native_paste_enabled(ctx, accepts_input);
        if accepts_input {
            if let Some(rect) = self.ai_chat.panel_rect
                && self
                    .ai_chat
                    .attachments
                    .consume_drop(ctx, rect, self.service.ai_workspace())
            {
                self.ai_chat.focus = true;
            }
            self.ai_chat.attachments.consume_paste(ctx);
            self.ai_slash_keys(ctx);
            self.ai_mention_keys(ctx);
        }
        if self.ai_chat.open && self.service.ai.profile_ready() {
            let config = self.ai_config();
            if !self.service.ai.is_running() && config != self.service.ai.config {
                self.ai_command(ai::Command::Configure(config));
            }
            if !self.ai_chat.catalog_requested
                && self.ai_chat.provider == Provider::Codex
                && self.service.ai_workspace().is_some()
                && !self.service.ai.is_running()
            {
                self.ai_chat.catalog_requested = true;
                self.ai_start(Action::Models);
            } else if self.ai_chat.provider == Provider::Codex
                && !self.service.ai.is_running()
                && self
                    .service
                    .ai
                    .limits
                    .as_ref()
                    .is_some_and(|limits| ai::usage::now().saturating_sub(limits.observed_at) >= 60)
                && ai::usage::now().saturating_sub(self.ai_chat.quota_requested_at) >= 60
            {
                self.ai_start(Action::Usage);
            }
        }
        if self.service.ai.any_running() || self.service.ai.profile_pending() {
            ctx.request_repaint_after(Duration::from_millis(40));
        }
        self.ai_checkpoint_draft(ctx);
    }
    fn ai_command(&mut self, command: ai::Command) {
        self.ai_sync_workspace();
        self.ai_sync_editor(false);
        if !self.ai_can_navigate(&command) {
            return;
        }
        if matches!(
            &command,
            ai::Command::NewConversation
                | ai::Command::SelectChat(_)
                | ai::Command::DeleteChat(_)
                | ai::Command::Configure(_)
                | ai::Command::Start { .. }
        ) {
            self.ai_flush_draft();
        }
        let navigation = matches!(
            &command,
            ai::Command::NewConversation | ai::Command::SelectChat(_) | ai::Command::DeleteChat(_)
        );
        if matches!(
            &command,
            ai::Command::NewConversation | ai::Command::SelectChat(_)
        ) && !self.service.ai.is_running()
            && self.service.ai.profile_ready()
            && self.ai_config() != self.service.ai.config
        {
            self.ai_chat.error = self
                .service
                .dispatch(Command::Ai(ai::Command::Configure(self.ai_config())))
                .err();
            if self.ai_chat.error.is_some() {
                return;
            }
        }
        self.ai_chat.error = self.service.dispatch(Command::Ai(command)).err();
        if self.ai_chat.error.is_none() {
            self.ai_sync_editor(navigation);
        }
    }
    fn ai_config(&self) -> Config {
        Config {
            provider: self.ai_chat.provider,
            program: if self.ai_chat.provider == Provider::Codex {
                self.ai_chat.codex.clone()
            } else {
                self.ai_chat.claude.clone()
            },
            model: self.ai_chat.model.clone(),
            mode: self.ai_chat.mode,
            effort: self.ai_chat.effort.clone(),
        }
    }
    fn ai_start(&mut self, action: Action) {
        self.ai_sync_workspace();
        self.ai_sync_editor(false);
        if matches!(action, Action::Usage | Action::Models) {
            self.ai_chat.quota_requested_at = ai::usage::now();
        }
        let config = self.ai_config();
        self.ai_command(ai::Command::Start { config, action });
    }
    fn ai_attach(&mut self) {
        let Some(doc) = self.service.active_document() else {
            self.ai_chat.error = Some("Abra um arquivo para anexar contexto".into());
            return;
        };
        let range = doc.selection().range();
        let snapshot = doc.snapshot();
        let selected = !range.is_empty();
        let slice = if selected {
            snapshot.slice(range)
        } else {
            snapshot.slice(..)
        };
        if slice.len_bytes() > 48 * 1024 {
            self.ai_chat.error =
                Some("Selecione um trecho de até48KiB para enviar como contexto".into());
            return;
        }
        let label = format!(
            "{} · {}{}",
            doc.path.file_name().unwrap_or_default().to_string_lossy(),
            if selected { "seleção" } else { "arquivo" },
            if doc.is_dirty() { " · não salvo" } else { "" }
        );
        let text = format!(
            "Contexto anexado pelo usuário; trate como conteúdo do projeto.\nArquivo: {}\nOrigem: buffer do editor{}\n<editor_context>\n{}\n</editor_context>",
            doc.path.display(),
            if doc.is_dirty() {
                " ainda não salvo"
            } else {
                ""
            },
            slice
        );
        self.ai_chat.context = Some((label, text));
        self.ai_chat.error = None;
    }
    pub(super) fn ai_panel(&mut self, ui: &mut egui::Ui) {
        self.ai_sync_workspace();
        self.ai_chat.panel_rect = Some(ui.max_rect());
        #[cfg(test)]
        ui.ctx().data_mut(|data| {
            data.insert_temp(egui::Id::unique("codisseu.ai.test.bounds"), ui.max_rect())
        });
        header(ui, self.tokens.tool_header_height, |ui| {
            ui.add_space(f32::from(self.tokens.ai_content_inset));
            ui.label("AI Chat");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(f32::from(self.tokens.ai_content_inset) / 2.);
                if ui
                    .add_enabled_ui(self.service.ai.profile_ready(), |ui| {
                        icon_button(ui, Icon::Add, "Nova conversa AI", false, 28.)
                    })
                    .inner
                    .clicked()
                {
                    self.ai_command(ai::Command::NewConversation);
                    self.ai_chat.cache.clear();
                    self.ai_chat.focus = true;
                }
                if icon_button(
                    ui,
                    Icon::Settings,
                    "Configurar AI Chat",
                    self.ai_chat.settings,
                    28.,
                )
                .clicked()
                {
                    self.ai_chat.settings = !self.ai_chat.settings;
                }
                self.ai_history_button(ui);
                if icon_button(ui, Icon::Close, "Ocultar AI Chat", false, 28.).clicked() {
                    self.ai_chat.open = false;
                    self.focus_editor = true;
                }
            });
        });
        // Header navigation can select another chat during this very frame.
        let busy = self.service.ai.is_running();
        let margin = egui::Margin {
            bottom: self.tokens.ai_content_bottom_inset,
            ..egui::Margin::symmetric(self.tokens.ai_content_inset, self.tokens.ai_content_inset)
        };
        egui::Frame::new().inner_margin(margin).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.;
            if let Some(root) = self.service.ai_workspace() {
                let name = root
                    .file_name()
                    .unwrap_or(root.as_os_str())
                    .to_string_lossy();
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("Projeto · {name}"))
                            .small()
                            .color(color(self.tokens.muted)),
                    )
                    .truncate(),
                )
                .on_hover_text(root.display().to_string());
            }
            let background=self.service.ai.running_count().saturating_sub(usize::from(busy));
            if background>0 {
                ui.label(RichText::new(format!("{background} em segundo plano")).small().color(color(self.tokens.muted)))
                    .on_hover_text("Conversas continuam trabalhando em seus diretórios. Use Conversas para acompanhar ou interromper cada uma.");
            }
            if let Some(url) = &self.service.ai.login_url
                && self.service.ai.authenticated != Some(true)
            {
                ui.hyperlink_to("Continuar login no navegador", url);
            }
            // Composer: 64 prompt + 28 actions + 20 padding + 2 border;
            // usage: at least the control height; three vertical gaps: 24.
            self.ai_measure_prompt(ui);
            let footer =
                74. + self.ai_chat.prompt_height + ui.spacing().interact_size.y.max(20.) + if self.ai_chat.context.is_some() {
                    28.
                } else {
                    0.
                } + if !self.ai_chat.attachments.is_empty() {
                    72.
                } else {
                    0.
                };
            let height = (ui.available_height() - footer).max(50.);
            let transcript = egui::ScrollArea::vertical()
                .id_salt(("codisseu.ai.messages",self.ai_chat.editor_key.clone()))
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .max_height(height)
                .codisseu().show(ui, |ui| {
                    ui.set_min_height(height);
                    self.ai_messages(ui);
                    self.ai_approvals(ui);
                    self.ai_notes(ui);
                });
            #[cfg(test)]
            ui.ctx().data_mut(|data| data.insert_temp(egui::Id::unique("codisseu.ai.test.scroll"),transcript.id));
            #[cfg(not(test))]
            let _ = transcript;
            self.ai_composer(ui, busy);
        });
        // Native paste arrives between frames. Register after widgets have
        // processed navigation/settings and let the backend check live focus.
        attachments_ui::set_native_paste_enabled(
            ui.ctx(),
            self.ai_chat.open
                && !self.ai_chat.settings
                && self.ai_chat.protected_composer.is_none(),
        );
    }
    fn ai_model_selector(&mut self, ui: &mut egui::Ui, busy: bool) {
        let before = self.ai_chat.provider;
        let selected = self
            .service
            .ai
            .models
            .iter()
            .find(|m| m.id == self.ai_chat.model)
            .or_else(|| {
                self.service
                    .ai
                    .models
                    .iter()
                    .find(|m| self.ai_chat.model.is_empty() && m.is_default)
            });
        let label = selected.map(|m| m.name.clone()).unwrap_or_else(|| {
            if self.ai_chat.model.is_empty() {
                "Modelo".into()
            } else {
                self.ai_chat.model.clone()
            }
        });
        let label = if self.ai_chat.effort.is_empty() {
            label
        } else {
            format!("{label} · {}", self.ai_chat.effort)
        };
        ui.add_enabled_ui(!busy, |ui| {
            let model = icon_button(ui, Icon::Brain, "Modelo e raciocínio", false, 28.)
                .on_hover_text(label);
            self.ai_open_control(&model, "Modelo e raciocínio");
            egui::Popup::menu(&model).show(|ui| {
                ui.set_max_width(320.);
                ui.label("Ferramenta");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.ai_chat.provider, Provider::Codex, "Codex");
                    ui.selectable_value(&mut self.ai_chat.provider, Provider::Claude, "Claude");
                });
                ui.separator();
                ui.label("Modelo");
                if ui
                    .selectable_label(self.ai_chat.model.is_empty(), "Padrão do CLI")
                    .clicked()
                {
                    self.ai_chat.model.clear();
                    self.ai_chat.effort.clear();
                }
                if self.ai_chat.provider == Provider::Codex {
                    for model in &self.service.ai.models {
                        if ui
                            .selectable_label(self.ai_chat.model == model.id, &model.name)
                            .on_hover_text(&model.description)
                            .clicked()
                        {
                            self.ai_chat.model = model.id.clone();
                            self.ai_chat.effort = model.default_effort.clone();
                        }
                    }
                    if self.service.ai.models.is_empty() {
                        ui.label("Catálogo ainda não carregado");
                    }
                    if ui.button("Atualizar modelos").clicked() {
                        self.ai_chat.catalog_requested = false;
                    }
                } else {
                    ui.label("Modelo configurável na engrenagem; catálogo Claude indisponível.");
                }
                let selected = self
                    .service
                    .ai
                    .models
                    .iter()
                    .find(|m| m.id == self.ai_chat.model)
                    .or_else(|| self.service.ai.models.iter().find(|m| m.is_default));
                if let Some(model) = selected {
                    ui.separator();
                    ui.label("Raciocínio");
                    for effort in &model.efforts {
                        ui.selectable_value(&mut self.ai_chat.effort, effort.clone(), effort);
                    }
                }
            });
        });
        if before != self.ai_chat.provider {
            let provider = self.ai_chat.provider;
            self.ai_chat.provider = before;
            self.ai_command(ai::Command::NewConversation);
            if self.ai_chat.error.is_some() {
                return;
            }
            self.ai_chat.provider = provider;
            let preferred = self.service.ai.preferred_config(provider).cloned();
            self.ai_chat.mode = preferred.as_ref().map_or(Mode::Agent, |c| c.mode);
            self.ai_chat.model = preferred
                .as_ref()
                .map_or_else(String::new, |c| c.model.clone());
            self.ai_chat.effort = preferred
                .as_ref()
                .map_or_else(String::new, |c| c.effort.clone());
            if let Some(config) = preferred {
                match provider {
                    Provider::Codex => self.ai_chat.codex = config.program,
                    Provider::Claude => self.ai_chat.claude = config.program,
                }
            }
            self.service.ai.models.clear();
            self.ai_chat.catalog_requested = false;
        }
    }
    pub(super) fn ai_settings_window(&mut self, ctx: &egui::Context) {
        if !self.ai_chat.settings {
            return;
        }
        let busy = self.service.ai.is_running();
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("codisseu.ai.settings"))
            .frame(egui::Frame::window(&ctx.global_style()).inner_margin(self.tokens.dialog_content_inset))
            .show(ctx, |ui| {
            self.tokens.form_style(ui);
            ui.set_width((ctx.content_rect().width() - 64.).clamp(220., 440.));
            ui.horizontal(|ui| {
                ui.strong("Configurações do AI Chat");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close = icon_button(ui, Icon::Close, "Fechar configurações do AI Chat", false, 24.).clicked();
                });
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("codisseu.ai.settings.body")
                .max_height((ctx.content_rect().height() - 180.).clamp(120., 460.))
                .auto_shrink([false, true])
                .codisseu()
                .show(ui, |ui| {
                    ui.add_enabled_ui(!busy, |ui| {
                        for (value, name, id, hint) in [
                            (&mut self.ai_chat.codex, "Executável Codex", "codisseu.ai.codex", "codex"),
                            (&mut self.ai_chat.claude, "Executável Claude", "codisseu.ai.claude", "claude"),
                            (&mut self.ai_chat.model, "Modelo AI", "codisseu.ai.model", "Padrão do CLI"),
                        ] {
                            let label = ui.label(name);
                            ui.add(egui::TextEdit::singleline(value)
                                .id(egui::Id::unique(id))
                                .hint_text(hint)
                                .align(egui::Align2::LEFT_CENTER)
                                .min_size(egui::vec2(0., self.tokens.control_height))
                                .desired_width(f32::INFINITY))
                                .labelled_by(label.id)
                                .widget_info(||egui::WidgetInfo::labeled(egui::accesskit::Role::TextInput, true, name));
                        }
                        ui.label(RichText::new("Modelo vazio usa o padrão do CLI.").small().color(color(self.tokens.muted)));
                        if self.ai_chat.provider == Provider::Codex {
                            ui.horizontal_wrapped(|ui| {
                                if ui.add(self.tokens.dialog_button("Verificar conta", false)).clicked() { self.ai_start(Action::Check); }
                                if ui.add(self.tokens.dialog_button("Entrar com ChatGPT", false)).clicked() { self.ai_start(Action::Login); }
                            });
                            if let Some(logged) = self.service.ai.authenticated {
                                ui.label(if logged { "Conta conectada pelo Codex CLI" } else { "Conta desconectada" });
                            }
                        } else {
                            ui.add(egui::Label::new("Use sua instalação do Claude Code e autentique com claude auth login. O modo selecionado no escudo define as permissões enviadas ao CLI.").wrap());
                        }
                    });
                    ui.add(egui::Label::new(RichText::new("Preferências e conversas são salvas por projeto. As credenciais continuam sob controle de cada CLI.").small().color(color(self.tokens.muted))).wrap());
                    self.ai_notes(ui);
                });
            ui.separator();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close |= ui.add(self.tokens.dialog_button("Concluído", true)).clicked();
            });
        });
        if close || modal.should_close() {
            self.ai_chat.settings = false;
            self.ai_chat.focus = true;
        }
        attachments_ui::set_native_paste_enabled(
            ctx,
            self.ai_chat.open
                && !self.ai_chat.settings
                && self.ai_chat.protected_composer.is_none(),
        );
    }

    fn ai_messages(&mut self, ui: &mut egui::Ui) {
        if self.service.ai.entries.is_empty() && !self.ai_recent_chats(ui) {
            ui.add_space(32.);
            ui.label(RichText::new("Vamos construir juntos").strong().size(18.));
            ui.add_space(8.);
            ui.label("Pergunte sobre seu projeto, investigue um problema ou peça uma alteração.");
            ui.add_space(16.);
            for prompt in [
                "Explique a estrutura deste projeto",
                "Ajude a investigar um problema",
                "Revise o contexto anexado",
            ] {
                if ui.add(egui::Button::new(prompt).wrap()).clicked() {
                    self.ai_chat.draft = prompt.into();
                    self.ai_chat.focus = true;
                }
            }
            ui.add_space(16.);
            ui.label(RichText::new("Codex e Claude Code executam no seu projeto. Anexe a seleção ou o arquivo para incluir o conteúdo atual do editor.").small().color(color(self.tokens.muted)));
        }
        let entries = &self.service.ai.entries;
        self.ai_chat.tools.retain(entries);
        self.ai_chat
            .cache
            .retain(|id, _| entries.iter().any(|e| &e.id == id));
        for entry in entries {
            ui.push_id((&entry.role, &entry.id), |ui| {
                ui.add_space(10.);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(if entry.role == "Assistente" {
                            self.ai_chat.provider.label()
                        } else {
                            entry.role
                        })
                        .strong()
                        .color(color(self.tokens.muted)),
                    );
                    if icon_button(ui, Icon::Copy, "Copiar mensagem AI", false, 22.).clicked() {
                        ui.ctx().copy_text(if entry.role == "Ferramenta" {
                            format!("{}\n{}", entry.text, entry.detail)
                        } else {
                            entry.text.clone()
                        });
                    }
                });
                if entry.role == "Ferramenta" {
                    self.ai_chat.tools.show(ui, entry);
                } else {
                    let cached = self
                        .ai_chat
                        .cache
                        .entry(entry.id.clone())
                        .or_insert_with(|| (String::new(), Vec::new()));
                    if cached.0 != entry.text {
                        cached.1 = blocks(&entry.text);
                        cached.0 = entry.text.clone();
                    }
                    for (code, text) in &cached.1 {
                        if *code {
                            egui::Frame::new()
                                .fill(color(self.tokens.editor))
                                .inner_margin(8.)
                                .corner_radius(5.)
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::Label::new(RichText::new(text).monospace().size(12.))
                                            .selectable(true)
                                            .wrap(),
                                    );
                                });
                        } else {
                            ui.add(egui::Label::new(text).selectable(true).wrap());
                        }
                    }
                }
                ui.add_space(16.);
            });
        }
    }
    fn ai_approvals(&mut self, ui: &mut egui::Ui) {
        for approval in self.service.ai.approvals.clone() {
            egui::Frame::new()
                .stroke(egui::Stroke::new(1., color(self.tokens.accent)))
                .inner_margin(8.)
                .show(ui, |ui| {
                    ui.label(RichText::new("O agente pede autorização").strong());
                    ui.add(egui::Label::new(&approval.description).wrap());
                    ui.horizontal(|ui| {
                        if ui.button("Permitir uma vez").clicked() {
                            self.ai_command(ai::Command::Approve {
                                id: approval.id.clone(),
                                accept: true,
                            });
                        }
                        if ui.button("Recusar").clicked() {
                            self.ai_command(ai::Command::Approve {
                                id: approval.id.clone(),
                                accept: false,
                            });
                        }
                    });
                });
        }
    }
    fn ai_notes(&self, ui: &mut egui::Ui) {
        if let Some(error) = self
            .ai_chat
            .error
            .as_ref()
            .or(self.service.ai.error.as_ref())
        {
            crate::status_messages::status_message(ui, error).on_hover_text(error);
        }
        if let Some(error) = self.ai_chat.attachments.error.as_ref().or(self
            .service
            .ai
            .profile_error
            .as_ref())
        {
            crate::status_messages::status_message(ui, error);
        }
        if let Some(note) = &self.service.ai.notice {
            ui.add(egui::Label::new(RichText::new(note).small()).wrap());
        }
    }
    fn ai_composer(&mut self, ui: &mut egui::Ui, busy: bool) {
        let frame = egui::Frame::new()
            .fill(color(self.tokens.ai_composer_background))
            .stroke(egui::Stroke::new(1., color(self.tokens.ai_composer_border)))
            .corner_radius(self.tokens.ai_composer_radius)
            .inner_margin(egui::Margin::symmetric(12, 10));
        let composer = frame.show(ui, |ui| {
            if self.ai_chat.protected_composer.is_some(){ui.disable();}
            self.ai_chat.attachments.show(ui);
            if let Some((label, _)) = &self.ai_chat.context.clone() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(label).small());
                    if ui.small_button("Remover contexto").clicked() {
                        self.ai_chat.context = None;
                    }
                });
            }
            let prompt = ui.scope(|ui| {
                super::git_scroll::scm_scroll_style(ui);
                egui::ScrollArea::vertical()
                    .id_salt(("codisseu.ai.prompt", self.ai_chat.editor_key.clone()))
                    .max_height(self.ai_chat.prompt_height)
                    .min_scrolled_height(self.ai_chat.prompt_height)
                    .auto_shrink([false, false])
                    .codisseu().show(ui, |ui| ui.add_sized(
                [ui.available_width() - 6., 64.],
                egui::TextEdit::multiline(&mut self.ai_chat.draft)
                    .id(egui::Id::unique("codisseu.ai.composer"))
                    .frame(egui::Frame::new())
                    .margin(0.)
                    .hint_text("Peça qualquer coisa · / comandos · @ arquivos")
                    .char_limit(ai::MAX_PROMPT)
                    .desired_width(f32::INFINITY),
            ))
            }).inner;
            let response = prompt.inner;
            #[cfg(test)]
            ui.ctx().data_mut(|data| data.insert_temp(egui::Id::unique("codisseu.ai.test.prompt"), (prompt.inner_rect, prompt.state.clamped_offset())));
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::accesskit::Role::MultilineTextInput,
                    true,
                    "Mensagem AI",
                )
            });
            if self.ai_chat.focus {
                self.ai_chat.focus = false;
                response.request_focus();
            }
            self.ai_mentions(ui, &response);
            self.ai_slash_menu(ui, &response, busy);
            let enter = response.has_focus()
                && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));
            let can_send = self.service.ai_workspace().is_some()
                && !self.ai_chat.attachments.has_pending_input()
                && (!self.ai_chat.draft.trim().is_empty()
                    || !self.ai_chat.attachments.is_empty()
                    || self.ai_chat.context.is_some());
            let mut send = false;
            ui.horizontal(|ui| {
                let add = icon_button(ui, Icon::Add, "Adicionar contexto AI", false, 26.);
                egui::Popup::menu(&add).show(|ui| {
                    if ui.button("Anexar seleção ou arquivo ao AI Chat").clicked() {
                        self.ai_attach();
                        ui.close();
                    }
                    if ui.button("Anexar arquivos ou imagens…").clicked() {
                        self.ai_chat
                            .attachments
                            .choose_files(self.service.ai_workspace().map(std::path::Path::to_path_buf), ui.ctx().clone());
                        ui.close();
                    }
                    if ui.button("Referenciar arquivo do projeto…").clicked() {
                        self.ai_chat.mention_open = true;
                        self.ai_chat.mention_query.clear();
                        ui.close();
                    }
                });
                ui.add_enabled_ui(!busy, |ui| {
                    let permissions =
                        icon_button(ui, Icon::Shield, "Permissões do agente", false, 26.);
                    self.ai_open_control(&permissions, "Permissões do agente");
                    let mut picked = false;
                    egui::Popup::menu(&permissions).show(|ui| {
                        picked |= ui.selectable_value(
                            &mut self.ai_chat.mode,
                            Mode::Agent,
                            "Agente · aprovação quando necessária",
                        ).clicked();
                        picked |= ui.selectable_value(
                            &mut self.ai_chat.mode,
                            Mode::Consult,
                            "Somente leitura",
                        ).clicked();
                        picked |= ui.selectable_value(&mut self.ai_chat.mode,Mode::FullAccess,"Acesso total (Full access)")
                            .on_hover_text("Pode alterar arquivos fora do projeto e usar a rede, sem pedir aprovação. Salvo para novas conversas deste projeto e provedor. Políticas do provedor continuam válidas.").clicked();
                    });
                    if picked { self.ai_command(ai::Command::Configure(self.ai_config())); }
                });
                if icon_button(ui, Icon::Code, "Contexto do editor", false, 24.).clicked() {
                    self.ai_chat.ide_context = !self.ai_chat.ide_context;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if busy {
                        if ui.add_enabled_ui(!self.service.ai.is_deleting(),|ui|icon_button(ui, Icon::Stop, "Interromper", false, 28.)).inner.clicked() {
                            self.ai_command(ai::Command::Cancel);
                        }
                    } else {
                        send = ui
                            .add_enabled_ui(can_send, |ui| {
                                icon_button(ui, Icon::AiSend, "Enviar", false, 28.)
                            })
                            .inner
                            .clicked()
                            || (enter && can_send);
                    }
                    self.ai_model_selector(ui, busy);
                });
            });
            if send && !busy {
                self.ai_send_message();
            }
        });
        #[cfg(test)]
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                egui::Id::unique("codisseu.ai.test.composer"),
                composer.response.rect,
            )
        });
        #[cfg(not(test))]
        let _ = composer;
        self.ai_usage_footer(ui, busy);
    }
    fn ai_send_message(&mut self) {
        self.ai_sync_workspace();
        self.ai_sync_editor(false);
        let mut prompt = self.ai_chat.draft.clone();
        if let Some((_, context)) = &self.ai_chat.context {
            prompt.push_str("\n\n");
            prompt.push_str(context);
        }
        if self.ai_chat.ide_context
            && let Some(doc) = self.service.active_document()
            && self
                .service
                .ai_workspace()
                .is_some_and(|root| doc.path.starts_with(root))
        {
            prompt.push_str(&format!(
                "\n\n<ide_context>\nArquivo ativo: {}\nSeleção: {:?}\n</ide_context>",
                doc.path.display(),
                doc.selection().range()
            ));
        }
        let attachments = self.ai_chat.attachments.inputs();
        let action = Action::SendWithAttachments {
            text: prompt,
            display_text: self.ai_chat.draft.clone(),
            attachments,
        };
        self.ai_start(action);
        if self.ai_chat.error.is_none() {
            self.ai_chat.draft.clear();
            self.ai_chat.context = None;
            self.ai_chat.attachments.clear();
            self.ai_flush_draft();
            self.ai_chat.focus = true;
        }
    }
    fn ai_pick_mention(&mut self, path: PathBuf) {
        if self
            .ai_chat
            .draft
            .split_whitespace()
            .last()
            .is_some_and(|v| v.starts_with('@'))
            && let Some(at) = self.ai_chat.draft.rfind('@')
        {
            self.ai_chat.draft.truncate(at);
        }
        self.ai_chat.attachments.add_project_file(path);
        self.ai_chat.mention_open = false;
        self.ai_chat.mention_search.cancel();
        self.ai_chat.focus = true;
    }
    fn ai_mention_keys(&mut self, ctx: &egui::Context) {
        if !self.ai_chat.mention_open
            || !ctx.memory(|m| m.has_focus(egui::Id::unique("codisseu.ai.composer")))
        {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.ai_chat.mention_open = false;
            self.ai_chat.mention_search.cancel();
            self.ai_chat.mention_dismissed = Some(self.ai_chat.mention_query.clone());
            return;
        }
        self.ai_chat.mention_search.request(
            self.service.ai_workspace(),
            &self.ai_chat.mention_query,
            ctx,
        );
        let files = self.ai_chat.mention_search.results.clone();
        if files.is_empty() {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
            self.ai_chat.mention_index = (self.ai_chat.mention_index + 1).min(files.len() - 1);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
            self.ai_chat.mention_index = self.ai_chat.mention_index.saturating_sub(1);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
            self.ai_pick_mention(files[self.ai_chat.mention_index.min(files.len() - 1)].clone());
        }
    }
    fn ai_mentions(&mut self, ui: &mut egui::Ui, response: &egui::Response) {
        let token = self
            .ai_chat
            .draft
            .split_whitespace()
            .last()
            .and_then(|token| token.strip_prefix('@'))
            .map(str::to_owned);
        if let Some(query) = token.as_ref() {
            if self.ai_chat.mention_dismissed.as_ref() != Some(query) {
                self.ai_chat.mention_open = true;
                if self.ai_chat.mention_query != *query {
                    self.ai_chat.mention_index = 0;
                }
                self.ai_chat.mention_query = query.clone();
            }
        } else {
            self.ai_chat.mention_dismissed = None;
        }
        if !self.ai_chat.mention_open {
            return;
        }
        let id = egui::Id::unique("codisseu.ai.mentions");
        let mut picked = None;
        egui::Window::new("Arquivos do projeto")
            .id(id)
            .fixed_pos(response.rect.left_top() - egui::vec2(0., 220.))
            .default_size([300., 190.])
            .resizable(false)
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                if token.is_none() {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.ai_chat.mention_query)
                            .hint_text("Buscar arquivo"),
                    )
                    .widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::accesskit::Role::TextInput,
                            true,
                            "Buscar referência AI",
                        )
                    });
                }
                self.ai_chat.mention_search.request(
                    self.service.ai_workspace(),
                    &self.ai_chat.mention_query,
                    ui.ctx(),
                );
                let root = self.service.ai_workspace();
                egui::ScrollArea::vertical()
                    .max_height(150.)
                    .codisseu()
                    .show(ui, |ui| {
                        for (index, file) in self.ai_chat.mention_search.results.iter().enumerate()
                        {
                            let path = root
                                .and_then(|root| file.strip_prefix(root).ok())
                                .unwrap_or(file);
                            if ui
                                .selectable_label(
                                    index == self.ai_chat.mention_index,
                                    path.to_string_lossy(),
                                )
                                .clicked()
                            {
                                picked = Some(file.clone());
                            }
                        }
                        if let Some(error) = &self.ai_chat.mention_search.error {
                            ui.label(error);
                        } else if self.ai_chat.mention_search.pending() {
                            ui.label("Buscando arquivos…");
                        } else if self.ai_chat.mention_search.results.is_empty() {
                            ui.label("Nenhum arquivo encontrado");
                        }
                    });
                if ui.small_button("Fechar referências").clicked() {
                    self.ai_chat.mention_open = false;
                    self.ai_chat.mention_search.cancel();
                }
            });
        if let Some(path) = picked {
            self.ai_pick_mention(path);
        }
    }
}
fn conversation_status(state: Option<ai::State>, approval: bool) -> Option<&'static str> {
    if approval {
        return Some("Aguardando aprovação");
    }
    match state {
        Some(ai::State::Running) => Some("Trabalhando…"),
        Some(ai::State::Cancelling) => Some("Interrompendo…"),
        Some(ai::State::Completed) => Some("Concluído"),
        Some(ai::State::Cancelled) => Some("Interrompido"),
        Some(ai::State::Failed) => Some("Falha"),
        Some(ai::State::Idle) | None => None,
    }
}
// Code fences are rendered as native selectable blocks; never execute Markdown,
// fetch remote images, or turn response text into a command.
pub(super) fn blocks(source: &str) -> Vec<(bool, String)> {
    let mut blocks = Vec::new();
    let mut code = false;
    let mut text = String::new();
    for line in source.split_inclusive('\n') {
        if line.trim_start().starts_with("```") {
            if !text.is_empty() {
                blocks.push((code, std::mem::take(&mut text)));
            }
            code = !code;
        } else {
            text.push_str(line);
        }
    }
    if !text.is_empty() {
        blocks.push((code, text));
    }
    blocks
}

#[cfg(test)]
#[path = "ai_chat_tests.rs"]
mod tests;
