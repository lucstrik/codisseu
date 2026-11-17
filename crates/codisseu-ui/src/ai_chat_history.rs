//! Compact, searchable project history over the same conversation commands.
//! Geometry follows the installed reference; destructive actions keep Codisseu's
//! explicit two-click contract. No remote/archived history is implied.
use super::*;

impl CodisseuApp {
    pub(super) fn ai_history_button(&mut self, ui: &mut egui::Ui) {
        ui.add_enabled_ui(self.service.ai.profile_ready(), |ui| {
            let button = icon_button(ui, Icon::Clock, "Conversas", false, 28.)
                .on_hover_text("Buscar conversas deste projeto");
            if button.clicked() || self.ai_chat.open_control == Some("Conversas") {
                self.ai_chat.history_query.clear();
                self.ai_chat.history_index = 0;
                self.ai_chat.history_focus = true;
            }
            self.ai_open_control(&button, "Conversas");
            let width = self
                .ai_chat
                .panel_rect
                .map_or(274., |rect| rect.width() - 16.);
            let left = self
                .ai_chat
                .panel_rect
                .map_or(button.rect.left(), |rect| rect.left() + 8.);
            let menu = egui::Popup::menu(&button)
                .at_position(egui::pos2(left, button.rect.bottom() + 2.))
                .align(egui::RectAlign::BOTTOM_START)
                .align_alternatives(&[])
                .width(width.max(180.))
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .frame(
                    egui::Frame::popup(ui.style())
                        .inner_margin(self.tokens.ai_history_padding)
                        .corner_radius(self.tokens.ai_history_popup_radius),
                )
                .show(|ui| {
                    ui.set_width(
                        (width - 2. * f32::from(self.tokens.ai_history_padding) - 2.).max(160.),
                    );
                    self.ai_history_menu(ui);
                });
            if menu.is_none() {
                self.ai_chat.delete_confirm = None;
            }
        });
    }

