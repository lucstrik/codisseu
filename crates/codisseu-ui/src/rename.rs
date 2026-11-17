//! Semantic rename: the provider proposes edits; AppService validates and applies.
use super::*;
mod conflicts;
mod inheritors;
mod inline;
mod name_input;
mod related;
mod source;
mod tree;

#[derive(Default)]
pub(super) struct RenameUi {
    labels: crate::tab_label::TabLabels,
    snippets: crate::signature_paint::SignaturePaint,
    name_input: name_input::NameInput,
    inheritors: inheritors::Chooser,
    inline: Option<inline::InlineRename>,
    options: codisseu_core::RenameOptions,
    apply_intent: Option<(u64, String)>,
    input_error: Option<String>,
    collapsed: HashSet<String>,
    visible: Vec<usize>,
    dialog: bool,
    pending_path: Option<PathBuf>,
    related_selected: usize,
    related_token: Option<u64>,
    focus_related: bool,
    conflict_open: bool,
    blocked_token: Option<u64>,
    conflict_selected: usize,
    candidate_selected: Option<usize>,
    focus_candidates: bool,
    focus_conflict: bool,
    focus_name: bool,
    initialized: bool,
    name: String,
    selected: usize,
    move_after_click: Option<isize>,
    focus_results: bool,
    show_after: bool,
    split_ratio: Option<f32>,
    preview_target: Option<(Option<u64>, usize, bool)>,
}
impl RenameUi {
    fn select_neighbor(&mut self, direction: isize) {
        if self.visible.is_empty() {
            return;
        }
        let current = self
            .visible
            .iter()
            .position(|i| *i == self.selected)
            .unwrap_or(0);
        self.selected = self.visible[current
            .saturating_add_signed(direction)
            .min(self.visible.len() - 1)];
    }
}

