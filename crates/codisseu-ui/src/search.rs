//! Document-local literal search. All text operations use the application service.
use super::*;
use std::ops::Range;

type SearchKey = (DocumentId, u64, String);

#[derive(Default)]
pub(super) struct SearchPanel {
    pub open: bool,
    replace: bool,
    query: String,
    replacement: String,
    focus: bool,
    submitted: Option<SearchKey>,
    received: Option<SearchKey>,
    current: Option<usize>,
    pub selection: Option<(DocumentId, u64, Range<usize>)>,
}

fn field_width(ui: &egui::Ui, tokens: &Tokens, following_buttons: &[&str]) -> f32 {
    let reserve: f32 = following_buttons
        .iter()
        .map(|label| {
            ui.fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap(
                        (*label).into(),
                        FontId::proportional(tokens.ui_font_size),
                        color(tokens.text),
                    )
                    .size()
                    .x
            }) + 2.0 * ui.spacing().button_padding.x
                + ui.spacing().item_spacing.x
        })
        .sum();
    (ui.available_size_before_wrap().x - reserve - 2.0 * ui.spacing().item_spacing.x)
        .clamp(tokens.find_field_min_width, tokens.find_field_max_width)
}

impl CodisseuApp {
    pub(super) fn open_search(&mut self, replace: bool) {
        if self.service.active.is_none() {
            return;
        }
        self.search_panel.open = true;
        self.search_panel.replace = replace;
        self.search_panel.focus = true;
        self.focus_editor = false;
    }

    fn close_search(&mut self) {
        let selection = self.current_search_range();
        self.search_panel.open = false;
        self.search_panel.submitted = None;
        self.search_panel.received = None;
        self.search_panel.selection = selection;
        self.send(Command::CancelSearch);
        self.focus_editor = true;
    }

    pub(super) fn search_input_focused(&self, ctx: &egui::Context) -> bool {
        ctx.memory(|m| {
            ["codisseu.find.query", "codisseu.find.replacement"]
                .iter()
                .any(|id| m.has_focus(egui::Id::unique(*id)))
        })
    }

    fn search_valid(&self) -> bool {
        self.search_panel.open
            && self.service.search.as_ref().is_some_and(|result| {
                self.service.active_document().is_some_and(|doc| {
                    result.document_id == doc.id
                        && result.version == doc.version
                        && result.query == self.search_panel.query
                })
            })
    }

    pub(super) fn current_search_range(&self) -> Option<(DocumentId, u64, Range<usize>)> {
        if !self.search_valid() {
            return None;
        }
        let result = self.service.search.as_ref()?;
        let range = result.ranges.get(self.search_panel.current?)?.clone();
        Some((result.document_id, result.version, range))
    }

    fn sync_search(&mut self) {
        if !self.search_panel.open {
            return;
        }
        let key = self
            .service
            .active_document()
            .map(|doc| (doc.id, doc.version, self.search_panel.query.clone()));
        if key != self.search_panel.submitted {
            self.send(Command::CancelSearch);
            self.search_panel.received = None;
            self.search_panel.current = None;
            self.search_panel.selection = None;
            self.search_panel.submitted = key.clone();
            if let Some((id, expected_version, query)) = &key
                && !query.is_empty()
            {
                self.send(Command::Find {
                    id: *id,
                    expected_version: *expected_version,
                    query: query.clone(),
                });
            }
        }
        if self.search_valid() && self.search_panel.received != key {
            self.search_panel.received = key;
            self.search_panel.current = None;
            self.navigate_match(false);
        }
    }

    pub(super) fn navigate_match(&mut self, previous: bool) {
        if !self.search_valid() {
            return;
        }
        let result = self.service.search.as_ref().unwrap();
        let count = result.ranges.len();
        if count == 0 {
            return;
        }
        let index = match self.search_panel.current {
            None => {
                if previous {
                    count - 1
                } else {
                    0
                }
            }
            Some(index) => {
                if previous {
                    (index + count - 1) % count
                } else {
                    (index + 1) % count
                }
            }
        };
        self.search_panel.current = Some(index);
        self.search_panel.selection = Some((
            result.document_id,
            result.version,
            result.ranges[index].clone(),
        ));
    }

    fn replace_matches(&mut self, all: bool) {
        if !self.search_valid() {
            return;
        }
        let result = self.service.search.as_ref().unwrap();
        let selection = if all {
            None
        } else {
            let Some(index) = self.search_panel.current else {
                return;
            };
            let Some(range) = result.ranges.get(index) else {
                return;
            };
            Some(range.clone())
        };
        self.send(Command::ReplaceMatches {
            id: result.document_id,
            expected_version: result.version,
            query: result.query.clone(),
            replacement: self.search_panel.replacement.clone(),
            selection,
        });
        // The button edits the document; subsequent Undo belongs to that edit.
        self.focus_editor = true;
    }