    fn ai_history_menu(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing = egui::vec2(0., 0.);
        let search_id = egui::Id::unique("codisseu.ai.history.search");
        let search_focused = ui.ctx().memory(|memory| memory.has_focus(search_id));
        let down = search_focused
            && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown));
        let up = search_focused
            && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp));
        let enter =
            search_focused && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
        let query_before = self.ai_chat.history_query.clone();
        ui.horizontal(|ui| {
            ui.add_space(8.);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(16., 24.), egui::Sense::hover());
            components::paint_icon(ui, rect, Icon::Search, color(self.tokens.muted));
            ui.add_space(8.);
            let search = ui.add_sized(
                [ui.available_width() - 8., 28.],
                egui::TextEdit::singleline(&mut self.ai_chat.history_query)
                    .id(search_id)
                    .font(egui::FontId::proportional(self.tokens.ai_history_meta_size))
                    .hint_text("Buscar conversas recentes")
                    .frame(egui::Frame::new()),
            );
            search.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::accesskit::Role::TextInput,
                    true,
                    "Buscar conversas AI",
                )
            });
            if self.ai_chat.history_focus {
                search.request_focus();
                self.ai_chat.history_focus = false;
            }
            if search.changed() {
                self.ai_chat.history_index = 0;
                self.ai_chat.delete_confirm = None;
            }
        });
        ui.separator();
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Conversas deste projeto")
                        .size(self.tokens.ai_history_meta_size)
                        .color(color(self.tokens.muted)),
                );
            });
        let query = self.ai_chat.history_query.trim().to_lowercase();
        let conversations: Vec<_> = self
            .service
            .ai
            .conversations()
            .into_iter()
            .filter(|(_, title)| title.to_lowercase().contains(&query))
            .collect();
        let count = conversations.len();
        self.ai_chat.history_index = self.ai_chat.history_index.min(count.saturating_sub(1));
        let mut select = None;
        // TextEdit keeps text navigation; unmodified vertical arrows navigate
        // results while focus stays in the search field.
        if search_focused && count > 0 {
            if down {
                self.ai_chat.history_index = (self.ai_chat.history_index + 1).min(count - 1);
            }
            if up {
                self.ai_chat.history_index = self.ai_chat.history_index.saturating_sub(1);
            }
            if enter {
                select = Some(conversations[self.ai_chat.history_index].0.clone());
            }
        }
        let max_height = (ui.ctx().content_rect().bottom() - ui.next_widget_position().y - 16.)
            .clamp(
                self.tokens.ai_history_row_height,
                ui.ctx().content_rect().height() * 0.6,
            );
        egui::ScrollArea::vertical()
            .id_salt("codisseu.ai.history.results")
            .max_height(max_height)
            .auto_shrink([false, true])
            .codisseu()
            .show(ui, |ui| {
                if conversations.is_empty() {
                    egui::Frame::new().inner_margin(8.).show(ui, |ui| {
                        ui.label(if query.is_empty() {
                            "Nenhuma conversa neste projeto"
                        } else {
                            "Nenhuma conversa encontrada"
                        });
                    });
                }
                for (index, (id, title)) in conversations.iter().enumerate() {
                    let highlight = search_focused && index == self.ai_chat.history_index;
                    let reveal =
                        highlight && (up || down || query_before != self.ai_chat.history_query);
                    if self.ai_history_row(ui, id, title, highlight, true, reveal) {
                        select = Some(id.clone());
                    }
                }
            });
        if let Some(id) = select {
            self.ai_chat.delete_confirm = None;
            self.ai_command(ai::Command::SelectChat(id));
            self.ai_chat.cache.clear();
            self.ai_chat.focus = true;
            ui.close();
        }
    }

    pub(super) fn ai_recent_chats(&mut self, ui: &mut egui::Ui) -> bool {
        let chats: Vec<_> = self
            .service
            .ai
            .conversations()
            .into_iter()
            .filter(|(id, title)| {
                self.service.ai.active_chat_id() != Some(id.as_str()) || title != "Nova conversa"
            })
            .collect();
        if chats.is_empty() {
            return false;
        }
        ui.label(
            RichText::new("Conversas recentes")
                .size(self.tokens.ai_history_meta_size)
                .color(color(self.tokens.muted)),
        );
        let mut select = None;
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 1.;
            for (id, title) in chats.iter().take(3) {
                if self.ai_history_row(ui, id, title, false, false, false) {
                    select = Some(id.clone());
                }
            }
        });
        if ui
            .add(egui::Button::new(format!("Ver todas ({})", chats.len())).frame(false))
            .clicked()
        {
            self.ai_chat.open_control = Some("Conversas");
        }
        if let Some(id) = select {
            self.ai_command(ai::Command::SelectChat(id));
            self.ai_chat.cache.clear();
        }
        true
    }

    fn ai_history_row(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        title: &str,
        highlight: bool,
        deletable: bool,
        reveal: bool,
    ) -> bool {
        ui.push_id(("ai.history.row", id), |ui| {
            let row = ui
                .allocate_exact_size(
                    egui::vec2(ui.available_width(), self.tokens.ai_history_row_height),
                    egui::Sense::hover(),
                )
                .0;
            let confirming = self.ai_chat.delete_confirm.as_deref() == Some(id);
            let action_width = if confirming { 80. } else { 26. };
            let title_area = egui::Rect::from_min_max(
                row.min,
                egui::pos2(
                    row.right() - if deletable { action_width } else { 0. },
                    row.bottom(),
                ),
            );
            let response = ui.interact(
                title_area,
                ui.scope_id().with("select"),
                egui::Sense::click(),
            );
            let selected = self.service.ai.active_chat_id() == Some(id);
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::accesskit::Role::Button, true, selected, title)
            });
            let status = self.service.ai.chat_status(id);
            let approval = self.service.ai.chat_approval_count(id) > 0;
            let state = conversation_status(status, approval);
            if let Some(state) = state {
                ui.ctx()
                    .accesskit_node_builder(response.id, |node| node.set_description(state));
            }
            let hovered = ui.rect_contains_pointer(row);
            if reveal {
                response.scroll_to_me(Some(egui::Align::Center));
            }
            if hovered || response.has_focus() || highlight || selected {
                ui.painter()
                    .rect_filled(row.shrink(1.), 6., ui.visuals().widgets.hovered.bg_fill);
            }
            let mut left = row.left() + 8.;
            if approval
                || matches!(
                    status,
                    Some(ai::State::Running | ai::State::Cancelling | ai::State::Failed)
                )
            {
                ui.painter().circle_filled(
                    egui::pos2(left + 3., row.center().y),
                    2.5,
                    color(if approval {
                        self.tokens.accent
                    } else if status == Some(ai::State::Failed) {
                        self.tokens.error
                    } else {
                        self.tokens.muted
                    }),
                );
                left += 12.;
            }
            let meta = self.service.ai.chat_updated_at(id).and_then(age_label);
            let meta_width = meta.as_ref().map_or(0., |age| {
                ui.painter()
                    .layout_no_wrap(
                        age.clone(),
                        egui::FontId::proportional(self.tokens.ai_history_meta_size),
                        color(self.tokens.muted),
                    )
                    .size()
                    .x
                    + 12.
            });
            let right = row.right()
                - 8.
                - if confirming {
                    action_width
                } else {
                    meta_width.max(if deletable && hovered {
                        action_width
                    } else {
                        0.
                    })
                };
            let mut job = egui::text::LayoutJob::simple(
                title.to_owned(),
                egui::FontId::proportional(self.tokens.ai_history_title_size),
                ui.visuals().text_color(),
                (right - left).max(1.),
            );
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
            ui.painter().galley(
                egui::pos2(left, row.center().y - galley.size().y / 2.),
                galley,
                ui.visuals().text_color(),
            );
            let mut action_focused = false;
            if deletable {
                let running = matches!(status, Some(ai::State::Running | ai::State::Cancelling));
                let scope = match self.service.ai.external_history_provider(id) {
                    Some(Provider::Codex) => "Apagar do Codisseu e do Codex",
                    Some(Provider::Claude) => {
                        "Remover do Codisseu; histórico externo do Claude é preservado"
                    }
                    None => "Remover do histórico deste projeto",
                };
                let action_rect = egui::Rect::from_min_max(
                    egui::pos2(row.right() - action_width, row.top()),
                    row.max,
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(action_rect), |ui| {
                    ui.add_enabled_ui(!running, |ui| {
                        ui.push_id("delete", |ui| {
                            if confirming {
                                let confirm = ui
                                    .add_sized(
                                        [80., 26.],
                                        egui::Button::new(
                                            RichText::new("Confirmar")
                                                .color(color(self.tokens.error)),
                                        )
                                        .frame(false),
                                    )
                                    .on_hover_text(scope);
                                confirm.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::accesskit::Role::Button,
                                        !running,
                                        format!("Confirmar exclusão {title}"),
                                    )
                                });
                                if confirm.clicked() {
                                    self.ai_chat.delete_confirm = None;
                                    self.ai_command(ai::Command::DeleteChat(id.to_owned()));
                                    self.ai_chat.cache.clear();
                                    ui.close();
                                }
                            } else {
                                let delete = ui
                                    .add_sized([26., 26.], egui::Button::new("").frame(false))
                                    .on_hover_text(scope);
                                delete.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::accesskit::Role::Button,
                                        !running,
                                        format!("Apagar conversa {title}"),
                                    )
                                });
                                action_focused = delete.has_focus();
                                if hovered || response.has_focus() || action_focused {
                                    components::paint_icon(
                                        ui,
                                        delete.rect,
                                        Icon::Delete,
                                        color(self.tokens.muted),
                                    );
                                }
                                if delete.clicked() {
                                    self.ai_chat.delete_confirm = Some(id.to_owned());
                                }
                            }
                        });
                    });
                });
            }
            if let Some(age) = meta
                && !hovered
                && !confirming
                && !response.has_focus()
                && !action_focused
            {
                ui.painter().text(
                    egui::pos2(row.right() - 8., row.center().y),
                    egui::Align2::RIGHT_CENTER,
                    age,
                    egui::FontId::proportional(self.tokens.ai_history_meta_size),
                    color(self.tokens.muted),
                );
            }
            response
                .on_hover_text(
                    state.map_or_else(|| title.to_owned(), |state| format!("{title}\n{state}")),
                )
                .clicked()
        })
        .inner
    }
}

fn age_label(updated_at: u64) -> Option<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    let seconds = now.checked_sub(updated_at)? / 1000;
    Some(if seconds < 60 {
        "agora".into()
    } else if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86400 {
        format!("{}h", seconds / 3600)
    } else {
        format!("{}d", seconds / 86400)
    })
}