fn row_id(token: Option<u64>, index: usize) -> egui::Id {
    egui::Id::unique(("codisseu.rename.row", token, index))
}
fn click_before_vertical_arrow(ctx: &egui::Context) -> bool {
    ctx.input(|i| {
        let click = i.events.iter().rposition(|e| {
            matches!(
                e,
                egui::Event::PointerButton {
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    ..
                }
            )
        });
        let arrow = i.events.iter().rposition(|e| {
            matches!(
                e,
                egui::Event::Key {
                    key: Key::ArrowDown | Key::ArrowUp,
                    pressed: true,
                    ..
                }
            )
        });
        matches!((click,arrow),(Some(c),Some(a))if c<a)
    })
}
impl CodisseuApp {
    pub(super) fn request_file_rename(&mut self, path: PathBuf) {
        self.close_documentation();
        self.send(Command::CancelRename);
        self.rename_ui = RenameUi {
            pending_path: Some(path.clone()),
            ..Default::default()
        };
        self.focus_editor = false;
        if !self
            .service
            .active_document()
            .is_some_and(|doc| doc.path == path)
            && let Err(error) = self.service.dispatch(Command::OpenFile(path))
        {
            self.service.last_error = Some(error);
            self.rename_ui.pending_path = None;
        }
    }
    fn prepare_pending_file_rename(&mut self) {
        let Some(path) = self.rename_ui.pending_path.as_ref() else {
            return;
        };
        self.focus_editor = false;
        let origin = self
            .service
            .active_document()
            .filter(|doc| doc.path == *path)
            .map(|doc| (doc.id, doc.version));
        if let Some((id, expected_version)) = origin {
            self.rename_ui.pending_path = None;
            match self.service.dispatch(Command::PrepareFileRename {
                id,
                expected_version,
            }) {
                Ok(()) => {
                    self.rename_ui.dialog = true;
                    self.rename_ui.focus_name = true;
                }
                Err(error) => self.service.last_error = Some(error),
            }
        } else if !self.service.is_busy() {
            self.rename_ui.pending_path = None;
        }
    }
    pub(super) fn request_rename(&mut self) {
        self.close_documentation();
        let Some(doc) = self.service.active_document() else {
            return;
        };
        let id = doc.id;
        let expected_version = doc.version;
        let character_index = self
            .caret
            .filter(|(owner, _)| *owner == id)
            .map_or(0, |(_, i)| i);
        let Some(view) = self.service.active_view() else {
            return;
        };
        self.request_rename_at(view, id, expected_version, character_index);
    }
    /// A context action belongs to its source view and immutable click snapshot.
    /// The selection head may be on another symbol when text remains selected.
    pub(super) fn request_rename_at(
        &mut self,
        view: codisseu_core::ViewId,
        id: codisseu_core::DocumentId,
        expected_version: u64,
        character_index: usize,
    ) {
        if self.service.active_view() != Some(view)
            || !self.service.view_document(view).is_some_and(|doc| {
                doc.id == id && doc.version == expected_version && !doc.is_read_only()
            })
        {
            self.service.last_error = Some("O editor mudou; abra Refatorar novamente".into());
            return;
        }
        self.close_documentation();
        if let Err(error) = self.service.dispatch(Command::PrepareRename {
            id,
            expected_version,
            character_index,
        }) {
            self.service.last_error = Some(error);
            return;
        }
        self.rename_ui = RenameUi {
            dialog: true,
            focus_name: true,
            ..Default::default()
        };
    }
    pub(super) fn request_rename_inline(&mut self) {
        let origin = self.service.active_document().map(|doc| {
            let caret = self
                .caret
                .filter(|(id, _)| *id == doc.id)
                .map_or(0, |(_, at)| at);
            inline::InlineRename::new(doc.id, doc.version, caret)
        });
        self.request_rename();
        self.rename_ui.inline = origin;
    }
    fn cancel_rename(&mut self) {
        self.send(Command::CancelRename);
        self.rename_ui.dialog = false;
        self.rename_ui.pending_path = None;
        self.rename_ui.inline = None;
        self.rename_ui.apply_intent = None;
        if self.tool_tab == ToolTab::Rename {
            self.show_activity = false;
        }
        self.focus_editor = true;
    }
    pub(super) fn rename_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if self.rename_ui.dialog
            && self.rename_ui.inline.is_some()
            && ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::F6))
        {
            self.rename_ui.inline = None;
            self.rename_ui.focus_name = true;
        }
        if self.rename_ui.pending_path.is_some() {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                self.cancel_rename();
            }
            return true;
        }
        if self.rename_ui.conflict_open
            || self.rename_ui.dialog
            || self.service.rename.choosing_methods()
            || self.service.rename.choosing_inheritors()
        {
            return true;
        }
        if self.tool_tab != ToolTab::Rename || !self.show_activity {
            return false;
        }
        let token = self.service.rename.context_token();
        let count: usize = self
            .service
            .rename
            .files
            .iter()
            .map(|file| file.edits.len())
            .sum();
        let focused = ctx
            .memory(|m| m.focused())
            .is_some_and(|id| (0..count).any(|n| id == row_id(token, n)));
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.cancel_rename();
            return true;
        }
        if !focused || count == 0 {
            return false;
        }
        let click_before_arrow = click_before_vertical_arrow(ctx);
        let direction = if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
            1
        } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
            -1
        } else {
            0
        };
        if direction != 0 {
            // Native X11 can deliver a click and the following arrow in one frame.
            // Render the click before moving from the clicked row in that case.
            if click_before_arrow {
                self.rename_ui.move_after_click = Some(direction);
            } else {
                self.rename_ui.select_neighbor(direction);
                self.rename_ui.focus_results = true;
            }
            ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
            return true;
        }
        false
    }
    pub(super) fn rename_dialog(&mut self, ctx: &egui::Context) {
        self.prepare_pending_file_rename();
        if self.service.rename.choosing_inheritors() {
            self.rename_inheritors_dialog(ctx);
            return;
        }
        // Session-only history contains names whose proposal passed validation.
        // It is bounded and survives opening another Rename dialog.
        if self.service.rename.can_apply() && self.service.rename.error.is_none() {
            let name = self.service.rename.new_name.clone();
            ctx.data_mut(|data| {
                let id = egui::Id::unique("codisseu.rename.verified_names");
                let mut history = data.get_temp::<Vec<String>>(id).unwrap_or_default();
                if !name.is_empty() && history.last() != Some(&name) {
                    history.retain(|previous| previous != &name);
                    history.push(name);
                    if history.len() > 20 {
                        history.remove(0);
                    }
                    data.insert_temp(id, history);
                }
            });
        }
        if let Some((token, name)) = &self.rename_ui.apply_intent {
            if self.service.rename.context_token() != Some(*token)
                || self.service.rename.new_name != *name
                || self.service.rename.error.is_some()
                || self.service.rename.semantic_blocked()
            {
                self.rename_ui.apply_intent = None;
            } else if self.service.rename.can_apply()
                && !self.service.rename.is_pending()
                && !self.service.rename.choosing_methods()
                && !self.service.rename.choosing_inheritors()
            {
                self.rename_ui.apply_intent = None;
                self.send(Command::ApplyRename);
                self.focus_editor = true;
            }
        }
        if !self.rename_ui.dialog {
            return;
        }
        if self.service.rename.is_prepared() && !self.rename_ui.initialized {
            self.rename_ui.name = self.service.rename.placeholder.clone().unwrap_or_default();
            self.rename_ui.options = self.service.rename.options();
            self.rename_ui.initialized = true;
            self.rename_ui.focus_name = true;
        }
        if self.rename_ui.inline.is_some() {
            self.rename_inline(ctx);
            return;
        }
        let php = self
            .service
            .active_document()
            .is_some_and(|d| d.path.extension().is_some_and(|e| e == "php"));
        let rust = self.service.active_document().is_some_and(|d| {
            matches!(
                codisseu_core::Language::for_path(&d.path),
                Some(codisseu_core::Language::Rust | codisseu_core::Language::Python)
            )
        });
        let kind = self.service.rename.symbol_kind();
        let file_rename = self.service.rename.is_file_rename();
        let extended = file_rename
            || !php
            || matches!(
                kind,
                codisseu_core::RenameSymbolKind::Property | codisseu_core::RenameSymbolKind::Class
            );
        let title = if file_rename {
            format!(
                "Rename file '{}' and its usages to:",
                self.service
                    .rename
                    .placeholder
                    .as_deref()
                    .unwrap_or_default()
            )
        } else {
            rename_title(kind, php, self.service.rename.placeholder.as_deref())
        };
        let mut preview = false;
        let mut refactor = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::unique("codisseu.rename.dialog"))
            .area(
                egui::Modal::default_area(egui::Id::unique("codisseu.rename.dialog"))
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(if (php && !file_rename) || (file_rename && !php) { -1. } else { 0. }, 0.)),
            )
            .backdrop_color(Color32::TRANSPARENT)
            .frame(egui::Frame::new().fill(color(self.tokens.panel)))
            .show(ctx, |ui| {
                self.tokens.form_style(ui);
                ui.visuals_mut().override_text_color = Some(color(self.tokens.rename_text));
                let width = (ctx.content_rect().width() - 64.).clamp(
                    240.,
                    if file_rename && php {
                        358.
                    } else if php || file_rename {
                        self.tokens.rename_dialog_width
                    } else {
                        self.tokens.rename_ts_dialog_width
                    },
                );
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(
                        width,
                        if extended {
                            self.tokens.rename_extended_dialog_height
                        } else {
                            self.tokens.rename_dialog_height
                        },
                    ),
                    egui::Sense::hover(),
                );
                let margin = self.tokens.rename_dialog_margin as f32;
                let title_rect = egui::Rect::from_min_size(
                    rect.min + egui::vec2(margin + 1., 7.),
                    egui::vec2(width - 2. * margin, 17.),
                );
                let label = ui.put(
                    title_rect,
                    egui::Label::new(
                        RichText::new(if file_rename { "Renomear arquivo e suas referências para:" } else { "Renomear símbolo e seus usos para:" })
                            .color(Color32::TRANSPARENT),
                    ),
                );
                source::paint_label(
                    &mut self.rename_ui.labels,
                    ui,
                    &title,
                    title_rect.left_center(),
                    title_rect,
                    color(self.tokens.panel),
                    &self.tokens,
                );
                let field = egui::Rect::from_min_size(
                    rect.min + egui::vec2(margin + 3., 36.),
                    egui::vec2(width - 2. * (margin + 3.), self.tokens.control_height),
                );
                let mut input = ui.new_child(egui::UiBuilder::new().max_rect(field));
                // Selection foreground and frame outline are independent. A
                // panel-colored selection stroke makes selected text unreadable.
                input.visuals_mut().selection.stroke.color = color(self.tokens.rename_text);
                input.visuals_mut().selection.bg_fill = color(self.tokens.rename_name_selection);
                let focused = ctx.memory(|m| m.has_focus(egui::Id::unique("codisseu.rename.name")));
                let field_frame = egui::Frame::new()
                    .fill(color(self.tokens.panel))
                    .corner_radius(self.tokens.control_corner_radius)
                    .inner_margin(egui::Margin { left:self.tokens.rename_name_padding[0],
                        right:self.tokens.rename_name_padding[0]+28,
                        top:self.tokens.rename_name_padding[1], bottom:self.tokens.rename_name_padding[1] })
                    .apply_stroke_and_expansion_without_layout_shift(
                        if focused {
                            egui::Stroke::new(2., color(self.tokens.panel))
                        } else {
                            egui::Stroke::new(1., color(self.tokens.control_border))
                        },
                        0.,
                    );
                if !self.service.rename.is_prepared() || self.service.rename.is_pending() {
                    input.disable();
                }
                let response = self
                    .rename_ui
                    .name_input
                    .show(
                        &mut input,
                        &mut self.rename_ui.name,
                        field,
                        field_frame,
                        &self.tokens,
                    )
                    .labelled_by(label.id);
                if response.changed() { self.rename_ui.input_error = None; }
                let history_rect = egui::Rect::from_min_max(
                    egui::pos2(field.right()-28.,field.top()+1.), field.max-egui::vec2(1.,1.));
                let history_button = ui.put(history_rect,
                    egui::Button::new(RichText::new("v").color(Color32::TRANSPARENT)).frame(false));
                history_button.widget_info(||egui::WidgetInfo::labeled(egui::accesskit::Role::Button,true,"Nomes verificados"));
                ui.painter().vline(history_rect.left(),history_rect.y_range(), egui::Stroke::new(1., color(self.tokens.control_border)));
                name_input::chevron(ui,history_rect.center(),color(self.tokens.rename_input_text));
                egui::Popup::menu(&history_button).width(220.).show(|ui| {
                    let mut history = ctx.data(|data| data.get_temp::<Vec<String>>(egui::Id::unique("codisseu.rename.verified_names"))).unwrap_or_default();
                    if let Some(original) = &self.service.rename.placeholder {
                        history.retain(|name| name != original);
                        history.push(original.clone());
                    }
                    for name in history.iter().rev() {
                        if ui.button(name).clicked() {
                            self.rename_ui.name.clone_from(name);
                            self.rename_ui.focus_name = true;
                            self.rename_ui.input_error = None;
                            ui.close();
                        }
                    }
                });
                if response.has_focus() {
                    // PS-262 Dark, native scale 1: focused outline extends one
                    // point beyond the field; keep text/inactive geometry fixed.
                    ui.painter().rect_stroke(
                        field.expand(1.),
                        egui::CornerRadius::same(
                            self.tokens.control_corner_radius.saturating_add(1),
                        ),
                        egui::Stroke::new(2., color(self.tokens.accent)),
                        egui::StrokeKind::Inside,
                    );
                }
                if self.rename_ui.focus_name && response.enabled() {
                    response.request_focus();
                    if let Some(mut state) = egui::TextEdit::load_state(ctx, response.id) {
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(if file_rename {
                                    std::path::Path::new(&self.rename_ui.name).file_stem().and_then(|s| s.to_str()).map_or(self.rename_ui.name.chars().count(), |s| s.chars().count())
                                } else { self.rename_ui.name.chars().count() }),
                            )));
                        state.store(ctx, response.id);
                    }
                    self.rename_ui.focus_name = false;
                }
                let enter = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                let comments = egui::Rect::from_min_size(
                    rect.min + egui::vec2(if php || file_rename { 15. } else { 246. }, if php && !file_rename { 75. } else { 104. }),
                    egui::vec2(if php { 233. } else { 225. }, 24.),
                );
                let text = egui::Rect::from_min_size(
                    rect.min + egui::vec2(if php || file_rename { 252. } else { 246. }, 75.),
                    egui::vec2(if php { 202. } else { 225. }, 24.),
                );
                if rust {
                    ui.put(egui::Rect::from_min_size(rect.min+egui::vec2(15.,75.),egui::vec2(width-30.,45.)),egui::Label::new("Somente referências semânticas. Comentários e strings permanecem inalterados.").wrap());
                }else{
                name_input::checkbox(ui, &mut self.rename_ui.labels, &self.tokens,
                    "Search in comments and strings", comments,
                    &mut self.rename_ui.options.search_comments_strings);
                }
                if !rust && (!file_rename || !php) {
                    name_input::checkbox(ui, &mut self.rename_ui.labels, &self.tokens,
                        "Search for text occurrences", text,
                        &mut self.rename_ui.options.search_text);
                }
                if file_rename {
                    name_input::checkbox(ui, &mut self.rename_ui.labels, &self.tokens,
                        "Search for references", egui::Rect::from_min_size(
                            rect.min + egui::vec2(15.,75.), egui::vec2(233.,24.)),
                        &mut self.rename_ui.options.search_file_references)
                        .on_hover_text("Desativar referências renomeia apenas o arquivo; imports e includes existentes não são atualizados.");
                }
                if !rust && !php && !file_rename {
                    name_input::checkbox(ui, &mut self.rename_ui.labels, &self.tokens,
                        "Search in JavaScript files", egui::Rect::from_min_size(
                            rect.min + egui::vec2(15.,75.), egui::vec2(226.,24.)),
                        &mut self.rename_ui.options.search_javascript)
                        .on_hover_text("Excluir arquivos JavaScript exige que nenhum vínculo comprovado seja omitido.");
                    name_input::checkbox(ui, &mut self.rename_ui.labels, &self.tokens,
                        "Search for dynamic references", egui::Rect::from_min_size(
                            rect.min + egui::vec2(15.,104.), egui::vec2(226.,24.)),
                        &mut self.rename_ui.options.search_dynamic_references)
                        .on_hover_text("Exibir referências incertas para revisão; sem prova a aplicação continua bloqueada.");
                }
                if php && !file_rename && kind == codisseu_core::RenameSymbolKind::Property {
                    name_input::checkbox(ui, &mut self.rename_ui.labels, &self.tokens,
                        "Rename accessors", egui::Rect::from_min_size(rect.min + egui::vec2(15.,104.), egui::vec2(233.,24.)),
                        &mut self.rename_ui.options.rename_accessors);
                }
                if php && !file_rename && kind == codisseu_core::RenameSymbolKind::Class {
                    name_input::checkbox(ui, &mut self.rename_ui.labels, &self.tokens,
                        "Rename inheritors", egui::Rect::from_min_size(rect.min + egui::vec2(15.,104.), egui::vec2(233.,24.)),
                        &mut self.rename_ui.options.rename_inheritors);
                }
                if self.service.rename.is_pending() {
                    ui.put(egui::Rect::from_min_size(rect.min + egui::vec2(margin, 104.),
                        egui::vec2(230., 20.)), egui::Label::new("Consultando o servidor…"));
                }
                let scope = egui::Rect::from_min_size(
                    rect.min + egui::vec2(margin + 1., if extended { 140. } else { 111. }),
                    egui::vec2(width - 2. * margin, 17.),
                );
                source::accessible_label(
                    &mut self.rename_ui.labels,
                    ui,
                    "Scope:",
                    scope,
                    &self.tokens,
                );
                ui.painter().hline(
                    scope.left() + 49. ..=scope.right(),
                    scope.center().y,
                    egui::Stroke::new(1., color(self.tokens.border)),
                );
                let scope_value = egui::Rect::from_min_size(
                    rect.min + egui::vec2(margin + 3., if extended { 165. } else { 136. }),
                    egui::vec2(width - 2. * (margin + 3.), self.tokens.control_height),
                );
                let scope_selector = egui::Rect::from_min_max(scope_value.min, scope_value.max-egui::vec2(34.,0.));
                let mut scope_ui = ui.new_child(egui::UiBuilder::new().max_rect(scope_selector));
                if rust {scope_ui.disable();}
                scope_ui.spacing_mut().interact_size.y = self.tokens.control_height;
                scope_ui.spacing_mut().button_padding = egui::vec2(9., 5.);
                scope_ui.visuals_mut().widgets.inactive.weak_bg_fill = color(self.tokens.rename_scope_fill);
                scope_ui.visuals_mut().widgets.hovered.weak_bg_fill = color(self.tokens.rename_scope_fill);
                let scope_name = match self.rename_ui.options.scope {
                    codisseu_core::RenameSearchScope::Project => "Project Files",
                    codisseu_core::RenameSearchScope::CurrentFile => "Current File",
                };
                let scope_response = egui::ComboBox::from_id_salt("codisseu.rename.scope")
                    .width(scope_selector.width())
                    .selected_text(RichText::new(scope_name).color(Color32::TRANSPARENT))
                    .icon(|ui, rect, visuals, _| name_input::chevron(ui,rect.center()+egui::vec2(2.,0.),visuals.fg_stroke.color))
                    .show_ui(&mut scope_ui, |ui| {
                        ui.selectable_value(&mut self.rename_ui.options.scope,
                            codisseu_core::RenameSearchScope::Project, "Project Files");
                        ui.selectable_value(&mut self.rename_ui.options.scope,
                            codisseu_core::RenameSearchScope::CurrentFile, "Current File");
                    }).response.on_hover_text("Escopo das ocorrências textuais opcionais. Referências semânticas são verificadas em todo o projeto.");
                source::paint_label(&mut self.rename_ui.labels, &mut scope_ui, scope_name,
                    scope_selector.left_center() + egui::vec2(10., 0.),
                    egui::Rect::from_min_max(scope_selector.min+egui::vec2(1.,1.), scope_selector.max-egui::vec2(25., 1.)),
                    color(self.tokens.rename_scope_fill), &self.tokens);
                let _ = scope_response;
                let ellipsis_rect = egui::Rect::from_min_max(scope_value.right_top()-egui::vec2(28.,0.),scope_value.max);
                let ellipsis = name_input::button(ui, &mut self.rename_ui.name_input,
                    &self.tokens, "...", ellipsis_rect, false, !rust);
                ellipsis.widget_info(||egui::WidgetInfo::labeled(egui::accesskit::Role::Button,true,"Escolher escopo de Rename"));
                egui::Popup::menu(&ellipsis).width(230.).show(|ui| {
                    ui.selectable_value(&mut self.rename_ui.options.scope,codisseu_core::RenameSearchScope::Project,"Project Files");
                    ui.selectable_value(&mut self.rename_ui.options.scope,codisseu_core::RenameSearchScope::CurrentFile,"Current File");
                    ui.separator();
                    ui.label("O escopo limita apenas comentários, strings e ocorrências em texto. Referências semânticas são verificadas em todo o projeto.");
                });
                if let Some(error) = self.rename_ui.input_error.as_ref().or(self.service.rename.error.as_ref()) {
                    crate::status_messages::status_message(ui, error);
                }
                let enabled = self.service.rename.is_prepared()
                    && !self.service.rename.is_pending()
                    && !self.rename_ui.name.trim().is_empty();
                let help_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.left() + 15., rect.bottom() - 36.), egui::vec2(22.,22.));
                let help = ui.put(help_rect, egui::Button::new("?").frame(false))
                    .on_hover_text("Ajuda de Rename");
                ui.painter().circle_stroke(help_rect.center(), 10.5,
                    egui::Stroke::new(1., color(self.tokens.control_border)));
                egui::Popup::menu(&help).width(330.).show(|ui| {
                    ui.label("Rename verifica declarações, referências e conflitos antes de aplicar.");
                    ui.label("Preview permite revisar as alterações. Refactor aplica após a verificação; contratos relacionados exigem confirmação.");
                    ui.label("As opções de texto incluem comentários, strings simples e arquivos .txt/.md. O escopo limita essas ocorrências opcionais.");
                    ui.label("Shift+F6 abre a edição no local; novamente abre este diálogo. Escape cancela. As alterações de texto só são gravadas com Save.");
                });
                let cancel_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.right() - 87., rect.bottom() - 39.),
                    egui::vec2(72., 28.),
                );
                let preview_rect = egui::Rect::from_min_size(
                    cancel_rect.min - egui::vec2(90., 0.),
                    egui::vec2(78., 28.),
                );
                cancel = name_input::button(
                    ui,
                    &mut self.rename_ui.name_input,
                    &self.tokens,
                    "Cancel",
                    cancel_rect,
                    false,
                    true,
                )
                .clicked();
                preview = name_input::button(
                    ui,
                    &mut self.rename_ui.name_input,
                    &self.tokens,
                    "Preview",
                    preview_rect,
                    false,
                    enabled,
                )
                .clicked();
                refactor = name_input::button(
                    ui, &mut self.rename_ui.name_input, &self.tokens, "Refactor",
                    egui::Rect::from_min_size(preview_rect.min - egui::vec2(92., 0.), egui::vec2(80., 28.)),
                    true, enabled,
                ).clicked() || (enter && enabled);
            });
        if cancel || ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.cancel_rename();
        } else if preview || refactor {
            self.start_rename_preview(refactor);
        }
    }
    fn start_rename_preview(&mut self, apply: bool) {
        self.rename_ui.apply_intent = None;
        self.rename_ui.input_error = None;
        let validation = if self.service.rename.is_file_rename() {
            codisseu_core::validate_file_rename_name(&self.rename_ui.name)
        } else if self.service.active_document().is_some_and(|d| {
            codisseu_core::Language::for_path(&d.path) == Some(codisseu_core::Language::Rust)
        }) {
            codisseu_core::validate_rust_rename_candidate(&self.rename_ui.name)
        } else if self.service.active_document().is_some_and(|d| {
            codisseu_core::Language::for_path(&d.path) == Some(codisseu_core::Language::Python)
        }) {
            codisseu_core::validate_python_rename_candidate(&self.rename_ui.name)
        } else {
            codisseu_core::validate_rename_candidate(&self.rename_ui.name)
        };
        if let Err(error) = validation {
            self.rename_ui.input_error = Some(error.to_owned());
            return;
        }
        let options = self.rename_ui.options;
        if let Err(error) = self.service.dispatch(Command::SetRenameOptions { options }) {
            self.rename_ui.input_error = Some(error.clone());
            self.service.last_error = Some(error);
            return;
        }
        let name = self.rename_ui.name.clone();
        if let Err(error) = self.service.dispatch(Command::PreviewRename {
            new_name: name.clone(),
        }) {
            self.rename_ui.input_error = Some(error.clone());
            self.service.last_error = Some(error);
            return;
        }
        if apply {
            self.rename_ui.apply_intent = self
                .service
                .rename
                .context_token()
                .map(|token| (token, name));
        }
        self.rename_ui.dialog = false;
        self.rename_ui.inline = None;
        self.rename_ui.preview_target = None;
        self.rename_ui.focus_results = true;
        self.tool_tab = ToolTab::Rename;
        self.show_activity = true;
    }
    pub(super) fn rename_panel(&mut self, ui: &mut egui::Ui) {
        let pending = self.service.rename.is_pending();
        header(ui, self.tokens.tool_header_height - 1., |ui| {
            let header = ui.max_rect();
            for (label, x, width) in [("Find", 14., 35.), ("Refactoring Preview", 69., 136.)] {
                source::accessible_label(
                    &mut self.rename_ui.labels,
                    ui,
                    label,
                    egui::Rect::from_center_size(
                        egui::pos2(header.left() + x + width / 2., header.center().y),
                        egui::vec2(width, 17.),
                    ),
                    &self.tokens,
                );
            }
            ui.painter().hline(
                header.left() + 57. ..=header.left() + 223.,
                header.bottom() - 1.,
                egui::Stroke::new(3., color(self.tokens.rename_checkbox_border)),
            );
            let close_rect = egui::Rect::from_center_size(
                egui::pos2(header.left() + 209., header.center().y),
                egui::vec2(20., 20.),
            );
            let mut close = ui.new_child(egui::UiBuilder::new().max_rect(close_rect));
            if icon_button(
                &mut close,
                Icon::Close,
                "Fechar preview de rename",
                false,
                20.,
            )
            .clicked()
            {
                self.cancel_rename();
            }
            let minimize = ui.put(
                egui::Rect::from_center_size(
                    egui::pos2(header.right() - 21., header.center().y),
                    egui::vec2(24., 24.),
                ),
                egui::Button::new("−").frame(false),
            );
            minimize.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::accesskit::Role::Button,
                    true,
                    "Minimizar preview de Rename",
                )
            });
            if minimize.clicked() {
                self.show_activity = false;
                self.focus_editor = true;
            }
        });
        if let Some(error) = &self.service.rename.error {
            crate::status_messages::status_message(ui, error);
        }
        if pending {
            ui.label("Validando alterações…");
        }
        let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
        let footer = egui::Rect::from_min_max(
            egui::pos2(
                rect.left(),
                (rect.bottom() - self.tokens.rename_actionbar_height).max(rect.top()),
            ),
            rect.max,
        );
        let body = egui::Rect::from_min_max(rect.min, footer.right_top());
        let split_x = body.left()
            + body.width()
                * self
                    .rename_ui
                    .split_ratio
                    .unwrap_or(self.tokens.rename_preview_ratio);
        let splitter = egui::Rect::from_min_max(
            egui::pos2(split_x - self.tokens.references_splitter_width, body.top()),
            egui::pos2(split_x, body.bottom()),
        );
        let response = ui.interact(
            splitter,
            egui::Id::unique("codisseu.rename.splitter"),
            egui::Sense::drag(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::accesskit::Role::Splitter,
                true,
                "Redimensionar preview de rename",
            )
        });
        if response.dragged()
            && let Some(p) = response.interact_pointer_pos()
        {
            self.rename_ui.split_ratio =
                Some(((p.x - body.left()) / body.width()).clamp(0.25, 0.8));
        }
        response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        ui.painter()
            .rect_filled(splitter, 0., color(self.tokens.border));
        let left = egui::Rect::from_min_max(body.min, splitter.left_bottom());
        let right = egui::Rect::from_min_max(splitter.right_top(), body.max);
        let toolbar =
            egui::Rect::from_min_max(left.min, egui::pos2(left.left() + 40., left.bottom()));
        let mut toolbar_ui = ui.new_child(egui::UiBuilder::new().max_rect(toolbar));
        toolbar_ui.set_clip_rect(toolbar.intersect(ui.clip_rect()));
        for (kind, y, label) in [
            (0, 30., "Ocorrência anterior"),
            (1, 56., "Próxima ocorrência"),
            (
                2,
                108.,
                if self.rename_ui.show_after {
                    "Preview"
                } else {
                    "Após renomear"
                },
            ),
            (3, 134., "Expandir todos os resultados"),
            (4, 160., "Recolher todos os resultados"),
        ] {
            let r =
                egui::Rect::from_min_size(toolbar.min + egui::vec2(7., y), egui::vec2(26., 26.));
            if tree::control(&mut toolbar_ui, r, label, kind, &self.tokens).clicked() {
                match kind {
                    0 | 1 => {
                        self.rename_ui
                            .select_neighbor(if kind == 0 { -1 } else { 1 });
                        self.rename_ui.focus_results = true;
                    }
                    2 => {
                        self.rename_ui.show_after = !self.rename_ui.show_after;
                    }
                    3 => {
                        self.rename_ui.collapsed.clear();
                    }
                    4 => {
                        self.rename_ui.collapsed.insert("all".into());
                    }
                    _ => {}
                }
            }
        }
        let tree_rect = egui::Rect::from_min_max(left.min + egui::vec2(40., 0.), left.max);
        let mut tree = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(tree_rect)
                .id_salt("codisseu.rename.tree")
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        tree.set_clip_rect(tree_rect.intersect(ui.clip_rect()));
        let token = self.service.rename.context_token();
        let count: usize = self
            .service
            .rename
            .files
            .iter()
            .map(|f| f.edits.len())
            .sum();
        let model = tree::Model::new(
            &self.service.rename.files,
            self.service.workspace.as_deref(),
            self.service.rename.symbol_kind(),
            self.service
                .rename
                .placeholder
                .as_deref()
                .unwrap_or("symbol"),
            &self.service.rename.new_name,
            self.service.rename.is_file_rename(),
        );
        if self.rename_ui.preview_target.is_none() {
            self.rename_ui.selected = model.first_reference.unwrap_or(0);
        }
        self.rename_ui.visible.clear();
        egui::ScrollArea::both()
            .id_salt("codisseu.rename.scroll")
            .codisseu()
            .show(&mut tree, |ui| {
                ui.set_min_width(ui.available_width());
                ui.add_space(5.);
                for resource in self.service.rename.resource_moves() {
                    let root = self
                        .service
                        .workspace
                        .as_deref()
                        .unwrap_or(std::path::Path::new(""));
                    ui.label(format!(
                        "    Renomear arquivo: {} → {}",
                        resource
                            .source
                            .strip_prefix(root)
                            .unwrap_or(&resource.source)
                            .display(),
                        resource
                            .target
                            .strip_prefix(root)
                            .unwrap_or(&resource.target)
                            .display()
                    ));
                }
                let mut position = 0;
                while position < model.rows.len() {
                    match &model.rows[position] {
                        tree::Row::Group {
                            key,
                            label,
                            indent,
                            icon,
                            end,
                        } => {
                            if !tree::group(
                                ui,
                                &mut self.rename_ui,
                                &self.tokens,
                                key,
                                label,
                                *indent,
                                icon.as_deref(),
                            ) {
                                position = *end;
                                continue;
                            }
                        }
                        tree::Row::Edit {
                            index,
                            file: file_index,
                            edit: edit_index,
                            indent,
                        } => {
                            let index = *index;
                            let file = &self.service.rename.files[*file_index];
                            let edit = &file.edits[*edit_index];
                            let path = file
                                .path
                                .strip_prefix(
                                    self.service
                                        .workspace
                                        .as_deref()
                                        .unwrap_or(std::path::Path::new("")),
                                )
                                .unwrap_or(&file.path);
                            let line = file
                                .before
                                .lines()
                                .nth(edit.start_line as usize)
                                .unwrap_or("");
                            let snippet: String = line
                                .trim()
                                .chars()
                                .take(512)
                                .map(|c| if c.is_control() { ' ' } else { c })
                                .collect();
                            let replacement: String = edit
                                .new_text
                                .chars()
                                .take(128)
                                .map(|c| if c.is_control() { ' ' } else { c })
                                .collect();
                            let label = format!(
                                "{}:{}:{}  {} → {}",
                                path.display(),
                                edit.start_line + 1,
                                edit.start_character + 1,
                                snippet,
                                replacement
                            );
                            self.rename_ui.visible.push(index);
                            let extension =
                                file.path.extension().and_then(|s| s.to_str()).unwrap_or("");
                            let galley = ui.painter().layout_no_wrap(
                                snippet.clone(),
                                FontId::proportional(self.tokens.ui_font_size),
                                color(self.tokens.text),
                            );
                            let (row, _) = ui.allocate_exact_size(
                                egui::vec2(
                                    ui.available_width().max(*indent + 7. + galley.size().x),
                                    self.tokens.row_height,
                                ),
                                egui::Sense::hover(),
                            );
                            let response =
                                ui.interact(row, row_id(token, index), egui::Sense::click());
                            let selected = index == self.rename_ui.selected;
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::accesskit::Role::ListBoxOption,
                                    true,
                                    selected,
                                    &label,
                                )
                            });
                            if selected {
                                ui.painter().rect_filled(
                                    row.shrink2(egui::vec2(12., 0.)),
                                    self.tokens.control_corner_radius,
                                    color(if response.has_focus() {
                                        self.tokens.selection
                                    } else {
                                        self.tokens.references_inactive_selection
                                    }),
                                );
                                if self.rename_ui.focus_results {
                                    if !response.has_focus() {
                                        response.request_focus();
                                    }
                                    ui.scroll_to_rect(
                                        egui::Rect::from_min_max(
                                            egui::pos2(ui.clip_rect().left(), row.top()),
                                            egui::pos2(ui.clip_rect().right(), row.bottom()),
                                        ),
                                        None,
                                    );
                                    self.rename_ui.focus_results = false;
                                }
                            }
                            if response.has_focus()
                                || response.clicked()
                                || (selected && self.rename_ui.focus_results)
                            {
                                ui.memory_mut(|m| {
                                    m.set_focus_lock_filter(
                                        response.id,
                                        egui::EventFilter {
                                            vertical_arrows: true,
                                            horizontal_arrows: true,
                                            ..Default::default()
                                        },
                                    )
                                });
                            }
                            if ui.is_rect_visible(row) {
                                // Highlight only the visible rows; collapsed/offscreen
                                // proposals never invoke syntax shaping on the UI thread.
                                let shaped = source::SourceLine::new(
                                    ui,
                                    &snippet,
                                    extension,
                                    &self.tokens,
                                    self.service.editor_preferences().ligatures,
                                );
                                let painter =
                                    ui.painter().with_clip_rect(row.intersect(ui.clip_rect()));
                                painter.text(
                                    row.left_center() + egui::vec2(*indent - 13., 0.),
                                    egui::Align2::LEFT_CENTER,
                                    (edit.start_line + 1).to_string(),
                                    FontId::proportional(self.tokens.ui_font_size),
                                    color(self.tokens.muted),
                                );
                                let background = color(if selected {
                                    if response.has_focus() {
                                        self.tokens.selection
                                    } else {
                                        self.tokens.references_inactive_selection
                                    }
                                } else {
                                    self.tokens.panel
                                });
                                tree::snippet(
                                    ui,
                                    &mut self.rename_ui.snippets,
                                    &shaped.galley,
                                    row,
                                    *indent,
                                    background,
                                    &self.tokens,
                                );
                            }
                            if response.clicked() {
                                self.rename_ui.selected = index;
                                self.rename_ui.candidate_selected = None;
                                if !response.has_focus() {
                                    response.request_focus();
                                }
                            }
                            response.on_hover_text(label);
                        }
                    }
                    position += 1;
                }
                let candidates = tree::candidate_indices(&self.service.rename);
                if !candidates.is_empty()
                    && tree::group(
                        ui,
                        &mut self.rename_ui,
                        &self.tokens,
                        "unverified",
                        &format!(
                            "Unverified candidates · not applicable ({})",
                            candidates.len()
                        ),
                        34.,
                        Some("@heading"),
                    )
                {
                    for index in candidates {
                        let issue = &self.service.rename.validation.as_ref().unwrap().issues[index];
                        let range = issue.range.as_ref().unwrap();
                        let path = issue
                            .path
                            .strip_prefix(
                                self.service
                                    .workspace
                                    .as_deref()
                                    .unwrap_or(std::path::Path::new("")),
                            )
                            .unwrap_or(&issue.path);
                        let label = format!(
                            "{}:{} · {}",
                            path.display(),
                            range.start_line + 1,
                            issue.message
                        );
                        let response = ui.selectable_label(
                            self.rename_ui.candidate_selected == Some(index),
                            &label,
                        );
                        if self.rename_ui.focus_candidates {
                            response.scroll_to_me(None);
                            self.rename_ui.focus_candidates = false;
                        }
                        if response.clicked() {
                            self.rename_ui.candidate_selected = Some(index);
                        }
                        if let Some(snapshot) = self
                            .service
                            .rename
                            .conflict_previews
                            .iter()
                            .find(|p| p.path == issue.path)
                        {
                            let snippet: String = snapshot
                                .before
                                .lines()
                                .nth(range.start_line as usize)
                                .unwrap_or("")
                                .chars()
                                .take(512)
                                .collect();
                            ui.label(
                                RichText::new(snippet)
                                    .monospace()
                                    .color(color(self.tokens.muted)),
                            );
                        }
                    }
                }
            });
        if let Some(direction) = self.rename_ui.move_after_click.take()
            && count > 0
        {
            self.rename_ui.select_neighbor(direction);
            self.rename_ui.focus_results = true;
            ui.ctx().request_repaint();
        }
        let mut selected = self.rename_ui.selected;
        let item = self.service.rename.files.iter().find_map(|file| {
            if selected < file.edits.len() {
                Some((file, selected))
            } else {
                selected -= file.edits.len();
                None
            }
        });
        ui.painter()
            .rect_filled(right, 0., color(self.tokens.editor));
        let mut code = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(right)
                .id_salt("codisseu.rename.code")
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        code.set_clip_rect(right.intersect(ui.clip_rect()));
        let clip = right;
        code.set_max_height(clip.height());
        if let Some(candidate) = self.rename_ui.candidate_selected {
            self.rename_candidate_code(&mut code, right, candidate);
        } else if let Some((file, index)) = item {
            let target = (token, self.rename_ui.selected, self.rename_ui.show_after);
            let reveal = self.rename_ui.preview_target != Some(target);
            self.rename_ui.preview_target = Some(target);
            let extension = file.path.extension().and_then(|p| p.to_str()).unwrap_or("");
            let text = if self.rename_ui.show_after {
                file.after.as_ref()
            } else {
                file.before.as_ref()
            };
            egui::ScrollArea::both()
                .id_salt("codisseu.rename.preview.scroll")
                .max_height(clip.height())
                .codisseu()
                .show(&mut code, |ui| {
                    let (lines, truncated) = preview_lines(text, file.edits[index].start_line);
                    if truncated {
                        ui.label("Preview limitado: até 14 linhas / 8 KiB");
                    }
                    for (number, line) in lines {
                        let shaped = source::SourceLine::new(
                            ui,
                            line,
                            extension,
                            &self.tokens,
                            self.service.editor_preferences().ligatures,
                        );
                        let galley = &shaped.galley;
                        let (row, _) = ui.allocate_exact_size(
                            egui::vec2(
                                ui.available_width().max(37. + galley.size().x),
                                self.tokens.editor_line_height,
                            ),
                            egui::Sense::hover(),
                        );
                        if number == file.edits[index].start_line as usize {
                            ui.painter()
                                .rect_filled(row, 0., color(self.tokens.caret_row));
                        }
                        let gutter = egui::Rect::from_min_max(
                            egui::pos2(clip.left(), row.top()),
                            egui::pos2(clip.left() + 37., row.bottom()),
                        );
                        let gutter_painter =
                            ui.painter().with_clip_rect(clip.intersect(ui.clip_rect()));
                        gutter_painter.rect_filled(
                            gutter,
                            0.,
                            color(if number == file.edits[index].start_line as usize {
                                self.tokens.caret_row
                            } else {
                                self.tokens.editor
                            }),
                        );
                        gutter_painter.text(
                            egui::pos2(clip.left() + 24., row.center().y),
                            egui::Align2::RIGHT_CENTER,
                            (number + 1).to_string(),
                            FontId::monospace(self.tokens.editor_font_size),
                            color(self.tokens.muted),
                        );
                        let painter = ui.painter().with_clip_rect(
                            egui::Rect::from_min_max(clip.min + egui::vec2(37., 0.), clip.max)
                                .intersect(ui.clip_rect()),
                        );
                        let edit = &file.edits[index];
                        if !self.rename_ui.show_after
                            && let Some(columns) = conflicts::conflict_symbol_columns(
                                line,
                                number,
                                Some((
                                    edit.start_line,
                                    edit.start_character,
                                    edit.end_line,
                                    edit.end_character,
                                )),
                            )
                        {
                            let start = shaped.x(columns.start).unwrap_or(0.);
                            let end = shaped.x(columns.end).unwrap_or(start);
                            let symbol_rect = egui::Rect::from_min_size(
                                row.min + egui::vec2(37. + start, 0.),
                                egui::vec2((end - start).max(0.), self.tokens.editor_line_height),
                            );
                            if reveal {
                                ui.scroll_to_rect(symbol_rect.expand2(egui::vec2(40., 0.)), None);
                            }
                            painter.rect_filled(
                                symbol_rect,
                                0.,
                                color(self.tokens.rename_conflict_name_background),
                            );
                        }
                        painter.galley(
                            row.left_top() + egui::vec2(37., 0.),
                            galley.clone(),
                            color(self.tokens.text),
                        );
                    }
                });
        }
        let mut actions = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(footer.shrink2(egui::vec2(49., 3.)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        self.tokens.form_style(&mut actions);
        let enabled = !pending
            && (!self.service.rename.files.is_empty()
                || !self.service.rename.resource_moves().is_empty())
            && self.service.rename.error.is_none()
            && self.service.rename.can_apply();
        let refactor = name_input::button(
            &mut actions,
            &mut self.rename_ui.name_input,
            &self.tokens,
            "Refactor",
            egui::Rect::from_min_size(footer.min + egui::vec2(49., 3.), egui::vec2(80., 28.)),
            false,
            enabled,
        );
        refactor.widget_info(|| {
            egui::WidgetInfo::labeled(egui::accesskit::Role::Button, enabled, "Refatorar")
        });
        if refactor.clicked() {
            self.send(Command::ApplyRename);
            self.focus_editor = true;
        }
        let cancel = name_input::button(
            &mut actions,
            &mut self.rename_ui.name_input,
            &self.tokens,
            "Cancel",
            egui::Rect::from_min_size(footer.min + egui::vec2(141., 3.), egui::vec2(72., 28.)),
            false,
            true,
        );
        cancel.widget_info(|| {
            egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, "Cancelar")
        });
        if cancel.clicked() {
            self.cancel_rename();
        }
        let has_inheritors = !self.service.rename.inheritor_choices().is_empty();
        if has_inheritors
            && name_input::button(
                &mut actions,
                &mut self.rename_ui.name_input,
                &self.tokens,
                "Alterar seleção",
                egui::Rect::from_min_size(footer.min + egui::vec2(225., 3.), egui::vec2(116., 28.)),
                false,
                !pending,
            )
            .clicked()
        {
            self.edit_rename_inheritors();
        }
        actions.add_space(if has_inheritors { 304. } else { 176. });
        let verified_pairs = self
            .service
            .rename
            .validation
            .as_ref()
            .map_or(0, |v| v.checked_override_pairs);
        let verification = if self.service.rename.can_apply() && verified_pairs > 0 {
            format!("Hierarquia conhecida: {verified_pairs} par(es) de contratos verificados.")
        } else if self.service.rename.can_apply() {
            match self.service.rename.validation.as_ref().map(|v| v.scope) {
                Some(codisseu_core::ValidationScope::KnownHierarchyMemberNamespaces) => {
                    "Verificação local: hierarquia de classes conhecida."
                }
                Some(codisseu_core::ValidationScope::KnownReceiverBindings) => {
                    "Verificação local: chamadas e receptores conhecidos."
                }
                Some(codisseu_core::ValidationScope::LexicalBindings) => {
                    "Verificação local: declarações e referências locais."
                }
                Some(codisseu_core::ValidationScope::KnownGlobalBindings) => {
                    "Verificação local: símbolos e imports conhecidos."
                }
                Some(codisseu_core::ValidationScope::KnownPropertyBindings) => {
                    "Verificação local: propriedades e receptores conhecidos."
                }
                Some(codisseu_core::ValidationScope::ResourceReferences) => {
                    "Verificação local: referências relativas de arquivo."
                }
                Some(codisseu_core::ValidationScope::ResourceOperationOnly) => {
                    "Somente arquivo: referências permanecerão inalteradas."
                }
                Some(codisseu_core::ValidationScope::PythonAnalyzerBindings) => {
                    "Basedpyright e validação estrutural Python dos arquivos locais."
                }
                Some(codisseu_core::ValidationScope::RustAnalyzerBindings) => {
                    "Rust-analyzer e validação estrutural Rust dos arquivos locais."
                }
                Some(codisseu_core::ValidationScope::CompilerBindings) => {
                    "Verificação local: vínculos e tipos do compilador TypeScript."
                }
                _ => "Verificação local: membros da mesma classe.",
            }
            .to_owned()
        } else if self.service.rename.semantic_blocked() {
            "Aplicação bloqueada pela verificação de rename.".into()
        } else if pending {
            "Verificando conflitos de rename…".into()
        } else {
            "Nenhum rename preparado.".into()
        };
        let note = self.service.rename.validation.as_ref().map_or(
            "Aguarde a proposta e a verificação do contexto coletado.",
            |v| v.note.as_str(),
        );
        actions
            .label(RichText::new(verification).color(color(self.tokens.muted)))
            .on_hover_text(note);
        if self.service.rename.semantic_blocked() && actions.button("Ver conflitos").clicked() {
            self.rename_ui.conflict_open = true;
            self.rename_ui.focus_conflict = true;
        }
    }
}

