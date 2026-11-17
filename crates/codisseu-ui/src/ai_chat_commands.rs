//! Local actions share the same commands as the chat controls.
use super::*;

#[derive(Default)]
pub(super) struct SlashState {
    query: Option<String>,
    dismissed: Option<String>,
    index: usize,
    reveal: bool,
}

#[derive(Clone, Copy)]
enum LocalAction {
    New,
    Model,
    Permissions,
    Status,
    Attach,
    Context,
    Settings,
    Stop,
}
struct SlashCommand {
    name: &'static str,
    description: &'static str,
    action: LocalAction,
}
const COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "/new",
        description: "Nova conversa neste projeto",
        action: LocalAction::New,
    },
    SlashCommand {
        name: "/model",
        description: "Escolher provedor, modelo e raciocínio",
        action: LocalAction::Model,
    },
    SlashCommand {
        name: "/permissions",
        description: "Escolher as permissões do agente",
        action: LocalAction::Permissions,
    },
    SlashCommand {
        name: "/status",
        description: "Ver tokens e limites da conversa",
        action: LocalAction::Status,
    },
    SlashCommand {
        name: "/attach",
        description: "Anexar arquivos ou imagens",
        action: LocalAction::Attach,
    },
    SlashCommand {
        name: "/context",
        description: "Anexar seleção ou arquivo do editor",
        action: LocalAction::Context,
    },
    SlashCommand {
        name: "/settings",
        description: "Configurações do AI Chat",
        action: LocalAction::Settings,
    },
    SlashCommand {
        name: "/stop",
        description: "Interromper o turno desta conversa",
        action: LocalAction::Stop,
    },
];
fn query(draft: &str) -> Option<String> {
    let query = draft.strip_prefix('/')?;
    (!query.chars().any(char::is_whitespace)).then(|| query.to_lowercase())
}
fn matches(query: &str) -> Vec<usize> {
    COMMANDS
        .iter()
        .enumerate()
        .filter_map(|(index, command)| command.name[1..].starts_with(query).then_some(index))
        .collect()
}
fn available(action: LocalAction, busy: bool) -> bool {
    match action {
        LocalAction::Model | LocalAction::Permissions => !busy,
        LocalAction::Stop => busy,
        _ => true,
    }
}
impl CodisseuApp {
    pub(super) fn ai_open_control(&mut self, response: &egui::Response, name: &'static str) {
        if self.ai_chat.open_control == Some(name) {
            egui::Popup::open_id(&response.ctx, egui::Popup::default_response_id(response));
            self.ai_chat.open_control = None;
        }
    }
    pub(super) fn ai_measure_prompt(&mut self, ui: &egui::Ui) {
        let focused = self.ai_chat.focus
            || ui
                .ctx()
                .memory(|m| m.has_focus(egui::Id::unique("codisseu.ai.composer")));
        let font = egui::TextStyle::Body.resolve(ui.style());
        let width = (ui.available_width() - 32.).max(24.);
        let cached = &self.ai_chat.prompt_measurement;
        if !cached
            .as_ref()
            .is_some_and(|(text, w, f, _)| text == &self.ai_chat.draft && *w == width && *f == font)
        {
            let height = ui.fonts_mut(|f| {
                f.layout(
                    self.ai_chat.draft.clone(),
                    font.clone(),
                    ui.visuals().text_color(),
                    width,
                )
                .size()
                .y
            });
            self.ai_chat.prompt_measurement =
                Some((self.ai_chat.draft.clone(), width, font, height));
        }
        let measured = self
            .ai_chat
            .prompt_measurement
            .as_ref()
            .map_or(64., |(_, _, _, h)| *h + 4.);
        let maximum = (ui.available_height() * 0.3).clamp(64., 180.);
        self.ai_chat.prompt_height = if focused {
            measured.clamp(64., maximum)
        } else {
            64.
        };
    }
    pub(super) fn ai_slash_keys(&mut self, ctx: &egui::Context) {
        if !ctx.memory(|m| m.has_focus(egui::Id::unique("codisseu.ai.composer"))) {
            return;
        }
        let Some(query) = query(&self.ai_chat.draft) else {
            return;
        };
        if self.ai_chat.slash.dismissed.as_ref() == Some(&query) {
            return;
        }
        if self.ai_chat.slash.query.as_ref() != Some(&query) {
            self.ai_chat.slash.index = 0;
        }
        self.ai_chat.slash.query = Some(query.clone());
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.ai_chat.slash.dismissed = Some(query);
            return;
        }
        let options = matches(&query);
        if options.is_empty() {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
            self.ai_chat.slash.index = (self.ai_chat.slash.index + 1) % options.len();
            self.ai_chat.slash.reveal = true;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
            self.ai_chat.slash.index =
                (self.ai_chat.slash.index + options.len() - 1) % options.len();
            self.ai_chat.slash.reveal = true;
        }
        if ctx.input_mut(|i| {
            i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::Tab)
        }) {
            self.ai_run_slash(options[self.ai_chat.slash.index.min(options.len() - 1)]);
        }
    }
    pub(super) fn ai_slash_menu(
        &mut self,
        _ui: &mut egui::Ui,
        response: &egui::Response,
        busy: bool,
    ) {
        let Some(query) = query(&self.ai_chat.draft) else {
            self.ai_chat.slash = Default::default();
            return;
        };
        if !response.has_focus() || self.ai_chat.slash.dismissed.as_ref() == Some(&query) {
            return;
        }
        if self.ai_chat.slash.query.as_ref() != Some(&query) {
            self.ai_chat.slash.index = 0;
        }
        self.ai_chat.slash.query = Some(query.clone());
        self.ai_chat.mention_open = false;
        let options = matches(&query);
        let mut picked = None;
        egui::Popup::from_response(response)
            .id(egui::Id::unique("codisseu.ai.slash"))
            .open(true)
            .align(egui::RectAlign::TOP_START)
            .width(response.rect.width().min(360.))
            .accessible_name("Comandos do AI Chat")
            .show(|ui| {
                ui.label(RichText::new("Comandos do Codisseu").small());
                egui::ScrollArea::vertical()
                    .max_height(230.)
                    .codisseu()
                    .show(ui, |ui| {
                        if options.is_empty() {
                            ui.label("Nenhum comando compatível");
                        }
                        for (index, &command_index) in options.iter().enumerate() {
                            let command = &COMMANDS[command_index];
                            let enabled = available(command.action, busy);
                            let row = ui.add_enabled(
                                enabled,
                                egui::Button::new(format!(
                                    "{}  ·  {}",
                                    command.name, command.description
                                ))
                                .wrap()
                                .selected(index == self.ai_chat.slash.index),
                            );
                            if index == self.ai_chat.slash.index && self.ai_chat.slash.reveal {
                                row.scroll_to_me(Some(egui::Align::Center));
                            }
                            if row.clicked() {
                                picked = Some(command_index);
                            }
                        }
                    });
            });
        self.ai_chat.slash.reveal = false;
        if let Some(index) = picked {
            self.ai_run_slash(index);
        }
    }
    fn ai_run_slash(&mut self, index: usize) {
        let action = COMMANDS[index].action;
        if !available(action, self.service.ai.is_running()) {
            return;
        }
        self.ai_chat.draft.clear();
        self.ai_chat.slash = Default::default();
        match action {
            LocalAction::New => self.ai_command(ai::Command::NewConversation),
            LocalAction::Model => {
                self.ai_chat.open_control = Some("Modelo e raciocínio");
            }
            LocalAction::Permissions => {
                self.ai_chat.open_control = Some("Permissões do agente");
            }
            LocalAction::Status => {
                self.ai_chat.open_control = Some("Uso de tokens e limites");
            }
            LocalAction::Attach => self.ai_chat.attachments.choose_files(
                self.service
                    .ai_workspace()
                    .map(std::path::Path::to_path_buf),
                self.ai_chat
                    .ui_context
                    .clone()
                    .expect("chat context initialized"),
            ),
            LocalAction::Context => self.ai_attach(),
            LocalAction::Settings => self.ai_chat.settings = true,
            LocalAction::Stop => self.ai_command(ai::Command::Cancel),
        }
        self.ai_chat.focus = true;
    }
}