    pub(super) fn search_bar(&mut self, ui: &mut egui::Ui) {
        if !self.search_panel.open {
            return;
        }
        self.sync_search();
        let tokens = self.tokens.clone();
        let mut next = false;
        let mut previous = false;
        let mut replace_one = false;
        let mut replace_all = false;
        let mut close = false;
        let valid = self.search_valid();
        let count = if valid {
            self.service.search.as_ref().unwrap().ranges.len()
        } else {
            0
        };
        let summary = if self.search_panel.query.is_empty() {
            "Busca literal · diferencia maiúsculas".to_owned()
        } else if self.service.search_pending() {
            "Buscando…".to_owned()
        } else if valid {
            let result = self.service.search.as_ref().unwrap();
            if result.truncated {
                format!(
                    "{} de {}+ · limite de navegação",
                    self.search_panel.current.map_or(0, |i| i + 1),
                    count
                )
            } else {
                format!(
                    "{} de {}",
                    self.search_panel.current.map_or(0, |i| i + 1),
                    count
                )
            }
        } else {
            "Sem resultado atual".to_owned()
        };
        egui::Frame::new()
            .fill(color(tokens.panel))
            .inner_margin(egui::Margin::symmetric(
                tokens.find_padding[0],
                tokens.find_padding[1],
            ))
            .show(ui, |ui| {
                tokens.form_style(ui);
                ui.set_min_width(ui.available_width());
                // Rows wrap on narrow editors; controls never spill over Database.
                ui.horizontal_wrapped(|ui| {
                    let label = ui.label("Localizar");
                    let width = field_width(ui, &tokens, &["Anterior", "Próximo", "Fechar busca"]);
                    let field = ui
                        .add(
                            egui::TextEdit::singleline(&mut self.search_panel.query)
                                .id(egui::Id::unique("codisseu.find.query"))
                                .desired_width(width)
                                .char_limit(4096),
                        )
                        .labelled_by(label.id);
                    if self.search_panel.focus {
                        field.request_focus();
                        self.search_panel.focus = false;
                    }
                    if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        previous = ui.input(|i| i.modifiers.shift);
                        next = !previous;
                        field.request_focus();
                    }
                    previous |= ui
                        .add_enabled(valid && count > 0, egui::Button::new("Anterior"))
                        .on_hover_text("Shift+F3")
                        .clicked();
                    next |= ui
                        .add_enabled(valid && count > 0, egui::Button::new("Próximo"))
                        .on_hover_text("F3")
                        .clicked();
                    close = ui.button("Fechar busca").on_hover_text("Escape").clicked();
                });
                if self.search_panel.replace {
                    ui.horizontal_wrapped(|ui| {
                        let label = ui.label("Substituir por");
                        let width = field_width(
                            ui,
                            &tokens,
                            &["Substituir ocorrência", "Substituir todas"],
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut self.search_panel.replacement)
                                .id(egui::Id::unique("codisseu.find.replacement"))
                                .desired_width(width)
                                .char_limit(4096),
                        )
                        .labelled_by(label.id);
                        let enabled = valid && count > 0 && !self.service.is_busy();
                        replace_one = ui
                            .add_enabled(enabled, egui::Button::new("Substituir ocorrência"))
                            .clicked();
                        replace_all = ui
                            .add_enabled(enabled, egui::Button::new("Substituir todas"))
                            .clicked();
                    });
                }
                ui.label(RichText::new(summary).color(color(tokens.muted)));
                if let Some(error) = &self.service.search_error {
                    crate::status_messages::status_message(ui, error);
                }
            });
        self.sync_search();
        if previous {
            self.navigate_match(true);
        }
        if next {
            self.navigate_match(false);
        }
        if replace_one {
            self.replace_matches(false);
        }
        if replace_all {
            self.replace_matches(true);
        }
        if close {
            self.close_search();
        }
    }

    pub(super) fn search_shortcuts(&mut self, ctx: &egui::Context) {
        let take = |modifiers, key| {
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(modifiers, key)))
        };
        if take(Modifiers::COMMAND, Key::F) {
            self.open_search(false);
        }
        if take(Modifiers::COMMAND, Key::H) {
            self.open_search(true);
        }
        if self.search_panel.open {
            if take(Modifiers::SHIFT, Key::F3) {
                self.navigate_match(true);
            } else if take(Modifiers::NONE, Key::F3) {
                self.navigate_match(false);
            }
            if !egui::Popup::is_any_open(ctx) && take(Modifiers::NONE, Key::Escape) {
                self.close_search();
            }
        }
    }
}