// Conflict trees retain their existing presentation and grouping contract.
fn rename_group(ui: &mut egui::Ui, indent: f32, label: &str, tokens: &Tokens) {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        FontId::proportional(tokens.ui_font_size),
        color(tokens.text),
    );
    let (row, _) = ui.allocate_exact_size(
        egui::vec2(
            ui.available_width().max(indent + galley.size().x + 12.),
            tokens.row_height,
        ),
        egui::Sense::hover(),
    );
    ui.painter()
        .with_clip_rect(row.intersect(ui.clip_rect()))
        .galley(
            row.left_center() + egui::vec2(indent, -galley.size().y / 2.),
            galley,
            color(tokens.text),
        );
}

// Keep rendering bounded even for a one-megabyte line or an enormous file.
fn preview_lines(text: &str, selected_line: u32) -> (Vec<(usize, &str)>, bool) {
    let mut remaining = 8 * 1024;
    let mut lines = Vec::new();
    let mut truncated = false;
    for (number, line) in text
        .split('\n')
        .enumerate()
        .skip(selected_line.saturating_sub(5) as usize)
        .take(14)
    {
        let mut end = line.len().min(remaining);
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        truncated |= end != line.len();
        lines.push((number, &line[..end]));
        remaining -= end;
        if remaining == 0 {
            break;
        }
    }
    (lines, truncated)
}
#[cfg(test)]
mod tests {
    #[test]
    fn rename_preview_limits_large_unicode_lines_without_splitting_utf8() {
        let text = format!("first\n{}\nlast", "🌧é".repeat(200_000));
        let (lines, truncated) = super::preview_lines(&text, 1);
        assert!(truncated);
        assert!(lines.iter().map(|(_, s)| s.len()).sum::<usize>() <= 8192);
        assert_eq!(lines[0], (0, "first"));
        let text = (0..100)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let (lines, truncated) = super::preview_lines(&text, 50);
        assert_eq!(lines.len(), 14);
        assert_eq!(lines[0], (45, "45"));
        assert!(!truncated);
    }
}

fn rename_title(kind: codisseu_core::RenameSymbolKind, php: bool, name: Option<&str>) -> String {
    use codisseu_core::RenameSymbolKind;
    let kind = match kind {
        RenameSymbolKind::Class => {
            if php {
                "Class"
            } else {
                "class"
            }
        }
        RenameSymbolKind::Function => {
            if php {
                "Function"
            } else {
                "function"
            }
        }
        RenameSymbolKind::Method => {
            if php {
                "Method"
            } else {
                "method"
            }
        }
        RenameSymbolKind::Property => {
            if php {
                "Field"
            } else {
                "field"
            }
        }
        RenameSymbolKind::Parameter => {
            if php {
                "Parameter"
            } else {
                "parameter"
            }
        }
        RenameSymbolKind::Variable => {
            if php {
                "Variable"
            } else {
                "variable"
            }
        }
        RenameSymbolKind::Imported | RenameSymbolKind::Unknown => "symbol",
    };
    format!("Rename {kind} '{}' and its usages to:", name.unwrap_or(""))
}

#[cfg(test)]
mod flow_tests {
    use super::*;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };
    use std::time::{Duration, Instant};
    fn wait(ui: &mut Harness<'_, CodisseuApp>, condition: impl Fn(&CodisseuApp) -> bool) {
        let end = Instant::now() + Duration::from_secs(15);
        loop {
            ui.step();
            if condition(ui.state()) {
                ui.step();
                return;
            }
            assert!(
                Instant::now() < end,
                "{} {:?}",
                ui.state().service.status,
                ui.state().service.rename.error
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn file_rename_waits_for_requested_document_and_preserves_dirty_source() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let first = root.join("first.php");
        let target = root.join("Tools.php");
        std::fs::write(&first, "<?php echo 1;\n").unwrap();
        let original = "<?php function weather() { return 1; }\n";
        std::fs::write(&target, original).unwrap();
        let mut service = AppService::new(root.join("session.json"));
        service.dispatch(Command::OpenWorkspace(root)).unwrap();
        let mut ui = Harness::builder().with_size([1280., 800.]).build_ui_state(
            |ui, app: &mut CodisseuApp| app.show(ui),
            CodisseuApp::new(service),
        );
        wait(&mut ui, |a| !a.service.is_busy());
        ui.state_mut()
            .service
            .dispatch(Command::OpenFile(first))
            .unwrap();
        wait(&mut ui, |a| !a.service.is_busy());
        ui.state_mut().request_file_rename(target.clone());
        assert!(!ui.state().rename_ui.dialog);
        assert!(ui.state().rename_ui.pending_path.is_some());
        wait(&mut ui, |a| a.service.rename.is_file_rename());
        assert_eq!(ui.state().service.active_document().unwrap().path, target);
        assert_eq!(ui.state().rename_ui.name, "Tools.php");
        let field = ui.get_by_label("Renomear arquivo e suas referências para:");
        assert!(field.is_focused());
        // Initial replacement changes the stem, leaving the extension selected out.
        ui.event(egui::Event::Text("Weather".into()));
        ui.step();
        assert_eq!(ui.state().rename_ui.name, "Weather.php");
        assert_eq!(
            ui.state().service.active_document().unwrap().text(),
            original
        );
        ui.key_press(Key::Escape);
        ui.step();
        assert!(!ui.state().rename_ui.dialog);
        ui.event(egui::Event::Text("// dirty\n".into()));
        ui.step();
        let dirty = ui
            .state()
            .service
            .active_document()
            .unwrap()
            .text()
            .to_owned();
        assert_ne!(dirty, original);
        ui.state_mut().request_file_rename(target.clone());
        wait(&mut ui, |a| a.service.rename.is_file_rename());
        assert_eq!(ui.state().service.active_document().unwrap().text(), dirty);
        ui.get_by_label("Renomear arquivo e suas referências para:")
            .click();
        ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
        ui.event(egui::Event::Text("../escape.php".into()));
        ui.step();
        ui.get_by_label("Preview").click();
        ui.step();
        assert!(ui.state().rename_ui.input_error.is_some());
        assert_eq!(ui.state().service.active_document().unwrap().text(), dirty);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), original);
        ui.get_by_label("Renomear arquivo e suas referências para:")
            .click();
        ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
        ui.event(egui::Event::Text("Weather.php".into()));
        ui.step();
        ui.get_by_label("Search for references").click();
        ui.step();
        ui.get_by_label("Preview").click();
        wait(&mut ui, |a| a.service.rename.can_apply());
        assert!(ui.state().service.rename.files.is_empty());
        assert_eq!(ui.state().service.rename.resource_moves().len(), 1);
        assert_eq!(
            ui.state().service.rename.validation.as_ref().unwrap().scope,
            codisseu_core::ValidationScope::ResourceOperationOnly
        );
        assert!(!ui.get_by_label("Refatorar").accesskit_node().is_disabled());
        ui.get_by_label("Refatorar").click();
        wait(&mut ui, |a| !a.service.is_busy());
        assert_eq!(ui.state().service.active_document().unwrap().text(), dirty);
        assert!(ui.state().service.active_document().unwrap().is_dirty());
        assert_eq!(
            ui.state().service.active_document().unwrap().path,
            target.with_file_name("Weather.php")
        );
        assert!(!target.exists());
        assert_eq!(
            std::fs::read_to_string(target.with_file_name("Weather.php")).unwrap(),
            original
        );
    }
    #[test]
    fn inline_cancel_more_options_and_refactor_require_verified_current_proposal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("weather.php");
        let original = "<?php function rainfall(): int { return 1; } function forecast(): int { return 2; } echo rainfall(); // rainfall\n";
        std::fs::write(&path, original).unwrap();
        let mut service = AppService::new(root.join("session.json"));
        service.dispatch(Command::OpenWorkspace(root)).unwrap();
        let mut ui = Harness::builder().with_size([1280., 800.]).build_ui_state(
            |ui, app: &mut CodisseuApp| app.show(ui),
            CodisseuApp::new(service),
        );
        wait(&mut ui, |a| !a.service.is_busy());
        ui.state_mut()
            .service
            .dispatch(Command::OpenFile(path.clone()))
            .unwrap();
        wait(&mut ui, |a| !a.service.is_busy());
        let id = ui.state().service.active.unwrap();
        ui.state_mut().caret = Some((id, original.find("rainfall").unwrap() + 2));
        ui.state_mut().caret_screen = Some(egui::pos2(430., 200.));
        ui.state_mut().request_rename_inline();
        wait(&mut ui, |a| a.service.rename.is_prepared());
        ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
        ui.event(egui::Event::Text("cancelled".into()));
        ui.step();
        assert_eq!(
            ui.state().service.active_document().unwrap().text(),
            original
        );
        assert!(ui.state().rename_ui.inline.is_some());
        let document = ui.state().service.active_document().unwrap();
        let version = document.version;
        let snapshot = document.snapshot();
        let (ghost, _, range) = ui
            .state_mut()
            .rename_inline_snapshot(id, version, &snapshot)
            .unwrap();
        assert_eq!(ghost.slice(range).to_string(), "cancelled");
        assert!(ghost.to_string().contains("function cancelled     ():"));
        assert!(ghost.to_string().contains("echo rainfall();"));
        ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
        ui.event(egui::Event::Text("café_Δ".into()));
        ui.step();
        assert_eq!(ui.state().rename_ui.name, "café_Δ");
        assert_eq!(
            ui.state().service.active_document().unwrap().version,
            version
        );
        assert_eq!(
            ui.state().service.active_document().unwrap().text(),
            original
        );
        ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
        ui.event(egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "你好".into(),
            active_range_chars: Some(0..2),
        }));
        ui.step();
        assert_eq!(
            ui.state().service.active_document().unwrap().text(),
            original
        );
        ui.event(egui::Event::Ime(egui::ImeEvent::Commit("你好".into())));
        ui.step();
        assert_eq!(ui.state().rename_ui.name, "你好");
        ui.key_press_modifiers(Modifiers::ALT | Modifiers::SHIFT, Key::O);
        ui.step();
        ui.get_by_label("Comments and strings").click();
        ui.step();
        assert!(ui.state().rename_ui.options.search_comments_strings);
        ui.key_press(Key::Escape);
        ui.step();
        assert!(ui.state().rename_ui.inline.is_some());
        ui.key_press(Key::Escape);
        ui.step();
        assert!(!ui.state().service.rename.is_open());
        assert_eq!(
            ui.state().service.active_document().unwrap().text(),
            original
        );
        ui.state_mut().caret = Some((id, original.find("rainfall").unwrap() + 2));
        ui.state_mut().request_rename_inline();
        wait(&mut ui, |a| a.service.rename.is_prepared());
        ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
        ui.event(egui::Event::Text("precipitation".into()));
        ui.step();
        ui.key_press_modifiers(Modifiers::SHIFT, Key::F6);
        ui.step();
        assert!(ui.state().rename_ui.inline.is_none());
        assert_eq!(ui.state().rename_ui.name, "precipitation");
        ui.get_by_label("Search in comments and strings").click();
        ui.step();
        assert!(ui.state().rename_ui.options.search_comments_strings);
        ui.get_by_label("?").click();
        ui.step();
        ui.step();
        ui.get_by_label("Rename verifica declarações, referências e conflitos antes de aplicar.");
        ui.get_by_label("?").click();
        ui.step();
        ui.step();
        ui.get_by_label("Escolher escopo de Rename").click();
        ui.step();
        ui.step();
        ui.get_by_label("Current File").click();
        ui.step();
        assert_eq!(
            ui.state().rename_ui.options.scope,
            codisseu_core::RenameSearchScope::CurrentFile
        );
        ui.get_by_label("Refactor").click();
        wait(&mut ui, |a| {
            !a.service.rename.is_open() && !a.service.is_busy()
        });
        assert_eq!(
            ui.state().service.active_document().unwrap().text(),
            original.replace("rainfall", "precipitation")
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), original);
        assert!(ui.state().rename_ui.apply_intent.is_none());
        let applied = ui
            .state()
            .service
            .active_document()
            .unwrap()
            .text()
            .to_owned();
        ui.state_mut().caret = Some((id, applied.find("precipitation").unwrap() + 2));
        ui.state_mut().request_rename();
        wait(&mut ui, |a| a.service.rename.is_prepared());
        ui.get_by_label("Nomes verificados").click();
        ui.step();
        ui.step();
        ui.get_by_role_and_label(egui::accesskit::Role::Button, "precipitation")
            .click();
        ui.step();
        assert_eq!(ui.state().rename_ui.name, "precipitation");
        ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
        ui.event(egui::Event::Text("not valid".into()));
        ui.step();
        ui.get_by_label("Refactor").click();
        ui.step();
        assert!(ui.state().rename_ui.input_error.is_some());
        assert!(ui.state().rename_ui.dialog);
        assert_eq!(
            ui.state().service.active_document().unwrap().text(),
            applied
        );
        ui.get_by_label("Renomear símbolo e seus usos para:")
            .click();
        ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
        ui.event(egui::Event::Text("forecast".into()));
        ui.step();
        ui.get_by_label("Refactor").click();
        wait(&mut ui, |a| a.service.rename.semantic_blocked());
        assert_eq!(
            ui.state().service.active_document().unwrap().text(),
            applied
        );
        assert!(ui.state().rename_ui.apply_intent.is_none());
        ui.get_by_label("Alterar nome").click();
        ui.step();
        ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
        ui.event(egui::Event::Text("corrected".into()));
        ui.step();
        ui.get_by_label("Preview").click();
        wait(&mut ui, |a| a.service.rename.can_apply());
        assert_eq!(
            ui.state().service.active_document().unwrap().text(),
            applied
        );
        assert!(
            ui.query_all_by_role(egui::accesskit::Role::ListBoxOption)
                .count()
                > 0
        );
        ui.get_by_label("Recolher todos os resultados").click();
        ui.step();
        ui.step();
        assert_eq!(
            ui.query_all_by_role(egui::accesskit::Role::ListBoxOption)
                .count(),
            0
        );
        ui.get_by_label("Expandir todos os resultados").click();
        ui.step();
        ui.step();
        assert!(
            ui.query_all_by_role(egui::accesskit::Role::ListBoxOption)
                .count()
                > 0
        );
        let selected = ui.state().rename_ui.selected;
        ui.get_by_label("Próxima ocorrência").click();
        ui.step();
        assert_ne!(ui.state().rename_ui.selected, selected);
        ui.get_by_label("Após renomear").click();
        ui.step();
        assert!(ui.state().rename_ui.show_after);
        ui.get_by_label("Cancelar").click();
        ui.step();
        assert!(!ui.state().service.rename.is_open());
        assert!(ui.state().rename_ui.apply_intent.is_none());
    }
}
